mod aggregate;
mod budget;
mod categories;
mod progress;
mod robustness;
mod rules;
mod stack;

use crate::model::{NodeId, Tree};

/// Finds a node by `/`-separated path of names below the root.
pub(super) fn find(tree: &Tree, path: &str) -> NodeId {
    let mut current = tree.root();

    for part in path.split('/').filter(|p| !p.is_empty()) {
        current = tree
            .children(current)
            .find(|&child| tree.name(child) == part.as_bytes())
            .unwrap_or_else(|| panic!("no node {path}"));
    }
    current
}
