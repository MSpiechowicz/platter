use super::find;
use crate::model::{NodeKind, SkipReason};
use crate::testing::{FakeFs, scan_fake};

#[test]
fn hardlinks_are_counted_once_per_device_and_inode() {
    let mut fs = FakeFs::new();
    fs.file("/a/data.bin", 1000)
        .hardlink("/a/data.bin", "/b/alias.bin")
        .file("/b/plain.bin", 10);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.totals().allocated, 1010);
    assert_eq!(tree.totals().apparent, 1010);
    assert_eq!(tree.totals().files, 3, "every link is still an entry");

    // The first link in breadth-first, name order owns the bytes; the others are free.
    assert_eq!(tree.allocated(find(&tree, "a/data.bin")), 1000);
    assert_eq!(tree.allocated(find(&tree, "b/alias.bin")), 0);
}

#[test]
fn the_same_inode_number_on_two_devices_is_two_files() {
    let mut fs = FakeFs::new();
    fs.file("/x/f", 100).file("/y/g", 100).file("/z/h", 100);
    fs.set_identity("/x/f", 1, 77, 2)
        .set_identity("/y/g", 2, 77, 2)
        .set_identity("/z/h", 1, 77, 2);

    let tree = scan_fake(&fs, false);

    assert_eq!(
        tree.totals().allocated,
        200,
        "f and h share (1, 77); g is (2, 77)"
    );
}

#[test]
fn symlinks_count_their_own_size_and_are_never_followed() {
    let mut fs = FakeFs::new();
    fs.file("/real/big.bin", 5000)
        .symlink("/link", 7)
        .symlink("/real/loop", 7);

    let tree = scan_fake(&fs, false);

    let link = find(&tree, "link");
    assert_eq!(tree.kind(link), NodeKind::Symlink);
    assert_eq!(tree.allocated(link), 7);
    assert!(!tree.has_children(link));
    assert_eq!(tree.totals().allocated, 5000 + 14);
    assert_eq!(fs.read_dirs(), vec!["/".to_owned(), "/real".to_owned()]);
}

#[test]
fn mounts_are_kept_but_not_descended_with_one_file_system() {
    let mut fs = FakeFs::new();
    fs.file("/home/f", 100)
        .mount("/mnt/usb")
        .file("/mnt/usb/movie.mkv", 9000);
    fs.set_size("/mnt/usb", 4096, 4096);

    let tree = scan_fake(&fs, true);

    let usb = find(&tree, "mnt/usb");
    assert_eq!(tree.kind(usb), NodeKind::Mount);
    assert_eq!(tree.allocated(usb), 0);
    assert!(!tree.has_children(usb));
    assert_eq!(tree.totals().allocated, 100);
    assert_eq!(tree.totals().dirs, 3, "home, mnt and the mount node");
    assert!(!fs.read_dirs().contains(&"/mnt/usb".to_owned()));
}

#[test]
fn a_mount_the_source_counts_as_the_same_filesystem_is_entered_with_one_file_system() {
    let mut fs = FakeFs::new();
    fs.file("/etc/hosts", 10)
        .same_filesystem_mount("/home")
        .file("/home/me/doc.txt", 700)
        .same_filesystem_mount("/home/me/games")
        .file("/home/me/games/save.dat", 300)
        .mount("/mnt/usb")
        .file("/mnt/usb/movie.mkv", 9000);

    let tree = scan_fake(&fs, true);

    for same_disk in ["home", "home/me/games"] {
        assert_eq!(
            tree.kind(find(&tree, same_disk)),
            NodeKind::Dir,
            "{same_disk} is part of the scan"
        );
    }
    assert_eq!(
        tree.kind(find(&tree, "mnt/usb")),
        NodeKind::Mount,
        "another disk is not"
    );
    assert_eq!(tree.totals().allocated, 10 + 700 + 300);
    assert_eq!(
        tree.allocated(find(&tree, "home")),
        1000,
        "a mount shows its real size, not 0"
    );
}

#[test]
fn mounts_are_descended_without_one_file_system() {
    let mut fs = FakeFs::new();
    fs.file("/home/f", 100)
        .mount("/mnt/usb")
        .file("/mnt/usb/movie.mkv", 9000);

    let tree = scan_fake(&fs, false);

    let usb = find(&tree, "mnt/usb");
    assert_eq!(tree.kind(usb), NodeKind::Dir);
    assert_eq!(tree.totals().allocated, 9100);
    assert!(fs.read_dirs().contains(&"/mnt/usb".to_owned()));
}

