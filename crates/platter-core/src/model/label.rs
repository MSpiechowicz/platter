//! A label the local model gave a folder (docs/architecture.md, "Advisor").

use super::NodeId;

/// What the model may say about a folder. There is no `Safe`: a model label alone never makes a folder
/// safe to clean, so the type cannot hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelVerdict {
    /// Probably regenerable or disposable; the user looks at it before anything happens.
    Review,
    /// Looks like the user's own or important data.
    Keep,
}

/// The model's verdict on one folder of a scan, with its one-sentence reason (plain text, already
/// cleaned and capped by the parser).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelLabel {
    pub node: NodeId,
    pub verdict: ModelVerdict,
    pub reason: String,
}
