use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::{ensure, node_dto, wire_mtime};
use crate::dto::{LodNode, NodeDto};
use crate::error::QueryError;
use crate::model::{Category, NodeId, NodeKind, Tree};

/// Deepest level a LOD query will ever return (keeps recursion, and JSON nesting, bounded).
pub const MAX_LOD_DEPTH: usize = 64;

/// A bounded view of the subtree under `root` for drawing: at most `max_nodes` nodes (the root and
/// synthetic nodes included) and at most `max_depth` levels below `root`.
///
/// Nodes are admitted largest first (by allocated size) from a priority queue seeded with the root's
/// children and refilled with the children of every admitted directory, so the budget goes to what is
/// visible. Whatever does not fit under an admitted directory is merged into one synthetic child of
/// kind `collapsed`, id [`NodeId::COLLAPSED`], named `N smaller items`. Children are returned largest
/// first, the synthetic node last. A directory cut off by the depth or node limit has no children in
/// the result but still reports `hasChildren`.
pub fn lod(tree: &Tree, root: NodeId, max_nodes: usize, max_depth: usize) -> Result<LodNode, QueryError> {
    ensure(tree, root)?;

    let budget = max_nodes.max(1);
    let max_depth = max_depth.min(MAX_LOD_DEPTH);

    let mut picker = Picker {
        tree,
        budget,
        max_depth,
        used: 1,
        picked: Vec::new(),
        heap: BinaryHeap::new(),
    };
    picker.picked.push(Picked::new(tree, root, 0));
    picker.offer_children(0);
    picker.run();

    Ok(picker.build(0))
}

/// A node admitted into the result.
struct Picked {
    id: NodeId,
    depth: usize,
    total_children: usize,
    admitted: Vec<usize>,
}

impl Picked {
    fn new(tree: &Tree, id: NodeId, depth: usize) -> Self {
        Self {
            id,
            depth,
            total_children: tree.child_count(id),
            admitted: Vec::new(),
        }
    }
}

/// Max-heap entry: largest allocated first, then lowest id; carries the index of the admitted parent.
type Candidate = (u64, Reverse<u32>, usize);

struct Picker<'a> {
    tree: &'a Tree,
    budget: usize,
    max_depth: usize,
    used: usize,
    picked: Vec<Picked>,
    heap: BinaryHeap<Candidate>,
}

impl Picker<'_> {
    fn offer_children(&mut self, parent: usize) {
        let entry = &self.picked[parent];
        if entry.depth >= self.max_depth {
            return;
        }

        let mut ids: Vec<NodeId> = self.tree.children(entry.id).collect();
        if ids.len() > self.budget {
            let by_size = |a: &NodeId, b: &NodeId| {
                self.tree
                    .allocated(*b)
                    .cmp(&self.tree.allocated(*a))
                    .then(a.cmp(b))
            };
            ids.select_nth_unstable_by(self.budget, by_size);
            ids.truncate(self.budget);
        }

        for id in ids {
            self.heap.push((self.tree.allocated(id), Reverse(id.0), parent));
        }
    }

    fn run(&mut self) {
        while let Some((_, Reverse(raw), parent)) = self.heap.pop() {
            let id = NodeId(raw);
            let cost = self.cost_of_admitting(parent);
            if self.used + cost > self.budget {
                continue;
            }

            self.used += cost;
            let depth = self.picked[parent].depth + 1;
            let index = self.picked.len();
            self.picked.push(Picked::new(self.tree, id, depth));
            self.picked[parent].admitted.push(index);

            if self.tree.kind(id) == NodeKind::Dir {
                self.offer_children(index);
            }
        }
    }

    /// Nodes this admission adds: the node itself, plus the `collapsed` placeholder when it is the
    /// parent's first admitted child and siblings remain; minus the placeholder when it is the last one.
    fn cost_of_admitting(&self, parent: usize) -> usize {
        let entry = &self.picked[parent];
        let first = entry.admitted.is_empty();
        let siblings_left = entry.total_children - entry.admitted.len() > 1;

        match (first, siblings_left) {
            (true, true) => 2,
            (false, false) => 0,
            _ => 1,
        }
    }

    fn build(&self, index: usize) -> LodNode {
        let entry = &self.picked[index];
        let tree = self.tree;

        let mut admitted: Vec<usize> = entry.admitted.clone();
        admitted.sort_by(|&a, &b| {
            let (a, b) = (self.picked[a].id, self.picked[b].id);
            tree.allocated(b).cmp(&tree.allocated(a)).then(a.cmp(&b))
        });

        let mut children: Vec<LodNode> = admitted.iter().map(|&child| self.build(child)).collect();
        if !admitted.is_empty() && admitted.len() < entry.total_children {
            children.push(self.collapsed(entry));
        }

        LodNode {
            node: node_dto(tree, entry.id),
            children,
        }
    }

    /// Synthetic node standing for every child of `entry` that was not admitted.
    fn collapsed(&self, entry: &Picked) -> LodNode {
        let tree = self.tree;
        let admitted: Vec<NodeId> = {
            let mut ids: Vec<NodeId> = entry.admitted.iter().map(|&i| self.picked[i].id).collect();
            ids.sort_unstable();
            ids
        };

        let mut dto = NodeDto {
            id: NodeId::COLLAPSED,
            name: String::new(),
            kind: NodeKind::Collapsed,
            allocated: 0,
            apparent: 0,
            files: 0,
            has_children: false,
            unreadable: false,
            skipped: None,
            category: Category::Other,
            mtime: i64::MIN,
        };
        let mut by_category = [0u64; Category::COUNT];
        let mut count = 0usize;

        for id in tree
            .children(entry.id)
            .filter(|id| admitted.binary_search(id).is_err())
        {
            count += 1;
            dto.allocated = dto.allocated.saturating_add(tree.allocated(id));
            dto.apparent = dto.apparent.saturating_add(tree.apparent(id));
            dto.files = dto.files.saturating_add(tree.files(id));
            dto.mtime = dto.mtime.max(tree.mtime(id));

            let slot = &mut by_category[tree.category(id).index()];
            *slot = slot.saturating_add(tree.allocated(id));
        }

        dto.category = Category::heaviest(&by_category).unwrap_or(Category::Other);
        dto.mtime = wire_mtime(dto.mtime);
        dto.name = if count == 1 {
            "1 smaller item".to_owned()
        } else {
            format!("{count} smaller items")
        };

        LodNode {
            node: dto,
            children: Vec::new(),
        }
    }
}
