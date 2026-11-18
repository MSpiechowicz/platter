//! Finding nodes of a finished tree by name, without building a path string for any node.

use std::path::{Component, Path};

use platter_core::{NodeId, NodeKind, Tree};

use crate::names::same_windows_name;

/// The node at the absolute path `target`, found by walking its components down from the scan root.
///
/// `None` when the path lies outside the scanned folder (above it or beside it) or does not exist.
///
/// With `fold_case` (Windows) names are compared the way Windows does, so a scan started as
/// `c:\users\me` still finds `C:\Users\me\AppData`. An exact name wins over a differently spelled one.
pub(crate) fn locate(tree: &Tree, target: &Path, fold_case: bool) -> Option<NodeId> {
    let relative = if fold_case {
        strip_prefix_folded(target, tree.root_path())?
    } else {
        target.strip_prefix(tree.root_path()).ok()?.to_path_buf()
    };
    let mut current = tree.root();

    for component in relative.components() {
        let Component::Normal(name) = component else {
            return None;
        };
        let name = name.as_encoded_bytes();

        current = match child_named(tree, current, name) {
            Some(child) => child,
            None if fold_case => child_named_folded(tree, current, name)?,
            None => return None,
        };
    }

    Some(current)
}

/// `target` below `base`, the way `Path::strip_prefix` does it, but comparing names the way Windows does
/// (see [`same_windows_name`]).
pub(crate) fn strip_prefix_folded(target: &Path, base: &Path) -> Option<std::path::PathBuf> {
    let mut parts = target.components();
    for expected in base.components() {
        let part = parts.next()?;
        if !same_os_str(part.as_os_str(), expected.as_os_str()) {
            return None;
        }
    }

    Some(parts.as_path().to_path_buf())
}

/// Whether two path parts are the same name on Windows.
pub(crate) fn same_os_str(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    same_windows_name(a.as_encoded_bytes(), b.as_encoded_bytes())
}

/// The child of `dir` whose name is `name` but for case (see [`same_windows_name`]), when exactly one is. Children are sorted
/// by exact name, so this looks at all of them: only for a name that has no exact match.
fn child_named_folded(tree: &Tree, dir: NodeId, name: &[u8]) -> Option<NodeId> {
    let mut found = tree
        .children(dir)
        .filter(|&child| same_windows_name(tree.name(child), name));
    let only = found.next()?;

    found.next().is_none().then_some(only)
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

/// Whether the parent of `node` holds a regular file called `marker` (not a folder, link or mount).
pub(crate) fn has_sibling_file(tree: &Tree, node: NodeId, marker: &str) -> bool {
    let Some(parent) = tree.parent(node) else {
        return false;
    };

    child_named(tree, parent, marker.as_bytes()).is_some_and(|sibling| tree.kind(sibling) == NodeKind::File)
}

/// The nearest folder, from the parent of `node` up to and including the scan root, that holds every one
/// of `markers` as a regular file (not a folder, link or mount). `None` when there is none, and for no
/// markers. Only folders inside the scan are looked at: a workspace above the scan root is not seen.
///
/// This is the one place that decides the workspace alternative of a name rule. The cleanup plan asks it
/// for the folder it has to check again before it acts.
pub fn workspace_root(tree: &Tree, node: NodeId, markers: &[&str]) -> Option<NodeId> {
    if markers.is_empty() {
        return None;
    }

    let mut folder = tree.parent(node);
    while let Some(dir) = folder {
        let holds_all = markers.iter().all(|marker| {
            child_named(tree, dir, marker.as_bytes()).is_some_and(|file| tree.kind(file) == NodeKind::File)
        });
        if holds_all {
            return Some(dir);
        }

        folder = tree.parent(dir);
    }

    None
}
