//! History over IPC. The history file (see `platter-history` and "History" in `docs/architecture.md`) is
//! written by the backend alone: a finished scan, the daily usage of the disk and each cleanup run are
//! recorded here, and the web view can only read an overview and ask for a recorded run to be put back.
//!
//! A history file that cannot be used never stops the app: the service then holds the reason, records
//! nothing, and says so in the overview. A file that another program had locked is tried again later,
//! a damaged one or one from a newer version is left alone. Recording is best effort and never panics
//! into a scan or a cleanup.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use platter_cleanup::{CleanupAction, CleanupReport, MARKED, TargetOutcome, TargetResult, restore_one};
use platter_core::{EntryProbe, Totals, Volume, VolumeProvider};
use platter_history::day::{day_number, day_text};
use platter_history::{
    CHECK_CAP, Method, MethodKind, NewCleanup, NewItem, Problem, ScanRecord, Status, Store, StoredCleanup,
    StoredItem, Usage, VolumeRef, check, first_day, summarize, window_start,
};
use platter_platform::SystemVolumes;
use platter_platform::localtime::local_offset;
use serde::Serialize;
use tauri::State;

use crate::commands::cleanup::{UndoDto, command_text, confirm_put_back, title_of};
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// The most days the overview covers.
const MAX_DAYS: i64 = 366;
/// The largest integer a JavaScript number holds exactly; sizes are clamped to it on the wire.
const MAX_SAFE: u64 = 9_007_199_254_740_991;
/// Folder names listed for a marked run before the rest is counted.
const NAMED_ITEMS: usize = 5;
/// How often the sampler looks at the disks.
const SAMPLE_EVERY: Duration = Duration::from_secs(60 * 60);
/// The oldest cleanup a restore may put back, in seconds (the longest overview window).
const RESTORE_WINDOW: i64 = MAX_DAYS * 86_400;
/// How long a file that another program had locked is left alone before it is tried again.
const RETRY_EVERY: Duration = Duration::from_secs(30);
/// How long the list of disks is reused when a scan or a cleanup is recorded.
const VOLUMES_FRESH: Duration = Duration::from_secs(30);
/// Overviews at once: one reads the file, and one more waits for it.
const OVERVIEWS: usize = 2;

/// The history file, and the one restore that may run at a time.
pub struct HistoryService {
    /// The store, or why there is none (and whether to try again).
    store: Mutex<Result<Store, Closed>>,
    /// Set while a restore or an undo puts things back: they exclude each other and a cleanup.
    restoring: AtomicBool,
    volumes: Box<dyn VolumeProvider>,
    /// The list of disks as the last recording saw it, and when.
    listed: Mutex<Option<(Instant, Vec<Volume>)>>,
    /// The ids of the cleanups the latest overview showed: only those can be restored.
    shown: Mutex<BTreeSet<i64>>,
    /// Overviews running or waiting (see [`Turn`]).
    overviews: AtomicUsize,
    /// Held while an overview reads: one at a time.
    reading: Mutex<()>,
    /// How long a locked file is left alone.
    retry_every: Duration,
}

/// Why there is no store, and where a locked file can be tried again.
struct Closed {
    problem: String,
    /// The folder of the file, when the file was only locked by another program.
    reopen: Option<PathBuf>,
    tried: Instant,
}

/// The single restoring slot, held until the restore or undo is over.
pub struct Restoring<'a>(&'a AtomicBool);

impl Drop for Restoring<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// A place in the line for an overview: one reads the file and one waits, so the web view cannot fill the
/// blocking pool with them. Dropping it leaves the line.
pub struct Turn(Arc<HistoryService>);

