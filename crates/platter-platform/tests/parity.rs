//! Both readers must see a real directory tree identically, and agree with an independent walk that
//! computes `du -s -B1` / `du -s -B1 --apparent-size` style numbers from plain metadata.

#![cfg(unix)]

use std::collections::HashSet;
use std::fs::{self, File, Permissions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use platter_core::{CancelToken, DirSource, ScanConfig, Totals, Tree, scan};
use platter_platform::GenericDirSource;

/// Fixture directory that restores permissions so the temp dir can always be deleted.
struct Fixture {
    dir: tempfile::TempDir,
    locked: PathBuf,
    _socket: UnixListener,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.locked, Permissions::from_mode(0o755));
    }
}

fn build_fixture() -> Fixture {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let root = dir.path();

    fs::create_dir_all(root.join("a/nested/deeper")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    fs::create_dir_all(root.join("empty")).unwrap();
    fs::create_dir_all(root.join("locked")).unwrap();

    fs::write(root.join("a/small.txt"), vec![b'x'; 10_000]).unwrap();
    fs::write(root.join("a/nested/data.bin"), vec![7u8; 1 << 20]).unwrap();
    fs::write(root.join("a/nested/deeper/tiny"), b"1").unwrap();

    // A hardlink pair in different directories.
    fs::write(root.join("a/hard1"), vec![1u8; 300_000]).unwrap();
    fs::hard_link(root.join("a/hard1"), root.join("b/hard2")).unwrap();

    // Symlinks: a loop to the parent, one to itself, a dangling one, one out of the tree.
    symlink("..", root.join("b/loop")).unwrap();
    symlink("self", root.join("b/self")).unwrap();
    symlink("does-not-exist", root.join("b/dangling")).unwrap();
    symlink(root, root.join("a/nested/to-root")).unwrap();

    // A sparse file: 64 MiB long, one block of data.
    let mut sparse = File::create(root.join("sparse.img")).unwrap();
    sparse.set_len(64 << 20).unwrap();
    sparse.write_all(b"head").unwrap();

    // A socket is neither file, directory nor symlink.
    let socket = UnixListener::bind(root.join("b/sock")).unwrap();

    // An unreadable directory with content that must not be seen.
    fs::write(root.join("locked/secret.txt"), vec![9u8; 5000]).unwrap();
    let locked = root.join("locked");
    fs::set_permissions(&locked, Permissions::from_mode(0o000)).unwrap();

    Fixture {
        dir,
        locked,
        _socket: socket,
    }
}

/// `chmod 000` has no effect for root (or with CAP_DAC_OVERRIDE); detect that instead of guessing.
fn locked_dir_is_readable(locked: &Path) -> bool {
    fs::read_dir(locked).is_ok()
}

fn scan_with<S: DirSource>(source: &S, root: &Path, one_file_system: bool) -> Tree {
    let config = ScanConfig {
        one_file_system,
        ..ScanConfig::default()
    };
    scan(source, root, &config, &CancelToken::new(), &|_| {}).expect("scan succeeds")
}

fn linux_source() -> platter_platform::DefaultDirSource {
    platter_platform::DefaultDirSource::default()
}

/// Independent oracle: plain `symlink_metadata` recursion, hardlinks once, never following symlinks.
fn expected_totals(root: &Path) -> Totals {
    fn visit(dir: &Path, totals: &mut Totals, seen: &mut HashSet<(u64, u64)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            totals.errors += 1;
            return;
        };

        for entry in entries.flatten() {
            let meta = fs::symlink_metadata(entry.path()).unwrap();
            let is_dir = meta.is_dir();

            if is_dir {
                totals.dirs += 1;
            } else {
                totals.files += 1;
            }
            if is_dir || meta.nlink() < 2 || seen.insert((meta.dev(), meta.ino())) {
                totals.allocated += meta.blocks() * 512;
                if !is_dir {
                    totals.apparent += meta.len();
                }
            }
            if is_dir {
                visit(&entry.path(), totals, seen);
            }
        }
    }

    // Like `du`: the root's own blocks count; directory lengths are not data and never count.
    let root_meta = fs::metadata(root).unwrap();
    let mut totals = Totals {
        allocated: root_meta.blocks() * 512,
        ..Totals::default()
    };
    visit(root, &mut totals, &mut HashSet::new());
    totals
}

#[test]
fn linux_and_generic_readers_report_identical_totals() {
    let fixture = build_fixture();
    let root = fixture.dir.path();

    let fast = scan_with(&linux_source(), root, false);
    let generic = scan_with(&GenericDirSource, root, false);

    assert_eq!(fast.totals(), generic.totals());
    assert_eq!(fast.len(), generic.len());
}