#[test]
fn the_tree_remembers_the_mounts_it_went_into_as_the_scan_saw_them() {
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/home")
        .file("/home/me/f", 1)
        .same_filesystem_bind_mount("/srv/bound")
        .file("/srv/bound/g", 1)
        .mount("/mnt/usb")
        .file("/mnt/usb/h", 1)
        .mount("/mnt/nas")
        .skip_mount("/mnt/nas", SkipReason::System)
        .dir("/plain/folder");

    let tree = scan_fake(&fs, false);

    let home = tree.entered_mount(find(&tree, "home")).expect("entered");
    assert_eq!((home.dev, home.ino), fs.identity("/home"));
    assert!(
        home.own_filesystem,
        "a subvolume of the scan root's own filesystem"
    );

    let bound = tree.entered_mount(find(&tree, "srv/bound")).expect("entered");
    assert!(!bound.own_filesystem, "a bind mount of a folder from elsewhere");

    let usb = tree.entered_mount(find(&tree, "mnt/usb")).expect("entered");
    assert!(
        !usb.own_filesystem,
        "another disk, entered because the scan may leave the root's disk"
    );

    assert_eq!(
        tree.entered_mount(find(&tree, "mnt/nas")),
        None,
        "a mount that was not entered is a mount node"
    );
    for plain in ["home/me", "plain/folder", "srv"] {
        assert_eq!(
            tree.entered_mount(find(&tree, plain)),
            None,
            "{plain} is no mount"
        );
    }
}

#[test]
fn a_directory_the_source_withholds_is_not_descended_even_without_one_file_system() {
    let mut fs = FakeFs::new();
    fs.file("/proc/1/status", 100).mount("/proc");
    fs.withhold_handle("/proc");
    fs.file("/etc/hosts", 10);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.kind(find(&tree, "proc")), NodeKind::Mount);
    assert_eq!(tree.totals().allocated, 10);
    assert!(!fs.read_dirs().contains(&"/proc".to_owned()));
}

#[test]
fn an_unreadable_directory_is_counted_and_kept_empty() {
    let mut fs = FakeFs::new();
    fs.file("/ok/f", 10)
        .unreadable_dir("/secret")
        .set_size("/secret", 4096, 4096);
    fs.file("/z/f", 20);

    let tree = scan_fake(&fs, false);

    let secret = find(&tree, "secret");
    assert_eq!(tree.kind(secret), NodeKind::Dir);
    assert!(!tree.has_children(secret));
    assert_eq!(tree.allocated(secret), 4096, "its own blocks still count");
    assert_eq!(tree.totals().errors, 1);
    assert_eq!(tree.totals().allocated, 4096 + 30);
}

#[test]
fn a_directory_that_could_not_be_read_is_flagged_and_an_empty_one_is_not() {
    let mut fs = FakeFs::new();
    fs.unreadable_dir("/secret/inner").unreadable_dir("/locked");
    fs.dir("/empty").file("/ok/f", 10);

    let tree = scan_fake(&fs, false);

    assert!(tree.is_unreadable(find(&tree, "locked")));
    assert!(tree.is_unreadable(find(&tree, "secret/inner")));

    for fine in ["", "secret", "empty", "ok", "ok/f"] {
        assert!(!tree.is_unreadable(find(&tree, fine)), "{fine:?}");
    }
    assert_eq!(tree.totals().errors, 2, "the error count is unchanged");
}

#[test]
fn an_unreadable_scan_root_is_flagged_too() {
    let mut fs = FakeFs::new();
    fs.unreadable_dir("/root");

    let config = crate::scan::ScanConfig::default();
    let tree = crate::scan::scan(
        &fs,
        std::path::Path::new("/root"),
        &config,
        &crate::scan::CancelToken::new(),
        &|_| {},
    )
    .unwrap();

    assert!(tree.is_unreadable(tree.root()));
    assert_eq!(tree.totals().errors, 1);
}

#[test]
fn a_directory_the_source_withholds_is_not_flagged() {
    let mut fs = FakeFs::new();
    fs.file("/proc/1/status", 100).mount("/proc");
    fs.withhold_handle("/proc");

    let tree = scan_fake(&fs, false);

    assert!(!tree.is_unreadable(find(&tree, "proc")));
    assert_eq!(tree.totals().errors, 0);
}

// Why a mount was not entered.

#[test]
fn each_mount_node_carries_the_reason_the_source_gave_and_nothing_else_does() {
    let mut fs = FakeFs::new();
    fs.file("/home/f", 1).file("/etc/hosts", 2);
    for (path, reason) in [
        ("/mnt/usb", SkipReason::OtherDisk),
        ("/proc", SkipReason::System),
        ("/.snapshots", SkipReason::Duplicate),
        ("/mnt/mystery", SkipReason::Unknown),
    ] {
        fs.mount(path).skip_mount(path, reason);
    }
    // A mount the source gives no reason for (and a plain withheld one) is unknown.
    fs.mount("/mnt/silent").withhold_handle("/mnt/silent");

    let tree = scan_fake(&fs, false);

    let reasons: Vec<(&str, Option<SkipReason>)> = [
        "mnt/usb",
        "proc",
        ".snapshots",
        "mnt/mystery",
        "mnt/silent",
        "home",
        "home/f",
        "etc/hosts",
    ]
    .into_iter()
    .map(|path| (path, tree.skip_reason(find(&tree, path))))
    .collect();
    assert_eq!(
        reasons,
        [
            ("mnt/usb", Some(SkipReason::OtherDisk)),
            ("proc", Some(SkipReason::System)),
            (".snapshots", Some(SkipReason::Duplicate)),
            ("mnt/mystery", Some(SkipReason::Unknown)),
            ("mnt/silent", Some(SkipReason::Unknown)),
            ("home", None),
            ("home/f", None),
            ("etc/hosts", None),
        ]
    );
}

