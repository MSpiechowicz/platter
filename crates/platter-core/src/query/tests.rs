use std::path::Path;

use crate::dto::{AgeBucket, LodNode, SortKey};
use crate::error::QueryError;
use crate::model::{Category, NodeId, NodeKind, Tree};
use crate::query::{NameFilter, children, children_matching, lod, node, path, summary};
use crate::scan::{CancelToken, ScanConfig, scan};
use crate::testing::{FakeFs, NOW, scan_fake};

const DAY: i64 = 86_400;

fn names(items: &[crate::dto::NodeDto]) -> Vec<&str> {
    items.iter().map(|n| n.name.as_str()).collect()
}

fn find(tree: &Tree, path: &str) -> NodeId {
    let mut current = tree.root();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        current = tree
            .children(current)
            .find(|&c| tree.name(c) == part.as_bytes())
            .expect("node exists");
    }
    current
}

/// /dir with five files of distinct size, name length, mtime and (via subdirs) file count.
fn paging_fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/dir/b.txt", 300).set_mtime("/dir/b.txt", 10);
    fs.file("/dir/A.txt", 100).set_mtime("/dir/A.txt", 50);
    fs.file("/dir/c.txt", 500).set_mtime("/dir/c.txt", 30);
    fs.file("/dir/many/1", 1)
        .file("/dir/many/2", 1)
        .file("/dir/many/3", 1)
        .set_mtime("/dir/many/1", 20);
    fs.file("/dir/d.txt", 200).set_mtime("/dir/d.txt", 40);
    fs
}

#[test]
fn children_sort_by_every_key() {
    let tree = scan_fake(&paging_fs(), false);
    let dir = find(&tree, "dir");
    let order = |sort| names(&children(&tree, dir, 0, 100, sort).unwrap().items).join(",");

    assert_eq!(order(SortKey::SizeDesc), "c.txt,b.txt,d.txt,A.txt,many");
    assert_eq!(order(SortKey::SizeAsc), "many,A.txt,d.txt,b.txt,c.txt");
    assert_eq!(order(SortKey::NameAsc), "A.txt,b.txt,c.txt,d.txt,many");
    assert_eq!(order(SortKey::FilesDesc), "many,A.txt,b.txt,c.txt,d.txt");
    assert_eq!(order(SortKey::MtimeDesc), "A.txt,d.txt,c.txt,many,b.txt");
}

#[test]
fn children_page_with_offset_and_limit() {
    let tree = scan_fake(&paging_fs(), false);
    let dir = find(&tree, "dir");

    let page = children(&tree, dir, 1, 2, SortKey::SizeDesc).unwrap();
    assert_eq!(page.parent_id, dir);
    assert_eq!(page.total, 5);
    assert_eq!(names(&page.items), ["b.txt", "d.txt"]);

    let tail = children(&tree, dir, 4, 10, SortKey::SizeDesc).unwrap();
    assert_eq!(names(&tail.items), ["many"]);

    let beyond = children(&tree, dir, 9, 10, SortKey::SizeDesc).unwrap();
    assert_eq!(beyond.total, 5);
    assert!(beyond.items.is_empty());

    let none = children(&tree, dir, 0, 0, SortKey::SizeDesc).unwrap();
    assert!(none.items.is_empty());
}

#[test]
fn a_name_filter_keeps_matching_children_ignoring_case_and_counts_only_them() {
    let tree = scan_fake(&paging_fs(), false);
    let dir = find(&tree, "dir");
    let filter = NameFilter::new("TXT").unwrap();

    let page = children_matching(&tree, dir, Some(&filter), 1, 2, SortKey::SizeDesc).unwrap();

    assert_eq!(page.total, 4, "total counts the matches, not the folder");
    assert_eq!(names(&page.items), ["b.txt", "d.txt"]);

    let a = NameFilter::new("a.").unwrap();
    let only_a = children_matching(&tree, dir, Some(&a), 0, 10, SortKey::NameAsc).unwrap();
    assert_eq!(names(&only_a.items), ["A.txt"]);

    let none = NameFilter::new("zzz").unwrap();
    let empty = children_matching(&tree, dir, Some(&none), 0, 10, SortKey::NameAsc).unwrap();
    assert_eq!((empty.total, empty.items.len()), (0, 0));
}

