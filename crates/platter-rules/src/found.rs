//! What one rule has matched so far.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use platter_core::{NodeId, Tree};

/// How many matched nodes a suggestion lists.
pub const MAX_NODES: usize = 20;

/// Running result of one rule: sums, the largest matches, and what the display path is made from.
#[derive(Default)]
pub(crate) struct Found {
    pub allocated: u64,
    pub items: u64,
    /// Hard-linked files inside the matched nodes.
    pub shared_files: u64,
    first: Option<NodeId>,
    largest: BinaryHeap<Reverse<(u64, Reverse<u32>)>>,
    ancestor: Ancestor,
    /// Which of a name rule's names matched, by index.
    pub names: Vec<usize>,
    /// Every matched node, in the order they were met.
    pub all: Vec<NodeId>,
}

impl Found {
    /// Records one matched node. `name` is the index of the matching name for a name rule.
    pub fn add(&mut self, tree: &Tree, node: NodeId, name: Option<usize>) {
        let allocated = tree.allocated(node);

        self.allocated = self.allocated.saturating_add(allocated);
        self.items += 1;
        self.shared_files = self.shared_files.saturating_add(tree.shared_files(node));
        self.first.get_or_insert(node);
        self.all.push(node);

        self.largest.push(Reverse((allocated, Reverse(node.0))));
        if self.largest.len() > MAX_NODES {
            self.largest.pop();
        }

        self.ancestor.add(tree, node);

        if let Some(index) = name
            && !self.names.contains(&index)
        {
            self.names.push(index);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items == 0
    }

    /// The only matched node, when there is exactly one.
    pub fn single(&self) -> Option<NodeId> {
        self.first.filter(|_| self.items == 1)
    }

    /// The deepest directory that holds every match.
    pub fn common_ancestor(&self) -> Option<NodeId> {
        self.ancestor.chain.last().copied()
    }

    /// The largest matches, largest first, ties by id.
    pub fn largest_nodes(&self) -> Vec<NodeId> {
        let mut best: Vec<_> = self.largest.iter().map(|Reverse(key)| *key).collect();
        best.sort_unstable_by(|a, b| b.cmp(a));

        best.into_iter().map(|(_, Reverse(id))| NodeId(id)).collect()
    }
}

/// The common ancestor of the parents of all matches, kept as the chain from the root.
#[derive(Default)]
struct Ancestor {
    chain: Vec<NodeId>,
    last_parent: Option<NodeId>,
}

impl Ancestor {
    fn add(&mut self, tree: &Tree, node: NodeId) {
        let Some(parent) = tree.parent(node) else {
            return;
        };

        // Siblings are matched one after another; the chain of the previous parent is still right.
        if self.last_parent == Some(parent) {
            return;
        }
        self.last_parent = Some(parent);

        let chain = tree.ancestry(parent);
        if self.chain.is_empty() {
            self.chain = chain;
            return;
        }

        let shared = self.chain.iter().zip(&chain).take_while(|(a, b)| a == b).count();
        self.chain.truncate(shared);
    }
}
