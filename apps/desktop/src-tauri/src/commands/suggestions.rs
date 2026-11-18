//! Read-only cleanup suggestions for a finished scan. Nothing is deleted or changed.

use std::sync::Arc;

use platter_cleanup::with_real_keeps;
use platter_core::dto::{ScanId, SuggestionsDto};
use platter_core::{EntryProbe, Units};
use platter_platform::Platform;
use platter_rules::evaluate_with_units;
use tauri::State;

use crate::commands::advisor::question::SizeUnitsDto;
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
///
/// `units` (`"GB"` or `"GiB"`, `"GB"` when absent) is how the sizes in the reasons are written. A change
/// of units makes the result again, like a change of labels.
#[tauri::command]
pub async fn get_suggestions(
    state: State<'_, AppState>,
    scan: ScanId,
    units: Option<SizeUnitsDto>,
) -> CommandResult<SuggestionsDto> {
    let units = Units::from(units.unwrap_or(SizeUnitsDto::Gb));
    let scans = Arc::clone(&state.scans);
    let platform = Arc::clone(&state.platform);
    let user_rules = Arc::clone(&state.user_rules);
    let advisor_enabled = state.advisor.enabled();

    tauri::async_runtime::spawn_blocking(move || {
        suggestions(
            &scans,
            &platform,
            scan,
            advisor_enabled,
            &user_rules.view(),
            units,
        )
    })
    .await?
}

/// The suggestions of `scan`, the sizes in the reasons written in `units`: `notFound` when it is unknown or
/// still running. A caller that reads only the verdicts or the totals passes [`Units::Decimal`], the
/// command's default, so it shares the stored result with the web view.
pub fn suggestions(
    scans: &ScanRegistry,
    platform: &Platform,
    scan: ScanId,
    advisor_enabled: bool,
    user_rules: &UserRulesView,
    units: Units,
) -> CommandResult<SuggestionsDto> {
    suggestions_with(
        scans,
        platform,
        &platform.entry_probe,
        scan,
        advisor_enabled,
        user_rules,
        units,
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
    units: Units,
) -> CommandResult<SuggestionsDto> {
    scans.suggestions_in(
        scan,
        advisor_enabled,
        user_rules.generation,
        units,
        |tree, labels| {
            let env = platform.environment_for(tree.root_path());
            let rules = with_real_keeps(user_rules.rule_set(), env.home.as_deref(), probe);

            evaluate_with_units(tree, &env, &rules, labels, units)
        },
    )
}