#[test]
fn a_blank_name_filter_filters_nothing() {
    assert_eq!(NameFilter::new(""), None);
    assert_eq!(NameFilter::new("   "), None);
}

#[test]
fn a_name_filter_ignores_case_beyond_ascii_and_reads_invalid_utf8_as_shown() {
    let filter = NameFilter::new("ŻÓŁ").unwrap();
    assert!(filter.matches("zażółć".as_bytes()));
    assert!(!filter.matches(b"zazolc"));

    let replacement = NameFilter::new("a\u{fffd}b").unwrap();
    assert!(replacement.matches(b"xa\xffbx"));
    assert!(NameFilter::new("A").unwrap().matches(b"\xffa"));
}

#[test]
fn a_name_filter_finds_a_match_among_many_children() {
    let mut fs = FakeFs::new();
    for i in 0..5000 {
        fs.file(&format!("/big/file-{i:05}"), 1);
    }
    fs.file("/big/Needle.txt", 1);
    let tree = scan_fake(&fs, false);
    let filter = NameFilter::new("needle").unwrap();

    let page = children_matching(&tree, find(&tree, "big"), Some(&filter), 0, 10, SortKey::SizeDesc).unwrap();

    assert_eq!((page.total, names(&page.items)), (1, vec!["Needle.txt"]));
}

#[test]
fn paging_through_a_directory_visits_each_child_once_even_with_ties() {
    let mut fs = FakeFs::new();
    for i in 0..25 {
        fs.file(&format!("/t/f{i:02}"), 10);
    }
    let tree = scan_fake(&fs, false);
    let dir = find(&tree, "t");

    let mut seen = Vec::new();
    for offset in (0..25).step_by(7) {
        seen.extend(children(&tree, dir, offset, 7, SortKey::SizeDesc).unwrap().items);
    }

    let ids: std::collections::BTreeSet<_> = seen.iter().map(|n| n.id).collect();
    assert_eq!(seen.len(), 25);
    assert_eq!(ids.len(), 25);
    assert_eq!(
        names(&seen[..3]),
        ["f00", "f01", "f02"],
        "ties fall back to name order"
    );
}

#[test]
fn unknown_nodes_are_not_found() {
    let tree = scan_fake(&paging_fs(), false);
    let missing = NodeId(9999);

    assert_eq!(
        children(&tree, missing, 0, 1, SortKey::SizeDesc).unwrap_err(),
        QueryError::NotFound(missing)
    );
    assert!(node(&tree, missing).is_err());
    assert!(lod(&tree, missing, 10, 3).is_err());
    assert!(path(&tree, missing).is_err());
    assert!(summary(&tree, missing).is_err());
}

#[test]
fn node_describes_a_directory_and_a_file() {
    let tree = scan_fake(&paging_fs(), false);

    let many = node(&tree, find(&tree, "dir/many")).unwrap();
    assert_eq!(many.name, "many");
    assert_eq!(many.kind, NodeKind::Dir);
    assert_eq!(many.files, 3);
    assert!(many.has_children);
    assert_eq!(many.allocated, 3);

    let file = node(&tree, find(&tree, "dir/c.txt")).unwrap();
    assert_eq!(file.kind, NodeKind::File);
    assert_eq!(file.files, 1);
    assert!(!file.has_children);
    assert_eq!(file.category, Category::Document);
    assert_eq!(file.mtime, 30);
}

fn count(lod: &LodNode) -> usize {
    1 + lod.children.iter().map(count).sum::<usize>()
}

fn depth(lod: &LodNode) -> usize {
    lod.children.iter().map(|c| 1 + depth(c)).max().unwrap_or(0)
}

fn lod_fs() -> FakeFs {
    let mut fs = FakeFs::new();
    for i in 0..40 {
        fs.file(&format!("/big/f{i:02}"), 1000 + i);
    }
    for i in 0..30 {
        fs.file(&format!("/small/f{i:02}"), 1 + i);
    }
    fs.file("/medium/a/b/c/d/deep.bin", 5000);
    fs
}

