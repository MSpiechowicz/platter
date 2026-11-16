//! Domain model: the arena [`Tree`], node identity and classification, scan totals and volumes.

mod category;
pub mod category_table;
mod environment;
mod id;
mod kind;
mod totals;
mod tree;
mod volume;
mod warning;

pub use category::Category;
pub use environment::{Environment, Os};
pub use id::NodeId;
pub use kind::{EntryKind, NodeKind, SkipReason};
pub use totals::{Progress, Totals};
pub use tree::Tree;
pub(crate) use tree::{DirInfo, NO_DIR, NO_PARENT, Node};
pub use volume::Volume;
pub use warning::ScanWarning;
