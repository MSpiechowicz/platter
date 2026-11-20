//! Updates (D3). The release's `latest.json` names the newest version, its notes and the signed AppImage
//! and Windows setup (`.github/workflows/release.yml`). The Tauri updater fetches it from the endpoint in
//! `tauri.conf.json` and checks every download against the public key there, so only a build signed with
//! the owner's key is installed.
//!
//! Three kinds of copy update themselves. An AppImage replaces itself and `restart_app` starts the new one.
//! So does a macOS app bundle (`<name>.app/Contents/MacOS/<program>`) whose folder the user can write to: the
//! updater unpacks the signed `.app.tar.gz` next to it, swaps the two bundles, and `restart_app` starts the new
//! one from the same path. A bundle in a folder that needs an administrator (the updater would ask for the
//! password in a system dialog) is told about the update and gets the release page, as a package does.
//! A copy installed by the Windows setup (NSIS, for the current user) downloads and checks the new setup
//! in `install_update` and keeps it, and `restart_app` runs it: the updater ends the app and the setup
//! starts the new version, so the installation waits for the user's click on Windows, also when the
//! download was automatic. A deb or rpm install, or a Windows copy that the setup did not install, is told
//! about the update and gets the release page: the updater would install the packages through `sudo` with
//! a password typed into a dialog, which Platter does not do (the package manager updates them).
//!
//! The web view is granted no updater permission. It calls these three commands, and the update that a
//! check found is kept here, so the web view never names what is downloaded.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::Mutex;

use platter_core::Os;
use platter_platform::environment::current_os;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::cleanup::CleanupRegistry;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// The most lines of release notes shown, and the most characters of each.
const MAX_NOTES: usize = 12;
const MAX_NOTE_CHARS: usize = 200;

/// The file the NSIS setup writes next to the program, and the one the updater needs to run again.
const NSIS_UNINSTALLER: &str = "uninstall.exe";

/// What the app keeps between the commands.
#[derive(Default)]
pub struct PendingUpdate {
    /// The update the last check found, until it is installed or a new check replaces it.
    found: Mutex<Option<Update>>,
    /// An update that was downloaded and checked but not installed (Windows: the setup waits for the click
    /// on restart). A later check does not drop it.
    downloaded: Mutex<Option<Downloaded>>,
}

/// A setup that was downloaded and whose signature was checked.
struct Downloaded {
    update: Update,
    bytes: Vec<u8>,
}

impl PendingUpdate {
    fn found(&self) -> std::sync::MutexGuard<'_, Option<Update>> {
        self.found
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn downloaded(&self) -> std::sync::MutexGuard<'_, Option<Downloaded>> {
        self.downloaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// How a copy of Platter takes an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallKind {
    /// An AppImage: replaced in place, started again by `restart_app`.
    AppImage,
    /// A macOS app bundle in a folder the user can write to: replaced in place by the updater, started again
    /// by `restart_app`.
    AppBundle,
    /// Installed by the Windows setup: the new setup is run by `restart_app`.
    Setup,
    /// Updated by something else (a package manager, the user): the release page is offered.
    Elsewhere,
}

impl InstallKind {
    fn can_install(self) -> bool {
        self != Self::Elsewhere
    }
}

/// What a check found. `update` is `None` when this is the newest version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckDto {
    pub current: String,
    pub update: Option<UpdateOfferDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOfferDto {
    pub version: String,
    /// What is new, one short line each.
    pub notes: Vec<String>,
    /// Size of the download in bytes, when the release says.
    pub size_bytes: Option<u64>,
    /// This copy can install it itself (it runs from an AppImage, from an app bundle in a folder the user can
    /// write to, or the Windows setup installed it). Otherwise the release page is offered.
    pub can_install: bool,
}

/// Progress of `install_update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum UpdateProgressDto {
    Downloading { received: u64, total: Option<u64> },
    Installing,
}