#[test]
fn lod_respects_max_nodes_and_merges_the_rest() {
    let tree = scan_fake(&lod_fs(), false);
    let result = lod(&tree, tree.root(), 20, 10).unwrap();

    assert_eq!(count(&result), 20);

    let big = result
        .children
        .iter()
        .find(|c| c.node.name == "big")
        .expect("big is kept");
    let collapsed = big.children.last().unwrap();
    assert_eq!(collapsed.node.id, NodeId::COLLAPSED);
    assert_eq!(collapsed.node.id.0, u32::MAX);
    assert_eq!(collapsed.node.kind, NodeKind::Collapsed);
    assert!(collapsed.children.is_empty());

    let shown: u64 = big.children[..big.children.len() - 1]
        .iter()
        .map(|c| c.node.allocated)
        .sum();
    assert_eq!(
        shown + collapsed.node.allocated,
        big.node.allocated,
        "nothing is lost in the merge"
    );
    let omitted = 40 - (big.children.len() - 1);
    assert_eq!(collapsed.node.name, format!("{omitted} smaller items"));
    assert_eq!(collapsed.node.files, omitted as u64);
}

#[test]
fn lod_expands_the_largest_directories_first() {
    let tree = scan_fake(&lod_fs(), false);
    let result = lod(&tree, tree.root(), 12, 10).unwrap();

    let by_name = |name: &str| result.children.iter().find(|c| c.node.name == name);
    assert!(
        !by_name("big").unwrap().children.is_empty(),
        "biggest directory is expanded"
    );
    assert!(
        by_name("small").is_none(),
        "the smallest directory loses out to big's files"
    );

    let collapsed = result.children.last().unwrap();
    assert_eq!(collapsed.node.kind, NodeKind::Collapsed);
    assert_eq!(collapsed.node.name, "1 smaller item");
    assert_eq!(collapsed.node.allocated, (1..=30).sum::<u64>());
    assert_eq!(count(&result), 12);
}

#[test]
fn lod_children_are_largest_first_with_the_collapsed_node_last() {
    let tree = scan_fake(&lod_fs(), false);
    let result = lod(&tree, tree.root(), 15, 10).unwrap();

    for node in std::iter::once(&result).chain(&result.children) {
        let sizes: Vec<u64> = node
            .children
            .iter()
            .filter(|c| c.node.kind != NodeKind::Collapsed)
            .map(|c| c.node.allocated)
            .collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
    }
}

#[test]
fn lod_respects_max_depth() {
    let tree = scan_fake(&lod_fs(), false);

    let shallow = lod(&tree, tree.root(), 1000, 2).unwrap();
    assert_eq!(depth(&shallow), 2);
    let medium = shallow.children.iter().find(|c| c.node.name == "medium").unwrap();
    let a = &medium.children[0];
    assert_eq!(a.node.name, "a");
    assert!(a.children.is_empty(), "cut off by depth");
    assert!(a.node.has_children);

    let full = lod(&tree, tree.root(), 1000, 20).unwrap();
    assert_eq!(depth(&full), 6);
    assert_eq!(count(&full), tree.len(), "everything fits, no collapsed nodes");
}

#[test]
fn lod_with_a_tiny_budget_still_returns_the_root() {
    let tree = scan_fake(&lod_fs(), false);

    let only_root = lod(&tree, tree.root(), 1, 5).unwrap();
    assert_eq!(count(&only_root), 1);
    assert_eq!(lod(&tree, tree.root(), 0, 5).unwrap().children.len(), 0);

    let zero_depth = lod(&tree, tree.root(), 100, 0).unwrap();
    assert!(zero_depth.children.is_empty());
}

#[test]
fn lod_of_a_subtree_starts_at_that_node() {
    let tree = scan_fake(&lod_fs(), false);
    let result = lod(&tree, find(&tree, "medium"), 100, 10).unwrap();

    assert_eq!(result.node.name, "medium");
    assert_eq!(count(&result), 6);
}

