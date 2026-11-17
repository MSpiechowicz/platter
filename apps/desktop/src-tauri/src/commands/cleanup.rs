//! Cleanup over IPC (C4). The web view never names a path, an action or a program: it asks for a plan
//! of a scan's suggestions by rule id, gets back a description of that plan and its id, and starts the
//! plan by id. The plan itself stays here.
//!
//! - A plan belongs to one scan, is used once, and expires after [`PLAN_LIFETIME`]. A scan that a
//!   cleanup has run on is outdated: no new plan is made for it, and an older plan of it is refused.
//! - Only one cleanup runs at a time.
//! - Before any plan runs, moving to the trash included, the backend asks the user in a native dialog
//!   that it builds from its own plan.
//! - `undo_cleanup` puts back what one run moved to the trash, from the results that run recorded. It asks
//!   the user first, in the same kind of dialog, listing the places it puts things back to.
//! - Each run that did something is also recorded in the history (see `history`), which can put its
//!   trashed items back too. The two exclude each other (and a running cleanup), and an item is put back
//!   once, whichever asks first.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use platter_cleanup::{
    ADVISOR, CleanupAction, CleanupEvent, CleanupOptions, CleanupPlan, CleanupReport, Disk, KeepVersions,
    KeptFolders, PathStyle, TargetOutcome, TargetResult, execute, plan_cleanup_with as make_plan,
    plan_labelled_with, plan_marked, undo,
};
use platter_core::dto::{ScanId, Verdict};
use platter_core::{CancelToken, ModelLabel, ModelVerdict, NodeId, Os};
use platter_platform::environment::home_trash;
use platter_rules::RuleSet;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

use crate::commands::advisor::Review;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;
use crate::user_rules;

pub type PlanId = u64;
pub type CleanupId = u64;

/// How long a plan may wait to be started.
pub const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// Plans kept at once; making another drops the oldest.
const MAX_PLANS: usize = 4;
/// Finished runs kept for `undo_cleanup`.
const MAX_DONE: usize = 4;
/// Rule ids one plan may name.
const MAX_RULES: usize = 64;
/// Items one marked plan may name.
const MAX_MARKED: usize = 1000;
/// Refusals listed in a plan.
const MAX_REFUSED: usize = 200;
/// Longest path shown in the confirmation, in characters; longer ones are shortened in the middle.
const SHOWN_PATH_CHARS: usize = 120;
/// Longest confirmation text, in characters.
const QUESTION_CHARS: usize = 6000;
/// Paths listed per group in a plan and in the confirmation.
const SAMPLE_PATHS: usize = 20;
/// The largest integer a JavaScript number holds exactly; sizes are clamped to it on the wire.
const MAX_SAFE: u64 = 9_007_199_254_740_991;
/// The title of the group the model's labels make (the same as its suggestion's).
const ADVISOR_TITLE: &str = "Labelled by the local model";

/// Asks the user to approve what the web view cannot approve for them: a cleanup that cannot be undone,
/// the model download. Shown by the backend, never by the web view, so a compromised page cannot approve
/// on the user's behalf.
pub trait Confirm: Send + Sync {
    fn confirm(&self, question: &Question) -> bool;
}

/// What a confirmation shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub title: String,
    pub text: String,
    /// The label of the button that approves ("Clean up", "Download"); the other is always "Cancel".
    pub accept: &'static str,
}

/// Plans waiting to be started, the running cleanup, and finished runs that can be undone.
#[derive(Default)]
pub struct CleanupRegistry {
    inner: Mutex<Inner>,
    /// Set while a plan is made: one at a time, so the web view cannot fill the blocking pool.
    planning: AtomicBool,
}

#[derive(Default)]
struct Inner {
    last_plan: PlanId,
    plans: HashMap<PlanId, Stored>,
    last_cleanup: CleanupId,
    running: Option<(CleanupId, CancelToken)>,
    /// The running cleanup got past its confirmation and is acting on the disk.
    acting: bool,
    done: Vec<Done>,
    /// Finished runs in `done` whose history record is still being written: no undo until it is settled.
    awaiting_history: HashSet<CleanupId>,
    /// When the user last declined a confirmation: a new one is not shown right away.
    declined: Option<Instant>,
}

/// A finished run that can be undone: what it moved to the trash.
#[derive(Debug)]
pub(crate) struct Done {
    pub cleanup: CleanupId,
    /// The id the history keeps the run under, when it was recorded there.
    pub history: Option<i64>,
    pub results: Vec<TargetResult>,
}

struct Stored {
    scan: ScanId,
    plan: Arc<CleanupPlan>,
    made: Instant,
    /// The generation of the user's rules the plan was made from.
    rules_generation: u64,
}

/// The single running slot, held from the moment a plan is taken until its run ends. Dropping it
/// frees the slot, so no early return, refusal or panic can leave it taken.
pub struct Reservation {
    registry: Arc<CleanupRegistry>,
    pub cleanup: CleanupId,
    pub cancel: CancelToken,
    pub scan: ScanId,
    pub plan: Arc<CleanupPlan>,
    pub made: Instant,
    /// The generation of the user's rules the plan was made from.
    pub rules_generation: u64,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut inner = self.registry.lock();
        if inner.running.as_ref().is_some_and(|(id, _)| *id == self.cleanup) {
            inner.running = None;
        }
    }
}

/// The one plan being made; dropping it lets the next one start.
struct Planning<'a>(&'a AtomicBool);

impl<'a> Planning<'a> {
    fn begin(registry: &'a CleanupRegistry) -> Option<Self> {
        registry
            .planning
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(&registry.planning))
    }
}

impl Drop for Planning<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// How long after a declined confirmation a new one is refused (no rapid re-asking).
const DECLINE_PAUSE: Duration = Duration::from_secs(2);

impl CleanupRegistry {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Stores a plan made without user rules (generation 0).
    #[cfg(test)]
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn store(&self, scan: ScanId, plan: CleanupPlan) -> PlanId {
        self.store_made_with(scan, plan, 0, |_| true)
            .expect("an unconditional store succeeds")
    }