/// Asks the release server whether a newer version exists.
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> CommandResult<UpdateCheckDto> {
    let current = app.package_info().version.to_string();
    // On Windows the updater ends the app by itself once it has started the setup. What has to happen
    // first (see `stop_for_update`) is done right before that.
    let on_exit = app.clone();
    let found = app
        .updater_builder()
        .on_before_exit(move || crate::stop_for_update(&on_exit))
        .build()
        .map_err(|error| CommandError::io(format!("Updates cannot be checked: {error}")))?
        .check()
        .await;
    let found = match found {
        Ok(found) => found,
        // The macOS build is the one a release may lack (its job never blocks publishing): a release
        // without it is not a failed check, this copy has nothing to update to. Elsewhere a missing entry
        // is a broken release and is shown as an error.
        Err(error) if cfg!(target_os = "macos") && release_lacks_this_platform(&error) => None,
        Err(error) => {
            return Err(CommandError::io(format!("Updates could not be checked: {error}")));
        }
    };

    let can_install = install_kind().can_install();
    let offer = found.as_ref().map(|update| UpdateOfferDto {
        version: update.version.clone(),
        notes: release_notes(update.body.as_deref().unwrap_or_default()),
        size_bytes: download_size(&update.raw_json, &update.target),
        can_install,
    });
    *pending.found() = found;

    Ok(UpdateCheckDto {
        current,
        update: offer,
    })
}

/// Whether the check failed only because the newest release has no entry for this platform in its
/// `latest.json` (the updater's `TargetNotFound`, or `TargetsNotFound` when it tried the fallback names).
fn release_lacks_this_platform(error: &tauri_plugin_updater::Error) -> bool {
    matches!(
        error,
        tauri_plugin_updater::Error::TargetNotFound(_) | tauri_plugin_updater::Error::TargetsNotFound(_)
    )
}

/// Downloads the update the last check found and checks its signature. An AppImage, or a macOS app bundle
/// in a folder the user can write to, then replaces itself. A copy installed by the Windows setup keeps
/// the setup instead, and `restart_app` runs it. The new version starts with `restart_app` in every case
/// (on macOS from the same bundle path, which now holds the new bundle). Refused when nothing was found, or
/// when this copy is none of the three.
#[tauri::command]
pub async fn install_update(
    pending: State<'_, PendingUpdate>,
    on_progress: Channel<UpdateProgressDto>,
) -> CommandResult<()> {
    let kind = install_kind();
    if kind == InstallKind::Elsewhere {
        return Err(CommandError::invalid(match current_os() {
            Os::Windows => {
                "This copy of Platter was not installed by its setup. Download the new setup from the release page."
            }
            Os::MacOs => {
                "This copy of Platter is not an app in a folder you can write to. Download the new version from the release page."
            }
            _ => "This copy of Platter was installed by a package. Update it with your package manager.",
        }));
    }
    let Some(update) = pending.found().clone() else {
        return Err(CommandError::invalid(
            "There is no update to install. Check for updates first.",
        ));
    };

    let mut received = 0_u64;
    // One message per whole percent (per MiB without a size), not one per chunk: thousands of messages
    // restart the bar's motion each time and make it stutter.
    let mut sent_step = None;
    let bytes = update
        .download(
            |chunk, total| {
                received += chunk as u64;
                let step = match total {
                    Some(total) if total > 0 => received.saturating_mul(100) / total,
                    _ => received >> 20,
                };
                if sent_step != Some(step) {
                    sent_step = Some(step);
                    let _ = on_progress.send(UpdateProgressDto::Downloading { received, total });
                }
            },
            || {},
        )
        .await
        .map_err(|error| CommandError::io(format!("The update could not be downloaded: {error}")))?;

    if kind == InstallKind::Setup {
        // The setup runs when the user restarts: it ends this app, so it is not started by itself.
        *pending.downloaded() = Some(Downloaded { update, bytes });
        pending.found().take();

        return Ok(());
    }

    let _ = on_progress.send(UpdateProgressDto::Installing);
    // Checking the signature and writing the new AppImage take a moment: off the async worker.
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await?
        .map_err(|error| CommandError::io(format!("The update could not be installed: {error}")))?;
    pending.found().take();

    Ok(())
}

