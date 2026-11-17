use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use platter_core::dto::{MAX_EXCLUDED, MAX_EXCLUDED_PATH_BYTES, ScanEvent, ScanId, ScanOptions};
use platter_core::{CancelToken, DirSource, Os, ScanConfig, scan};
use platter_platform::Platform;
use platter_platform::environment::current_os;
use platter_platform::root::ResolvedRoot;
use tauri::State;
use tauri::ipc::Channel;

use crate::commands::history::HistoryService;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, ScanRegistry};

/// Receives every event of one scan: progress first, then exactly one terminal event.
pub type EmitFn = Arc<dyn Fn(ScanEvent) + Send + Sync>;

/// Starts a scan and returns its id at once. Events arrive on `on_event`.
///
/// Async on purpose: a synchronous command runs on the main thread, and resolving the root can stall on
/// a hung network mount, which would freeze the window. [`start`] does that part on the blocking pool.
#[tauri::command]
pub async fn start_scan(
    state: State<'_, AppState>,
    options: ScanOptions,
    on_event: Channel<ScanEvent>,
) -> CommandResult<ScanId> {
    // Only a root the backend offered (a volume, a folder from its dialog): the web view cannot name
    // an arbitrary path to scan, and through a scan, to plan a cleanup in.
    let Some(pinned) = state.allowed_root(&options.root) else {
        return Err(CommandError::invalid(
            "scan a disk from the list or a folder chosen in the folder dialog",
        ));
    };

    let emit: EmitFn = Arc::new(move |event| {
        // The webview may have gone away; there is nobody to tell.
        let _ = on_event.send(event);
    });

    start_pinned(&state, options, pinned, emit).await
}

/// Cancels a running scan. Unknown and already ended scans are not an error (nothing to cancel), so a
/// cancel that races with the scan finishing needs no special handling in the UI.
#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>, scan: ScanId) -> CommandResult<()> {
    state.scans.cancel(scan);

    Ok(())
}

/// Frees a scan's tree (cancelling it first when it is still running). Unknown ids are not an error.
#[tauri::command]
pub fn drop_scan(state: State<'_, AppState>, scan: ScanId) -> CommandResult<()> {
    state.scans.drop_scan(scan);

    Ok(())
}

/// Validates the root (on the blocking pool), registers the scan and runs it on a dedicated thread.
///
/// A scan lasts seconds to minutes and does its own fan-out on a private rayon pool, so the driver
/// thread only waits. A plain thread keeps it off the async runtime's blocking pool, which is shared
/// with the queries.
#[cfg(test)]
pub async fn start(state: &AppState, options: ScanOptions, emit: EmitFn) -> CommandResult<ScanId> {
    start_pinned(state, options, None, emit).await
}

/// [`start`], refusing when the root is no longer the folder with identity `pinned` (a picked folder
/// swapped for a link, or for another folder, since it was picked).
pub async fn start_pinned(
    state: &AppState,
    options: ScanOptions,
    pinned: Option<(u64, u64)>,
    emit: EmitFn,
) -> CommandResult<ScanId> {
    check_excluded(&options.excluded)?;

    // `canonicalize` touches the filesystem and can block for as long as a mount is unresponsive. The
    // scan is not registered yet, so a failure here leaves nothing behind and emits no event.
    let requested = options.root.clone();
    let resolved = tauri::async_runtime::spawn_blocking(move || resolve_root(&requested)).await??;
    if let Some(identity) = pinned
        && resolved.identity != Some(identity)
    {
        return Err(CommandError::invalid(
            "the folder changed since it was chosen, so choose it again",
        ));
    }
    let root = resolved.path;

    let excluded = options.excluded;
    let platform = Arc::clone(&state.platform);
    let (excluded, excluded_unresolved) = tauri::async_runtime::spawn_blocking({
        let root = root.clone();
        move || excluded_identities(&platform, &root, &excluded)
    })
    .await?;
    let config = ScanConfig {
        excluded,
        excluded_unresolved,
        ..scan_config(options.one_file_system, resolved.identity)
    };

    let (id, cancel) = state.scans.begin()?;

    let platform = Arc::clone(&state.platform);
    let registry = Arc::clone(&state.scans);
    let emit = recording(Arc::clone(&state.history), root.clone(), emit);
    let spawned = thread::Builder::new()
        .name(format!("platter-scan-driver-{id}"))
        .spawn(move || {
            run_scan(
                &platform.dir_source,
                &registry,
                id,
                &cancel,
                &root,
                &config,
                &*emit,
            );
        });

    if let Err(error) = spawned {
        state.scans.abandon(id);
        return Err(error.into());
    }

    Ok(id)
}

