//! Platter's cleanup rules: they look at a finished scan and say what could be cleaned.
//!
//! This crate is read-only by construction. It takes a [`Tree`](platter_core::Tree) and an
//! [`Environment`] and returns labels ([`Suggestions`]); it has no dependency but `platter-core`, uses no
//! file, process or system-clock API (`clippy.toml` and `tests/read_only.rs` keep it that way) and has
//! no code that deletes, moves or runs anything. Acting on a suggestion belongs to a later milestone and
//! to other code.
//!
//! - [`Rule`]: data. An id, a title, a [`Verdict`], a reason template and a [`Matcher`].
//! - `table.rs`: the built-in rules, one line each ([`RuleSet::builtin`]).
//! - [`evaluate`]: one top-down pass; the outermost match wins, so no byte is counted twice.
//!   [`evaluate_with`] adds the local model's labels as one more `review` suggestion.
//! - [`RuleSet::with_user`]: adds the user's own rules as `user-keep` (first) and `user-safe` (last), matched by
//!   exact folder.
//! - [`Explainer`]: what covers a folder, what is matched inside it and what beside it.
//!
//! ```text
//! let suggestions = evaluate(&tree, &environment, &RuleSet::builtin());
//! ```

mod display;
mod evaluate;
mod explain;
mod found;
mod names;
mod plan;
mod reason;
mod resolve;
mod rule;
mod table;

pub use evaluate::{ADVISOR_ID, RuleMatches, Suggestions, evaluate, evaluate_with, matches, scan_is_kept};
pub use explain::{Covering, Explainer};
pub use found::MAX_NODES;
pub use names::{fold_windows_name, same_windows_name};
pub use platter_core::dto::{SuggestionDto, SuggestionSource, SuggestionsDto, Verdict, VerdictTotals};
pub use platter_core::{Environment, Os};
pub use rule::{Age, Files, Matcher, Named, Rule, RuleSet};
