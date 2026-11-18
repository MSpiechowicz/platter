//! The user's own rules over IPC (A4): "always keep this folder" and "this folder is safe to clean". The
//! web view names a folder by a node id of a scan; the backend derives the exact path itself, so a rule
//! can only be about a folder the scan really found. The model can never create a rule: only these
//! commands do, and each is a click of the user.
//!
//! Every command returns the whole list. While the rules file is broken (`problem` is set) nothing can be
//! added or removed, and every cleanup plan is refused, until `reset_user_rules` moves the file aside.
//! Any change drops the cleanup plans that were waiting to be started.

use std::path::Path;
use std::sync::Arc;

use platter_cleanup::Protected;
use platter_core::dto::ScanId;
use platter_core::{EntryProbe, NodeId, NodeKind, UserRule, UserVerdict, check_folder};
use platter_platform::paths::node_path;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;
use crate::user_rules::UserRulesView;

/// What a rule says about its folder: `'keep' | 'safe'` on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserVerdictDto {
    Keep,
    Safe,
}

impl From<UserVerdictDto> for UserVerdict {
    fn from(verdict: UserVerdictDto) -> Self {
        match verdict {
            UserVerdictDto::Keep => Self::Keep,
            UserVerdictDto::Safe => Self::Safe,
        }
    }
}

