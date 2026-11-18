//! Carrying out a plan. Each target is checked again right before it is acted on; a run can be
//! cancelled between targets, never in the middle of one; a failure on one target does not stop the
//! others.

use std::path::Path;

use platter_core::ports::{ChildTrashed, not_recycled, unconfirmed};
use platter_core::{
    CancelToken, EntryKind, EntryProbe, EntryRemover, ExpectedEntry, Os, RemoveMode, Removed, ToolCommand,
    ToolFailure, ToolRunner,
};

use crate::action::CleanupAction;
use crate::guard::{VerifiedTarget, markers_present, same_file};
use crate::path::{PathStyle, display};
use crate::plan::{CleanupPlan, PlanGroup};

/// How many direct children of one folder a run moves to the trash at most (`TrashChildren`): the ones it
/// leaves alone (links, in use) are not counted. The rest are left for the next run, and the report says so.
pub const MAX_TRASHED_CHILDREN: usize = 2000;

/// What happened to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetOutcome {
    /// Deleted (or emptied). `errors` lists what inside could not be removed.
    Removed(Removed),
    /// Gone from its place, but there is no item in the trash that Platter can put back: the system deleted
    /// it for good (too large for the Recycle Bin, say), or it may be in the trash and cannot be told from
    /// other items, or could not be checked afterwards. The reason. Never recorded as trashed.
    NotRecycled(String),
    /// The system did not answer, so it is not known whether the entry was moved (it may be waiting on a
    /// prompt nobody sees). Not a removal: nothing is counted as freed and nothing is offered for restoring.
    /// The reason.
    Unconfirmed(String),
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
    /// The user's trash folder (`~/.Trash` on macOS: the remover itself picks the `.Trashes/<uid>` of
    /// another volume); `None` refuses every move to the trash. Not needed on Windows, where the Recycle
    /// Bin is the shell's and the remover ignores it.
    pub trash_dir: Option<&'a Path>,
    /// Runs the tools that own a cache; `None` leaves those rules undone.
    pub tools: Option<&'a dyn ToolRunner>,
}

/// Puts one entry that a run moved to the trash (`trashed`) back at `original`, when it is still the
/// entry the plan saw (`identity`, and whether it `is_dir`) and nothing has taken its old name since.
/// A name that is taken is refused (see [`EntryRemover::restore`] for how far that holds on each OS). The
/// one way an entry is put back, for a run still in memory ([`undo`]) and for one read back from the
/// history.
pub fn restore_one<R: EntryRemover>(
    remover: &R,
    trashed: &[u8],
    original: &[u8],
    identity: (u64, u64),
    is_dir: bool,
) -> Result<(), String> {
    let expected = ExpectedEntry {
        dev: identity.0,
        ino: identity.1,
        mount_id: None,
        birth: None,
        is_dir,
        root_depth: 0,
    };

    remover
        .restore(trashed, &expected, original)
        .map_err(|error| error.to_string())
}