#[test]
fn path_returns_full_path_and_segments() {
    let mut fs = FakeFs::new();
    fs.file("/home/me/docs/a.txt", 1);
    let tree = scan(
        &fs,
        Path::new("/home/me"),
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .unwrap();

    let id = find(&tree, "docs/a.txt");
    let dto = path(&tree, id).unwrap();
    assert_eq!(dto.path, "/home/me/docs/a.txt");
    assert_eq!(
        dto.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["me", "docs", "a.txt"]
    );
    assert_eq!(dto.segments[0].id, tree.root());
    assert_eq!(dto.segments[2].id, id);

    let root = path(&tree, tree.root()).unwrap();
    assert_eq!(root.path, "/home/me");
    assert_eq!(root.segments.len(), 1);
}

#[test]
fn path_bytes_join_the_root_and_names_without_doubling_separators() {
    let mut fs = FakeFs::new();
    fs.file("/home/me/docs/a.txt", 1);
    let scan_at = |root: &str| {
        scan(
            &fs,
            Path::new(root),
            &ScanConfig::default(),
            &CancelToken::new(),
            &|_| {},
        )
        .unwrap()
    };

    let tree = scan_at("/home/me");
    assert_eq!(tree.path_bytes(find(&tree, "docs/a.txt")), b"/home/me/docs/a.txt");
    assert_eq!(tree.path_bytes(tree.root()), b"/home/me");

    let tree = scan_at("/");
    assert_eq!(tree.path_bytes(find(&tree, "home/me")), b"/home/me");
    assert_eq!(tree.path_bytes(tree.root()), b"/");
}

#[test]
fn summary_breaks_down_by_category_and_age() {
    let mut fs = FakeFs::new();
    fs.file("/s/a.mp4", 1000).set_mtime("/s/a.mp4", NOW - DAY);
    fs.file("/s/b.mp4", 500).set_mtime("/s/b.mp4", NOW - 20 * DAY);
    fs.file("/s/c.jpg", 200).set_mtime("/s/c.jpg", NOW - 60 * DAY);
    fs.file("/s/d.pdf", 100).set_mtime("/s/d.pdf", NOW - 200 * DAY);
    fs.file("/s/e.zip", 50).set_mtime("/s/e.zip", NOW - 500 * DAY);
    fs.file("/s/f.txt", 25).set_mtime("/s/f.txt", NOW - 1000 * DAY);
    fs.file("/s/g.txt", 5).set_mtime("/s/g.txt", NOW + 3 * DAY);
    fs.set_size("/s", 4096, 4096);

    let tree = scan_fake(&fs, false);
    let result = summary(&tree, find(&tree, "s")).unwrap();

    let by_cat: Vec<_> = result
        .by_category
        .iter()
        .map(|c| (c.category, c.allocated, c.files))
        .collect();
    assert_eq!(
        by_cat[0],
        (Category::Video, 1500 + 4096, 2),
        "video dominates /s, so its own blocks count as video"
    );
    assert!(by_cat.contains(&(Category::Image, 200, 1)));
    assert!(by_cat.contains(&(Category::Document, 130, 3)));
    assert!(by_cat.contains(&(Category::Archive, 50, 1)));
    let total: u64 = result.by_category.iter().map(|c| c.allocated).sum();
    assert_eq!(total, result.node.allocated, "every byte is attributed once");

    let buckets: Vec<_> = result.by_age.iter().map(|a| a.bucket).collect();
    assert_eq!(
        buckets,
        [
            AgeBucket::Week,
            AgeBucket::Month,
            AgeBucket::Quarter,
            AgeBucket::Year,
            AgeBucket::TwoYears,
            AgeBucket::Older
        ]
    );
    let sizes: Vec<_> = result.by_age.iter().map(|a| a.allocated).collect();
    // The directory's own blocks follow its newest mtime, which is g.txt's (in the future: week).
    assert_eq!(sizes, [1000 + 5 + 4096, 500, 200, 100, 50, 25]);
    let files: u64 = result.by_age.iter().map(|a| a.files).sum();
    assert_eq!(files, 7);
}

#[test]
fn summary_age_bucket_edges() {
    let mut fs = FakeFs::new();
    let edges = [
        (7, "w"),
        (8, "m"),
        (30, "m2"),
        (31, "q"),
        (90, "q2"),
        (91, "y"),
        (365, "y2"),
        (366, "t"),
        (730, "t2"),
        (731, "o"),
    ];
    for (days, name) in edges {
        fs.file(&format!("/e/{name}"), 1)
            .set_mtime(&format!("/e/{name}"), NOW - days * DAY);
    }
    let tree = scan_fake(&fs, false);

    let result = summary(&tree, find(&tree, "e")).unwrap();
    let files: Vec<_> = result.by_age.iter().map(|a| a.files).collect();
    // "up to N days" is inclusive: exactly 7 days is still a week.
    assert_eq!(files, [1, 2, 2, 2, 2, 1]);
}

#[test]
fn largest_dirs_of_a_flat_folder_are_the_eight_biggest_children() {
    let mut fs = FakeFs::new();
    for i in 0..20 {
        fs.file(&format!("/r/child{i:02}/own.bin"), 100 * (i + 1));
        fs.file(&format!("/r/child{i:02}/grand/big.bin"), 10 * (i + 1));
        fs.file(&format!("/r/child{i:02}/grand/great/huge.bin"), 1);
    }
    fs.file("/r/loose.bin", 1_000_000);
    let tree = scan_fake(&fs, false);

    let root = find(&tree, "r");
    let result = summary(&tree, root).unwrap();

    assert_eq!(result.largest_dirs.len(), 8);
    assert!(result.largest_dirs.iter().all(|d| d.kind == NodeKind::Dir));
    assert!(result.largest_dirs.iter().all(|d| d.id != root));
    assert!(
        result
            .largest_dirs
            .windows(2)
            .all(|w| w[0].allocated >= w[1].allocated)
    );

    let names: Vec<_> = result.largest_dirs.iter().map(|d| d.name.as_str()).collect();
    let expected: Vec<_> = (12..20).rev().map(|i| format!("child{i:02}")).collect();
    assert_eq!(names, expected, "enough children: nothing is expanded");
}

fn largest_dir_names(fs: &FakeFs, root: &str) -> Vec<String> {
    let tree = scan_fake(fs, false);
    let result = summary(&tree, find(&tree, root)).unwrap();

    result.largest_dirs.into_iter().map(|dir| dir.name).collect()
}

#[test]
fn largest_dirs_leave_out_a_directory_dominated_by_its_child() {
    let mut fs = FakeFs::new();
    // /h/user holds 1000 of /h/user's parent's 1100 bytes (about 91%), so only the deeper one is listed.
    fs.file("/h/home/user/data.bin", 1000);
    fs.file("/h/home/readme", 100);
    fs.file("/h/other/x.bin", 50);

    assert_eq!(largest_dir_names(&fs, "h"), ["user", "other"]);
}

#[test]
fn largest_dirs_keep_a_directory_when_no_child_reaches_the_share() {
    let mut fs = FakeFs::new();
    // 790 of 1000 is 79%: just under the threshold, so `home` is not replaced. It has a single child
    // directory, which would only swap it for a smaller folder, so it is not expanded to fill the list.
    fs.file("/h/home/user/data.bin", 790);
    fs.file("/h/home/readme", 210);

    assert_eq!(largest_dir_names(&fs, "h"), ["home"]);
}

#[test]
fn largest_dirs_drop_a_directory_whose_child_holds_exactly_the_share() {
    let mut fs = FakeFs::new();
    fs.file("/h/home/user/data.bin", 800);
    fs.file("/h/home/readme", 200);

    assert_eq!(largest_dir_names(&fs, "h"), ["user"], "80% is enough");
}

#[test]
fn largest_dirs_follow_a_dominant_chain_to_its_bottom() {
    let mut fs = FakeFs::new();
    // a > b > c each hold (nearly) everything of their parent; only the bottom `c` is listed.
    fs.file("/h/a/b/c/data.bin", 1000);
    fs.file("/h/a/note", 10);
    fs.file("/h/z/data.bin", 5);

    assert_eq!(largest_dir_names(&fs, "h"), ["c", "z"]);
}

#[test]
fn largest_dirs_follow_a_dominant_chain_for_at_most_six_levels() {
    let mut fs = FakeFs::new();
    fs.file("/h/d1/d2/d3/d4/d5/d6/d7/d8/data.bin", 1000);

    // d1 is level 1, so d6 is level 6, the deepest a candidate may be.
    assert_eq!(largest_dir_names(&fs, "h"), ["d6"]);
}

#[test]
fn largest_dirs_fill_the_list_after_leaving_dominated_directories_out() {
    let mut fs = FakeFs::new();
    for i in 0..5 {
        // Each child is dominated by its only grandchild, so the five children drop out.
        fs.file(&format!("/r/c{i}/g{i}/data.bin"), 1000 + i);
    }
    for i in 0..5 {
        fs.file(&format!("/r/s{i}/data.bin"), 10 + i);
    }

    let names = largest_dir_names(&fs, "r");
    assert_eq!(names, ["g4", "g3", "g2", "g1", "g0", "s4", "s3", "s2"]);
}

#[test]
fn largest_dirs_expand_the_largest_folders_to_fill_the_list() {
    let mut fs = FakeFs::new();
    // a: three equal folders (900), b: two (490 with a file), d: three (450), c: a folder with a file.
    for name in ["a1", "a2", "a3"] {
        fs.file(&format!("/r/a/{name}/x.bin"), 300);
    }
    fs.file("/r/b/b1/x.bin", 200)
        .file("/r/b/b2/x.bin", 190)
        .file("/r/b/note", 100);
    for name in ["d1", "d2", "d3"] {
        fs.file(&format!("/r/d/{name}/x.bin"), 150);
    }
    fs.file("/r/c/x.bin", 100);

    // 4 candidates -> expand a (6) -> expand b (7) -> expand d (9); the eight largest remain, and the
    // smallest, `c`, falls out. Equal sizes are ordered by id (creation order here).
    let names = largest_dir_names(&fs, "r");
    assert_eq!(names, ["a1", "a2", "a3", "b1", "b2", "d1", "d2", "d3"]);
}

#[test]
fn largest_dirs_do_not_expand_a_folder_that_would_not_add_one() {
    let mut fs = FakeFs::new();
    // `a` has two folders, so it is expanded; `b` has one folder and stays.
    fs.file("/r/a/a1/x.bin", 400).file("/r/a/a2/x.bin", 300);
    fs.file("/r/b/b1/x.bin", 150).file("/r/b/y.bin", 100);

    assert_eq!(largest_dir_names(&fs, "r"), ["a1", "a2", "b"]);
}

#[test]
fn largest_dirs_apply_the_dominance_rule_again_after_expanding() {
    let mut fs = FakeFs::new();
    // `a` splits into a1 (holding 500, all of it in a1x) and a2 (400); the dominated a1 is replaced.
    fs.file("/r/a/a1/a1x/x.bin", 500).file("/r/a/a2/x.bin", 400);
    fs.file("/r/b/x.bin", 50);

    assert_eq!(largest_dir_names(&fs, "r"), ["a1x", "a2", "b"]);
}

#[test]
fn largest_dirs_return_fewer_than_eight_when_nothing_can_be_expanded() {
    let mut fs = FakeFs::new();
    fs.file("/r/a/x.bin", 300)
        .file("/r/b/x.bin", 200)
        .file("/r/c/x.bin", 100);

    assert_eq!(largest_dir_names(&fs, "r"), ["a", "b", "c"]);
}

#[test]
fn largest_dirs_never_list_empty_directories_or_mounts() {
    let mut fs = FakeFs::new();
    fs.dir("/r/empty").file("/r/real/x.bin", 100);
    fs.file("/r/usb/x.bin", 999).mount("/r/usb");
    // A folder that only holds empty folders is not expandable.
    fs.dir("/r/shell/e1")
        .dir("/r/shell/e2")
        .set_size("/r/shell", 4096, 4096);

    // With `one_file_system` the other disk's mount point is a `mount` node of size 0.
    let tree = scan_fake(&fs, true);
    assert_eq!(tree.kind(find(&tree, "r/usb")), NodeKind::Mount);

    let result = summary(&tree, find(&tree, "r")).unwrap();
    let names: Vec<_> = result.largest_dirs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["shell", "real"]);
}