impl Drop for Turn {
    fn drop(&mut self) {
        self.0.overviews.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Turn {
    /// A place in the line, or `None` when it is full.
    pub fn join(history: &Arc<HistoryService>) -> Option<Self> {
        let before = history.overviews.fetch_add(1, Ordering::SeqCst);
        let turn = Self(Arc::clone(history));
        // Over the limit: dropping `turn` gives the place back.
        (before < OVERVIEWS).then_some(turn)
    }
}

impl HistoryService {
    /// The history in the user's data folder (`$XDG_DATA_HOME/platter/history.sqlite`).
    pub fn production() -> Self {
        let dir = platter_platform::data_dir();

        Self::with_store(open_in(dir.as_deref()), Box::new(SystemVolumes), RETRY_EVERY)
    }

    /// The history in `dir` (`None`: no place for one), listing `volumes` as the disks. A locked file is
    /// tried again at the next use.
    #[cfg(test)]
    pub fn in_dir(dir: Option<&Path>, volumes: impl VolumeProvider + 'static) -> Self {
        Self::with_store(open_in(dir), Box::new(volumes), Duration::ZERO)
    }

    /// An empty history in memory, on the disks `volumes` lists.
    pub fn in_memory(volumes: impl VolumeProvider + 'static) -> Self {
        let store = Store::open_in_memory().map_err(|problem| Closed::for_good(problem.to_string()));

        Self::with_store(store, Box::new(volumes), RETRY_EVERY)
    }

    fn with_store(
        store: Result<Store, Closed>,
        volumes: Box<dyn VolumeProvider>,
        retry_every: Duration,
    ) -> Self {
        Self {
            store: Mutex::new(store),
            restoring: AtomicBool::new(false),
            volumes,
            listed: Mutex::new(None),
            shown: Mutex::new(BTreeSet::new()),
            overviews: AtomicUsize::new(0),
            reading: Mutex::new(()),
            retry_every,
        }
    }

    /// The store, or why there is none. A file that was only locked is opened again here when it has been
    /// left alone for a while.
    fn lock(&self) -> MutexGuard<'_, Result<Store, Closed>> {
        // A panic cannot leave the file half-written (every write is one transaction), so recover.
        let mut guard = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        let reopen = match &*guard {
            Err(closed) if closed.tried.elapsed() >= self.retry_every => closed.reopen.clone(),
            _ => None,
        };
        if let Some(dir) = reopen {
            *guard = open_in(Some(&dir));
        }

        guard
    }

    /// Why nothing is recorded, when the file cannot be used.
    #[cfg(test)]
    #[cfg_attr(not(unix), allow(dead_code))]
    pub fn problem(&self) -> Option<String> {
        self.lock().as_ref().err().map(|closed| closed.problem.clone())
    }

    /// Takes the restoring slot, or `None` when a restore or an undo has it.
    pub fn begin_restore(&self) -> Option<Restoring<'_>> {
        self.restoring
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Restoring(&self.restoring))
    }

    /// Whether a restore or an undo is putting things back.
    pub fn is_restoring(&self) -> bool {
        self.restoring.load(Ordering::SeqCst)
    }

    /// Runs `write` on the store; `None` when there is none, the write failed or it panicked. Recording is
    /// best effort: it never fails a scan or a cleanup, and never panics into one.
    fn record<T>(&self, write: impl FnOnce(&mut Store) -> Result<T, Problem>) -> Option<T> {
        guarded(|| {
            let mut guard = self.lock();
            let store = guard.as_mut().ok()?;

            write(store).ok()
        })
        .flatten()
    }

    /// The disks as listed a short while ago, or now when that is older or has no disk holding `path`.
    fn volume_of(&self, path: &Path) -> Option<Volume> {
        let mut listed = self
            .listed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some((at, volumes)) = listed.as_ref()
            && at.elapsed() < VOLUMES_FRESH
            && let Some(found) = holding(volumes, path)
        {
            return Some(found);
        }

        let volumes = self.volumes.list();
        let found = holding(&volumes, path);
        *listed = Some((Instant::now(), volumes));

        found
    }

    /// Records a finished scan of `root`, and the first usage sample of the day of its volume.
    pub fn record_scan(&self, root: &Path, totals: &Totals) {
        // The whole of it is guarded: finding the disk reads the system, and none of it may break a scan.
        guarded(|| {
            let volume = self.volume_of(root)?;
            let now = unix_now();
            let scan = ScanRecord {
                at: now,
                allocated: totals.allocated,
                apparent: totals.apparent,
                files: totals.files,
                dirs: totals.dirs,
                errors: totals.errors,
            };

            self.record(|store| {
                let at = volume_ref(&volume);
                store.record_scan(at, &scan)?;
                sample(store, &volume, now)
            })
        });
    }

    /// Takes the day's first usage sample of every stored volume that is mounted now.
    pub fn sample_all(&self) {
        guarded(|| {
            let listed = self.volumes.list();
            let now = unix_now();

            self.record(|store| {
                for stored in store.volumes()? {
                    let mounted = listed
                        .iter()
                        .find(|volume| volume.id == stored.mount_point && volume.total > 0);
                    if let Some(volume) = mounted {
                        sample(store, volume, now)?;
                    }
                }

                Ok(())
            })
        });
    }

    /// Records a cleanup run of a scan of `root` that carried out `groups` (rule id and action of each)
    /// with `results`. `None` when it did nothing, its volume is unknown or there is no history; else
    /// the id the run is kept under. `kept` is called with that id while the store is still held, so
    /// nobody can read the run from the history before the caller knows where it is kept.
    pub fn record_run(
        &self,
        root: &Path,
        groups: &[(&str, CleanupAction)],
        results: &[TargetResult],
        report: Option<&CleanupReport>,
        kept: impl FnOnce(i64),
    ) -> Option<i64> {
        guarded(|| {
            let cleanup = describe_run(groups, results, report, unix_now())?;
            let volume = self.volume_of(root)?;

            self.record(|store| {
                let id = store.record_cleanup(volume_ref(&volume), &cleanup)?;
                kept(id);

                Ok(id)
            })
        })
        .flatten()
    }

    /// Marks the entries an undo put back, found by their paths.
    pub fn mark_undone(&self, cleanup: i64, paths: &[(Vec<u8>, Vec<u8>)]) {
        if paths.is_empty() {
            return;
        }

        self.record(|store| store.mark_restored_matching(cleanup, paths, unix_now()));
    }

    /// The overview of the last `days` days. Looks at the trash through `probe`, so it can be slow. One at a
    /// time: another call waits for the one that reads.
    pub fn overview(&self, days: u32, probe: &impl EntryProbe) -> HistoryOverviewDto {
        let _reading = self
            .reading
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = unix_now();
        let offset = local_offset(now);

        let read = {
            let guard = self.lock();
            match guard.as_ref() {
                Ok(store) => read(store, now, offset, days),
                Err(closed) => Err(closed.problem.clone()),
            }
        };
        let (volume, samples, cleanups) = match read {
            Ok(Some(found)) => found,
            Ok(None) => return self.shown_nothing(HistoryOverviewDto::empty(days, now, offset, None)),
            Err(problem) => {
                return self.shown_nothing(HistoryOverviewDto::empty(days, now, offset, Some(problem)));
            }
        };

        let summary = summarize(now, offset, days, &samples, &cleanups);
        let checked = check(&summary.cleanups, probe, CHECK_CAP);
        let states = checked.statuses;
        let rows: Vec<HistoryCleanupDto> = summary
            .cleanups
            .iter()
            .zip(&states)
            .map(|(cleanup, state)| HistoryCleanupDto::new(cleanup, offset, *state))
            .collect();
        let restorable = states
            .iter()
            .map(|state| state.restorable)
            .fold(0, u64::saturating_add);
        // Only what is shown can be asked to be put back.
        *self.shown.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            rows.iter().map(|row| row.id).collect();
        // What the probe found gone is not kept by path any longer: the status stays, the paths do not.
        self.forget_gone(&checked.gone);

        HistoryOverviewDto {
            volume: Some(HistoryVolumeDto {
                id: volume.mount_point,
                label: volume.label,
            }),
            days,
            today: summary.today,
            samples: summary
                .samples
                .iter()
                .map(|sample| HistorySampleDto {
                    day: sample.day.clone(),
                    used: clamp(sample.used),
                    total: clamp(sample.total),
                })
                .collect(),
            without: summary.without.iter().map(|used| clamp(*used)).collect(),
            freed: clamp(summary.freed),
            cleanup_count: rows.len() as u64,
            cleanups: rows,
            restorable: clamp(restorable),
            growth_per_day: summary.growth_per_day.map(clamp),
            days_until_full: summary.days_until_full,
            thin: summary.thin,
            problem: None,
        }
    }

    /// Forgets the paths of the entries an overview found gone from the trash (at most [`CHECK_CAP`], since
    /// the probe looked at no more). Not while something is being put back: an entry that is on its way
    /// out of the trash looks gone, and an undo finds its entry by the paths. The write is skipped then,
    /// and the next overview finds the entry again if it is still gone.
    fn forget_gone(&self, gone: &[i64]) {
        if gone.is_empty() {
            return;
        }
        let Some(_restoring) = self.begin_restore() else {
            return;
        };

        self.record(|store| store.mark_gone(gone));
    }

    /// An overview that shows no cleanup: none can be restored from it.
    fn shown_nothing(&self, overview: HistoryOverviewDto) -> HistoryOverviewDto {
        self.shown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();

        overview
    }

    /// What `cleanup` still has to put back, when it may be put back: shown by the latest overview, on the
    /// volume the history shows now, recorded within the last 366 days, with entries not put back yet.
    /// `Ok(None)` when it is none of those.
    fn to_restore(&self, cleanup: i64) -> Result<Option<Vec<StoredItem>>, String> {
        let guard = self.lock();
        let store = guard.as_ref().map_err(|closed| closed.problem.clone())?;
        let problem = |problem: Problem| problem.to_string();

        let shown = self.shown.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !shown.contains(&cleanup) {
            return Ok(None);
        }
        drop(shown);

        let Some(volume) = store.shown_volume().map_err(problem)? else {
            return Ok(None);
        };
        let since = unix_now().saturating_sub(RESTORE_WINDOW);
        let items = store
            .items_to_restore(cleanup, volume.id, since)
            .map_err(problem)?;

        Ok(items.filter(|items| !items.is_empty()))
    }

    fn mark_restored(&self, items: &[i64]) {
        if items.is_empty() {
            return;
        }

        self.record(|store| store.mark_restored(items, unix_now()));
    }
}