/// Puts back what a run moved to the trash (`results` of its report). Each entry is checked against
/// what the plan saw and refuses an old place that is taken. Returns, per trashed entry,
/// its readable path and what happened.
pub fn undo<R: EntryRemover>(results: &[TargetResult], remover: &R) -> Vec<(String, Result<(), String>)> {
    results
        .iter()
        .filter_map(|result| match &result.outcome {
            TargetOutcome::Trashed(to) => Some((result, to)),
            _ => None,
        })
        .map(|(result, to)| {
            let outcome = restore_one(remover, to, &result.exact, result.identity, result.is_dir);
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

            // A target is one result, except a folder emptied by moving its children to the trash: each
            // child is a result of its own, so that each can be put back.
            for result in act(plan, disk, group, target) {
                on_event(CleanupEvent::TargetDone(result.clone()));
                report.results.push(result);
            }
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
    group: &PlanGroup,
    target: &VerifiedTarget,
) -> Vec<TargetResult> {
    let own = |outcome| TargetResult {
        rule_id: group.rule_id,
        path: target.display_path(),
        exact: target.path().to_vec(),
        identity: target.identity(),
        is_dir: target.is_dir(),
        allocated: target.allocated(),
        outcome,
    };

    if let Err(what) = still_the_same(plan, disk.probe, target) {
        return vec![own(TargetOutcome::Skipped(what))];
    }

    let (dev, ino) = target.identity();
    let expected = ExpectedEntry {
        dev,
        ino,
        mount_id: target.mount_id(),
        birth: target.birth(),
        is_dir: target.is_dir(),
        root_depth: plan.root_depth,
    };

    let result = match group.action {
        CleanupAction::Delete => disk.remover.remove(target.path(), &expected, RemoveMode::Whole),
        CleanupAction::EmptyFolder if target.is_dir() => {
            disk.remover
                .remove(target.path(), &expected, RemoveMode::Contents)
        }
        CleanupAction::EmptyFolder => {
            return vec![own(TargetOutcome::Skipped(
                "only a folder can be emptied".to_owned(),
            ))];
        }
        CleanupAction::Trash => {
            let Some(trash_dir) = trash_dir_for(plan, disk) else {
                return vec![own(TargetOutcome::Skipped("no trash folder is known".to_owned()))];
            };
            let outcome = match disk.remover.trash(target.path(), &expected, trash_dir) {
                Ok(to) => TargetOutcome::Trashed(to),
                Err(error) => match (not_recycled(&error), unconfirmed(&error)) {
                    (Some(why), _) => TargetOutcome::NotRecycled(why.to_owned()),
                    (None, Some(why)) => TargetOutcome::Unconfirmed(why.to_owned()),
                    (None, None) => TargetOutcome::Failed(error.to_string()),
                },
            };
            return vec![own(outcome)];
        }
        CleanupAction::TrashChildren => return trash_children(plan, disk, group, target, &expected),
        CleanupAction::EmptyTrash => return vec![own(empty_trash(plan, disk, target))],
        CleanupAction::RunTool(_) => {
            return vec![own(TargetOutcome::Skipped("tools are not run yet".to_owned()))];
        }
    };

    vec![own(match result {
        Ok(removed) => TargetOutcome::Removed(removed),
        Err(error) => TargetOutcome::Failed(error.to_string()),
    })]
}

/// The folder a move to the trash goes under: the user's trash folder (on macOS `~/.Trash`, which is
/// always known when the home is), or on Windows nothing in particular (the Recycle Bin is the shell's
/// and the remover ignores it).
fn trash_dir_for<'a, P: EntryProbe, R: EntryRemover>(
    plan: &CleanupPlan,
    disk: &Disk<'a, P, R>,
) -> Option<&'a Path> {
    match (disk.trash_dir, plan.os) {
        (Some(dir), _) => Some(dir),
        (None, Os::Windows) => Some(Path::new("")),
        (None, _) => None,
    }
}

