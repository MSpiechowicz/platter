//! Platter core: the domain of a disk-space analyzer, free of any operating-system code.
//!
//! - [`model`]: the arena [`Tree`], node ids, categories, totals, volumes.
//! - [`ports`]: the traits an OS adapter implements ([`DirSource`], [`VolumeProvider`]).
//! - [`mod@scan`]: the parallel walker that turns a `DirSource` into a `Tree`.
//! - [`budget`]: the entry and memory allowance a scan reserves while directories are listed.
//! - [`query`]: paged children, level-of-detail, path and summary queries returning [`dto`] types.
//! - [`dto`]: the serde wire types shared with the frontend.
//! - `testing` (feature `testing`): an in-memory `DirSource` for tests.
//!
//! A typical caller (CLI, IPC layer):
//!
//! ```text
//! let tree = scan(&platform.dir_source, root, &ScanConfig::default(), &cancel, &|p| emit(p))?;
//! let page = query::children(&tree, tree.root(), 0, 50, SortKey::SizeDesc)?;
//! ```

pub mod budget;
pub mod dto;
pub mod error;
pub mod model;
pub mod ports;
pub mod query;
pub mod scan;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use budget::{BudgetLimits, EntryBudget, Lease, LimitHit};
pub use error::{QueryError, ScanError, ScanLimit};
pub use model::{
    Category, EntryKind, Environment, MAX_RULE_PATH_BYTES, MAX_RULES_FILE_BYTES, MAX_USER_RULES, ModelLabel,
    ModelVerdict, NodeId, NodeKind, Os, Progress, RULES_FILE_VERSION, RulesFileError, ScanWarning,
    SkipReason, Totals, Tree, UserDir, UserRule, UserRulesFile, UserVerdict, Volume, check_folder,
    parse_user_rules, render_user_rules,
};
pub use ports::{
    DirListing, DirSource, EntryMeta, EntryProbe, EntryRemover, ExpectedEntry, ProbedEntry, RawEntry,
    RemoveMode, Removed, RootInfo, SourceOptions, ToolCommand, ToolFailure, ToolRunner, VolumeProvider,
};
pub use query::{FolderMatch, find_folders};
pub use scan::{CancelToken, DEFAULT_MAX_ENTRIES, DEFAULT_MAX_MEMORY_BYTES, ProgressFn, ScanConfig, scan};
