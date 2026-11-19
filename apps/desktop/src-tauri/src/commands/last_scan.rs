//! `open_last_scan`: the scan kept from the last run, put back as a finished scan (docs/architecture.md,
//! "Files on disk").

use std::sync::Arc;

use platter_core::dto::{ScanId, ScanOptions};
use platter_core::{EntryProbe, NodeId, ProbedEntry, ScanWarning, Totals, Tree};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::cleanup::Question;
use crate::commands::scan::resolve_root;
use crate::error::{CommandError, CommandResult};
use crate::last_scan::{LastScan, elapsed_ms, finished_at};
use crate::state::AppState;

/// A scan opened from disk: what `ScanEvent::Finished` says of a scan, and the options it ran with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastScanDto {
    pub scan_id: ScanId,
    pub root_id: NodeId,
    pub totals: Totals,
    /// As in `ScanEvent::Finished`.
    pub elapsed_ms: u64,
    /// Omitted from the JSON when there are none, as in `ScanEvent::Finished`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ScanWarning>,
    /// What a new scan of the same folder is started with.
    pub options: ScanOptions,
    /// A cleanup changed the disk after the scan, or the folder is no longer the one that was scanned.
    pub outdated: bool,
    /// Unix seconds at which the scan finished (it started `elapsedMs` earlier).
    pub scanned_at: i64,
}

/// What the backend asks before it forgets the scan kept on disk.
fn forget_question() -> Question {
    Question {
        title: "Forget the saved scan?".to_owned(),
        text:
            "Platter deletes the scan it keeps on disk, with the names of every file and folder in it. The \
               scan on screen stays until Platter closes. The next scan is kept again."
                .to_owned(),
        accept: "Forget",
    }
}

/// Deletes the scan kept on disk, after the user confirms it in the backend's own dialog. `cancelled` when
/// the user says no. The scan on screen is not touched: only the copy on disk goes.
#[tauri::command]
pub async fn forget_last_scan(state: State<'_, AppState>) -> CommandResult<()> {
    forget(&state).await
}

pub(crate) async fn forget(state: &AppState) -> CommandResult<()> {
    let question = forget_question();
    let confirm = state.confirm();
    let approved = tauri::async_runtime::spawn_blocking(move || confirm.confirm(&question))
        .await
        .unwrap_or(false);

    if !approved {
        return Err(CommandError::new(
            platter_core::dto::ApiErrorCode::Cancelled,
            "forgetting the saved scan was not confirmed",
        ));
    }

    let store = Arc::clone(&state.last_scan);
    tauri::async_runtime::spawn_blocking(move || store.forget()).await??;

    Ok(())
}

/// Opens the scan kept on disk, if there is one that can be used, and registers it like a finished scan.
/// `null` when there is none: nothing was kept, the files are damaged or from another version, the folder
/// is gone or is not the folder that was scanned, or a scan runs now or was started while this read.
#[tauri::command]
pub async fn open_last_scan(state: State<'_, AppState>) -> CommandResult<Option<LastScanDto>> {
    open(&state).await
}

/// What tells that the mounts the scan saw are still the mounts at their places.
type MountCheck = Arc<dyn Fn(&Tree) -> bool + Send + Sync>;

pub async fn open(state: &AppState) -> CommandResult<Option<LastScanDto>> {
    let platform = Arc::clone(&state.platform);

    open_with(
        state,
        Arc::new(move |tree| mounts_unchanged(&platform.entry_probe, tree)),
    )
    .await
}

async fn open_with(state: &AppState, mounts_unchanged: MountCheck) -> CommandResult<Option<LastScanDto>> {
    // A tree larger than the pool is not read at all.
    let Some(ticket) = state
        .scans
        .restore_pool(platter_platform::available_memory_bytes())
    else {
        return Ok(None);
    };

    // Reading a large tree and looking at the folder (a network disk may not answer) are slow.
    let store = Arc::clone(&state.last_scan);
    let found = tauri::async_runtime::spawn_blocking(move || {
        let saved = store.load(ticket.pool)?;
        let resolved = resolve_root(&saved.options.root).ok()?;
        if resolved.path != saved.tree.root_path() {
            return None;
        }

        // Only the very folder the scan saw is opened again: a cleanup below it, and the folder allowed
        // for a new scan, both stand on that. Where the system tells no identity, nothing is opened.
        let scanned = saved.tree.root_identity();
        let identity = resolved
            .identity
            .filter(|found| *found == (scanned.dev, scanned.ino))?;
        let mounts_unchanged = mounts_unchanged(&saved.tree);

        Some((saved, identity, mounts_unchanged))
    })
    .await?;
    let Some((saved, identity, mounts_unchanged)) = found else {
        return Ok(None);
    };

    let LastScan {
        tree,
        options,
        outdated,
    } = saved;

    let mut dto = LastScanDto {
        scan_id: 0,
        root_id: tree.root(),
        totals: tree.totals(),
        elapsed_ms: elapsed_ms(&tree),
        warnings: tree.warnings().to_vec(),
        options,
        // Mount ids are not the same after a restart (or a mount that was made again): such a scan shows
        // what was there, but a cleanup would refuse every mount it passes, so it is offered a rescan.
        outdated: outdated || !mounts_unchanged,
        scanned_at: finished_at(&tree),
    };

    let available = platter_platform::available_memory_bytes();
    // `None`: a scan was started meanwhile, and it is not evicted for this older one.
    let Some(scan_id) = state.scans.restore(tree, dto.outdated, available, &ticket)? else {
        return Ok(None);
    };
    dto.scan_id = scan_id;

    // A new scan of the same folder, started with these options, must pass `start_scan`: the folder is
    // allowed as if it had just been picked, pinned to the folder that was scanned, unless it was allowed
    // already (then it keeps what it was allowed with).
    state.allow_picked_if_new(dto.options.root.clone(), Some(identity));

    Ok(Some(dto))
}

/// Whether the folder the scan opened and the mounts it went into are still the same mounts: the same
/// device and inode, and the same mount id when the scan saw one. A mount id is the kernel's number for a
/// mount and is not kept across a restart, so after one every scan reads as changed. What cannot be looked
/// at counts as changed.
fn mounts_unchanged(probe: &impl EntryProbe, tree: &Tree) -> bool {
    let looked_at = |id: NodeId| -> Option<Option<ProbedEntry>> {
        let found = probe.probe(&tree.path_bytes(id)).ok()?;

        Some(found.last().copied())
    };

    // The top (`/`) has no component to look at, as in a cleanup.
    let root = tree.root_identity();
    match looked_at(NodeId::ROOT) {
        None => return false,
        Some(Some(now)) if !root.is(now.meta.dev, now.meta.ino, now.mount_id) => return false,
        Some(_) => {}
    }

    tree.entered_mounts().all(|(id, scanned)| {
        looked_at(id).flatten().is_some_and(|now| {
            now.meta.dev == scanned.dev
                && now.meta.ino == scanned.ino
                && scanned.mount_id.is_none_or(|seen| now.mount_id == Some(seen))
        })
    })
}

#[cfg(test)]
mod tests;