/// `emit`, and the history hears of a scan that finished (and takes the day's first usage sample).
fn recording(history: Arc<HistoryService>, root: PathBuf, emit: EmitFn) -> EmitFn {
    Arc::new(move |event| {
        if let ScanEvent::Finished { totals, .. } = &event {
            history.record_scan(&root, totals);
        }
        emit(event);
    })
}

/// The scan settings the app uses.
///
/// `identity` makes the scan refuse to run if the folder was swapped for another one since it was
/// resolved. The memory budget is half of what is free right now, so a huge folder ends in a clear
/// message ("needs more memory than Platter may use for a scan") instead of an out-of-memory kill; the
/// entry cap is only the hard limit of the node ids.
pub fn scan_config(one_file_system: bool, identity: Option<(u64, u64)>) -> ScanConfig {
    ScanConfig {
        one_file_system,
        root_identity: identity,
        max_entries: None,
        max_memory_bytes: Some(platter_platform::suggested_memory_budget()),
        ..ScanConfig::default()
    }
}

/// Refuses a list of folders to leave out that is too long or holds a path that is not absolute, before
/// anything is read. A relative path would mean whatever folder the app happens to run in, and `~` is
/// not expanded.
pub fn check_excluded(excluded: &[String]) -> CommandResult<()> {
    if excluded.len() > MAX_EXCLUDED {
        return Err(CommandError::invalid(format!(
            "at most {MAX_EXCLUDED} folders can be left out of a scan, not {}",
            excluded.len()
        )));
    }

    for path in excluded {
        if path.len() > MAX_EXCLUDED_PATH_BYTES {
            return Err(CommandError::invalid(format!(
                "a folder to leave out has a path longer than {MAX_EXCLUDED_PATH_BYTES} bytes"
            )));
        }
        if !Path::new(path).is_absolute() {
            return Err(CommandError::invalid(format!(
                "{path} is not an absolute path, so it cannot be left out of a scan"
            )));
        }
    }

    Ok(())
}

/// How long resolving every folder to leave out may take. The scan starts after it with what was resolved.
pub const EXCLUDED_BUDGET: Duration = Duration::from_secs(3);

/// The `(dev, ino)` of every folder to leave out that exists now, and whether some could not be resolved.
///
/// Each path is looked at on a thread of its own and all of them together get [`EXCLUDED_BUDGET`]: a path on a
/// network disk that does not answer must not keep a scan from starting. Such a look is left to finish on its
/// own. The look walks the path component by component by descriptor (the entry probe of cleanup), so it does
/// not trigger an automount at the folder itself. Only a path with a link on the way is resolved through the link instead. A folder
/// that was removed, is not a folder, cannot be read or did not answer in time is not left out, and the
/// second value says so (the scan carries [`ScanWarning::ExcludedUnresolved`](platter_core::ScanWarning)).
pub fn excluded_identities(
    platform: &Arc<Platform>,
    root: &Path,
    excluded: &[String],
) -> (Vec<(u64, u64)>, bool) {
    let (sender, receiver) = mpsc::channel();

    // A path whose look could not even start is unresolved at once.
    let mut looking = 0;
    for (index, path) in excluded.iter().enumerate() {
        let (sender, platform, path) = (sender.clone(), Arc::clone(platform), path.clone());
        let spawned = thread::Builder::new()
            .name("platter-exclude".to_owned())
            .spawn(move || {
                let _ = sender.send((index, identity_of(&platform, &path)));
            });
        looking += usize::from(spawned.is_ok());
    }
    drop(sender);

    let deadline = Instant::now() + EXCLUDED_BUDGET;
    let mut identities = Vec::with_capacity(excluded.len());
    let mut resolved = vec![false; excluded.len()];
    let mut answered = 0;
    while answered < looking {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok((index, found)) => {
                answered += 1;
                resolved[index] = found.is_some();
                identities.extend(found);
            }
            Err(_) => break,
        }
    }
    // Only a folder inside this scan matters: one elsewhere (an unplugged disk) would not have been scanned anyway.
    let fold_case = current_os() == Os::Windows;
    let unresolved = excluded
        .iter()
        .zip(&resolved)
        .any(|(path, &done)| !done && is_inside(Path::new(path), root, fold_case));

    identities.sort_unstable();
    identities.dedup();
    (identities, unresolved)
}

