//! Platter's cleanup: from the suggestions a user selected to a plan of targets that were checked one by
//! one against the disk as it is now.
//!
//! - [`CleanupAction`] (`action.rs`): how each rule's matches are cleaned (deleted, emptied, moved to
//!   the trash, or left to the tool that owns them).
//! - [`VerifiedTarget`] (`guard.rs`): a target that passed every check. Only the guard can make one, so
//!   code that acts on targets cannot be handed an unchecked path.
//! - [`plan_cleanup`] (`plan.rs`): runs the rules again, verifies every match of the selected rules and
//!   returns a [`CleanupPlan`] with what would be cleaned and what was refused and why.
//! - [`label_candidates`] (`candidates.rs`) picks the folders the local model may label; [`plan_labelled`]
//!   plans what it labelled `review`, through the same guard.
//! - [`execute`] (`execute.rs`): carries a plan out through the [`EntryRemover`](platter_core::EntryRemover)
//!   port, checking every target again right before it acts.
//!
//! Tools that own a cache (C3) are not run yet. The guard does not assume anything the scan tree cannot
//! promise (see docs/plan/NEXT.md, C1): identity comes from the [`EntryProbe`](platter_core::EntryProbe)
//! at plan time, mounts are checked on every component below the scan root, and paths are compared
//! component by component as exact bytes.

mod action;
mod candidates;
mod execute;
mod guard;
mod kept;
mod look;
mod path;
mod plan;
mod protect;

pub use action::{CleanupAction, Tool, action_for};
pub use candidates::{CandidateLimits, label_candidates};
pub use execute::{
    CleanupEvent, CleanupReport, Disk, TargetOutcome, TargetResult, execute, restore_one, undo,
};
pub use guard::{Refusal, VerifiedTarget};
pub use kept::{KEPT_LEARNING_BUDGET, KeptFolders, with_real_keeps};
pub use plan::{
    ADVISOR, CleanupPlan, MARKED, PlanGroup, Refused, plan_cleanup, plan_cleanup_with, plan_labelled,
    plan_labelled_with, plan_marked,
};
pub use protect::{PROTECTED_LEARNING_BUDGET, Protected};