    /// Stores a plan made from the user's rules of generation `rules_generation`, unless `current` says
    /// the rules changed meanwhile (`invalid`).
    ///
    /// `current` reads the rules file, so it is never asked while the registry is locked (cancelling a
    /// cleanup and quitting take the same lock). It is asked before the plan is stored and again after.
    /// A change of the rules moves the generation first and drops the stored plans after it (see
    /// `drop_plans`): a change before the second question is seen by it and the plan is taken back out;
    /// one after it drops the plan itself. So a plan made from old rules is never left stored.
    pub(crate) fn store_made_with(
        &self,
        scan: ScanId,
        plan: CleanupPlan,
        rules_generation: u64,
        current: impl Fn(u64) -> bool,
    ) -> CommandResult<PlanId> {
        let changed =
            || CommandError::invalid("your rules changed while the plan was made, so make a new plan");
        if !current(rules_generation) {
            return Err(changed());
        }

        let id = {
            let mut inner = self.lock();
            inner.last_plan += 1;
            let id = inner.last_plan;

            inner
                .plans
                .retain(|_, stored| stored.made.elapsed() < PLAN_LIFETIME);
            while inner.plans.len() >= MAX_PLANS {
                let oldest = inner.plans.keys().copied().min().unwrap_or(id);
                inner.plans.remove(&oldest);
            }
            inner.plans.insert(
                id,
                Stored {
                    scan,
                    plan: Arc::new(plan),
                    made: Instant::now(),
                    rules_generation,
                },
            );
            id
        };

        if !current(rules_generation) {
            self.lock().plans.remove(&id);
            return Err(changed());
        }

        Ok(id)
    }

    /// Whether a plan is waiting to be started.
    #[cfg(test)]
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn has_plan(&self, id: PlanId) -> bool {
        self.lock().plans.contains_key(&id)
    }

    /// Forgets every plan waiting to be started: the user's rules changed, so none of them is still what
    /// the rules would plan. A cleanup that is running is not touched.
    pub fn drop_plans(&self) {
        self.lock().plans.clear();
    }

    /// Takes a plan out (it can be started once) and reserves the single running slot for it. A busy
    /// slot is `invalid` and leaves the plan in place; an unknown or used plan is `notFound`.
    ///
    /// `restoring` says whether things are being put back. It is asked with the registry locked and before
    /// the plan is taken, and a restore takes its slot before it looks for a running cleanup: so either this
    /// sees the restore and leaves the plan in place (`invalid`), or the restore sees this cleanup.
    pub(crate) fn take_and_reserve(
        self: &Arc<Self>,
        id: PlanId,
        restoring: &dyn Fn() -> bool,
    ) -> CommandResult<Reservation> {
        let mut inner = self.lock();
        if inner.running.is_some() {
            return Err(CommandError::invalid("a cleanup is already running"));
        }
        if !inner.plans.contains_key(&id) {
            return Err(CommandError::not_found(format!(
                "plan {id} does not exist or was used"
            )));
        }
        // Checked before the plan is taken: a request right after a refusal leaves the plan usable.
        if inner.declined.is_some_and(|at| at.elapsed() < DECLINE_PAUSE) {
            return Err(CommandError::invalid(
                "a cleanup was just declined, so wait a moment",
            ));
        }
        if restoring() {
            return Err(CommandError::invalid(
                "things are being put back, so start the cleanup when that has finished",
            ));
        }
        let stored = inner
            .plans
            .remove(&id)
            .ok_or_else(|| CommandError::not_found(format!("plan {id} does not exist or was used")))?;
        if stored.made.elapsed() >= PLAN_LIFETIME {
            return Err(CommandError::invalid("the plan has expired, so make a new one"));
        }

        inner.last_cleanup += 1;
        let cleanup = inner.last_cleanup;
        let cancel = CancelToken::new();
        inner.running = Some((cleanup, cancel.clone()));
        inner.acting = false;
        Ok(Reservation {
            registry: Arc::clone(self),
            cleanup,
            cancel,
            scan: stored.scan,
            plan: stored.plan,
            made: stored.made,
            rules_generation: stored.rules_generation,
        })
    }

