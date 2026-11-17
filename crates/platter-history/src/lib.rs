//! Platter's history: what happened to the disk over time. A usage sample per local day, the scans
//! that finished, and the cleanups that ran (with what they moved to the trash, so it can be put back),
//! kept in one SQLite file, and the statistics the History view shows.
//!
//! - [`store`]: the file, its checks and migrations, and writing and reading records.
//! - [`stats`]: the window, the "without cleanups" series, the growth rate. Pure.
//! - [`probe`]: whether a run's trashed entries are still in the trash.
//! - [`day`]: local calendar days.
//! - `testing` (feature `testing`): a way to lock a history file, for other crates' tests.
//!
//! The contract is in `docs/architecture.md`, "History".

pub mod day;
pub mod probe;
pub mod stats;
pub mod store;
#[cfg(feature = "testing")]
pub mod testing;

pub use probe::{CHECK_CAP, Checked, RunStatus, Status, check, statuses};
pub use stats::{MAX_WINDOW_DAYS, MIN_DAYS, Summary, first_day, summarize, window_start};
pub use store::{
    Method, MethodKind, NewCleanup, NewItem, Problem, Sample, ScanRecord, Store, StoredCleanup, StoredItem,
    StoredVolume, Usage, VolumeRef,
};
