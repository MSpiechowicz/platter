//! Updates (D3). The release's `latest.json` names the newest version, its notes and the signed AppImage
//! (`.github/workflows/release.yml`). The Tauri updater fetches it from the endpoint in `tauri.conf.json` and
//! checks every download against the public key there, so only a build signed with the owner's key is
//! installed.
//!
//! Only an AppImage replaces itself. A deb or rpm install is told about the update and gets the release
//! page: the updater would install those packages through `sudo` with a password typed into a dialog,
//! which Platter does not do (the package manager updates them).
//!
//! The web view is granted no updater permission. It calls these three commands, and the update that a
//! check found is kept here, so the web view never names what is downloaded.

use std::sync::Mutex;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{CommandError, CommandResult};

/// The most lines of release notes shown, and the most characters of each.
const MAX_NOTES: usize = 12;
const MAX_NOTE_CHARS: usize = 200;

/// The update the last check found, until it is installed or a new check replaces it.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

impl PendingUpdate {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Update>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
    /// This copy can install it itself (it runs from an AppImage). Otherwise the release page is offered.
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
    let found = app
        .updater()
        .map_err(|error| CommandError::io(format!("Updates cannot be checked: {error}")))?
        .check()
        .await
        .map_err(|error| CommandError::io(format!("Updates could not be checked: {error}")))?;

    let offer = found.as_ref().map(|update| UpdateOfferDto {
        version: update.version.clone(),
        notes: release_notes(update.body.as_deref().unwrap_or_default()),
        size_bytes: download_size(&update.raw_json, &update.target),
        can_install: runs_from_appimage(std::env::var_os("APPIMAGE").as_deref()),
    });
    *pending.lock() = found;

    Ok(UpdateCheckDto {
        current,
        update: offer,
    })
}

/// Downloads the update the last check found, checks its signature and replaces the AppImage. The new
/// version starts with `restart_app`. Refused when nothing was found, or when this copy is not an AppImage.
#[tauri::command]
pub async fn install_update(
    pending: State<'_, PendingUpdate>,
    on_progress: Channel<UpdateProgressDto>,
) -> CommandResult<()> {
    if !runs_from_appimage(std::env::var_os("APPIMAGE").as_deref()) {
        return Err(CommandError::invalid(
            "This copy of Platter was installed by a package. Update it with your package manager.",
        ));
    }
    let Some(update) = pending.lock().clone() else {
        return Err(CommandError::invalid(
            "There is no update to install. Check for updates first.",
        ));
    };

    let mut received = 0_u64;
    let bytes = update
        .download(
            |chunk, total| {
                received += chunk as u64;
                let _ = on_progress.send(UpdateProgressDto::Downloading { received, total });
            },
            || {},
        )
        .await
        .map_err(|error| CommandError::io(format!("The update could not be downloaded: {error}")))?;

    let _ = on_progress.send(UpdateProgressDto::Installing);
    // Checking the signature and writing the new AppImage take a moment: off the async worker.
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await?
        .map_err(|error| CommandError::io(format!("The update could not be installed: {error}")))?;
    pending.lock().take();

    Ok(())
}

/// Ends the app and starts it again, from the installed update. Quitting goes through the same steps as
/// closing the window (a running cleanup finishes its item first).
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
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

    use serde_json::json;

    use super::*;

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
        let manifest = json!({
            "version": "0.2.0",
            "notes": "Faster scans\n- Fix: trash counted twice\n",
            "pub_date": "2026-11-17T16:15:08Z",
            "platforms": {
                "linux-x86_64-appimage": {
                    "url": "https://github.com/MSpiechowicz/platter/releases/download/v0.2.0/Platter_0.2.0_amd64.AppImage",
                    "signature": "c2ln",
                    "size": 128_000_000
                },
                "linux-x86_64": {
                    "url": "https://github.com/MSpiechowicz/platter/releases/download/v0.2.0/Platter_0.2.0_amd64.AppImage",
                    "signature": "c2ln",
                    "size": 128_000_000
                }
            }
        });

        let release: tauri_plugin_updater::RemoteRelease = serde_json::from_value(manifest.clone()).unwrap();

        assert_eq!(release.version.to_string(), "0.2.0");
        for target in ["linux-x86_64-appimage", "linux-x86_64"] {
            assert!(
                release
                    .download_url(target)
                    .unwrap()
                    .as_str()
                    .ends_with("_amd64.AppImage")
            );
            assert_eq!(release.signature(target).unwrap(), "c2ln");
            assert_eq!(download_size(&manifest, target), Some(128_000_000));
        }
        assert_eq!(
            release_notes(release.notes.as_deref().unwrap()),
            ["Faster scans", "Fix: trash counted twice"]
        );
    }

    #[test]
    fn only_an_appimage_installs_updates_itself() {
        assert!(runs_from_appimage(Some(OsStr::new(
            "/home/me/.local/bin/platter"
        ))));
        assert!(!runs_from_appimage(None));
        assert!(!runs_from_appimage(Some(OsStr::new(""))));
        assert!(!runs_from_appimage(Some(OsStr::new("relative/Platter.AppImage"))));
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