    /// The run passed its confirmation and is about to act on the disk.
    fn acting(&self, cleanup: CleanupId) {
        let mut inner = self.lock();
        if inner.running.as_ref().is_some_and(|(id, _)| *id == cleanup) {
            inner.acting = true;
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.lock().running.is_some()
    }

    fn declined(&self) {
        self.lock().declined = Some(Instant::now());
    }

    /// Keeps a run's results for undo. A run that touched nothing keeps no record (it would only push
    /// a real one out); a run that emptied the trash makes every older record useless.
    #[cfg(test)]
    pub(crate) fn record(
        &self,
        cleanup: CleanupId,
        history: Option<i64>,
        results: Vec<TargetResult>,
        emptied_trash: bool,
    ) {
        keep(&mut self.lock(), cleanup, history, results, emptied_trash);
    }

    /// As `record`, for a run whose history record is written next: it cannot be undone before
    /// [`Self::settle`] says the record is there (or that there is none), so an undo and the history
    /// never both think they own its items.
    pub(crate) fn record_awaiting_history(
        &self,
        cleanup: CleanupId,
        results: Vec<TargetResult>,
        emptied_trash: bool,
    ) {
        let mut inner = self.lock();
        if keep(&mut inner, cleanup, None, results, emptied_trash) {
            inner.awaiting_history.insert(cleanup);
        }
    }

    /// The history record of a run is written (`history` is its id) or will not be: undo may use the run.
    /// Settling twice is fine, and a later settling without an id keeps the id of an earlier one.
    pub(crate) fn settle(&self, cleanup: CleanupId, history: Option<i64>) {
        let mut inner = self.lock();
        inner.awaiting_history.remove(&cleanup);
        if let Some(history) = history
            && let Some(done) = inner.done.iter_mut().find(|done| done.cleanup == cleanup)
        {
            done.history = Some(history);
        }
    }

    fn cancel(&self, cleanup: CleanupId) {
        if let Some((id, cancel)) = &self.lock().running
            && *id == cleanup
        {
            cancel.cancel();
        }
    }

    /// For quitting the app: asks the running cleanup to stop after the target in progress and waits
    /// up to `limit` for it, so no recursive delete is cut off half-way by the process ending.
    pub fn stop_for_exit(&self, limit: Duration) {
        let started = Instant::now();
        {
            let inner = self.lock();
            let Some((_, cancel)) = &inner.running else {
                return;
            };
            cancel.cancel();
            // Still waiting for the user's answer: nothing is acting on the disk, nothing to wait for
            // (and the dialog itself needs the main thread this runs on).
            if !inner.acting {
                return;
            }
        }
        while self.lock().running.is_some() && started.elapsed() < limit {
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// The place of a finished run in `done`, when it can be undone now.
    fn position_of(inner: &Inner, cleanup: CleanupId) -> CommandResult<usize> {
        if inner.awaiting_history.contains(&cleanup) {
            return Err(CommandError::invalid(
                "the cleanup is still being recorded, so undo in a moment",
            ));
        }

        inner
            .done
            .iter()
            .position(|done| done.cleanup == cleanup)
            .ok_or_else(|| CommandError::not_found(format!("cleanup {cleanup} cannot be undone")))
    }

    /// What an undo of a finished run would put back, as the original path and the size of each entry,
    /// without taking the run (so a declined confirmation leaves it as it was).
    pub(crate) fn trashed_by(&self, cleanup: CleanupId) -> CommandResult<Vec<(Vec<u8>, u64)>> {
        let inner = self.lock();
        let position = Self::position_of(&inner, cleanup)?;

        Ok(inner.done[position]
            .results
            .iter()
            .filter(|result| matches!(result.outcome, TargetOutcome::Trashed(_)))
            .map(|result| (result.exact.clone(), result.allocated))
            .collect())
    }

    /// The results of a finished run, taken out: a run is undone once.
    pub(crate) fn take_done(&self, cleanup: CleanupId) -> CommandResult<Done> {
        let mut inner = self.lock();
        let position = Self::position_of(&inner, cleanup)?;

        Ok(inner.done.remove(position))
    }

    /// The results of the run the history keeps under `history`, taken out so that an undo cannot put
    /// them back while the history does; `None` when it is not (or no longer) there.
    pub(crate) fn take_by_history(&self, history: i64) -> Option<Done> {
        let mut inner = self.lock();
        let position = inner.done.iter().position(|done| done.history == Some(history))?;

        Some(inner.done.remove(position))
    }

    /// Puts back the results whose undo failed, so it can be tried again.
    pub(crate) fn keep_for_retry(&self, done: Done) {
        if !done.results.is_empty() {
            self.lock().done.push(done);
        }
    }
}

/// Keeps `results` as a run that can be undone, when it moved something to the trash. Returns whether it
/// did.
fn keep(
    inner: &mut Inner,
    cleanup: CleanupId,
    history: Option<i64>,
    results: Vec<TargetResult>,
    emptied_trash: bool,
) -> bool {
    if emptied_trash {
        inner.done.clear();
        inner.awaiting_history.clear();
    }
    if !results
        .iter()
        .any(|result| matches!(result.outcome, TargetOutcome::Trashed(_)))
    {
        return false;
    }

    inner.done.push(Done {
        cleanup,
        history,
        results,
    });
    if inner.done.len() > MAX_DONE {
        let gone = inner.done.remove(0);
        inner.awaiting_history.remove(&gone.cleanup);
    }

    true
}

// ---- Wire types (sent only; nothing here is ever read back from the web view) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionDto {
    Delete,
    EmptyFolder,
    EmptyTrash,
    Trash,
    RunTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanGroupDto {
    pub rule_id: String,
    pub title: String,
    pub verdict: Verdict,
    pub action: ActionDto,
    pub items: u64,
    pub allocated: u64,
    /// The command a tool runs, as text; absent for other actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub needs_elevation: bool,
    /// The first few paths, for showing.
    pub sample_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefusedDto {
    pub rule_id: String,
    pub path: String,
    pub reason: String,
}

/// The options a plan was made with, as the web view sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupOptionsDto {
    /// 1, 2 or 3.
    pub keep_package_versions: u8,
    pub ask_for_admin: bool,
}

impl From<&CleanupOptions> for CleanupOptionsDto {
    fn from(options: &CleanupOptions) -> Self {
        Self {
            keep_package_versions: options.keep_package_versions.count(),
            ask_for_admin: options.ask_for_admin,
        }
    }
}

/// The options the web view may send with a plan command. A missing option has its default (keep 2
/// versions, ask for administrator rights). Anything else than 1, 2 or 3 for the versions, or a value that
/// is not true or false for the question, is `invalid`: the values are checked here, not trusted.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CleanupOptionsArg {
    keep_package_versions: Option<serde_json::Value>,
    ask_for_admin: Option<serde_json::Value>,
}

impl CleanupOptionsArg {
    /// The options to plan with. `None`, a missing field and `null` each mean the default.
    pub fn resolve(arg: Option<Self>) -> CommandResult<CleanupOptions> {
        let mut options = CleanupOptions::default();
        let Some(arg) = arg else {
            return Ok(options);
        };

        if let Some(value) = arg.keep_package_versions {
            options.keep_package_versions = value
                .as_u64()
                .and_then(KeepVersions::from_count)
                .ok_or_else(|| CommandError::invalid("keep 1, 2 or 3 versions of each package"))?;
        }
        if let Some(value) = arg.ask_for_admin {
            options.ask_for_admin = value
                .as_bool()
                .ok_or_else(|| CommandError::invalid("asking for administrator rights is on or off"))?;
        }

        Ok(options)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPlanDto {
    pub plan_id: PlanId,
    pub scan_id: ScanId,
    pub groups: Vec<PlanGroupDto>,
    /// At most [`MAX_REFUSED`] refusals; `refused_more` counts the rest.
    pub refused: Vec<RefusedDto>,
    pub refused_more: u64,
    pub not_found: Vec<String>,
    pub allocated: u64,
    /// The options the plan was made with. The tools in `groups` were picked with them.
    pub options: CleanupOptionsDto,
    /// Starting it asks the user first (something cannot be undone).
    pub needs_confirmation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OutcomeKind {
    Removed,
    Trashed,
    ByTool,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeDto {
    pub kind: OutcomeKind,
    /// Why it was skipped or failed, or the command a tool ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Problems inside the target (entries that could not be removed, mounts left alone).
    pub problems: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CleanupEventDto {
    GroupStarted {
        rule_id: String,
        targets: u64,
    },
    TargetDone {
        rule_id: String,
        path: String,
        allocated: u64,
        outcome: OutcomeDto,
    },
    GroupFinished {
        rule_id: String,
    },
    Finished {
        cleanup_id: CleanupId,
        /// The scan the cleanup ran on is outdated now: rescan to see the disk as it is.
        outdated_scan: ScanId,
        cleaned: u64,
        with_problems: u64,
        /// Free space after minus before (absent when it could not be measured).
        #[serde(skip_serializing_if = "Option::is_none")]
        freed: Option<i64>,
        cancelled: bool,
        tools_not_run: Vec<String>,
        /// Something was moved to the trash and can be put back with `undo_cleanup`.
        can_undo: bool,
    },
    Failed {
        message: String,
        /// The scan the cleanup ran on is outdated now (as after `finished`).
        outdated_scan: ScanId,
        /// What reached the trash before it stopped can be put back with `undo_cleanup`.
        can_undo: bool,
        cleanup_id: CleanupId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoDto {
    pub path: String,
    pub restored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---- Commands ----

/// Plans the cleanup of `suggestions` (rule ids from `get_suggestions`) of a finished scan. The optional
/// `options` (`{ keepPackageVersions: 1 | 2 | 3, askForAdmin: bool }`) are fixed into the plan.
#[tauri::command]
pub async fn plan_cleanup(
    state: State<'_, AppState>,
    scan: ScanId,
    suggestions: Vec<String>,
    options: Option<CleanupOptionsArg>,
) -> CommandResult<CleanupPlanDto> {
    plan(&state, scan, suggestions, CleanupOptionsArg::resolve(options)?).await
}

/// Plans moving what the user marked in the Explorer (node ids of a finished scan) to the trash. Each
/// item is a `review` item, checked like a rule's match; started and confirmed like any other plan.
#[tauri::command]
pub async fn plan_marked_cleanup(
    state: State<'_, AppState>,
    scan: ScanId,
    nodes: Vec<NodeId>,
    options: Option<CleanupOptionsArg>,
) -> CommandResult<CleanupPlanDto> {
    plan_marked_nodes(&state, scan, nodes, CleanupOptionsArg::resolve(options)?).await
}

/// Starts a plan (once). Asks for confirmation first when something cannot be undone; a refused
/// confirmation is the error `cancelled`. Events arrive on `on_event`, the last one `finished` or
/// `failed`.
#[tauri::command]
pub async fn start_cleanup(
    state: State<'_, AppState>,
    plan: PlanId,
    on_event: Channel<CleanupEventDto>,
) -> CommandResult<CleanupId> {
    let emit: Emit = Arc::new(move |event| {
        let _ = on_event.send(event);
    });
    start(&state, plan, emit).await
}

/// Stops a running cleanup after the target in progress. Unknown or ended ids are not an error.
#[tauri::command]
pub fn cancel_cleanup(state: State<'_, AppState>, cleanup: CleanupId) -> CommandResult<()> {
    state.cleanups.cancel(cleanup);
    Ok(())
}

/// Puts back what a finished cleanup moved to the trash (once). Asks for confirmation first, in a native
/// dialog listing what goes back: a refused confirmation is the error `cancelled`, and leaves the run
/// as it was.
#[tauri::command]
pub async fn undo_cleanup(state: State<'_, AppState>, cleanup: CleanupId) -> CommandResult<Vec<UndoDto>> {
    undo_run(&state, cleanup).await
}

pub async fn undo_run(state: &AppState, cleanup: CleanupId) -> CommandResult<Vec<UndoDto>> {
    // One restore at a time: this and a restore from the history never put the same entry back together.
    let Some(_restoring) = state.history.begin_restore() else {
        return Err(CommandError::invalid(
            "something is being put back, so undo when it has finished",
        ));
    };
    // Not while a cleanup runs: it could be emptying the trash that is being restored from.
    if state.cleanups.is_running() {
        return Err(CommandError::invalid(
            "a cleanup is running, so undo when it has finished",
        ));
    }
    // Looked at, not taken: a refused request shows no dialog, and a declined one leaves the run as it was.
    let asked = state.cleanups.trashed_by(cleanup)?;
    confirm_put_back(state, &asked).await?;
    // The dialog may have been open for a long time. Nothing can have started or put anything back
    // meanwhile (this holds the restoring slot), and the run is checked again as it is taken.
    let done = state.cleanups.take_done(cleanup)?;
    let history = done.history;
    let kept = done.results.clone();
    let results = done.results;
    let platform = Arc::clone(&state.platform);

    let worked = tauri::async_runtime::spawn_blocking(move || {
        let undone = undo(&results, &platform.entry_remover);
        // What could not be put back stays, so undo can be tried again.
        let (failed, back): (Vec<_>, Vec<_>) = results
            .into_iter()
            .filter(|result| matches!(result.outcome, TargetOutcome::Trashed(_)))
            .zip(&undone)
            .partition(|(_, (_, outcome))| outcome.is_err());
        let failed: Vec<TargetResult> = failed.into_iter().map(|(result, _)| result).collect();
        let back: Vec<TargetResult> = back.into_iter().map(|(result, _)| result).collect();
        (undone, failed, back)
    })
    .await;
    let Ok((undone, failed, back)) = worked else {
        // Some entries may be back already: the disk changed, and every entry stays for a retry
        // (one that is back is refused next time, since its trash record is gone).
        state.cleanups.keep_for_retry(Done {
            cleanup,
            history,
            results: kept,
        });
        state.scans.disk_changed();
        return Err(CommandError::io("putting things back stopped unexpectedly"));
    };

    state.cleanups.keep_for_retry(Done {
        cleanup,
        history,
        results: failed,
    });
    if let Some(history) = history {
        let paths: Vec<(Vec<u8>, Vec<u8>)> = back
            .into_iter()
            .filter_map(|result| match result.outcome {
                TargetOutcome::Trashed(to) => Some((result.exact, to)),
                _ => None,
            })
            .collect();
        state.history.mark_undone(history, &paths);
    }
    // Putting things back changed the disk too.
    state.scans.disk_changed();

    Ok(undone
        .into_iter()
        .map(|(path, outcome)| UndoDto {
            path,
            restored: outcome.is_ok(),
            error: outcome.err(),
        })
        .collect())
}

pub type Emit = Arc<dyn Fn(CleanupEventDto) + Send + Sync>;

pub async fn plan(
    state: &AppState,
    scan: ScanId,
    suggestions: Vec<String>,
    options: CleanupOptions,
) -> CommandResult<CleanupPlanDto> {
    // A broken rules file refuses every plan: nothing then says which folders the user keeps.
    let user = user_rules::current(&state.user_rules).await?;
    user.usable()?;
    let rules = user.rule_set();
    if suggestions.is_empty() || suggestions.len() > MAX_RULES {
        return Err(CommandError::invalid(format!(
            "choose between 1 and {MAX_RULES} suggestions"
        )));
    }
    if let Some(unknown) = suggestions
        .iter()
        .find(|id| id.as_str() != ADVISOR && !rules.rules().iter().any(|rule| rule.id == id.as_str()))
    {
        return Err(CommandError::invalid(format!("{unknown} is not a rule")));
    }
    if state.scans.is_outdated(scan) {
        return Err(CommandError::invalid(
            "the disk changed since this scan, so rescan first",
        ));
    }

    let tree = state.scans.tree(scan)?;
    // The `advisor` suggestion exists while the advisor is on and the scan has labels: planning it
    // asks for the same.
    let labels = if suggestions.iter().any(|id| id == ADVISOR) {
        review_labels(state, scan)?
    } else {
        Vec::new()
    };
    let Some(_planning) = Planning::begin(&state.cleanups) else {
        return Err(CommandError::invalid(
            "a plan is being made, so try again in a moment",
        ));
    };
    let platform = Arc::clone(&state.platform);
    let generation = user.generation;
    let plan = tauri::async_runtime::spawn_blocking(move || {
        let env = platform.environment_for(tree.root_path());
        let ids: Vec<&str> = suggestions
            .iter()
            .map(String::as_str)
            .filter(|id| *id != ADVISOR)
            .collect();
        // Learnt once for the request: one overall time limit, however many plans it makes.
        let kept = KeptFolders::learn_for(PathStyle::for_os(env.os), &rules, &platform.entry_probe);
        let mut plan = make_plan(&tree, &env, &rules, &ids, &platform.entry_probe, &kept, &options);

        if !labels.is_empty() {
            // Exactly the labels the suggestion lists, no more: a label a rule already speaks for is
            // the rule's to clean.
            let listed = Review::new(&tree, &platform, user.rules.clone()).listed_labels(&labels);
            if listed.is_empty() {
                return Err(CommandError::invalid("no labelled folder can be cleaned"));
            }
            plan.absorb(plan_labelled_with(
                &tree,
                &env,
                &rules,
                &listed,
                &platform.entry_probe,
                &kept,
                &options,
            ));
        }

        Ok(plan)
    })
    .await??;

    let dto = describe(&plan, scan, 0);
    let id = store_plan(state, scan, plan, generation)?;
    Ok(CleanupPlanDto { plan_id: id, ..dto })
}

/// Stores a plan made from the user's rules of `generation`, refusing it when they changed since.
fn store_plan(state: &AppState, scan: ScanId, plan: CleanupPlan, generation: u64) -> CommandResult<PlanId> {
    state.cleanups.store_made_with(scan, plan, generation, |made| {
        state.user_rules.generation() == made
    })
}

/// The labels of `scan` that `advisor` may be planned from: the advisor must be on, and the scan must
/// have `review` labels (`invalid` otherwise).
fn review_labels(state: &AppState, scan: ScanId) -> CommandResult<Vec<ModelLabel>> {
    if !state.advisor.enabled() {
        return Err(CommandError::invalid(
            "the local model is switched off in the settings",
        ));
    }

    let labels: Vec<ModelLabel> = state
        .scans
        .labels(scan)?
        .map(|set| {
            set.labels
                .iter()
                .filter(|label| label.verdict == ModelVerdict::Review)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if labels.is_empty() {
        return Err(CommandError::invalid(
            "the local model has not labelled any folder to clean in this scan",
        ));
    }

    Ok(labels)
}

/// Plans moving the folders and files the user marked in the Explorer (`nodes`) to the trash.
pub async fn plan_marked_nodes(
    state: &AppState,
    scan: ScanId,
    nodes: Vec<NodeId>,
    options: CleanupOptions,
) -> CommandResult<CleanupPlanDto> {
    let user = user_rules::current(&state.user_rules).await?;
    user.usable()?;
    if nodes.is_empty() || nodes.len() > MAX_MARKED {
        return Err(CommandError::invalid(format!(
            "mark between 1 and {MAX_MARKED} items"
        )));
    }
    if state.scans.is_outdated(scan) {
        return Err(CommandError::invalid(
            "the disk changed since this scan, so rescan first",
        ));
    }

    let tree = state.scans.tree(scan)?;
    if let Some(unknown) = nodes.iter().find(|node| !tree.contains(**node)) {
        return Err(CommandError::not_found(format!("node {unknown} does not exist")));
    }
    let Some(_planning) = Planning::begin(&state.cleanups) else {
        return Err(CommandError::invalid(
            "a plan is being made, so try again in a moment",
        ));
    };
    let platform = Arc::clone(&state.platform);
    let generation = user.generation;
    let plan = tauri::async_runtime::spawn_blocking(move || {
        let env = platform.environment_for(tree.root_path());
        plan_marked(
            &tree,
            &env,
            &user.rule_set(),
            &nodes,
            &platform.entry_probe,
            &options,
        )
    })
    .await?;

    let dto = describe(&plan, scan, 0);
    let id = store_plan(state, scan, plan, generation)?;
    Ok(CleanupPlanDto { plan_id: id, ..dto })
}

pub async fn start(state: &AppState, plan_id: PlanId, emit: Emit) -> CommandResult<CleanupId> {
    // From here on the slot is released when `reservation` is dropped, however this function ends. Not while
    // things are being put back: the registry asks, with its lock held, and then leaves the plan in place.
    let reservation = state
        .cleanups
        .take_and_reserve(plan_id, &|| state.history.is_restoring())?;
    let scan = reservation.scan;
    let plan = Arc::clone(&reservation.plan);

    if plan.groups().is_empty() {
        return Err(CommandError::invalid("the plan has nothing to clean"));
    }
    still_current(state, &reservation)?;

    // Every plan that does something is confirmed by the user in the backend's own dialog, also one
    // that only moves things to the trash (what is in the trash is one "Empty the trash" from gone).
    let Some(question) = question(&plan) else {
        return Err(CommandError::invalid("the plan has nothing to clean"));
    };
    let confirm = state.confirm();
    let approved = tauri::async_runtime::spawn_blocking(move || confirm.confirm(&question))
        .await
        .unwrap_or(false);
    if !approved {
        state.cleanups.declined();
        return Err(CommandError::new(
            platter_core::dto::ApiErrorCode::Cancelled,
            "the cleanup was not confirmed",
        ));
    }
    // The dialog may have been open for a long time.
    still_current(state, &reservation)?;

    let cleanup = reservation.cleanup;
    state.cleanups.acting(cleanup);
    let platform = Arc::clone(&state.platform);
    let scans = Arc::clone(&state.scans);
    let cleanups = Arc::clone(&state.cleanups);
    let history = Arc::clone(&state.history);
    let spawned = thread::Builder::new()
        .name(format!("platter-cleanup-{cleanup}"))
        .spawn(move || {
            let reservation = reservation;
            let results = Mutex::new(Vec::new());
            let emptied_trash = plan
                .groups()
                .iter()
                .any(|group| group.action() == CleanupAction::EmptyTrash);

            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let trash = home_trash(platform.environment().home.as_deref());
                let disk = Disk {
                    probe: &platform.entry_probe,
                    remover: &platform.entry_remover,
                    trash_dir: trash.as_deref(),
                    tools: Some(&platform.tool_runner),
                };
                execute(&plan, &disk, &reservation.cancel, &mut |event| {
                    if let CleanupEvent::TargetDone(result) = &event {
                        lock(&results).push(result.clone());
                    }
                    emit(event_dto(event));
                })
            }));

            // Whatever happened, the disk may have changed: no scan describes it any more.
            scans.disk_changed();
            // Kept for undo even after a panic: what reached the trash so far can still be put back.
            let results = std::mem::take(&mut *lock(&results));
            let event = match &outcome {
                Ok(report) => finished_dto(cleanup, scan, report),
                Err(_) => CleanupEventDto::Failed {
                    message: "the cleanup stopped unexpectedly".to_owned(),
                    outdated_scan: scan,
                    can_undo: results
                        .iter()
                        .any(|result| matches!(result.outcome, TargetOutcome::Trashed(_))),
                    cleanup_id: cleanup,
                },
            };
            let groups: Vec<(&str, CleanupAction)> = plan
                .groups()
                .iter()
                .map(|group| (group.rule_id(), group.action()))
                .collect();
            // Kept for undo first, but not undoable until the history has settled its record (below).
            cleanups.record_awaiting_history(cleanup, results.clone(), emptied_trash);
            // The slot is free before anything touches the history file, so a slow or locked file never
            // holds up the next cleanup.
            drop(reservation);

            // Whatever the history does (it may be slow, or panic), the run is settled and the UI hears of
            // its end.
            let settled = Settled {
                cleanups: &cleanups,
                cleanup,
            };
            history.record_run(plan.scan_root(), &groups, &results, outcome.as_ref().ok(), |id| {
                cleanups.settle(cleanup, Some(id))
            });
            drop(settled);
            emit(event);
        });

    // A failed spawn drops the closure, and with it the reservation.
    spawned?;
    Ok(cleanup)
}

/// Lets undo use a run once the history is done with it, however the worker gets there.
struct Settled<'a> {
    cleanups: &'a CleanupRegistry,
    cleanup: CleanupId,
}

impl Drop for Settled<'_> {
    fn drop(&mut self) {
        self.cleanups.settle(self.cleanup, None);
    }
}

/// The plan is still young enough, and its scan still exists and still describes the disk.
fn still_current(state: &AppState, reservation: &Reservation) -> CommandResult<()> {
    if reservation.made.elapsed() >= PLAN_LIFETIME {
        return Err(CommandError::invalid("the plan has expired, so make a new one"));
    }
    if state.scans.is_outdated(reservation.scan) || state.scans.tree(reservation.scan).is_err() {
        return Err(CommandError::invalid(
            "the disk changed since this plan was made, so rescan first",
        ));
    }
    // A plan stored before the user's rules changed is dropped with the change; this catches one that was
    // already taken (waiting for its confirmation) and a rules file that was edited or broke by hand.
    if state.user_rules.generation() != reservation.rules_generation {
        return Err(CommandError::invalid(
            "your rules changed since this plan was made, so make a new plan",
        ));
    }
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---- Describing plans, questions and events ----

fn action_dto(action: CleanupAction) -> ActionDto {
    match action {
        CleanupAction::Delete => ActionDto::Delete,
        CleanupAction::EmptyFolder => ActionDto::EmptyFolder,
        CleanupAction::EmptyTrash => ActionDto::EmptyTrash,
        // Moves to the trash, as `Trash` does: what is inside the folder is what goes.
        CleanupAction::Trash | CleanupAction::TrashChildren => ActionDto::Trash,
        CleanupAction::RunTool(_) => ActionDto::RunTool,
    }
}

pub(crate) fn command_text(action: CleanupAction) -> Option<(String, bool)> {
    match action {
        CleanupAction::RunTool(tool) => Some((
            format!("{} {}", tool.program(), tool.args().join(" ")),
            tool.needs_elevation(),
        )),
        _ => None,
    }
}

pub(crate) fn title_of(rule_id: &str) -> String {
    if rule_id == platter_cleanup::MARKED {
        return "Marked in the Explorer".to_owned();
    }
    if rule_id == ADVISOR {
        return ADVISOR_TITLE.to_owned();
    }
    RuleSet::builtin()
        .with_user(Vec::new())
        .rules()
        .iter()
        .find(|rule| rule.id == rule_id)
        .map_or_else(|| rule_id.to_owned(), |rule| rule.title.to_owned())
}

fn clamp(value: u64) -> u64 {
    value.min(MAX_SAFE)
}

/// The wire description of `plan`.
pub fn describe(plan: &CleanupPlan, scan: ScanId, plan_id: PlanId) -> CleanupPlanDto {
    let groups = plan
        .groups()
        .iter()
        .map(|group| {
            let command = command_text(group.action());
            PlanGroupDto {
                rule_id: group.rule_id().to_owned(),
                title: title_of(group.rule_id()),
                verdict: group.verdict(),
                action: action_dto(group.action()),
                items: group.targets().len() as u64,
                allocated: clamp(group.allocated()),
                needs_elevation: command.as_ref().is_some_and(|(_, elevated)| *elevated),
                command: command.map(|(text, _)| text),
                sample_paths: group
                    .targets()
                    .iter()
                    .take(SAMPLE_PATHS)
                    .map(|target| target.display_path())
                    .collect(),
            }
        })
        .collect();

    CleanupPlanDto {
        plan_id,
        scan_id: scan,
        groups,
        refused_more: (plan.left_out.len() + plan.refused.len()).saturating_sub(MAX_REFUSED) as u64,
        // What the options left out comes first, so a long list of refusals never hides it.
        refused: plan
            .left_out
            .iter()
            .map(|left_out| RefusedDto {
                rule_id: left_out.rule_id.to_owned(),
                path: left_out.path.clone(),
                reason: left_out.reason.to_owned(),
            })
            .chain(plan.refused.iter().map(|refused| RefusedDto {
                rule_id: refused.rule_id.to_owned(),
                path: refused.path.clone(),
                reason: refused.reason.to_string(),
            }))
            .take(MAX_REFUSED)
            .collect(),
        not_found: plan.not_found.clone(),
        allocated: clamp(plan.allocated),
        options: plan.options().into(),
        needs_confirmation: question(plan).is_some(),
    }
}

/// The confirmation a plan needs: every plan with something to do is confirmed. `None` for an empty
/// plan.
///
/// What each group does comes first, with counts and sizes, so it stays in view whatever the paths
/// look like; then up to [`SAMPLE_PATHS`] paths. Paths are shown safely: control and invisible
/// formatting characters (a newline, a right-to-left override) are written as `\u{...}`, a name that
/// is not valid text is marked, long paths are shortened in the middle, and the whole text is capped.
pub fn question(plan: &CleanupPlan) -> Option<Question> {
    let groups = plan.groups();
    if groups.is_empty() {
        return None;
    }

    let bin = bin_name(plan.os());
    let lasting = groups.iter().any(|group| {
        !matches!(
            group.action(),
            CleanupAction::Trash | CleanupAction::TrashChildren
        )
    });
    let mut lines = vec![
        if lasting {
            "Platter is about to do the following. Some of it cannot be undone.".to_owned()
        } else {
            format!("Platter is about to move the following to the {bin}. It can be put back from there.")
        },
        String::new(),
    ];

    for group in groups {
        let title = title_of(group.rule_id());
        let count = group.targets().len();
        let size = human_bytes(group.allocated());
        lines.push(match group.action() {
            CleanupAction::Delete => format!("• Delete for good: {title}, {count} items, {size}"),
            CleanupAction::EmptyFolder => format!("• Empty for good: {title}, {count} folders, {size}"),
            CleanupAction::EmptyTrash => {
                format!("• Empty the trash for good: everything in it, {size} when scanned")
            }
            CleanupAction::Trash => format!("• Move to the {bin}: {title}, {count} items, {size}"),
            CleanupAction::TrashChildren => {
                format!("• Move what is inside to the {bin}: {title}, {count} folders, {size}")
            }
            CleanupAction::RunTool(tool) => format!(
                "• Run{}: {} {}",
                if tool.needs_elevation() {
                    " as administrator"
                } else {
                    ""
                },
                tool.program(),
                tool.args().join(" ")
            ),
        });
    }

    let paths: Vec<&platter_cleanup::VerifiedTarget> = groups
        .iter()
        .filter(|group| !matches!(group.action(), CleanupAction::RunTool(_)))
        .flat_map(|group| group.targets())
        .collect();
    if !paths.is_empty() {
        lines.push(String::new());
        lines.push("Items:".to_owned());
        for target in paths.iter().take(SAMPLE_PATHS) {
            lines.push(format!("    {}", shown_path(&target.display_path())));
        }
        if paths.len() > SAMPLE_PATHS {
            lines.push(format!("    … and {} more", paths.len() - SAMPLE_PATHS));
        }
    }

    Some(Question {
        title: "Clean up?".to_owned(),
        text: capped(lines.join("\n")),
        accept: "Clean up",
    })
}

/// What the place for things that can be put back is called on `os`.
fn bin_name(os: Os) -> &'static str {
    if os == Os::Windows { "Recycle Bin" } else { "trash" }
}

/// `text`, cut to [`QUESTION_CHARS`] characters.
fn capped(mut text: String) -> String {
    if text.chars().count() > QUESTION_CHARS {
        text = text.chars().take(QUESTION_CHARS).collect();
        text.push_str("\n…");
    }

    text
}

/// The confirmation for putting entries back from the trash: what comes first is how many and how big,
/// then up to [`SAMPLE_PATHS`] of the places they go back to (as in [`question`], shown safely). Each
/// entry is its original path and its size.
pub(crate) fn put_back_question(items: &[(Vec<u8>, u64)]) -> Question {
    let size = human_bytes(items.iter().map(|(_, size)| *size).fold(0, u64::saturating_add));
    let count = items.len();
    let os = platter_platform::environment::current_os();
    let bin = bin_name(os);
    // Linux moves with `RENAME_NOREPLACE`. The Windows shell has nothing like it: Platter checks that the
    // old name is free right before, which is all it can promise.
    let overwrite = if os == Os::Windows {
        "Platter checks first that the old place is free, and leaves it alone when something is there."
    } else {
        "Nothing that is there now is overwritten."
    };
    let mut lines = vec![
        format!(
            "Platter is about to put the following back from the {bin}, to where it was before the cleanup. \
             {overwrite}"
        ),
        String::new(),
        format!(
            "• Put back: {count} {}, {size}",
            if count == 1 { "item" } else { "items" }
        ),
    ];

    if count > 0 {
        lines.push(String::new());
        lines.push("Items:".to_owned());
        for (original, _) in items.iter().take(SAMPLE_PATHS) {
            lines.push(format!("    {}", shown_path(&String::from_utf8_lossy(original))));
        }
        if count > SAMPLE_PATHS {
            lines.push(format!("    … and {} more", count - SAMPLE_PATHS));
        }
    }

    Question {
        title: "Put back?".to_owned(),
        text: capped(lines.join("\n")),
        accept: "Put back",
    }
}

/// Asks the user, in the backend's own dialog, to put `items` back. A refusal, or a dialog that fails, is
/// `cancelled`. Unlike a declined cleanup it does not pause the next confirmation: nothing is started.
pub(crate) async fn confirm_put_back(state: &AppState, items: &[(Vec<u8>, u64)]) -> CommandResult<()> {
    let question = put_back_question(items);
    let confirm = state.confirm();
    let approved = tauri::async_runtime::spawn_blocking(move || confirm.confirm(&question))
        .await
        .unwrap_or(false);

    if approved {
        Ok(())
    } else {
        Err(CommandError::new(
            platter_core::dto::ApiErrorCode::Cancelled,
            "putting things back was not confirmed",
        ))
    }
}

/// A path as the confirmation shows it (see [`question`]).
fn shown_path(path: &str) -> String {
    // Shortened first, by characters, then escaped: an escape is never cut in half.
    let chars: Vec<char> = path.chars().collect();
    let kept: Vec<char> = if chars.len() > SHOWN_PATH_CHARS {
        let half = SHOWN_PATH_CHARS / 2 - 2;
        let mut kept = chars[..half].to_vec();
        kept.extend(" … ".chars());
        kept.extend_from_slice(&chars[chars.len() - half..]);
        kept
    } else {
        chars
    };

    let mut shown = String::with_capacity(path.len());
    for c in kept {
        if c.is_control() || is_invisible_format(c) {
            shown.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
        } else {
            shown.push(c);
        }
    }
    if path.contains('\u{fffd}') {
        shown.push_str("  (a name here is not valid text)");
    }
    shown
}

/// Characters that change how text is laid out, or break a line, without being seen: bidirectional
/// controls, joiners, soft hyphens, line and paragraph separators, variation selectors and blank
/// fillers.
fn is_invisible_format(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180B..=0x180F | 0x200B..=0x200F
            | 0x2028..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0x2800 | 0x3000 | 0x3164 | 0xFE00..=0xFE0F
            | 0xFEFF | 0xFFA0 | 0xFFF9..=0xFFFB | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F
            | 0xE0100..=0xE01EF
    )
}

/// Bytes for people: `286.1 MiB`.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn outcome_dto(outcome: &TargetOutcome) -> OutcomeDto {
    let (kind, detail, problems) = match outcome {
        TargetOutcome::Removed(removed) => (
            OutcomeKind::Removed,
            None,
            removed.errors.len() as u64 + removed.more_errors + removed.skipped_mounts,
        ),
        // Gone from its place with nothing to put back (deleted for good, or it may be in the Recycle Bin):
        // it is told as removed, with the reason and one problem, never as trashed.
        TargetOutcome::NotRecycled(why) => (OutcomeKind::Removed, Some(why.clone()), 1),
        TargetOutcome::Trashed(_) => (OutcomeKind::Trashed, None, 0),
        TargetOutcome::ByTool(command) => (OutcomeKind::ByTool, Some(command.clone()), 0),
        TargetOutcome::Skipped(why) => (OutcomeKind::Skipped, Some(why.clone()), 0),
        TargetOutcome::Failed(why) => (OutcomeKind::Failed, Some(why.clone()), 0),
    };
    OutcomeDto {
        kind,
        detail,
        problems,
    }
}

fn event_dto(event: CleanupEvent) -> CleanupEventDto {
    match event {
        CleanupEvent::GroupStarted { rule_id, targets } => CleanupEventDto::GroupStarted {
            rule_id: rule_id.to_owned(),
            targets: targets as u64,
        },
        CleanupEvent::TargetDone(result) => CleanupEventDto::TargetDone {
            rule_id: result.rule_id.to_owned(),
            path: result.path.clone(),
            allocated: clamp(result.allocated),
            outcome: outcome_dto(&result.outcome),
        },
        CleanupEvent::GroupFinished { rule_id } => CleanupEventDto::GroupFinished {
            rule_id: rule_id.to_owned(),
        },
    }
}

fn finished_dto(cleanup: CleanupId, scan: ScanId, report: &CleanupReport) -> CleanupEventDto {
    let clean = |outcome: &OutcomeDto| {
        matches!(
            outcome.kind,
            OutcomeKind::Removed | OutcomeKind::Trashed | OutcomeKind::ByTool
        ) && outcome.problems == 0
    };
    let cleaned = report
        .results
        .iter()
        .filter(|result| clean(&outcome_dto(&result.outcome)))
        .count() as u64;

    CleanupEventDto::Finished {
        cleanup_id: cleanup,
        outdated_scan: scan,
        cleaned,
        with_problems: report.results.len() as u64 - cleaned,
        freed: report
            .freed
            .map(|freed| freed.clamp(-(MAX_SAFE as i128), MAX_SAFE as i128) as i64),
        cancelled: report.cancelled,
        tools_not_run: report.tools_not_run.iter().map(|id| (*id).to_owned()).collect(),
        can_undo: report
            .results
            .iter()
            .any(|result| matches!(result.outcome, TargetOutcome::Trashed(_))),
    }
}

/// The confirmation elsewhere than Linux: the dialog plugin's native message dialog.
#[cfg(not(target_os = "linux"))]
pub struct NativeConfirm<R: tauri::Runtime> {
    pub app: tauri::AppHandle<R>,
}

#[cfg(not(target_os = "linux"))]
impl<R: tauri::Runtime> Confirm for NativeConfirm<R> {
    fn confirm(&self, question: &Question) -> bool {
        use tauri::Manager;
        use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

        let mut dialog = self
            .app
            .dialog()
            .message(question.text.clone())
            .title(question.title.clone())
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                question.accept.to_owned(),
                "Cancel".to_owned(),
            ));
        // Attached to Platter's window, so it opens over the app rather than wherever the pointer is.
        if let Some(window) = self.app.get_webview_window("main") {
            dialog = dialog.parent(&window);
        }
        dialog.blocking_show()
    }
}

/// The confirmation on Linux: a GTK message dialog over the app window. The window title is the app's
/// name and the heading is the question, said once; Cancel is the default, so Enter does not approve;
/// the text is plain (never markup).
#[cfg(target_os = "linux")]
pub struct GtkConfirm<R: tauri::Runtime> {
    pub app: tauri::AppHandle<R>,
}

#[cfg(target_os = "linux")]
impl<R: tauri::Runtime> Confirm for GtkConfirm<R> {
    fn confirm(&self, question: &Question) -> bool {
        use gtk::prelude::*;
        use tauri::Manager;

        let (answer, answered) = std::sync::mpsc::channel();
        let window = self.app.get_webview_window("main");
        let question = question.clone();

        // GTK lives on the main thread; this call blocks only the cleanup's own worker.
        let posted = self.app.run_on_main_thread(move || {
            let parent = window.and_then(|window| window.gtk_window().ok());
            let dialog = gtk::MessageDialog::new(
                parent.as_ref(),
                gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
                gtk::MessageType::Warning,
                gtk::ButtonsType::None,
                &question.title,
            );
            dialog.set_title("Platter");
            dialog.set_secondary_use_markup(false);
            dialog.set_secondary_text(Some(&question.text));
            dialog.add_button("Cancel", gtk::ResponseType::Cancel);
            dialog.add_button(question.accept, gtk::ResponseType::Accept);
            dialog.set_default_response(gtk::ResponseType::Cancel);
            if let Some(cancel) = dialog.widget_for_response(gtk::ResponseType::Cancel) {
                cancel.grab_focus();
            }

            let response = dialog.run();
            dialog.close();
            let _ = answer.send(response == gtk::ResponseType::Accept);
        });

        posted.is_ok() && answered.recv().unwrap_or(false)
    }
}

/// Used until the app has set up its dialog, and in tests that do not set one: nothing is approved.
pub struct RefuseAll;

impl Confirm for RefuseAll {
    fn confirm(&self, _question: &Question) -> bool {
        false
    }
}

#[cfg(test)]
pub(crate) mod tests;
