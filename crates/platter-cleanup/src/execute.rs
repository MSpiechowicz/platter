//! Carrying out a plan. Each target is checked again right before it is acted on; a run can be
//! cancelled between targets, never in the middle of one; a failure on one target does not stop the
//! others.

use std::path::Path;

use platter_core::{
    CancelToken, EntryKind, EntryProbe, EntryRemover, ExpectedEntry, RemoveMode, Removed, ToolCommand,
    ToolFailure, ToolRunner,
};

use crate::action::CleanupAction;
use crate::guard::{VerifiedTarget, markers_present, same_file};
use crate::plan::CleanupPlan;

/// What happened to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetOutcome {
    /// Deleted (or emptied). `errors` lists what inside could not be removed.
    Removed(Removed),
    /// Moved to the trash, to this exact path.
    Trashed(Vec<u8>),
    /// Cleaned by the tool that owns it (the command that ran).
    ByTool(String),
    /// Not touched: it changed since the plan, or the action is not available.
    Skipped(String),
    /// The action failed before anything was removed.
    Failed(String),
}

/// One target and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetResult {
    pub rule_id: &'static str,
    /// The path for people to read.
    pub path: String,
    /// The exact path, and what the plan saw there (to put a trashed entry back).
    pub exact: Vec<u8>,
    pub identity: (u64, u64),
    pub is_dir: bool,
    /// Bytes the scan counted for it.
    pub allocated: u64,
    pub outcome: TargetOutcome,
}

/// Progress, sent while a plan runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupEvent {
    GroupStarted { rule_id: &'static str, targets: usize },
    TargetDone(TargetResult),
    GroupFinished { rule_id: &'static str },
}

/// The outcome of a run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanupReport {
    pub results: Vec<TargetResult>,
    /// Rules whose owning tool was not run, because no tool runner was given.
    pub tools_not_run: Vec<&'static str>,
    /// The run stopped early because it was cancelled.
    pub cancelled: bool,
    /// Free space after minus before, on the scan root's filesystem. This is what the run freed, give or
    /// take what other programs wrote meanwhile; `None` when free space could not be read.
    pub freed: Option<i128>,
}

/// Where acting code reaches the disk.
pub struct Disk<'a, P: EntryProbe, R: EntryRemover> {
    pub probe: &'a P,
    pub remover: &'a R,
    /// The user's trash folder; `None` refuses every move to the trash.
    pub trash_dir: Option<&'a Path>,
    /// Runs the tools that own a cache; `None` leaves those rules undone.
    pub tools: Option<&'a dyn ToolRunner>,
}

/// Puts back what a run moved to the trash (`results` of its report). Each entry is checked against
/// what the plan saw and never overwrites anything now at its old place. Returns, per trashed entry,
/// its readable path and what happened.
pub fn undo<R: EntryRemover>(results: &[TargetResult], remover: &R) -> Vec<(String, Result<(), String>)> {
    results
        .iter()
        .filter_map(|result| match &result.outcome {
            TargetOutcome::Trashed(to) => Some((result, to)),
            _ => None,
        })
        .map(|(result, to)| {
            let expected = ExpectedEntry {
                dev: result.identity.0,
                ino: result.identity.1,
                mount_id: None,
                is_dir: result.is_dir,
                root_depth: 0,
            };
            let outcome = remover
                .restore(to, &expected, &result.exact)
                .map_err(|error| error.to_string());
            (result.path.clone(), outcome)
        })
        .collect()
}