/// Ends the app and starts it again, from the installed update. Quitting goes through the same steps as
/// closing the window (a running cleanup finishes its item first). On Windows, with a setup downloaded by
/// `install_update`, it runs the setup: that ends the app and the setup starts the new version. That is
/// refused (`invalid`, nothing changes) while a cleanup runs, because the updater stops the cleanup before
/// it knows that the setup starts. When the setup cannot be started the app stays, the setup is kept and
/// the error is returned.
#[tauri::command]
pub async fn restart_app(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    if pending.downloaded().is_some() {
        refuse_setup_while_cleaning(&state.cleanups)?;
    }
    let downloaded = pending.downloaded().take();

    if let Some(downloaded) = downloaded {
        let (downloaded, installed) = tauri::async_runtime::spawn_blocking(move || {
            let installed = downloaded.update.install(&downloaded.bytes);

            (downloaded, installed)
        })
        .await?;

        // `install` ends the process once the setup has started: this is reached when it did not start.
        if let Err(error) = installed {
            *pending.downloaded() = Some(downloaded);

            return Err(CommandError::io(format!(
                "The update could not be installed: {error}"
            )));
        }
    }

    app.restart()
}

/// The setup is not run while a cleanup runs: the updater cancels it before it starts the setup, and a
/// setup that then fails to start would leave a cleanup that was stopped for nothing.
fn refuse_setup_while_cleaning(cleanups: &CleanupRegistry) -> CommandResult<()> {
    if cleanups.is_running() {
        return Err(CommandError::invalid(
            "a cleanup is running, so restart when it has finished",
        ));
    }

    Ok(())
}

/// How this copy takes an update, from where it runs.
fn install_kind() -> InstallKind {
    install_kind_of(
        current_os(),
        std::env::var_os("APPIMAGE").as_deref(),
        std::env::current_exe().ok().as_deref(),
        &platter_platform::folder_writable,
    )
}

/// An AppImage on Linux. On Windows a copy that the NSIS setup installed: the setup writes its uninstaller
/// (`uninstall.exe`) into the folder of the program, and a copy that was only unpacked or built does not
/// have one. On macOS an app bundle whose folder `writable` says the user can write to (the updater swaps the
/// bundle for the new one, and asks an administrator's password in a system dialog when it cannot). Nothing
/// else updates itself.
fn install_kind_of(
    os: Os,
    appimage: Option<&OsStr>,
    exe: Option<&Path>,
    writable: &dyn Fn(&Path) -> bool,
) -> InstallKind {
    match os {
        Os::Linux if runs_from_appimage(appimage) => InstallKind::AppImage,
        Os::Windows if installed_by_setup(exe) => InstallKind::Setup,
        Os::MacOs if bundle_folder(exe).is_some_and(writable) => InstallKind::AppBundle,
        _ => InstallKind::Elsewhere,
    }
}

/// The folder that holds the app bundle the program `exe` is in: `/Applications` for
/// `/Applications/Platter.app/Contents/MacOS/platter`. `None` for a program that is anywhere else in or
/// outside a bundle (a build in `target/`, a binary on the path).
fn bundle_folder(exe: Option<&Path>) -> Option<&Path> {
    let macos_folder = exe?.parent()?;
    if macos_folder.file_name()? != "MacOS" {
        return None;
    }

    let contents = macos_folder.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }

    let bundle = contents.parent()?;
    let named = bundle.file_name()?.to_str()?;
    if named.len() <= ".app".len() || !named.ends_with(".app") {
        return None;
    }

    bundle.parent()
}

/// Whether the folder of the program `exe` holds the setup's uninstaller (a file, not a link to one).
fn installed_by_setup(exe: Option<&Path>) -> bool {
    let Some(folder) = exe.and_then(Path::parent) else {
        return false;
    };

    std::fs::symlink_metadata(folder.join(NSIS_UNINSTALLER)).is_ok_and(|meta| meta.is_file())
}

