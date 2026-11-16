//! Domain model: the arena [`Tree`], node identity and classification, scan totals and volumes.

mod category;
pub mod category_table;
mod environment;
mod id;
mod kind;
mod label;
mod totals;
mod tree;
mod user_rule;
mod volume;
mod warning;

pub use category::Category;
pub use environment::{Environment, Os, UserDir};
pub use id::NodeId;
pub use kind::{EntryKind, NodeKind, SkipReason};
pub use label::{ModelLabel, ModelVerdict};
pub use totals::{Progress, Totals};
pub use tree::Tree;
pub(crate) use tree::{DirInfo, NO_DIR, NO_PARENT, Node};
pub use user_rule::{
    MAX_RULE_PATH_BYTES, MAX_RULES_FILE_BYTES, MAX_USER_RULES, RULES_FILE_VERSION, RulesFileError, UserRule,
    UserRulesFile, UserVerdict, check_folder, parse as parse_user_rules, render as render_user_rules,
};
pub use volume::Volume;
pub use warning::ScanWarning;