/// Runs `plan`. Every target is probed again first and handed to the remover with what the plan saw, so
/// a target that changed since the plan is left alone.
pub fn execute<P: EntryProbe, R: EntryRemover>(
    plan: &CleanupPlan,
    disk: &Disk<'_, P, R>,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(CleanupEvent),
) -> CleanupReport {
    let mut report = CleanupReport::default();
    // Measure both sides after the filesystem committed, so earlier pending frees are not counted.
    let _ = disk.remover.settle(&plan.scan_root);
    let before = disk.remover.free_bytes(&plan.scan_root).ok();

    // Once the administrator password was refused, no other prompt is shown in this run.
    let mut refused: Option<String> = None;

    'groups: for group in &plan.groups {
        if let CleanupAction::RunTool(tool) = group.action {
            let Some(runner) = disk.tools else {
                report.tools_not_run.push(group.rule_id);
                continue;
            };
            if cancel.is_cancelled() {
                report.cancelled = true;
                break 'groups;
            }

            on_event(CleanupEvent::GroupStarted {
                rule_id: group.rule_id,
                targets: group.targets.len(),
            });
            let outcome = if let (true, Some(why)) = (tool.needs_elevation, &refused) {
                TargetOutcome::Skipped(why.clone())
            } else {
                run_tool(runner, tool, &mut refused)
            };
            for target in &group.targets {
                let result = TargetResult {
                    rule_id: group.rule_id,
                    path: target.display_path(),
                    exact: target.path().to_vec(),
                    identity: target.identity(),
                    is_dir: target.is_dir(),
                    allocated: target.allocated(),
                    outcome: outcome.clone(),
                };
                on_event(CleanupEvent::TargetDone(result.clone()));
                report.results.push(result);
            }
            on_event(CleanupEvent::GroupFinished {
                rule_id: group.rule_id,
            });
            continue;
        }

        on_event(CleanupEvent::GroupStarted {
            rule_id: group.rule_id,
            targets: group.targets.len(),
        });

        for target in &group.targets {
            if cancel.is_cancelled() {
                report.cancelled = true;
                on_event(CleanupEvent::GroupFinished {
                    rule_id: group.rule_id,
                });
                break 'groups;
            }

            let outcome = act(plan, disk, group.action, target);
            let result = TargetResult {
                rule_id: group.rule_id,
                path: target.display_path(),
                exact: target.path().to_vec(),
                identity: target.identity(),
                is_dir: target.is_dir(),
                allocated: target.allocated(),
                outcome,
            };
            on_event(CleanupEvent::TargetDone(result.clone()));
            report.results.push(result);
        }

        on_event(CleanupEvent::GroupFinished {
            rule_id: group.rule_id,
        });
    }

    // Deletes on btrfs free their space only when the filesystem commits; have it commit now.
    let _ = disk.remover.settle(&plan.scan_root);
    let after = disk.remover.free_bytes(&plan.scan_root).ok();
    report.freed = before
        .zip(after)
        .map(|(before, after)| i128::from(after) - i128::from(before));
    report
}

fn run_tool(
    runner: &dyn ToolRunner,
    tool: crate::action::Tool,
    refused: &mut Option<String>,
) -> TargetOutcome {
    let command = ToolCommand {
        program: tool.program,
        args: tool.args,
        elevated: tool.needs_elevation,
    };
    let shown = format!("{} {}", tool.program, tool.args.join(" "));

    match runner.run(&command) {
        Ok(()) => TargetOutcome::ByTool(shown),
        Err(ToolFailure::NotInstalled) => {
            TargetOutcome::Skipped(format!("{} is not installed", tool.program))
        }
        Err(failure @ (ToolFailure::NotAuthorized | ToolFailure::NoElevation)) => {
            let why = if failure == ToolFailure::NotAuthorized {
                "the administrator password was not given"
            } else {
                "this system cannot ask for the administrator password (pkexec is missing)"
            };
            *refused = Some(why.to_owned());
            TargetOutcome::Skipped(why.to_owned())
        }
        Err(ToolFailure::Failed { code, message }) => {
            let code = code.map_or_else(|| "a signal".to_owned(), |code| format!("code {code}"));
            TargetOutcome::Failed(format!("{shown} ended with {code}: {message}"))
        }
        Err(ToolFailure::Io(error)) => TargetOutcome::Failed(format!("{shown} could not start: {error}")),
    }
}

fn act<P: EntryProbe, R: EntryRemover>(
    plan: &CleanupPlan,
    disk: &Disk<'_, P, R>,
    action: CleanupAction,
    target: &VerifiedTarget,
) -> TargetOutcome {
    if let Err(what) = still_the_same(plan, disk.probe, target) {
        return TargetOutcome::Skipped(what);
    }

    let (dev, ino) = target.identity();
    let expected = ExpectedEntry {
        dev,
        ino,
        mount_id: target.mount_id(),
        is_dir: target.is_dir(),
        root_depth: plan.root_depth,
    };

    let result = match action {
        CleanupAction::Delete => disk.remover.remove(target.path(), &expected, RemoveMode::Whole),
        CleanupAction::EmptyFolder if target.is_dir() => {
            disk.remover
                .remove(target.path(), &expected, RemoveMode::Contents)
        }
        CleanupAction::EmptyFolder => {
            return TargetOutcome::Skipped("only a folder can be emptied".to_owned());
        }
        CleanupAction::Trash => {
            let Some(trash_dir) = disk.trash_dir else {
                return TargetOutcome::Skipped("no trash folder is known".to_owned());
            };
            return match disk.remover.trash(target.path(), &expected, trash_dir) {
                Ok(to) => TargetOutcome::Trashed(to),
                Err(error) => TargetOutcome::Failed(error.to_string()),
            };
        }
        CleanupAction::EmptyTrash => return empty_trash(plan, disk, target),
        CleanupAction::RunTool(_) => return TargetOutcome::Skipped("tools are not run yet".to_owned()),
    };

    match result {
        Ok(removed) => TargetOutcome::Removed(removed),
        Err(error) => TargetOutcome::Failed(error.to_string()),
    }
}