/// Whether this copy runs from an AppImage: the AppImage runtime sets `APPIMAGE` to its absolute path.
fn runs_from_appimage(appimage: Option<&std::ffi::OsStr>) -> bool {
    appimage.is_some_and(|path| path.as_encoded_bytes().first() == Some(&b'/'))
}

/// The release notes as short lines: list markers, blank lines and control characters removed, at most
/// [`MAX_NOTES`] lines of at most [`MAX_NOTE_CHARS`] characters.
fn release_notes(body: &str) -> Vec<String> {
    body.lines()
        .map(|line| {
            let line = line.trim();
            let line = line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .unwrap_or(line);
            let clean: String = line.chars().filter(|c| !c.is_control()).collect();
            let clean = clean.trim();
            match clean.char_indices().nth(MAX_NOTE_CHARS) {
                Some((cut, _)) => format!("{}…", clean[..cut].trim_end()),
                None => clean.to_owned(),
            }
        })
        .filter(|line| !line.is_empty())
        .take(MAX_NOTES)
        .collect()
}

/// The download's size, which the release puts next to the platform's file (`platforms.<target>.size`).
fn download_size(raw: &serde_json::Value, target: &str) -> Option<u64> {
    raw.get("platforms")?.get(target)?.get("size")?.as_u64()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::sync::Arc;

    use platter_cleanup::CleanupPlan;
    use platter_core::dto::ApiErrorCode;
    use serde_json::json;

    use super::*;

    #[test]
    fn a_release_without_a_build_for_this_system_is_no_failed_check() {
        use tauri_plugin_updater::Error;

        assert!(release_lacks_this_platform(&Error::TargetNotFound(
            "darwin-aarch64".to_owned()
        )));
        assert!(release_lacks_this_platform(&Error::TargetsNotFound(vec![
            "darwin-aarch64-app".to_owned(),
            "darwin-aarch64".to_owned(),
        ])));

        assert!(!release_lacks_this_platform(&Error::ReleaseNotFound));
        assert!(!release_lacks_this_platform(&Error::Network("down".to_owned())));
    }

    #[test]
    fn notes_become_short_lines_without_list_markers() {
        let notes =
            release_notes("- Faster scans\n\n* Fix: trash counted twice\n  Plain line  \n\u{1b}[31mred");

        assert_eq!(
            notes,
            [
                "Faster scans",
                "Fix: trash counted twice",
                "Plain line",
                "[31mred"
            ]
        );
    }

    #[test]
    fn long_notes_are_cut() {
        let long = "x".repeat(500);
        let many = vec!["- item"; 40].join("\n");

        assert_eq!(release_notes(&long)[0].chars().count(), MAX_NOTE_CHARS + 1);
        assert_eq!(release_notes(&many).len(), MAX_NOTES);
        assert!(release_notes("").is_empty());
    }

    #[test]
    fn the_size_is_read_from_the_platform_that_was_matched() {
        let raw = json!({
            "platforms": {
                "linux-x86_64-appimage": { "url": "u", "signature": "s", "size": 123 },
                "linux-x86_64": { "url": "u", "signature": "s" }
            }
        });

        assert_eq!(download_size(&raw, "linux-x86_64-appimage"), Some(123));
        assert_eq!(download_size(&raw, "linux-x86_64"), None);
        assert_eq!(download_size(&json!({}), "linux-x86_64"), None);
    }

    /// The shape `.github/workflows/release.yml` writes (its `jq` step), read by the updater itself.
    #[test]
    fn the_release_manifest_is_one_the_updater_reads_for_every_install_kind() {
        let platform = |file: &str, size: u64| {
            json!({
                "url": format!("https://github.com/MSpiechowicz/platter/releases/download/v0.2.0/{file}"),
                "signature": "c2ln",
                "size": size
            })
        };
        let appimage = platform("Platter_0.2.0_amd64.AppImage", 128_000_000);
        let setup = platform("Platter_0.2.0_x64-setup.exe", 64_000_000);
        let manifest = json!({
            "version": "0.2.0",
            "notes": "Faster scans\n- Fix: trash counted twice\n",
            "pub_date": "2026-11-17T16:15:08Z",
            "platforms": {
                "linux-x86_64-appimage": appimage,
                "linux-x86_64": appimage,
                "windows-x86_64-nsis": setup,
                "windows-x86_64": setup
            }
        });

        let release: tauri_plugin_updater::RemoteRelease = serde_json::from_value(manifest.clone()).unwrap();

        assert_eq!(release.version.to_string(), "0.2.0");
        for (target, file, size) in [
            ("linux-x86_64-appimage", "_amd64.AppImage", 128_000_000),
            ("linux-x86_64", "_amd64.AppImage", 128_000_000),
            ("windows-x86_64-nsis", "_x64-setup.exe", 64_000_000),
            ("windows-x86_64", "_x64-setup.exe", 64_000_000),
        ] {
            assert!(
                release.download_url(target).unwrap().as_str().ends_with(file),
                "{target}"
            );
            assert_eq!(release.signature(target).unwrap(), "c2ln", "{target}");
            assert_eq!(download_size(&manifest, target), Some(size), "{target}");
        }
        assert_eq!(
            release_notes(release.notes.as_deref().unwrap()),
            ["Faster scans", "Fix: trash counted twice"]
        );
    }

    #[test]
    fn only_an_appimage_installs_updates_itself_on_linux() {
        let nowhere = None;
        let appimage = |path: &str| install_kind_of(Os::Linux, Some(OsStr::new(path)), nowhere, &|_| false);

        assert_eq!(appimage("/home/me/.local/bin/platter"), InstallKind::AppImage);
        assert_eq!(appimage(""), InstallKind::Elsewhere);
        assert_eq!(appimage("relative/Platter.AppImage"), InstallKind::Elsewhere);
        assert_eq!(
            install_kind_of(Os::Linux, None, nowhere, &|_| false),
            InstallKind::Elsewhere
        );
        assert!(InstallKind::AppImage.can_install());
        assert!(!InstallKind::Elsewhere.can_install());
    }

    #[test]
    fn on_windows_only_a_copy_next_to_the_setups_uninstaller_installs_updates_itself() {
        let installed = tempfile::tempdir().unwrap();
        let unpacked = tempfile::tempdir().unwrap();
        for folder in [&installed, &unpacked] {
            std::fs::write(folder.path().join("platter.exe"), b"MZ").unwrap();
        }
        std::fs::write(installed.path().join(NSIS_UNINSTALLER), b"MZ").unwrap();
        let kind = |folder: &tempfile::TempDir| {
            install_kind_of(
                Os::Windows,
                None,
                Some(&folder.path().join("platter.exe")),
                &|_| false,
            )
        };

        assert_eq!(kind(&installed), InstallKind::Setup);
        assert_eq!(kind(&unpacked), InstallKind::Elsewhere);
        assert_eq!(
            install_kind_of(Os::Windows, None, None, &|_| false),
            InstallKind::Elsewhere
        );
        assert!(InstallKind::Setup.can_install());
    }

    #[test]
    fn an_uninstaller_that_is_a_folder_does_not_make_a_copy_installed() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join(NSIS_UNINSTALLER)).unwrap();

        assert_eq!(
            install_kind_of(
                Os::Windows,
                None,
                Some(&folder.path().join("platter.exe")),
                &|_| false
            ),
            InstallKind::Elsewhere
        );
    }

    #[test]
    fn an_appimage_variable_or_an_uninstaller_means_nothing_on_the_other_system() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join(NSIS_UNINSTALLER), b"MZ").unwrap();
        let exe = folder.path().join("platter");

        assert_eq!(
            install_kind_of(Os::Linux, None, Some(&exe), &|_| true),
            InstallKind::Elsewhere,
            "an uninstall.exe next to a Linux program"
        );
        assert_eq!(
            install_kind_of(
                Os::Windows,
                Some(OsStr::new("/home/me/Platter.AppImage")),
                None,
                &|_| true
            ),
            InstallKind::Elsewhere
        );
        assert_eq!(
            install_kind_of(Os::MacOs, None, Some(&exe), &|_| true),
            InstallKind::Elsewhere,
            "a program that is not in a bundle"
        );
        assert_eq!(
            install_kind_of(
                Os::Linux,
                None,
                Some(Path::new("/Applications/Platter.app/Contents/MacOS/platter")),
                &|_| true
            ),
            InstallKind::Elsewhere,
            "a bundle path means nothing on Linux"
        );
    }

    // Uses the real `folder_writable`, which answers only on Linux and macOS (always `false` on Windows).
    #[cfg(unix)]
    #[test]
    fn on_macos_a_bundle_in_a_folder_the_user_can_write_to_installs_updates_itself() {
        let folder = tempfile::tempdir().unwrap();
        let macos = folder.path().join("Platter.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let exe = macos.join("platter");
        let kind =
            |exe: &Path| install_kind_of(Os::MacOs, None, Some(exe), &platter_platform::folder_writable);

        assert_eq!(kind(&exe), InstallKind::AppBundle);
        assert!(InstallKind::AppBundle.can_install());
        assert_eq!(
            install_kind_of(Os::MacOs, None, Some(&exe), &|_| false),
            InstallKind::Elsewhere,
            "a folder the user cannot write to"
        );
        assert_eq!(
            kind(&folder.path().join("missing/Gone.app/Contents/MacOS/platter")),
            InstallKind::Elsewhere,
            "a folder that is not there is not one the user can write to"
        );
    }

    #[test]
    fn the_folder_asked_about_is_the_one_that_holds_the_bundle() {
        let folder = |exe: &str| bundle_folder(Some(Path::new(exe))).map(Path::to_path_buf);

        assert_eq!(
            folder("/Applications/Platter.app/Contents/MacOS/platter"),
            Some("/Applications".into())
        );
        assert_eq!(
            folder("/Users/me/Applications/Platter.app/Contents/MacOS/platter"),
            Some("/Users/me/Applications".into())
        );
        for not_a_bundle in [
            "/Users/me/platter/target/release/platter",
            "/Applications/Platter.app/Contents/Resources/platter",
            "/Applications/Platter/Contents/MacOS/platter",
            "/Applications/.app/Contents/MacOS/platter",
            "/Applications/Platter.app/MacOS/platter",
            "platter",
        ] {
            assert_eq!(folder(not_a_bundle), None, "{not_a_bundle}");
        }
        assert_eq!(bundle_folder(None), None);
    }

    #[test]
    fn the_setup_waits_for_a_running_cleanup_and_is_not_stopped_by_an_idle_one() {
        let cleanups = Arc::new(CleanupRegistry::default());
        let plan = cleanups.store(1, CleanupPlan::default());
        assert!(
            refuse_setup_while_cleaning(&cleanups).is_ok(),
            "a plan that waits is no run"
        );

        let running = cleanups.take_and_reserve(plan, &|| false).unwrap();
        let refused = refuse_setup_while_cleaning(&cleanups).unwrap_err();
        assert_eq!(refused.code(), ApiErrorCode::Invalid);
        assert!(cleanups.is_running(), "refusing leaves the run alone");

        drop(running);
        assert!(
            refuse_setup_while_cleaning(&cleanups).is_ok(),
            "the slot is free again"
        );
    }

    #[test]
    fn the_progress_is_sent_in_the_contract_form() {
        assert_eq!(
            serde_json::to_value(UpdateProgressDto::Downloading {
                received: 5,
                total: Some(10)
            })
            .unwrap(),
            json!({ "kind": "downloading", "received": 5, "total": 10 })
        );
        assert_eq!(
            serde_json::to_value(UpdateProgressDto::Installing).unwrap(),
            json!({ "kind": "installing" })
        );
    }
}