/// Whether `path` is `root` or lies inside it. With `fold_case` (Windows) names are compared without
/// regard to ASCII case, as the file system does: a folder typed as `c:\users\me\x` is inside `C:\Users\me`.
fn is_inside(path: &Path, root: &Path, fold_case: bool) -> bool {
    if !fold_case {
        return path.starts_with(root);
    }

    let folded = |path: &Path| PathBuf::from(path.to_string_lossy().to_ascii_lowercase());

    folded(path).starts_with(folded(root))
}

/// The identity of one folder to leave out, or `None` when it cannot be used. On Windows it is the volume
/// serial number and the file index, from `resolve_root`.
fn identity_of(platform: &Platform, path: &str) -> Option<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        use platter_core::{EntryKind, EntryProbe};

        match platform.entry_probe.probe(path.as_bytes()) {
            Ok(found) => match found.last()?.meta {
                meta if meta.kind == EntryKind::Dir => return Some((meta.dev, meta.ino)),
                // The path itself is a link: resolved through it below.
                meta if meta.kind == EntryKind::Symlink => {}
                _ => return None,
            },
            // A link on the way (`/home` on some systems): resolved through it below.
            Err(error) if error.kind() == io::ErrorKind::NotADirectory => {}
            Err(_) => return None,
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = platform;

    platter_platform::root::resolve_root(Path::new(path))
        .ok()?
        .identity
}

/// Runs a scan to completion and reports it: progress events while it walks, then one terminal event.
///
/// The tree is stored in the registry before `finished` is emitted, so the UI can query as soon as it
/// sees the event. A scan dropped while running ends as `cancelled`.
pub fn run_scan<S: DirSource>(
    source: &S,
    registry: &ScanRegistry,
    id: ScanId,
    cancel: &CancelToken,
    root: &Path,
    config: &ScanConfig,
    emit: &(dyn Fn(ScanEvent) + Sync),
) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        scan(source, root, config, cancel, &|progress| {
            emit(ScanEvent::from(progress))
        })
    }));

    let terminal = match outcome {
        Ok(Ok(tree)) => match registry.finish(id, tree) {
            Some(tree) => ScanEvent::finished(id, &tree),
            None => ScanEvent::Cancelled,
        },
        Ok(Err(error)) => {
            registry.abandon(id);
            ScanEvent::failed(&error)
        }
        Err(_) => {
            registry.abandon(id);
            ScanEvent::Failed {
                message: "the scan stopped unexpectedly".to_owned(),
            }
        }
    };

    emit(terminal);
}

/// The root comes from the UI (a volume mount point or a picked folder). It must be an absolute path to
/// an existing directory; symlinks are resolved so the tree records the real location, and the
/// directory's identity is kept so the scan can tell if it was swapped before it started.
pub fn resolve_root(root: &str) -> CommandResult<ResolvedRoot> {
    if root.trim().is_empty() {
        return Err(CommandError::invalid("the scan root is empty"));
    }

    let path = Path::new(root);
    if !path.is_absolute() {
        return Err(CommandError::invalid(format!("{root} is not an absolute path")));
    }

    platter_platform::root::resolve_root(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => CommandError::invalid(format!("{root} does not exist")),
        io::ErrorKind::NotADirectory => CommandError::invalid(format!("{root} is not a directory")),
        _ => CommandError::io(format!("cannot open {root}: {error}")),
    })
}

#[cfg(test)]
mod tests;
