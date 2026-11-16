use super::find;
use crate::model::{Category, NodeKind};
use crate::testing::{FakeFs, scan_fake};

fn sample() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.dir("/a").set_size("/a", 4096, 4096);
    fs.file("/a/one.txt", 100).file_sized("/a/two.exe", 300, 1000);
    fs.dir("/b/c")
        .set_size("/b", 4096, 4096)
        .set_size("/b/c", 4096, 4096);
    fs.file("/b/c/three.mp4", 5000).set_mtime("/b/c/three.mp4", 900);
    fs.set_mtime("/a/one.txt", 500);
    fs
}

#[test]
fn directories_aggregate_their_subtree() {
    let tree = scan_fake(&sample(), false);

    let a = find(&tree, "a");
    assert_eq!(tree.allocated(a), 4096 + 100 + 300);
    assert_eq!(
        tree.apparent(a),
        100 + 1000,
        "a directory's own length is not data"
    );
    assert_eq!(tree.files(a), 2);

    let b = find(&tree, "b");
    assert_eq!(tree.allocated(b), 4096 + 4096 + 5000);
    assert_eq!(tree.files(b), 1);
    assert_eq!(tree.child_count(b), 1);
}

#[test]
fn totals_match_the_root_and_count_entries() {
    let tree = scan_fake(&sample(), false);
    let totals = tree.totals();

    assert_eq!(totals.allocated, 4096 * 3 + 100 + 300 + 5000);
    assert_eq!(totals.apparent, 100 + 1000 + 5000);
    assert_eq!(totals.allocated, tree.allocated(tree.root()));
    assert_eq!(totals.files, 3);
    assert_eq!(totals.dirs, 3);
    assert_eq!(totals.errors, 0);
    assert_eq!(tree.len(), 7);
}

#[test]
fn directory_mtime_is_the_newest_inside() {
    let tree = scan_fake(&sample(), false);

    assert_eq!(tree.mtime(find(&tree, "a")), 500);
    assert_eq!(tree.mtime(find(&tree, "b")), 900);
    assert_eq!(tree.mtime(tree.root()), 900);
}

#[test]
fn directory_category_is_the_one_with_most_bytes() {
    let tree = scan_fake(&sample(), false);

    assert_eq!(tree.category(find(&tree, "b")), Category::Video);
    assert_eq!(tree.category(find(&tree, "b/c/three.mp4")), Category::Video);
    assert_eq!(
        tree.category(find(&tree, "a")),
        Category::App,
        "two.exe (300) outweighs one.txt (100)"
    );
    assert_eq!(tree.category(tree.root()), Category::Video);
}

#[test]
fn well_known_directories_impose_their_category_on_the_subtree() {
    let mut fs = FakeFs::new();
    fs.file("/proj/node_modules/lib/index.js", 1000);
    fs.file("/proj/node_modules/lib/logo.png", 4000);
    fs.file("/proj/src/main.rs", 10);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.category(find(&tree, "proj/node_modules")), Category::Package);
    assert_eq!(
        tree.category(find(&tree, "proj/node_modules/lib/index.js")),
        Category::Package
    );
    assert_eq!(
        tree.category(find(&tree, "proj/node_modules/lib/logo.png")),
        Category::Package
    );
    assert_eq!(tree.category(find(&tree, "proj/src/main.rs")), Category::Code);
    assert_eq!(tree.category(find(&tree, "proj")), Category::Package);
}

#[test]
fn children_are_contiguous_sorted_and_follow_their_parent() {
    let tree = scan_fake(&sample(), false);

    for index in 0..tree.len() as u32 {
        let id = crate::model::NodeId(index);
        let range = tree.children(id);
        let ids: Vec<_> = range.collect();

        for window in ids.windows(2) {
            assert_eq!(window[1].0, window[0].0 + 1, "children of {index} are contiguous");
            assert!(
                tree.name(window[0]) < tree.name(window[1]),
                "children are name-sorted"
            );
        }
        for child in ids {
            assert!(child.0 > index, "a parent precedes its children");
            assert_eq!(tree.parent(child), Some(id));
        }
    }
    assert_eq!(tree.kind(find(&tree, "a")), NodeKind::Dir);
}

#[test]
fn root_is_named_after_the_scanned_directory() {
    let mut fs = FakeFs::new();
    fs.file("/data/x/y.txt", 1);

    let config = crate::scan::ScanConfig::default();
    let tree = crate::scan::scan(
        &fs,
        std::path::Path::new("/data/x"),
        &config,
        &crate::scan::CancelToken::new(),
        &|_| {},
    )
    .unwrap();

    assert_eq!(&*tree.name_lossy(tree.root()), "x");
    assert_eq!(tree.root_path(), std::path::Path::new("/data/x"));
    assert_eq!(
        tree.path_of(find(&tree, "y.txt")),
        std::path::Path::new("/data/x/y.txt")
    );
}

#[test]
fn sizes_saturate_instead_of_overflowing() {
    let huge = i64::MAX as u64;
    let mut fs = FakeFs::new();
    fs.file("/a", huge).file("/b", huge).file("/c", huge);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.allocated(tree.root()), u64::MAX);
    assert_eq!(tree.apparent(tree.root()), u64::MAX);
    assert_eq!(tree.totals().allocated, u64::MAX);
    assert_eq!(tree.totals().apparent, u64::MAX);
    assert_eq!(tree.totals().files, 3);
}
