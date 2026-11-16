//! Read-only cleanup suggestions for a finished scan. Nothing is deleted or changed.

use std::sync::Arc;

use platter_cleanup::with_real_keeps;
use platter_core::EntryProbe;
use platter_core::dto::{ScanId, SuggestionsDto};
use platter_platform::Platform;
use platter_rules::evaluate_with;
use tauri::State;

use crate::error::CommandResult;
use crate::state::{AppState, ScanRegistry};
use crate::user_rules::UserRulesView;

/// What could be cleaned in a finished scan, labelled by the built-in rules, the user's own rules and,
/// while the advisor is switched on, by the local model (the `advisor` suggestion, source `model`).
///
/// The rules walk the whole tree (milliseconds, but proportional to its size), so they run on the
/// blocking pool. The result is kept with the scan and made again only when the model's labels, the
/// advisor option or the user's rules change; repeated calls are free. While the rules file is broken the
/// user's rules are left out (and every plan is refused until the file is dealt with).
#[tauri::command]
pub async fn get_suggestions(state: State<'_, AppState>, scan: ScanId) -> CommandResult<SuggestionsDto> {
    let scans = Arc::clone(&state.scans);
    let platform = Arc::clone(&state.platform);
    let user_rules = Arc::clone(&state.user_rules);
    let advisor_enabled = state.advisor.enabled();

    tauri::async_runtime::spawn_blocking(move || {
        suggestions(&scans, &platform, scan, advisor_enabled, &user_rules.view())
    })
    .await?
}

/// The suggestions of `scan`: `notFound` when it is unknown or still running.
pub fn suggestions(
    scans: &ScanRegistry,
    platform: &Platform,
    scan: ScanId,
    advisor_enabled: bool,
    user_rules: &UserRulesView,
) -> CommandResult<SuggestionsDto> {
    suggestions_with(
        scans,
        platform,
        &platform.entry_probe,
        scan,
        advisor_enabled,
        user_rules,
    )
}

/// [`suggestions`] with the disk looked at through `probe`.
pub(crate) fn suggestions_with(
    scans: &ScanRegistry,
    platform: &Platform,
    probe: &impl EntryProbe,
    scan: ScanId,
    advisor_enabled: bool,
    user_rules: &UserRulesView,
) -> CommandResult<SuggestionsDto> {
    scans.suggestions(scan, advisor_enabled, user_rules.generation, |tree, labels| {
        let env = platform.environment_for(tree.root_path());
        let rules = with_real_keeps(user_rules.rule_set(), env.home.as_deref(), probe);

        evaluate_with(tree, &env, &rules, labels)
    })
}
