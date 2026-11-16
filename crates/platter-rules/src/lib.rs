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
//!
//! ```text
//! let suggestions = evaluate(&tree, &environment, &RuleSet::builtin());
//! ```

mod display;
mod evaluate;
mod found;
mod plan;
mod reason;
mod resolve;
mod rule;
mod table;

pub use evaluate::{Suggestions, evaluate};
pub use found::MAX_NODES;
pub use platter_core::dto::{SuggestionDto, SuggestionsDto, Verdict, VerdictTotals};
pub use platter_core::{Environment, Os};
pub use rule::{Age, Files, Matcher, Named, Rule, RuleSet};
