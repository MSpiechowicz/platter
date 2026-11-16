use serde::{Deserialize, Serialize};

/// Index of a node in a [`Tree`](super::Tree) arena. Serializes as a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The scan root; always node 0.
    pub const ROOT: NodeId = NodeId(0);

    /// Id of the synthetic `collapsed` node produced by LOD queries. Never a real node.
    pub const COLLAPSED: NodeId = NodeId(u32::MAX);

    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
