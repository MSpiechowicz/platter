//! Helpers shared by the tests that need a scanned tree.
#![allow(dead_code)]

use std::path::Path;

use platter_core::testing::{FakeFs, NOW, scan_fake};
use platter_core::{NodeId, Tree};

pub const DAY: i64 = 86_400;

pub fn tree_of(fs: &FakeFs) -> Tree {
    scan_fake(fs, false)
}

/// The node at an absolute path of the scanned fake.
pub fn find(tree: &Tree, path: &str) -> NodeId {
    let mut node = tree.root();

    for part in Path::new(path).components().skip(1) {
        let name = part.as_os_str().as_encoded_bytes();
        node = tree
            .children(node)
            .find(|child| tree.name(*child) == name)
            .unwrap_or_else(|| panic!("{path} is not in the tree"));
    }
    node
}

/// A time `days` before the scan.
pub fn ago(days: i64) -> i64 {
    NOW - days * DAY
}