impl From<UserVerdict> for UserVerdictDto {
    fn from(verdict: UserVerdict) -> Self {
        match verdict {
            UserVerdict::Keep => Self::Keep,
            UserVerdict::Safe => Self::Safe,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRuleDto {
    pub id: u32,
    /// The folder for people to read: the home folder is `~`.
    pub path: String,
    pub verdict: UserVerdictDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRulesDto {
    pub rules: Vec<UserRuleDto>,
    /// Why the rules file cannot be used, in plain text; every plan is refused while this is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// The rules the user has made, in the order they were made.
#[tauri::command]
pub async fn list_user_rules(state: State<'_, AppState>) -> CommandResult<UserRulesDto> {
    list(&state).await
}

/// Says `keep` or `safe` about the folder `node` of a finished scan. A folder that already has a rule gets
/// the new verdict. `safe` is refused for the home folder, the scan root and protected places.
#[tauri::command]
pub async fn add_user_rule(
    state: State<'_, AppState>,
    scan: ScanId,
    node: NodeId,
    verdict: UserVerdictDto,
) -> CommandResult<UserRulesDto> {
    add(&state, scan, node, verdict).await
}

/// Removes a rule. An id that is not there changes nothing.
#[tauri::command]
pub async fn remove_user_rule(state: State<'_, AppState>, id: u32) -> CommandResult<UserRulesDto> {
    remove(&state, id).await
}

/// Moves a broken rules file to `rules.json.broken` and starts with no rules. `invalid` when the file is
/// not broken.
#[tauri::command]
pub async fn reset_user_rules(state: State<'_, AppState>) -> CommandResult<UserRulesDto> {
    reset(&state).await
}

pub async fn list(state: &AppState) -> CommandResult<UserRulesDto> {
    let view = crate::user_rules::current(&state.user_rules).await?;

    Ok(dto(&view, state))
}

pub async fn add(
    state: &AppState,
    scan: ScanId,
    node: NodeId,
    verdict: UserVerdictDto,
) -> CommandResult<UserRulesDto> {
    let tree = state.scans.tree(scan)?;
    if !tree.contains(node) {
        return Err(CommandError::not_found(format!("node {node:?} does not exist")));
    }
    if tree.kind(node) != NodeKind::Dir {
        return Err(CommandError::invalid("only a folder can have a rule"));
    }

    // The exact path of the node, not its display text.
    let folder = node_path(&tree, node);
    if folder.to_str().is_none() {
        return Err(CommandError::invalid(format!(
            "the folder {} has a name that is not valid text, so a rule cannot name it",
            folder.display()
        )));
    }
    check_folder(&folder).map_err(|error| CommandError::invalid(error.to_string()))?;

    let verdict = UserVerdict::from(verdict);
    let platform = Arc::clone(&state.platform);
    let store = Arc::clone(&state.user_rules);
    let view = tauri::async_runtime::spawn_blocking(move || {
        if verdict == UserVerdict::Safe {
            let env = platform.environment_for(tree.root_path());
            safe_allowed(&folder, node == tree.root(), &env, &platform.entry_probe)?;
        }

        store.set(&folder, verdict)
    })
    .await??;

    changed(state);
    Ok(dto(&view, state))
}

pub async fn remove(state: &AppState, id: u32) -> CommandResult<UserRulesDto> {
    let store = Arc::clone(&state.user_rules);
    let view = tauri::async_runtime::spawn_blocking(move || store.remove(id)).await??;

    changed(state);
    Ok(dto(&view, state))
}

pub async fn reset(state: &AppState) -> CommandResult<UserRulesDto> {
    let store = Arc::clone(&state.user_rules);
    let view = tauri::async_runtime::spawn_blocking(move || store.reset()).await??;

    changed(state);
    Ok(dto(&view, state))
}

/// What a change of the rules does to the rest: no plan that was waiting is still what the rules would
/// plan, so all are dropped (also by a call that turned out to change nothing: that costs the user one more
/// click and never leaves a stale plan). The suggestions are made again by themselves, as their key holds
/// the rules' generation.
fn changed(state: &AppState) {
    state.cleanups.drop_plans();
}

/// `safe` is refused for the home folder, the scan root and every place cleanup protects, by path and by
/// identity (a second path to the home folder, such as a bind mount, is the home folder). The plan checks
/// them again, so a rule written into the file by hand gets no further.
fn safe_allowed(
    folder: &Path,
    is_scan_root: bool,
    env: &platter_core::Environment,
    probe: &impl EntryProbe,
) -> CommandResult<()> {
    if is_scan_root {
        return Err(CommandError::invalid(
            "this is the folder that was scanned. Platter never cleans it, so it cannot be marked safe",
        ));
    }
    if env.home.as_deref() == Some(folder) {
        return Err(CommandError::invalid(
            "this is your home folder. Platter never cleans it, so it cannot be marked safe",
        ));
    }

    let Some(protected) = Protected::on_disk(env, probe) else {
        return Err(CommandError::invalid(
            "Platter does not know your home folder, so it cannot tell which places are protected",
        ));
    };
    if let Some(place) = protected.refuses_on_disk(probe, folder.as_os_str().as_encoded_bytes()) {
        return Err(CommandError::invalid(format!(
            "this is, holds or lies inside a protected place ({place}). Platter never cleans it, so it cannot be marked safe"
        )));
    }

    Ok(())
}

fn dto(view: &UserRulesView, state: &AppState) -> UserRulesDto {
    let home = state.platform.environment().home;

    UserRulesDto {
        rules: view
            .rules
            .iter()
            .map(|rule| rule_dto(rule, home.as_deref()))
            .collect(),
        problem: view.problem.clone(),
    }
}

fn rule_dto(rule: &UserRule, home: Option<&Path>) -> UserRuleDto {
    let path = match home.and_then(|home| rule.folder.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.to_string_lossy()),
        None => rule.folder.to_string_lossy().into_owned(),
    };

    UserRuleDto {
        id: rule.id,
        path,
        verdict: rule.verdict.into(),
    }
}

// The Linux disk model (a `FakeFs` with `/mnt`, `/var/cache/pacman`, `/etc`): the paths and the built-in
// rules it exercises are Linux ones (macOS has neither the rules nor names that are not UTF-8).
#[cfg(all(test, target_os = "linux"))]
mod tests;

// The same commands on a real folder, on every system.
#[cfg(test)]
mod native_tests;