#[test]
fn the_fake_names_another_disk_when_one_file_system_leaves_a_mount_out_and_nothing_otherwise() {
    let mut fs = FakeFs::new();
    fs.file("/mnt/usb/movie.mkv", 9000).mount("/mnt/usb");

    let staying = scan_fake(&fs, true);
    assert_eq!(
        staying.skip_reason(find(&staying, "mnt/usb")),
        Some(SkipReason::OtherDisk)
    );

    let everywhere = scan_fake(&fs, false);
    assert_eq!(everywhere.kind(find(&everywhere, "mnt/usb")), NodeKind::Dir);
    assert_eq!(everywhere.skip_reason(find(&everywhere, "mnt/usb")), None);
}

#[test]
fn reasons_cost_nothing_for_the_other_nodes_and_follow_the_node_ids() {
    let mut fs = FakeFs::new();
    for dir in ["a", "b", "c"] {
        fs.file(&format!("/{dir}/f"), 1);
        fs.mount(&format!("/{dir}/m"))
            .skip_mount(&format!("/{dir}/m"), SkipReason::Duplicate);
    }
    fs.mount("/c/other").skip_mount("/c/other", SkipReason::OtherDisk);

    let tree = scan_fake(&fs, false);

    assert_eq!(
        tree.skipped.len(),
        4,
        "one small record per listed mount, none per entry"
    );
    assert!(
        tree.skipped.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "ascending ids"
    );
    assert_eq!(
        tree.skip_reason(find(&tree, "c/other")),
        Some(SkipReason::OtherDisk)
    );
    assert_eq!(tree.skip_reason(find(&tree, "b/m")), Some(SkipReason::Duplicate));
}

// Hard-linked files.

#[test]
fn the_tree_counts_the_hardlinked_files_below_a_node_at_any_depth() {
    let mut fs = FakeFs::new();
    fs.file("/store/x/one", 10)
        .file("/store/x/y/two", 20)
        .file("/store/z/three", 30)
        .file("/store/plain", 5)
        .file("/other/link-one", 10)
        .file("/other/deeper/than/the/rest/link-two", 20)
        .file("/solo/f", 7);
    fs.hardlink("/store/x/one", "/other/link-one2");
    fs.hardlink("/store/x/y/two", "/other/deeper/than/the/rest/link-two2");
    fs.hardlink("/store/z/three", "/solo/three-again");

    let tree = scan_fake(&fs, false);
    let shared = |path: &str| tree.shared_files(find(&tree, path));

    // Every link counts, the one that owns the bytes and the ones that do not.
    assert_eq!(shared("store"), 3);
    assert_eq!(shared("store/x"), 2);
    assert_eq!(shared("store/x/y"), 1);
    assert_eq!(shared("store/z"), 1);
    assert_eq!(shared("other"), 2);
    assert_eq!(shared("other/deeper"), 1);
    assert_eq!(shared("solo"), 1);
    assert_eq!(shared(""), 6, "the whole tree: three files and their other links");

    // A file counts itself; a file with one link and a folder without any count nothing.
    assert_eq!(shared("store/x/one"), 1);
    assert_eq!(shared("store/plain"), 0);
    assert_eq!(shared("other/link-one"), 0, "it is not linked to anything");
    assert_eq!(shared("other/link-one2"), 1);
}

#[test]
fn a_tree_without_hardlinks_has_no_shared_files_and_keeps_no_list() {
    let mut fs = FakeFs::new();
    fs.file("/a/b/c", 1).file("/a/d", 2);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.shared_files(tree.root()), 0);
    assert!(tree.shared.is_empty());
}

#[test]
fn shared_files_agree_with_a_plain_walk_of_the_subtree_on_a_wide_tree() {
    let mut fs = FakeFs::new();
    for dir in 0..12 {
        for sub in 0..4 {
            fs.file(&format!("/d{dir:02}/s{sub}/f"), 1)
                .file(&format!("/d{dir:02}/s{sub}/g"), 1);
        }
    }
    // Links between files of different branches.
    for dir in 0..11 {
        fs.hardlink(&format!("/d{dir:02}/s1/f"), &format!("/d{:02}/s2/h", dir + 1));
    }
    fs.hardlink("/d00/s0/g", "/d05/s3/g-link");

    let tree = scan_fake(&fs, false);

    fn walk(tree: &crate::model::Tree, id: crate::model::NodeId) -> u64 {
        let own = u64::from(tree.shared.binary_search(&id.0).is_ok());
        own + tree.children(id).map(|child| walk(tree, child)).sum::<u64>()
    }
    for index in 0..tree.len() as u32 {
        let id = crate::model::NodeId(index);
        assert_eq!(
            tree.shared_files(id),
            walk(&tree, id),
            "node {index} {}",
            tree.name_lossy(id)
        );
    }
}
