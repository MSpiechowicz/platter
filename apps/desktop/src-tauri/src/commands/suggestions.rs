//! Read-only cleanup suggestions for a finished scan. Nothing is deleted or changed.

use std::sync::Arc;

use platter_core::dto::{ScanId, SuggestionsDto};
use platter_platform::Platform;
use platter_rules::{RuleSet, evaluate};
use tauri::State;

use crate::error::CommandResult;
use crate::state::{AppState, ScanRegistry};

/// What could be cleaned in a finished scan, labelled by the built-in rules.
///
/// The rules walk the whole tree (milliseconds, but proportional to its size), so they run on the
/// blocking pool. The result is computed once per scan and kept with it; repeated calls are free.
#[tauri::command]
pub async fn get_suggestions(state: State<'_, AppState>, scan: ScanId) -> CommandResult<SuggestionsDto> {
    let scans = Arc::clone(&state.scans);
    let platform = Arc::clone(&state.platform);

    tauri::async_runtime::spawn_blocking(move || suggestions(&scans, &platform, scan)).await?
}

/// The suggestions of `scan`: `notFound` when it is unknown or still running.
pub fn suggestions(scans: &ScanRegistry, platform: &Platform, scan: ScanId) -> CommandResult<SuggestionsDto> {
    scans.suggestions(scan, |tree| {
        evaluate(
            tree,
            &platform.environment_for(tree.root_path()),
            &RuleSet::builtin(),
        )
    })
}