/// The disks a service lists: none for the state tests build without a history, or the ones a test gives.
#[derive(Debug, Default)]
pub struct FixedVolumes(pub Vec<Volume>);

impl FixedVolumes {
    /// One disk that holds every path: `/`, 1000 bytes in all and 400 of them free.
    #[cfg(test)]
    pub fn root() -> Self {
        Self(vec![volume("/", "Root", 1000, 400)])
    }
}

impl VolumeProvider for FixedVolumes {
    fn list(&self) -> Vec<Volume> {
        self.0.clone()
    }
}

/// A disk as a test lists it.
#[cfg(test)]
pub fn volume(mount_point: &str, label: &str, total: u64, available: u64) -> Volume {
    Volume {
        id: mount_point.to_owned(),
        mount_point: mount_point.into(),
        label: label.to_owned(),
        device: String::new(),
        fs_type: "ext4".to_owned(),
        total,
        available,
        removable: false,
    }
}

/// Starts [`HistoryService::sample_all`] now and every hour, for as long as the app runs.
pub fn spawn_sampler(history: Arc<HistoryService>) {
    let spawned = thread::Builder::new()
        .name("platter-history-sampler".to_owned())
        .spawn(move || {
            loop {
                history.sample_all();
                thread::sleep(SAMPLE_EVERY);
            }
        });
    // Without the thread the day is still sampled when a scan finishes.
    drop(spawned);
}

