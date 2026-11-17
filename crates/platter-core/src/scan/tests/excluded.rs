use std::path::Path;

use super::find;
use crate::model::{NodeKind, SkipReason, Tree};
use crate::scan::{CancelToken, ScanConfig, scan};
use crate::testing::{FakeFs, NOW};

fn scan_excluding(fs: &FakeFs, root: &str, excluded: &[&str]) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        excluded: excluded.iter().map(|path| fs.identity(path)).collect(),
        ..ScanConfig::default()
    };

    scan(fs, Path::new(root), &config, &CancelToken::new(), &|_| {}).expect("the fake scans")
}

fn project() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/me/notes.txt", 10)
        .file("/home/me/secret/diary.txt", 500)
        .file("/home/me/secret/deep/more.bin", 700)
        .file("/home/me/work/report.doc", 40);
    fs
}

#[test]
fn an_excluded_folder_is_a_mount_node_left_out_by_the_user() {
    let fs = project();

    let tree = scan_excluding(&fs, "/", &["/home/me/secret"]);

    let secret = find(&tree, "home/me/secret");
    assert_eq!(tree.kind(secret), NodeKind::Mount);
    assert_eq!(tree.skip_reason(secret), Some(SkipReason::Excluded));
    assert_eq!(tree.allocated(secret), 0);
    assert_eq!(tree.apparent(secret), 0);
    assert!(!tree.has_children(secret));
}

#[test]
fn nothing_inside_an_excluded_folder_is_read_listed_or_counted() {
    let fs = project();

    let tree = scan_excluding(&fs, "/", &["/home/me/secret"]);

    assert!(!fs.read_dirs().contains(&"/home/me/secret".to_owned()));
    assert!(!fs.read_dirs().contains(&"/home/me/secret/deep".to_owned()));
    assert_eq!(tree.totals().files, 2, "notes.txt and report.doc");
    assert_eq!(tree.totals().allocated, 50);
    assert_eq!(tree.totals().apparent, 50);

    let names: Vec<String> = (0..tree.len() as u32)
        .map(|id| String::from_utf8_lossy(tree.name(crate::model::NodeId(id))).into_owned())
        .collect();
    assert!(!names.iter().any(|name| name == "diary.txt" || name == "deep"));
}

#[test]
fn folders_beside_an_excluded_one_are_still_scanned() {
    let fs = project();

    let tree = scan_excluding(&fs, "/", &["/home/me/secret"]);

    assert_eq!(tree.allocated(find(&tree, "home/me/work")), 40);
    assert_eq!(tree.kind(find(&tree, "home/me/work")), NodeKind::Dir);
    assert_eq!(tree.skip_reason(find(&tree, "home/me/work")), None);
}

#[test]
fn a_folder_nested_in_an_excluded_one_is_left_out_with_it() {
    let fs = project();

    let tree = scan_excluding(&fs, "/", &["/home/me/secret/deep", "/home/me/secret"]);

    assert_eq!(tree.kind(find(&tree, "home/me/secret")), NodeKind::Mount);
    assert_eq!(tree.totals().files, 2);
}

#[test]
fn an_excluded_folder_inside_a_kept_scan_root_is_left_out_with_the_root_below_it() {
    let fs = project();

    let tree = scan_excluding(&fs, "/home/me", &["/home/me/secret"]);

    assert_eq!(
        tree.skip_reason(find(&tree, "secret")),
        Some(SkipReason::Excluded)
    );
    assert_eq!(tree.totals().allocated, 50);
}

#[test]
fn the_scan_root_is_never_excluded() {
    let fs = project();

    let tree = scan_excluding(&fs, "/home/me", &["/home/me"]);

    assert_eq!(tree.kind(tree.root()), NodeKind::Dir);
    assert_eq!(tree.totals().files, 4);
    assert_eq!(tree.skip_reason(find(&tree, "secret")), None);
}

