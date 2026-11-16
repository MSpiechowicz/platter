//! Finding nodes of a finished tree by name, without building a path string for any node.

use std::path::{Component, Path};

use platter_core::{NodeId, NodeKind, Tree};

/// The node at the absolute path `target`, found by walking its components down from the scan root.
///
/// `None` when the path lies outside the scanned folder (above it or beside it) or does not exist.
pub(crate) fn locate(tree: &Tree, target: &Path) -> Option<NodeId> {
    let relative = target.strip_prefix(tree.root_path()).ok()?;
    let mut current = tree.root();

    for component in relative.components() {
        let Component::Normal(name) = component else {
            return None;
        };
        current = child_named(tree, current, name.as_encoded_bytes())?;
    }

    Some(current)
}

/// The child of `dir` called `name`. Children are stored sorted by name, so this is a binary search.
pub(crate) fn child_named(tree: &Tree, dir: NodeId, name: &[u8]) -> Option<NodeId> {
    let mut children = tree.children(dir);
    let count = u32::try_from(children.len()).ok()?;
    let first = children.next()?.0;

    let (mut low, mut high) = (first, first + count);
    while low < high {
        let middle = low + (high - low) / 2;

        match tree.name(NodeId(middle)).cmp(name) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Some(NodeId(middle)),
        }
    }

    None
}

/// Whether the parent of `node` holds a file (not a folder) called `marker`.
pub(crate) fn has_sibling_file(tree: &Tree, node: NodeId, marker: &str) -> bool {
    let Some(parent) = tree.parent(node) else {
        return false;
    };

    child_named(tree, parent, marker.as_bytes()).is_some_and(|sibling| tree.kind(sibling) != NodeKind::Dir)
}