impl Closed {
    /// A file that is no use now and will not be later.
    fn for_good(problem: String) -> Self {
        Self {
            problem,
            reopen: None,
            tried: Instant::now(),
        }
    }
}

/// The file in `dir`, made private before SQLite can create it with another mode.
fn open_in(dir: Option<&Path>) -> Result<Store, Closed> {
    open_in_with(dir, |_| {})
}

/// As [`open_in`]. `between` runs after the file was checked and before SQLite opens it by name (a test
/// swaps the file there).
fn open_in_with(dir: Option<&Path>, between: impl FnOnce(&Path)) -> Result<Store, Closed> {
    let Some(dir) = dir else {
        return Err(Closed::for_good(
            "Platter cannot tell where to keep its history, because there is no home folder".to_owned(),
        ));
    };
    platter_platform::create_private_dir(dir).map_err(|_| {
        Closed::for_good("the folder for the history file could not be made or is not yours".to_owned())
    })?;

    let path = dir.join("history.sqlite");
    // Made here with mode 0600 (a link, another kind of file and a file of another user are refused, and
    // one that was more open is closed), then handed to SQLite. The checked file stays open until SQLite
    // has opened the name, so its inode cannot be given to another file meanwhile.
    let checked = platter_platform::open_private_file(&path).map_err(|_| {
        Closed::for_good(
            "the history file is a link, is not yours or could not be opened, and Platter does not use it"
                .to_owned(),
        )
    })?;
    between(&path);

    let store = Store::open(&path).map_err(|problem| Closed {
        reopen: problem.is_busy().then(|| dir.to_path_buf()),
        problem: problem.to_string(),
        tried: Instant::now(),
    })?;

    // SQLite opened the name again: it must still lead to the file that was checked.
    match platter_platform::app_dirs::is_same_file(&checked, &path) {
        Ok(true) => Ok(store),
        _ => Err(Closed::for_good(
            "the history file was replaced while it was being opened, and Platter does not use it".to_owned(),
        )),
    }
}