#[test]
fn the_root_is_scanned_when_it_is_listed_among_other_exclusions() {
    let fs = project();

    let tree = scan_excluding(&fs, "/home/me", &["/home/me", "/home/me/work"]);

    assert_eq!(tree.totals().files, 3);
    assert_eq!(tree.skip_reason(find(&tree, "work")), Some(SkipReason::Excluded));
}

#[test]
fn an_exclusion_that_matches_nothing_changes_the_scan_in_no_way() {
    let fs = project();
    let config = ScanConfig {
        now: Some(NOW),
        excluded: vec![(9, 9_999)],
        ..ScanConfig::default()
    };

    let tree = scan(&fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap();
    let plain = scan_excluding(&fs, "/", &[]);

    assert_eq!(tree.totals(), plain.totals());
    assert_eq!(tree.len(), plain.len());
}

#[test]
fn a_file_with_the_identity_of_an_excluded_folder_is_not_left_out() {
    let mut fs = project();
    fs.set_identity("/home/me/notes.txt", 1, 123_456, 1)
        .set_identity("/home/me/secret", 1, 123_456, 1);

    let tree = scan_excluding(&fs, "/", &["/home/me/secret"]);

    assert_eq!(tree.kind(find(&tree, "home/me/notes.txt")), NodeKind::File);
}

#[test]
fn an_excluded_folder_wins_over_the_reason_the_source_gave_for_a_mount() {
    let mut fs = project();
    fs.mount("/home/me/nas")
        .file("/home/me/nas/movie.mkv", 9000)
        .skip_mount("/home/me/nas", SkipReason::OtherDisk);

    let tree = scan_excluding(&fs, "/", &["/home/me/nas"]);

    assert_eq!(
        tree.skip_reason(find(&tree, "home/me/nas")),
        Some(SkipReason::Excluded)
    );
}

#[test]
fn an_excluded_mount_that_would_be_entered_is_not_entered() {
    let mut fs = project();
    fs.same_filesystem_mount("/home/me/games")
        .file("/home/me/games/save.dat", 3000);

    let tree = scan_excluding(&fs, "/", &["/home/me/games"]);

    assert_eq!(tree.kind(find(&tree, "home/me/games")), NodeKind::Mount);
    assert_eq!(
        tree.skip_reason(find(&tree, "home/me/games")),
        Some(SkipReason::Excluded)
    );
    assert!(!fs.read_dirs().contains(&"/home/me/games".to_owned()));
}

#[test]
fn the_result_does_not_depend_on_the_number_of_threads_with_exclusions() {
    let fs = project();

    let totals: Vec<_> = [1, 8]
        .into_iter()
        .map(|threads| {
            let config = ScanConfig {
                threads: Some(threads),
                now: Some(NOW),
                excluded: vec![fs.identity("/home/me/secret")],
                ..ScanConfig::default()
            };
            let tree = scan(&fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap();
            (tree.totals(), tree.len())
        })
        .collect();

    assert_eq!(totals[0], totals[1]);
}

#[test]
fn a_folder_holds_an_excluded_one_only_when_it_lies_inside_it() {
    let mut fs = project();
    fs.mount("/home/me/work/nas")
        .skip_mount("/home/me/work/nas", SkipReason::System);

    let tree = scan_excluding(&fs, "/", &["/home/me/secret/deep"]);

    assert!(tree.holds_excluded(find(&tree, "home/me/secret")));
    assert!(tree.holds_excluded(find(&tree, "home")));
    assert!(tree.holds_excluded(tree.root()));
    assert!(
        !tree.holds_excluded(find(&tree, "home/me/secret/deep")),
        "not itself"
    );
    assert!(
        !tree.holds_excluded(find(&tree, "home/me/work")),
        "a skipped mount is not an exclusion"
    );
    assert!(!tree.holds_excluded(find(&tree, "home/me/notes.txt")));
}