#[test]
fn largest_dirs_are_deterministic() {
    let mut fs = FakeFs::new();
    // Many ties: equal sizes everywhere, so only the id order can decide.
    for i in 0..4 {
        for j in 0..4 {
            fs.file(&format!("/r/d{i}/e{j}/x.bin"), 10);
        }
    }

    let ids = |threads: usize| {
        let config = ScanConfig {
            threads: Some(threads),
            now: Some(NOW),
            ..ScanConfig::default()
        };
        let tree = scan(&fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap();
        let first = summary(&tree, find(&tree, "r")).unwrap();
        let again = summary(&tree, find(&tree, "r")).unwrap();
        assert_eq!(first.largest_dirs, again.largest_dirs);

        first
            .largest_dirs
            .iter()
            .map(|d| (d.id, d.name.clone()))
            .collect::<Vec<_>>()
    };

    let one = ids(1);
    assert_eq!(one, ids(8));
    assert_eq!(one.len(), 8);
    assert!(
        one.windows(2).all(|w| w[0].0 < w[1].0),
        "ties are broken by ascending id"
    );
}

fn huge_fs() -> FakeFs {
    let huge = i64::MAX as u64;
    let mut fs = FakeFs::new();
    for name in ["a", "b", "c", "d"] {
        fs.file(&format!("/h/{name}.bin"), huge);
    }
    fs
}

#[test]
fn summary_saturates_instead_of_overflowing() {
    let tree = scan_fake(&huge_fs(), false);

    let result = summary(&tree, tree.root()).unwrap();

    assert_eq!(result.node.allocated, u64::MAX);
    assert_eq!(result.by_category[0].allocated, u64::MAX);
    assert_eq!(
        result.by_age[5].allocated,
        u64::MAX,
        "mtime 0 is in the oldest bucket"
    );
}

#[test]
fn lod_collapsed_node_saturates_instead_of_overflowing() {
    let tree = scan_fake(&huge_fs(), false);

    let view = lod(&tree, find(&tree, "h"), 3, 4).unwrap();

    let collapsed = view.children.last().unwrap();
    assert_eq!(collapsed.node.kind, NodeKind::Collapsed);
    assert_eq!(
        collapsed.node.allocated,
        u64::MAX,
        "three sizes of i64::MAX saturate"
    );
    assert_eq!(collapsed.node.apparent, u64::MAX);
}

#[test]
fn extreme_mtimes_are_bucketed_and_clamped_for_the_wire() {
    let mut fs = FakeFs::new();
    fs.file("/m/old", 10).set_mtime("/m/old", i64::MIN);
    fs.file("/m/future", 20).set_mtime("/m/future", i64::MAX);
    fs.file("/m/negative", 30).set_mtime("/m/negative", -5);
    let tree = scan_fake(&fs, false);

    let result = summary(&tree, find(&tree, "m")).unwrap();
    let by_bucket = |bucket| result.by_age.iter().find(|a| a.bucket == bucket).unwrap();

    assert_eq!(
        by_bucket(AgeBucket::Older).allocated,
        10 + 30,
        "i64::MIN and a negative mtime are old"
    );
    assert_eq!(
        by_bucket(AgeBucket::Week).allocated,
        20,
        "a future mtime is recent"
    );

    let page = children(&tree, find(&tree, "m"), 0, 10, SortKey::NameAsc).unwrap();
    let wire: Vec<_> = page.items.iter().map(|n| (n.name.as_str(), n.mtime)).collect();
    assert_eq!(
        wire,
        [("future", 253_402_300_799), ("negative", 0), ("old", 0)],
        "mtime is clamped to 1970..=9999 at the DTO boundary"
    );
    assert_eq!(node(&tree, find(&tree, "m")).unwrap().mtime, 253_402_300_799);
}

#[test]
fn a_mount_node_says_why_it_was_left_out_through_every_query_and_other_nodes_say_nothing() {
    use crate::model::SkipReason;

    let mut fs = FakeFs::new();
    fs.file("/r/real/x.bin", 100)
        .file("/r/usb/y.bin", 999)
        .mount("/r/usb");
    fs.mount("/r/proc").skip_mount("/r/proc", SkipReason::System);

    let tree = scan_fake(&fs, true);
    let root = find(&tree, "r");

    let page = children(&tree, root, 0, 10, SortKey::NameAsc).unwrap();
    let skipped: Vec<_> = page
        .items
        .iter()
        .map(|item| (item.name.as_str(), item.skipped))
        .collect();
    assert_eq!(
        skipped,
        [
            ("proc", Some(SkipReason::System)),
            ("real", None),
            ("usb", Some(SkipReason::OtherDisk)),
        ]
    );

    let single = node(&tree, find(&tree, "r/usb")).unwrap();
    assert_eq!(
        (single.kind, single.skipped),
        (NodeKind::Mount, Some(SkipReason::OtherDisk))
    );

    let wire = serde_json::to_value(&page.items[1]).unwrap();
    assert!(wire.get("skipped").is_none(), "{wire}");
    let wire = serde_json::to_value(&page.items[2]).unwrap();
    assert_eq!(wire["skipped"], "otherDisk");
}