/// The disk with the longest mount point that is a prefix of `path`.
fn holding(volumes: &[Volume], path: &Path) -> Option<Volume> {
    volumes
        .iter()
        .filter(|volume| path.starts_with(&volume.mount_point))
        .max_by_key(|volume| volume.mount_point.components().count())
        .cloned()
}

/// Runs `work`, and gives up on it when it panics.
fn guarded<T>(work: impl FnOnce() -> T) -> Option<T> {
    catch_unwind(AssertUnwindSafe(work)).ok()
}

fn volume_ref(volume: &Volume) -> VolumeRef<'_> {
    VolumeRef {
        mount_point: &volume.id,
        label: &volume.label,
    }
}

/// The day's first sample of `volume`: what is used on it now.
fn sample(store: &mut Store, volume: &Volume, now: i64) -> Result<(), Problem> {
    let day = day_text(day_number(now, local_offset(now)));
    let usage = Usage {
        at: now,
        total: volume.total,
        used: volume.total.saturating_sub(volume.available),
    };
    store.record_sample(volume_ref(volume), &day, usage)?;

    Ok(())
}

/// The shown volume with its samples and cleanups of the window.
type Read = (
    platter_history::StoredVolume,
    Vec<platter_history::Sample>,
    Vec<StoredCleanup>,
);