/// The parts of a freedesktop trash folder that hold trashed items; each is emptied, never removed.
const TRASH_PARTS: [&[u8]; 3] = [b"files", b"info", b"expunged"];

/// Empties the trash folder `target`: the contents of its parts, and its `directorysizes` cache. A part
/// that does not exist is skipped; one that is a link, another mount or on another disk is refused.
fn empty_trash<P: EntryProbe, R: EntryRemover>(
    plan: &CleanupPlan,
    disk: &Disk<'_, P, R>,
    target: &VerifiedTarget,
) -> TargetOutcome {
    if !target.is_dir() {
        return TargetOutcome::Skipped("a trash folder is a folder".to_owned());
    }

    let mut total = Removed::default();
    let parts = TRASH_PARTS
        .iter()
        .map(|part| (*part, true, RemoveMode::Contents))
        .chain([(&b"directorysizes"[..], false, RemoveMode::Whole)]);

    for (name, dir, mode) in parts {
        let path = join(target.path(), name);
        match part_of(disk.probe, target, &path, dir) {
            Ok(Some(mut expected)) => {
                expected.root_depth = plan.root_depth;
                match disk.remover.remove(&path, &expected, mode) {
                    Ok(removed) => total.absorb(removed),
                    Err(error) => total.note(path, error.to_string()),
                }
            }
            Ok(None) => {}
            Err(what) => total.note(path, what),
        }
    }

    TargetOutcome::Removed(total)
}

/// What `path`, a part of the trash folder `target`, is now: `None` when it does not exist, an error
/// when it is not a plain folder (or file, for `dir == false`) on the trash folder's own disk.
fn part_of(
    probe: &impl EntryProbe,
    target: &VerifiedTarget,
    path: &[u8],
    dir: bool,
) -> Result<Option<ExpectedEntry>, String> {
    let found = match probe.probe(path) {
        Ok(found) => found,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let Some(now) = found.last() else {
        return Err("the path is empty".to_owned());
    };

    let wanted = if dir { EntryKind::Dir } else { EntryKind::File };
    if now.meta.kind != wanted {
        return Err("not what a trash folder holds here (a link or another kind of entry)".to_owned());
    }
    if now.meta.is_mount_root || now.meta.dev != target.identity().0 {
        return Err("on another disk than the trash folder".to_owned());
    }
    // The folder holding it, from the same look at the disk, must be the trash folder the plan checked.
    let holder = found.len().checked_sub(2).and_then(|index| found.get(index));
    if holder.is_none_or(|holder| (holder.meta.dev, holder.meta.ino) != target.identity()) {
        return Err("the trash folder was replaced since the plan".to_owned());
    }

    Ok(Some(ExpectedEntry {
        dev: now.meta.dev,
        ino: now.meta.ino,
        mount_id: now.mount_id,
        is_dir: dir,
        root_depth: 0,
    }))
}

fn join(path: &[u8], name: &[u8]) -> Vec<u8> {
    let mut joined = path.to_vec();
    joined.push(b'/');
    joined.extend_from_slice(name);
    joined
}

/// The probe's view of the target now, compared with the plan. The remover checks again as it acts;
/// this catches a change before any trash record or directory is touched.
fn still_the_same(
    plan: &CleanupPlan,
    probe: &impl EntryProbe,
    target: &VerifiedTarget,
) -> Result<(), String> {
    let found = probe
        .probe(target.path())
        .map_err(|error| format!("changed since the plan: {error}"))?;
    let Some(now) = found.last() else {
        return Err("changed since the plan: the path is empty".to_owned());
    };

    if found
        .get(plan.root_depth..)
        .is_none_or(|below| below.iter().any(|entry| entry.meta.is_mount_root))
    {
        return Err("changed since the plan: a mount lies on the way".to_owned());
    }
    let kind_ok = match now.meta.kind {
        EntryKind::Dir => target.is_dir(),
        EntryKind::File => !target.is_dir(),
        _ => false,
    };
    if !kind_ok || (now.meta.dev, now.meta.ino) != target.identity() {
        return Err("changed since the plan: another entry now has its name".to_owned());
    }
    if !target.is_dir() && !same_file(&now.meta, target.mtime(), target.apparent()) {
        return Err("changed since the plan: it was modified".to_owned());
    }
    markers_present(probe, target.markers()).map_err(|what| format!("changed since the plan: {what}"))?;

    Ok(())
}