/// Moves every direct child of the folder `target` to the trash, one result each. A folder with nothing
/// inside (or that cannot be emptied) is one result of its own.
fn trash_children<P: EntryProbe, R: EntryRemover>(
    plan: &CleanupPlan,
    disk: &Disk<'_, P, R>,
    group: &PlanGroup,
    target: &VerifiedTarget,
    expected: &ExpectedEntry,
) -> Vec<TargetResult> {
    let own = |outcome| TargetResult {
        rule_id: group.rule_id,
        path: target.display_path(),
        exact: target.path().to_vec(),
        identity: target.identity(),
        is_dir: target.is_dir(),
        allocated: target.allocated(),
        outcome,
    };

    if !target.is_dir() {
        return vec![own(TargetOutcome::Skipped(
            "only a folder can be emptied".to_owned(),
        ))];
    }
    let Some(trash_dir) = trash_dir_for(plan, disk) else {
        return vec![own(TargetOutcome::Skipped("no trash folder is known".to_owned()))];
    };

    let done = match disk
        .remover
        .trash_children(target.path(), expected, trash_dir, MAX_TRASHED_CHILDREN)
    {
        Ok(done) => done,
        Err(error) => return vec![own(TargetOutcome::Failed(error.to_string()))],
    };

    let style = PathStyle::for_os(plan.os);
    let mut results: Vec<TargetResult> = done
        .children
        .into_iter()
        .map(|child| {
            let exact = style.child(target.path(), &child.name);
            let outcome = match child.outcome {
                ChildTrashed::To(to) => TargetOutcome::Trashed(to),
                ChildTrashed::NotRecycled(why) => TargetOutcome::NotRecycled(why),
                ChildTrashed::Unconfirmed(why) => TargetOutcome::Unconfirmed(why),
                ChildTrashed::Left(why) => TargetOutcome::Skipped(why),
            };

            TargetResult {
                rule_id: group.rule_id,
                path: display(&exact),
                exact,
                identity: (child.dev, child.ino),
                is_dir: child.is_dir,
                allocated: child.allocated,
                outcome,
            }
        })
        .collect();

    if done.more > 0 {
        results.push(own(TargetOutcome::Skipped(format!(
            "{} more entries inside were left for the next cleanup",
            done.more
        ))));
    } else if results.is_empty() {
        results.push(own(TargetOutcome::Skipped(
            "there is nothing inside it".to_owned(),
        )));
    }

    results
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
        let path = PathStyle::for_os(plan.os).child(target.path(), name);
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
        birth: now.birth,
        is_dir: dir,
        root_depth: 0,
    }))
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
    let reborn = matches!((now.birth, target.birth()), (Some(now), Some(planned)) if now != planned);
    if !kind_ok || (now.meta.dev, now.meta.ino) != target.identity() || reborn {
        return Err("changed since the plan: another entry now has its name".to_owned());
    }
    if !target.is_dir() && !same_file(&now.meta, target.mtime(), target.apparent()) {
        return Err("changed since the plan: it was modified".to_owned());
    }
    markers_present(probe, PathStyle::for_os(plan.os), target.markers())
        .map_err(|what| format!("changed since the plan: {what}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Mutex;

    use platter_core::ports::{NotRecycled, TrashedChild, TrashedChildren, Unconfirmed};
    use platter_core::{EntryMeta, ProbedEntry};

    use super::*;

    /// A disk where every path exists as a folder (the last entry as `kind`) with one identity.
    struct Disk {
        style: PathStyle,
        identity: (u64, u64),
    }

    impl EntryProbe for Disk {
        fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
            let parts = self.style.components(path).ok_or(io::ErrorKind::InvalidInput)?;

            Ok((0..parts.len())
                .map(|position| {
                    let last = position + 1 == parts.len();
                    ProbedEntry {
                        meta: EntryMeta {
                            kind: EntryKind::Dir,
                            allocated: 0,
                            apparent: 0,
                            mtime: 0,
                            dev: self.identity.0,
                            ino: if last { self.identity.1 } else { 1 },
                            nlink: 1,
                            is_mount_root: false,
                        },
                        mount_id: None,
                        birth: None,
                    }
                })
                .collect())
        }
    }

    /// A remover that answers as told and remembers what it was asked.
    #[derive(Default)]
    struct Bin {
        children: Mutex<Option<io::Result<TrashedChildren>>>,
        single: Mutex<Option<io::Result<Vec<u8>>>>,
        asked: Mutex<Vec<(Vec<u8>, std::path::PathBuf, usize)>>,
    }

    impl EntryRemover for Bin {
        fn remove(&self, _: &[u8], _: &ExpectedEntry, _: RemoveMode) -> io::Result<Removed> {
            panic!("nothing is deleted for good on Windows");
        }

        fn trash(&self, path: &[u8], _: &ExpectedEntry, trash_dir: &Path) -> io::Result<Vec<u8>> {
            self.asked
                .lock()
                .unwrap()
                .push((path.to_vec(), trash_dir.to_path_buf(), 0));
            self.single.lock().unwrap().take().expect("a trash was expected")
        }

        fn trash_children(
            &self,
            path: &[u8],
            _: &ExpectedEntry,
            trash_dir: &Path,
            limit: usize,
        ) -> io::Result<TrashedChildren> {
            self.asked
                .lock()
                .unwrap()
                .push((path.to_vec(), trash_dir.to_path_buf(), limit));
            self.children
                .lock()
                .unwrap()
                .take()
                .expect("children were expected")
        }

        fn free_bytes(&self, _: &Path) -> io::Result<u64> {
            Ok(0)
        }
    }

    fn plan_of(action: CleanupAction, target: VerifiedTarget, os: Os) -> CleanupPlan {
        let style = PathStyle::for_os(os);
        let mut plan = CleanupPlan {
            os,
            root_depth: style.components(target.path()).map_or(0, |parts| parts.len() - 1),
            ..CleanupPlan::default()
        };
        plan.groups.push(PlanGroup {
            rule_id: "win-temp",
            verdict: platter_core::dto::Verdict::Safe,
            action,
            targets: vec![target],
            allocated: 1000,
        });

        plan
    }

    fn run(plan: &CleanupPlan, bin: &Bin) -> CleanupReport {
        let probe = Disk {
            style: PathStyle::for_os(plan.os),
            identity: (7, 9),
        };
        let disk = Disk_ {
            probe: &probe,
            remover: bin,
            trash_dir: None,
            tools: None,
        };

        execute(plan, &disk, &CancelToken::new(), &mut |_| {})
    }

    type Disk_<'a> = super::Disk<'a, Disk, Bin>;

    const TEMP: &[u8] = b"C:\\Users\\u\\AppData\\Local\\Temp";

    fn child(name: &str, outcome: ChildTrashed) -> TrashedChild {
        TrashedChild {
            name: name.as_bytes().to_vec(),
            dev: 7,
            ino: 100,
            is_dir: false,
            allocated: 10,
            outcome,
        }
    }

    #[test]
    fn a_folder_emptied_into_the_bin_is_one_result_per_child_with_its_own_path() {
        let plan = plan_of(
            CleanupAction::TrashChildren,
            VerifiedTarget::for_test(TEMP, true, (7, 9)),
            Os::Windows,
        );
        let bin = Bin::default();
        *bin.children.lock().unwrap() = Some(Ok(TrashedChildren {
            children: vec![
                child(
                    "a.tmp",
                    ChildTrashed::To(b"C:\\$Recycle.Bin\\S-1\\$R1.tmp".to_vec()),
                ),
                child("b.tmp", ChildTrashed::Left("it is in use".to_owned())),
                child(
                    "c.tmp",
                    ChildTrashed::NotRecycled("it is gone but not in the bin".to_owned()),
                ),
            ],
            more: 0,
        }));

        let report = run(&plan, &bin);

        assert_eq!(report.results.len(), 3);
        let first = &report.results[0];
        assert_eq!(first.exact, b"C:\\Users\\u\\AppData\\Local\\Temp\\a.tmp");
        assert_eq!(first.path, "C:\\Users\\u\\AppData\\Local\\Temp\\a.tmp");
        assert_eq!(first.identity, (7, 100));
        assert_eq!(
            first.outcome,
            TargetOutcome::Trashed(b"C:\\$Recycle.Bin\\S-1\\$R1.tmp".to_vec())
        );
        assert_eq!(
            report.results[1].outcome,
            TargetOutcome::Skipped("it is in use".to_owned())
        );
        let TargetOutcome::NotRecycled(why) = &report.results[2].outcome else {
            panic!("{:?}", report.results[2].outcome)
        };
        assert!(why.contains("not in the bin"), "{why}");
    }

    #[test]
    fn the_windows_remover_is_asked_without_a_trash_folder_and_with_the_limit() {
        let plan = plan_of(
            CleanupAction::TrashChildren,
            VerifiedTarget::for_test(TEMP, true, (7, 9)),
            Os::Windows,
        );
        let bin = Bin::default();
        *bin.children.lock().unwrap() = Some(Ok(TrashedChildren {
            children: vec![child("a.tmp", ChildTrashed::To(b"C:\\x".to_vec()))],
            more: 5,
        }));

        let report = run(&plan, &bin);

        let asked = bin.asked.lock().unwrap();
        assert_eq!(asked[0].0, TEMP);
        assert_eq!(
            asked[0].1,
            std::path::PathBuf::new(),
            "the Recycle Bin is the shell's"
        );
        assert_eq!(asked[0].2, MAX_TRASHED_CHILDREN);
        let last = report.results.last().unwrap();
        assert_eq!(last.path, String::from_utf8_lossy(TEMP));
        assert_eq!(
            last.outcome,
            TargetOutcome::Skipped("5 more entries inside were left for the next cleanup".to_owned())
        );
    }

    #[test]
    fn a_folder_with_nothing_inside_or_that_cannot_be_emptied_is_one_result() {
        let plan = plan_of(
            CleanupAction::TrashChildren,
            VerifiedTarget::for_test(TEMP, true, (7, 9)),
            Os::Windows,
        );

        let empty = Bin::default();
        *empty.children.lock().unwrap() = Some(Ok(TrashedChildren::default()));
        let report = run(&plan, &empty);
        assert_eq!(
            report.results[0].outcome,
            TargetOutcome::Skipped("there is nothing inside it".to_owned())
        );

        let refused = Bin::default();
        *refused.children.lock().unwrap() = Some(Err(io::Error::other("changed since the plan: a mount")));
        let report = run(&plan, &refused);
        assert_eq!(report.results.len(), 1);
        assert!(matches!(&report.results[0].outcome, TargetOutcome::Failed(why) if why.contains("a mount")));
    }

    #[test]
    fn a_single_entry_is_trashed_and_one_the_system_took_without_recycling_is_not_called_trashed() {
        let file = b"C:\\Users\\u\\Downloads\\old.iso";
        let plan = plan_of(
            CleanupAction::Trash,
            VerifiedTarget::for_test(file, false, (7, 9)),
            Os::Windows,
        );
        // The disk of this test reports folders; a file target is seen as changed.
        let bin = Bin::default();
        let report = run(&plan, &bin);
        assert!(
            matches!(&report.results[0].outcome, TargetOutcome::Skipped(why) if why.contains("another entry"))
        );
        assert!(
            bin.asked.lock().unwrap().is_empty(),
            "a changed target is never handed over"
        );

        let plan = plan_of(
            CleanupAction::Trash,
            VerifiedTarget::for_test(TEMP, true, (7, 9)),
            Os::Windows,
        );
        let bin = Bin::default();
        *bin.single.lock().unwrap() = Some(Err(NotRecycled("deleted for good".to_owned()).into_error()));
        let report = run(&plan, &bin);
        assert_eq!(
            report.results[0].outcome,
            TargetOutcome::NotRecycled("deleted for good".to_owned())
        );

        let bin = Bin::default();
        *bin.single.lock().unwrap() = Some(Err(Unconfirmed("it may be waiting".to_owned()).into_error()));
        let report = run(&plan, &bin);
        assert_eq!(
            report.results[0].outcome,
            TargetOutcome::Unconfirmed("it may be waiting".to_owned()),
            "an unknown result is neither a removal nor a plain failure"
        );

        let bin = Bin::default();
        *bin.single.lock().unwrap() = Some(Ok(b"C:\\$Recycle.Bin\\S-1\\$R2".to_vec()));
        let report = run(&plan, &bin);
        assert_eq!(
            report.results[0].outcome,
            TargetOutcome::Trashed(b"C:\\$Recycle.Bin\\S-1\\$R2".to_vec())
        );
    }

    #[test]
    fn on_linux_a_move_to_the_trash_still_needs_a_trash_folder() {
        let plan = plan_of(
            CleanupAction::Trash,
            VerifiedTarget::for_test(b"/home/u/x", true, (7, 9)),
            Os::Linux,
        );
        let bin = Bin::default();

        let report = run(&plan, &bin);

        assert_eq!(
            report.results[0].outcome,
            TargetOutcome::Skipped("no trash folder is known".to_owned())
        );
    }
}