fn read(store: &Store, now: i64, offset: i64, days: u32) -> Result<Option<Read>, String> {
    let problem = |problem: Problem| problem.to_string();

    let Some(volume) = store.shown_volume().map_err(problem)? else {
        return Ok(None);
    };
    let from = day_text(first_day(now, offset, days));
    let samples = store.samples(volume.id, &from).map_err(problem)?;
    let cleanups = store
        .cleanups_since(volume.id, window_start(now, offset, days), CHECK_CAP)
        .map_err(problem)?;

    Ok(Some((volume, samples, cleanups)))
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

fn clamp(value: u64) -> u64 {
    value.min(MAX_SAFE)
}

// ---- What a run is recorded as ----

fn did_something(outcome: &TargetOutcome) -> bool {
    matches!(
        outcome,
        TargetOutcome::Removed(_)
            | TargetOutcome::NotRecycled(_)
            | TargetOutcome::Trashed(_)
            | TargetOutcome::ByTool(_)
    )
}

/// What to keep of a run: `None` when nothing was removed, trashed or cleaned by a tool.
fn describe_run(
    groups: &[(&str, CleanupAction)],
    results: &[TargetResult],
    report: Option<&CleanupReport>,
    at: i64,
) -> Option<NewCleanup> {
    if !results.iter().any(|result| did_something(&result.outcome)) {
        return None;
    }

    let mut what: Vec<String> = Vec::new();
    let mut methods: Vec<Method> = Vec::new();
    let mut emptied_trash = false;
    let mut emptied_trashes: Vec<Vec<u8>> = Vec::new();

    for (rule_id, action) in groups {
        let done: Vec<&TargetResult> = results
            .iter()
            .filter(|result| result.rule_id == *rule_id && did_something(&result.outcome))
            .collect();
        if done.is_empty() {
            continue;
        }

        for label in labels(rule_id, &done) {
            if !what.contains(&label) {
                what.push(label);
            }
        }

        for method in methods_of(*action, &done) {
            if !methods.contains(&method) {
                methods.push(method);
            }
        }
        if *action == CleanupAction::EmptyTrash {
            emptied_trash = true;
            emptied_trashes.extend(done.iter().filter_map(|result| emptied_completely(result)));
        }
    }

    let removed = results.iter().filter(|result| {
        matches!(
            result.outcome,
            TargetOutcome::Removed(_) | TargetOutcome::NotRecycled(_) | TargetOutcome::ByTool(_)
        )
    });
    let removed_here = removed
        .clone()
        .map(|result| result.allocated)
        .fold(0, u64::saturating_add);
    let measured = report.and_then(|report| report.freed);
    let freed = if removed.count() == 0 {
        // Moving to the trash frees nothing on the same filesystem, whatever the free space did.
        0
    } else {
        match measured {
            Some(measured) => u64::try_from(measured.max(0)).unwrap_or(u64::MAX),
            None => removed_here,
        }
    };

    let items: Vec<NewItem> = results
        .iter()
        .filter_map(|result| match &result.outcome {
            TargetOutcome::Trashed(to) => Some(NewItem {
                original: result.exact.clone(),
                trashed: to.clone(),
                identity: result.identity,
                is_dir: result.is_dir,
                allocated: result.allocated,
            }),
            _ => None,
        })
        .collect();

    Some(NewCleanup {
        at,
        freed_measured: measured.map(|measured| i64::try_from(measured).unwrap_or(i64::MAX)),
        freed,
        trashed: items
            .iter()
            .map(|item| item.allocated)
            .fold(0, u64::saturating_add),
        cancelled: report.is_some_and(|report| report.cancelled),
        emptied_trash,
        emptied_trashes,
        what,
        methods,
        items,
    })
}

/// The trash folder an "empty the trash" result emptied, when it did so without a single problem. One that
/// left something behind (an entry it could not remove, another mount) is not "emptied": what it left is
/// still in the trash and still to be put back.
fn emptied_completely(result: &TargetResult) -> Option<Vec<u8>> {
    match &result.outcome {
        TargetOutcome::Removed(removed)
            if removed.errors.is_empty() && removed.more_errors == 0 && removed.skipped_mounts == 0 =>
        {
            Some(result.exact.clone())
        }
        _ => None,
    }
}

/// What a group is called in the history: its title, or for the items the user marked, their names.
fn labels(rule_id: &str, done: &[&TargetResult]) -> Vec<String> {
    if rule_id != MARKED {
        return vec![title_of(rule_id)];
    }

    let names: BTreeSet<String> = done.iter().map(|result| name_of(&result.path)).collect();
    let mut labels: Vec<String> = names.iter().take(NAMED_ITEMS).cloned().collect();
    if names.len() > NAMED_ITEMS {
        labels.push(format!("{} more", names.len() - NAMED_ITEMS));
    }

    labels
}

fn name_of(path: &str) -> String {
    // A path that starts like `C:\` is a Windows one: its separator is the backslash.
    let separator = if is_windows_path(path) { '\\' } else { '/' };

    path.trim_end_matches(separator)
        .rsplit(separator)
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// Whether `path` starts with a drive root, as a path on Windows does.
fn is_windows_path(path: &str) -> bool {
    matches!(path.as_bytes(), [letter, b':', b'\\', ..] if letter.is_ascii_alphabetic())
}

/// How a group got rid of what it did: by its action, except that an entry that went without a restorable
/// item in the trash (`NotRecycled`) was deleted, not trashed. History never shows a move to the trash for
/// something that has nothing to put back. The store has no place for the reason, which the run's report
/// carries.
fn methods_of(action: CleanupAction, done: &[&TargetResult]) -> Vec<Method> {
    let planned = method_of(action);
    if planned.kind != MethodKind::Trash {
        return vec![planned];
    }

    let mut methods = Vec::new();
    if done
        .iter()
        .any(|result| matches!(result.outcome, TargetOutcome::Trashed(_)))
    {
        methods.push(planned);
    }
    if done
        .iter()
        .any(|result| matches!(result.outcome, TargetOutcome::NotRecycled(_)))
    {
        methods.push(Method {
            kind: MethodKind::Deleted,
            command: None,
        });
    }

    methods
}

fn method_of(action: CleanupAction) -> Method {
    let (kind, command) = match action {
        CleanupAction::Delete | CleanupAction::EmptyFolder => (MethodKind::Deleted, None),
        CleanupAction::Trash | CleanupAction::TrashChildren => (MethodKind::Trash, None),
        CleanupAction::EmptyTrash => (MethodKind::EmptiedTrash, None),
        CleanupAction::RunTool(_) => (MethodKind::Tool, command_text(action).map(|(text, _)| text)),
    };

    Method { kind, command }
}

// ---- Wire types (sent only) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryVolumeDto {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySampleDto {
    pub day: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCleanupDto {
    pub id: i64,
    pub at: i64,
    pub day: String,
    pub what: Vec<String>,
    pub freed: u64,
    pub methods: Vec<Method>,
    pub status: Status,
    pub restorable: u64,
}

impl HistoryCleanupDto {
    fn new(cleanup: &StoredCleanup, offset: i64, state: platter_history::RunStatus) -> Self {
        Self {
            id: cleanup.id,
            at: cleanup.at,
            day: day_text(day_number(cleanup.at, offset)),
            what: cleanup.what.clone(),
            freed: clamp(cleanup.freed.saturating_add(cleanup.trashed)),
            methods: cleanup.methods.clone(),
            status: state.status,
            restorable: clamp(state.restorable),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryOverviewDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<HistoryVolumeDto>,
    pub days: u32,
    pub today: String,
    pub samples: Vec<HistorySampleDto>,
    pub without: Vec<u64>,
    pub cleanups: Vec<HistoryCleanupDto>,
    pub freed: u64,
    pub cleanup_count: u64,
    pub restorable: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub growth_per_day: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_until_full: Option<u64>,
    pub thin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

impl HistoryOverviewDto {
    /// Nothing recorded (yet), or nothing readable (`problem`).
    fn empty(days: u32, now: i64, offset: i64, problem: Option<String>) -> Self {
        Self {
            volume: None,
            days,
            today: day_text(day_number(now, offset)),
            samples: Vec::new(),
            without: Vec::new(),
            cleanups: Vec::new(),
            freed: 0,
            cleanup_count: 0,
            restorable: 0,
            growth_per_day: None,
            days_until_full: None,
            thin: true,
            problem,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRestoreDto {
    pub restored: u64,
    pub items: Vec<UndoDto>,
}

// ---- Commands ----

/// The last `days` days (1 to 366) of the disk the history shows.
#[tauri::command]
pub async fn history_overview(state: State<'_, AppState>, days: i64) -> CommandResult<HistoryOverviewDto> {
    overview(&state, days).await
}

/// Puts back what a recorded cleanup moved to the trash and that is not back yet. Asks for confirmation
/// first, in a native dialog listing what goes back: a refused confirmation is the error `cancelled`.
#[tauri::command]
pub async fn restore_history_cleanup(
    state: State<'_, AppState>,
    cleanup: i64,
) -> CommandResult<HistoryRestoreDto> {
    restore_run(&state, cleanup).await
}

pub async fn overview(state: &AppState, days: i64) -> CommandResult<HistoryOverviewDto> {
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(CommandError::invalid(format!(
            "choose between 1 and {MAX_DAYS} days"
        )));
    }
    let days = u32::try_from(days).unwrap_or(1);
    // One overview reads at a time and one more waits: more would only pile up on the blocking pool.
    let Some(turn) = Turn::join(&state.history) else {
        return Err(CommandError::invalid(
            "the history is being read, so try again in a moment",
        ));
    };
    let history = Arc::clone(&state.history);
    let platform = Arc::clone(&state.platform);

    // Looking at the trash touches the disk.
    let overview = tauri::async_runtime::spawn_blocking(move || {
        let _turn = turn;
        history.overview(days, &platform.entry_probe)
    })
    .await?;

    Ok(overview)
}

/// What `cleanup` still has to put back, or why nothing can be.
fn restorable(state: &AppState, cleanup: i64) -> CommandResult<Vec<StoredItem>> {
    match state.history.to_restore(cleanup) {
        Ok(Some(items)) => Ok(items),
        Ok(None) => Err(CommandError::not_found(format!(
            "cleanup {cleanup} cannot be put back from the history"
        ))),
        Err(problem) => Err(CommandError::invalid(problem)),
    }
}

/// What a restore moves, of an item: what the dialog showed and what is checked before it moves.
type ShownItem<'a> = (i64, &'a [u8], &'a [u8], (u64, u64), bool);

fn shown_item(item: &StoredItem) -> ShownItem<'_> {
    (item.id, &item.original, &item.trashed, item.identity, item.is_dir)
}

pub async fn restore_run(state: &AppState, cleanup: i64) -> CommandResult<HistoryRestoreDto> {
    let Some(_restoring) = state.history.begin_restore() else {
        return Err(CommandError::invalid(
            "something is being put back, so try again when it has finished",
        ));
    };
    // Not while a cleanup runs: it could be emptying the trash that is being restored from.
    if state.cleanups.is_running() {
        return Err(CommandError::invalid(
            "a cleanup is running, so put things back when it has finished",
        ));
    }

    // A refused request shows no dialog, and a declined one has taken nothing.
    let listed = restorable(state, cleanup)?;
    let asked: Vec<(Vec<u8>, u64)> = listed
        .iter()
        .map(|item| (item.original.clone(), item.allocated))
        .collect();
    confirm_put_back(state, &asked).await?;

    // The dialog may have been open for a long time: the run must still be one that can be put back, and
    // be the same entries, with the same paths, so what moves is exactly what the dialog showed. No
    // cleanup can have started meanwhile (this holds the restoring slot).
    let items = restorable(state, cleanup)?;
    if items.iter().map(shown_item).ne(listed.iter().map(shown_item)) {
        return Err(CommandError::invalid(
            "the cleanup changed while you were asked, so try again",
        ));
    }
    // Undo cannot restore this run while it is restored here.
    let taken = state.cleanups.take_by_history(cleanup);
    let platform = Arc::clone(&state.platform);

    let worked = tauri::async_runtime::spawn_blocking(move || {
        items
            .into_iter()
            .map(|item| {
                let outcome = restore_one(
                    &platform.entry_remover,
                    &item.trashed,
                    &item.original,
                    item.identity,
                    item.is_dir,
                );
                (item, outcome)
            })
            .collect::<Vec<_>>()
    })
    .await;
    let Ok(restored) = worked else {
        // Some entries may be back already: the run stays for an undo, which is refused for those.
        if let Some(done) = taken {
            state.cleanups.keep_for_retry(done);
        }
        state.scans.disk_changed();
        return Err(CommandError::io("putting things back stopped unexpectedly"));
    };

    let back: Vec<i64> = restored
        .iter()
        .filter(|(_, outcome)| outcome.is_ok())
        .map(|(item, _)| item.id)
        .collect();
    state.history.mark_restored(&back);

    if let Some(mut done) = taken {
        // What did not come back stays for an undo.
        done.results.retain(|result| match &result.outcome {
            TargetOutcome::Trashed(to) => !restored.iter().any(|(item, outcome)| {
                outcome.is_ok() && item.original == result.exact && item.trashed == *to
            }),
            _ => false,
        });
        state.cleanups.keep_for_retry(done);
    }
    // Putting things back changed the disk.
    state.scans.disk_changed();

    Ok(HistoryRestoreDto {
        restored: back.len() as u64,
        items: restored
            .into_iter()
            .map(|(item, outcome)| UndoDto {
                path: String::from_utf8_lossy(&item.original).into_owned(),
                restored: outcome.is_ok(),
                error: outcome.err(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests;