#[test]
fn totals_match_an_independent_metadata_walk() {
    let fixture = build_fixture();
    let root = fixture.dir.path();

    let expected = expected_totals(root);
    for tree in [
        scan_with(&linux_source(), root, false),
        scan_with(&GenericDirSource, root, false),
    ] {
        assert_eq!(tree.totals(), expected);
    }
}

fn find(tree: &Tree, path: &str) -> platter_core::NodeId {
    let mut current = tree.root();

    for part in path.split('/') {
        current = tree
            .children(current)
            .find(|&c| tree.name(c) == part.as_bytes())
            .expect("node exists");
    }
    current
}

#[test]
fn fixture_features_are_handled() {
    let fixture = build_fixture();
    let tree = scan_with(&linux_source(), fixture.dir.path(), false);
    let totals = tree.totals();

    // The hardlink pair is charged once, to the first link in name order.
    assert!(tree.allocated(find(&tree, "a/hard1")) >= 300_000);
    assert_eq!(tree.allocated(find(&tree, "b/hard2")), 0);
    assert_eq!(tree.apparent(find(&tree, "b/hard2")), 0);

    // The sparse file is 64 MiB long but almost nothing is allocated.
    assert_eq!(tree.apparent(find(&tree, "sparse.img")), 64 << 20);
    assert!(tree.allocated(find(&tree, "sparse.img")) < 1 << 20);

    // The socket is a leaf of its own kind.
    assert_eq!(tree.kind(find(&tree, "b/sock")), platter_core::NodeKind::Other);

    if locked_dir_is_readable(&fixture.locked) {
        eprintln!("running with permission overrides: skipping the unreadable-directory assertion");
    } else {
        assert_eq!(totals.errors, 1, "exactly the chmod 000 directory failed");
        assert!(!tree.has_children(find(&tree, "locked")));
    }
}

#[test]
fn a_chmod_000_directory_is_flagged_unreadable_and_an_empty_one_is_not() {
    let fixture = build_fixture();

    if locked_dir_is_readable(&fixture.locked) {
        eprintln!("running with permission overrides: skipping the unreadable-directory assertion");
        return;
    }

    let linux = scan_with(&linux_source(), fixture.dir.path(), false);
    let generic = scan_with(&GenericDirSource, fixture.dir.path(), false);

    for tree in [&linux, &generic] {
        assert!(tree.is_unreadable(find(tree, "locked")));
        assert!(
            !tree.is_unreadable(find(tree, "empty")),
            "an empty folder is readable"
        );
        assert!(!tree.is_unreadable(find(tree, "a/nested")));
        assert!(!tree.is_unreadable(tree.root()));

        let flagged = (0..tree.len() as u32)
            .filter(|&index| tree.is_unreadable(platter_core::NodeId(index)))
            .count();
        assert_eq!(flagged, 1);
    }

    // The flag reaches the wire too.
    let dto = platter_core::query::node(&linux, find(&linux, "locked")).unwrap();
    assert!(dto.unreadable);
    let empty = platter_core::query::node(&linux, find(&linux, "empty")).unwrap();
    assert!(!empty.unreadable);
}

#[test]
fn symlink_loops_terminate_and_count_only_the_links() {
    let fixture = build_fixture();
    let tree = scan_with(&linux_source(), fixture.dir.path(), false);

    // 4 symlinks, none followed: the scan would not end otherwise.
    let symlinks = (0..tree.len() as u32)
        .map(platter_core::NodeId)
        .filter(|&id| tree.kind(id) == platter_core::NodeKind::Symlink)
        .count();
    assert_eq!(symlinks, 4);
}

#[test]
fn a_symlinked_root_is_resolved_by_both_readers() {
    let fixture = build_fixture();
    let link = fixture.dir.path().join("a/nested/to-root");

    let fast = scan_with(&linux_source(), &link, false);
    let generic = scan_with(&GenericDirSource, &link, false);

    assert_eq!(fast.totals(), generic.totals());
    assert!(fast.totals().files > 5);
}

#[test]
fn a_file_is_not_a_valid_root() {
    let fixture = build_fixture();
    let file = fixture.dir.path().join("a/small.txt");
    let (config, cancel) = (ScanConfig::default(), CancelToken::new());

    assert!(scan(&linux_source(), &file, &config, &cancel, &|_| {}).is_err());
    assert!(scan(&GenericDirSource, &file, &config, &cancel, &|_| {}).is_err());
}
