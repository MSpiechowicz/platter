use std::io::{self, Cursor};

use super::*;
use crate::scan::MAX_DEPTH;
use crate::testing::{FakeFs, scan_fake};
use crate::{NodeId, ScanWarning, SkipReason};

/// A tree with every kind of entry and list the snapshot carries: folders, files, a hard link, a link, a
/// special file, an unreadable folder, mounts that were entered and left out, and a warning.
fn sample() -> Tree {
    let mut fs = FakeFs::new();
    fs.file("/photos/2024/a.jpg", 20_000)
        .file("/photos/b.jpg", 10_000)
        .file("/notes.txt", 5)
        .hardlink("/notes.txt", "/photos/notes-link.txt")
        .symlink("/link", 7)
        .special("/pipe")
        .unreadable_dir("/locked")
        .mount("/disks/usb")
        .same_filesystem_mount("/disks/subvolume")
        .mount("/disks/skipped")
        .skip_mount("/disks/skipped", SkipReason::System)
        .file("/disks/subvolume/inside.bin", 900)
        .warn(ScanWarning::MountTableUnavailable);

    scan_fake(&fs, true)
}

fn snapshot_of(tree: &Tree) -> Vec<u8> {
    let mut bytes = Vec::new();
    tree.write_snapshot(&mut bytes).unwrap();

    bytes
}

fn read(bytes: &[u8]) -> io::Result<Tree> {
    Tree::read_snapshot(&mut Cursor::new(bytes))
}

fn assert_same(left: &Tree, right: &Tree) {
    assert_eq!(left.names, right.names);
    assert_eq!(left.nodes, right.nodes);
    assert_eq!(left.dirs, right.dirs);
    assert_eq!(left.root_path, right.root_path);
    assert_eq!(left.scanned_at, right.scanned_at);
    assert_eq!(left.elapsed, right.elapsed);
    assert_eq!(left.totals, right.totals);
    assert_eq!(left.warnings, right.warnings);
    assert_eq!(left.unreadable, right.unreadable);
    assert_eq!(left.skipped, right.skipped);
    assert_eq!(left.entered_mounts, right.entered_mounts);
    assert_eq!(left.root_identity, right.root_identity);
    assert_eq!(left.shared, right.shared);
}

/// Asks a tree everything a query can ask of every entry: it must not panic.
fn walk(tree: &Tree) {
    for index in 0..tree.len() as u32 {
        let id = NodeId(index);

        let _ = (tree.path_of(id), tree.path_bytes(id), tree.name_lossy(id));
        let _ = (tree.kind(id), tree.category(id), tree.files(id), tree.mtime(id));
        let _ = (
            tree.is_unreadable(id),
            tree.skip_reason(id),
            tree.entered_mount(id),
        );
        let _ = (
            tree.shared_files(id),
            tree.has_unreadable(id),
            tree.holds_excluded(id),
        );
        let _ = (tree.ancestry(id), tree.children(id).count());
    }
}

#[test]
fn a_tree_comes_back_equal_with_the_same_answers() {
    let tree = sample();
    assert!(!tree.shared.is_empty() && !tree.unreadable.is_empty());
    assert!(!tree.skipped.is_empty() && !tree.entered_mounts.is_empty());
    assert!(!tree.warnings.is_empty());

    let back = read(&snapshot_of(&tree)).unwrap();

    assert_same(&tree, &back);
    assert_eq!(
        back.memory_bytes(),
        back.nodes.len() * size_of::<Node>()
            + back.names.len()
            + back.dirs.len() * size_of::<DirInfo>()
            + back.unreadable.len() * 4
            + back.skipped.len() * size_of::<(u32, SkipReason)>()
            + back.entered_mounts.len() * size_of::<(u32, ScannedMount)>()
            + back.shared.len() * 4,
        "the tree read back holds no spare capacity"
    );
    for index in 0..tree.len() as u32 {
        let id = NodeId(index);

        assert_eq!(tree.path_of(id), back.path_of(id));
        assert_eq!(
            tree.children(id).collect::<Vec<_>>(),
            back.children(id).collect::<Vec<_>>()
        );
        assert_eq!(tree.skip_reason(id), back.skip_reason(id));
        assert_eq!(tree.entered_mount(id), back.entered_mount(id));
        assert_eq!(tree.shared_files(id), back.shared_files(id));
        assert_eq!(tree.has_unreadable(id), back.has_unreadable(id));
        assert_eq!(tree.is_unreadable(id), back.is_unreadable(id));
    }
}

#[test]
fn the_extremes_of_a_tree_survive() {
    let mut tree = sample();
    tree.scanned_at = i64::MIN;
    tree.elapsed = Duration::new(u64::MAX, 999_999_999);
    tree.totals.allocated = u64::MAX;
    tree.root_identity = ScannedRoot {
        dev: u64::MAX,
        ino: u64::MAX,
        mount_id: Some(u64::MAX),
    };
    tree.nodes[1].mtime = i64::MIN;
    tree.nodes[1].allocated = u64::MAX;
    tree.warnings = vec![
        ScanWarning::ExcludedUnresolved,
        ScanWarning::MountTableUnavailable,
    ];

    let back = read(&snapshot_of(&tree)).unwrap();

    assert_same(&tree, &back);
}

#[test]
fn a_root_path_that_is_not_text_survives_on_unix() {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let mut tree = sample();
        tree.root_path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/data/\xff\xfe"));

        let back = read(&snapshot_of(&tree)).unwrap();

        assert_eq!(back.root_path, tree.root_path);
    }
}

#[test]
fn the_smallest_tree_round_trips() {
    let tree = scan_fake(&FakeFs::new(), false);
    assert_eq!(tree.len(), 1);

    let back = read(&snapshot_of(&tree)).unwrap();

    assert_same(&tree, &back);
}

#[test]
fn input_that_is_not_a_snapshot_or_of_another_version_is_refused() {
    let good = snapshot_of(&sample());

    let mut wrong_magic = good.clone();
    wrong_magic[0] ^= 0xFF;
    let mut other_version = good.clone();
    other_version[8] = other_version[8].wrapping_add(1);

    for (what, bytes) in [
        ("wrong magic", wrong_magic),
        ("another version", other_version),
        ("empty", Vec::new()),
        ("text", b"{\"hello\": \"world\"}".to_vec()),
    ] {
        let error = read(&bytes).expect_err(what);

        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
            ),
            "{what}: {error}"
        );
    }
}

#[test]
fn a_snapshot_that_ends_early_or_goes_on_is_refused_at_every_length() {
    let good = snapshot_of(&sample());

    for length in 0..good.len() {
        let error = read(&good[..length]).expect_err("a truncated snapshot");

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof, "cut at {length}");
    }

    let mut longer = good.clone();
    longer[good.len() - 1] ^= 1;
    assert!(read(&longer).is_err(), "a damaged end marker");
}

#[test]
fn a_damaged_byte_is_an_error_or_a_tree_every_query_survives() {
    let tree = sample();
    let good = snapshot_of(&tree);

    for index in 0..good.len() {
        for flip in [0x01u8, 0x80, 0xFF] {
            let mut bytes = good.clone();
            bytes[index] ^= flip;

            if let Ok(found) = read(&bytes) {
                walk(&found);
            }
        }
    }
}

#[test]
fn counts_the_stream_cannot_back_are_refused_without_allocating_them() {
    let good = snapshot_of(&sample());
    // Offsets: magic 8, version 4, time 8, elapsed 12, totals 40, identity 8 + 8 + 9 = 97 is where the
    // counts start (names, nodes, dirs, ...).
    let counts = 97;

    for (field, claim) in [
        (0, u64::from(u32::MAX)),
        (1, u64::from(u32::MAX) - 1),
        (2, 1 << 30),
    ] {
        let mut bytes = good.clone();
        let at = counts + field * 8;
        bytes[at..at + 8].copy_from_slice(&claim.to_le_bytes());

        let error = read(&bytes).expect_err("a count the data does not back");

        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
            ),
            "field {field}: {error}"
        );
    }

    let mut absurd = good.clone();
    absurd[counts + 8..counts + 16].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(read(&absurd).unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn a_tree_over_the_memory_limit_is_refused_before_it_is_read() {
    let tree = sample();
    let bytes = snapshot_of(&tree);
    // What the tree read back holds: the copy in `tree` may hold spare capacity.
    let held = read(&bytes).unwrap().memory_bytes() as u64;

    let roomy = Tree::read_snapshot_limited(&mut Cursor::new(&bytes), held).unwrap();
    assert_same(&tree, &roomy);

    // Only the header is read: the rest of the data is not even looked at.
    let header_only = &bytes[..160];
    let error = Tree::read_snapshot_limited(&mut Cursor::new(header_only), held - 1).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
}

/// Writes `tree` after `damage` and reads it back: the writer checks nothing, the reader must.
fn refused_after(damage: impl FnOnce(&mut Tree)) -> io::Error {
    let mut tree = sample();
    damage(&mut tree);

    read(&snapshot_of(&tree)).expect_err("a tree that breaks an invariant")
}

fn first_of(tree: &Tree, kind: NodeKind) -> usize {
    tree.nodes
        .iter()
        .position(|node| node.kind == kind)
        .expect("the sample has one")
}

/// A change to a tree that breaks an invariant.
type Damage = Box<dyn FnOnce(&mut Tree)>;

#[test]
fn a_tree_that_breaks_an_invariant_is_refused() {
    let damages: Vec<(&str, Damage)> = vec![
        ("root with a parent", Box::new(|t| t.nodes[0].parent = 1)),
        (
            "root that is a file",
            Box::new(|t| t.nodes[0].kind = NodeKind::File),
        ),
        (
            "a collapsed node",
            Box::new(|t| t.nodes[1].kind = NodeKind::Collapsed),
        ),
        (
            "a name outside the names",
            Box::new(|t| t.nodes[1].name_off = u32::MAX),
        ),
        (
            "a name running past the names",
            Box::new(|t| t.nodes[1].name_len = u16::MAX),
        ),
        ("a parent after its child", Box::new(|t| t.nodes[1].parent = 2)),
        (
            "a parent outside the tree",
            Box::new(|t| t.nodes[1].parent = u32::MAX - 1),
        ),
        ("the wrong parent", Box::new(|t| t.nodes[2].parent = 1)),
        ("a folder with no record", Box::new(|t| t.nodes[0].dir = NO_DIR)),
        ("a folder record out of range", Box::new(|t| t.dirs.truncate(1))),
        (
            "an extra folder record",
            Box::new(|t| t.dirs.push(DirInfo::default())),
        ),
        (
            "a file with a folder record",
            Box::new(|t| {
                let file = first_of(t, NodeKind::File);
                t.nodes[file].dir = 0;
            }),
        ),
        (
            "children outside the tree",
            Box::new(|t| t.dirs[0].child_count = u32::MAX),
        ),
        (
            "children that start early",
            Box::new(|t| t.dirs[0].first_child = 0),
        ),
        (
            "children that start late",
            Box::new(|t| t.dirs[0].first_child += 1),
        ),
        ("too few children", Box::new(|t| t.dirs[0].child_count -= 1)),
        ("unreadable not sorted", Box::new(|t| t.unreadable = vec![5, 3])),
        ("unreadable repeated", Box::new(|t| t.unreadable = vec![0, 0])),
        ("unreadable outside", Box::new(|t| t.unreadable = vec![u32::MAX])),
        (
            "unreadable file",
            Box::new(|t| {
                let file = first_of(t, NodeKind::File) as u32;
                t.unreadable = vec![file];
            }),
        ),
        (
            "skipped that is not a mount",
            Box::new(|t| t.skipped = vec![(0, SkipReason::System)]),
        ),
        (
            "entered mount that is a file",
            Box::new(|t| {
                let file = first_of(t, NodeKind::File) as u32;
                t.entered_mounts[0].0 = file;
            }),
        ),
        ("shared that is a folder", Box::new(|t| t.shared = vec![0])),
        ("shared outside", Box::new(|t| t.shared = vec![u32::MAX])),
    ];

    for (what, damage) in damages {
        let error = refused_after(damage);

        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{what}: {error}");
    }
}

#[test]
fn an_unknown_enum_byte_is_refused() {
    let tree = sample();
    let good = snapshot_of(&tree);
    // The header is 97 + 7 * 8 bytes, then the root path (4 + length), the warnings (1 + one) and the names.
    let path = tree.root_path.as_os_str().len();
    let nodes_at = 97 + 56 + 4 + path + 1 + tree.warnings.len() + tree.names.len();
    let (kind_at, category_at) = (nodes_at + 14, nodes_at + 15);
    assert_eq!(good[kind_at], 0, "the root is a folder");

    for at in [kind_at, category_at] {
        let mut bytes = good.clone();
        bytes[at] = 200;

        assert_eq!(
            read(&bytes).unwrap_err().kind(),
            io::ErrorKind::InvalidData,
            "byte {at}"
        );
    }
}

#[test]
fn a_snapshot_streams_through_a_small_writer_and_reader() {
    struct Dribble<T>(T);

    impl<T: Write> Write for Dribble<T> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let take = buf.len().min(3);
            self.0.write(&buf[..take])
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    impl<T: Read> Read for Dribble<T> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let take = buf.len().min(5);
            self.0.read(&mut buf[..take])
        }
    }

    let tree = sample();
    let mut written = Dribble(Vec::new());
    tree.write_snapshot(&mut written).unwrap();

    let back = Tree::read_snapshot(&mut Dribble(Cursor::new(written.0))).unwrap();

    assert_same(&tree, &back);
}

#[test]
fn a_failing_writer_is_an_error() {
    struct Full;

    impl Write for Full {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("disk full"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    assert!(sample().write_snapshot(&mut Full).is_err());
}

/// A tree made by hand: `(parent, kind, name)` of every entry in id order (the root first, named by its
/// path) and the folder records, in the order of the folders.
fn raw(entries: &[(u32, NodeKind, &[u8])], dirs: Vec<DirInfo>) -> Tree {
    let mut names = Vec::new();
    let mut nodes = Vec::new();
    let mut next_dir = 0;

    for &(parent, kind, name) in entries {
        let dir = if kind == NodeKind::Dir {
            next_dir += 1;
            next_dir - 1
        } else {
            NO_DIR
        };
        nodes.push(Node {
            parent,
            name_off: names.len() as u32,
            dir,
            name_len: name.len() as u16,
            kind,
            category: Category::Other,
            allocated: 1,
            apparent: 1,
            mtime: 0,
        });
        names.extend_from_slice(name);
    }

    Tree {
        names,
        nodes,
        dirs,
        root_path: PathBuf::from("/r"),
        scanned_at: 1,
        elapsed: Duration::ZERO,
        totals: Totals::default(),
        warnings: Vec::new(),
        unreadable: Vec::new(),
        skipped: Vec::new(),
        entered_mounts: Vec::new(),
        root_identity: ScannedRoot {
            dev: 1,
            ino: 2,
            mount_id: None,
        },
        shared: Vec::new(),
    }
}

fn dir_info(first_child: u32, child_count: u32) -> DirInfo {
    DirInfo {
        first_child,
        child_count,
        files: 0,
    }
}

/// The root and a chain of `depth` folders below it, all named `name`.
fn chain(depth: usize, name: &[u8]) -> Tree {
    let mut entries: Vec<(u32, NodeKind, &[u8])> = vec![(NO_PARENT, NodeKind::Dir, b"r")];
    let mut dirs = Vec::new();

    for level in 0..depth {
        entries.push((level as u32, NodeKind::Dir, name));
        dirs.push(dir_info(level as u32 + 1, 1));
    }
    dirs.push(dir_info(0, 0));

    raw(&entries, dirs)
}

/// The root with one file named `name`.
fn named(name: &[u8]) -> Tree {
    raw(
        &[(NO_PARENT, NodeKind::Dir, b"r"), (0, NodeKind::File, name)],
        vec![dir_info(1, 1)],
    )
}

fn assert_refused(tree: &Tree, what: &str) {
    let error = read(&snapshot_of(tree)).expect_err(what);

    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{what}: {error}");
}

#[test]
fn the_deepest_tree_the_scanner_makes_comes_back_and_a_deeper_one_is_refused() {
    let depth = MAX_DEPTH as usize;
    let mut fs = FakeFs::new();
    fs.file(&format!("/{}x", "d/".repeat(depth + 10)), 1);
    let scanned = scan_fake(&fs, false);
    assert!(scanned.len() > depth, "the scan went as deep as it reads");
    assert_same(&scanned, &read(&snapshot_of(&scanned)).unwrap());

    // The scanner reads folders down to depth `MAX_DEPTH`, so entries are at most one level below that.
    assert!(read(&snapshot_of(&chain(depth + 1, b"d"))).is_ok());
    assert_refused(&chain(depth + 2, b"d"), "one level too deep");
}

#[test]
fn a_chain_that_shares_one_long_name_cannot_make_a_path_of_megabytes() {
    let long = [b'n'; 1000];

    assert_refused(&chain(100_000, &long), "100 thousand levels");
    assert_refused(&chain(1000, &long), "a path of a megabyte inside the depth limit");
    assert!(
        read(&snapshot_of(&chain(60, &long))).is_ok(),
        "a long but real path"
    );
}

#[test]
fn names_that_are_not_file_names_are_refused() {
    for (what, name) in [
        ("empty", &b""[..]),
        ("dot", b"."),
        ("dot dot", b".."),
        ("a slash", b"a/b"),
        ("a nul", b"a\0b"),
    ] {
        assert_refused(&named(name), what);
    }

    assert!(
        read(&snapshot_of(&named(b"..."))).is_ok(),
        "three dots are a name"
    );
    assert!(read(&snapshot_of(&named(b".hidden"))).is_ok());
    // A backslash is a separator on Windows only; on Unix it is a letter of a name.
    assert_eq!(read(&snapshot_of(&named(b"a\\b"))).is_err(), cfg!(windows));
}

#[test]
fn ids_that_are_not_breadth_first_are_refused() {
    use NodeKind::{Dir, File};

    // Root lists A and B; B's child comes before A's: contiguous and every parent first, yet not the
    // order of a breadth-first scan.
    let entries: [(u32, NodeKind, &[u8]); 5] = [
        (NO_PARENT, Dir, b"r"),
        (0, Dir, b"a"),
        (0, Dir, b"b"),
        (2, File, b"b1"),
        (1, File, b"a1"),
    ];
    let depth_first_ids = raw(&entries, vec![dir_info(1, 2), dir_info(4, 1), dir_info(3, 1)]);
    assert_refused(&depth_first_ids, "B's child before A's");

    let breadth_first = raw(
        &[
            entries[0],
            entries[1],
            entries[2],
            (1, File, b"a1"),
            (2, File, b"b1"),
        ],
        vec![dir_info(1, 2), dir_info(3, 1), dir_info(4, 1)],
    );
    assert!(
        read(&snapshot_of(&breadth_first)).is_ok(),
        "the same tree in scan order"
    );

    let mut with_gap = raw(
        &[
            entries[0],
            entries[1],
            entries[2],
            (1, File, b"a1"),
            (2, File, b"b1"),
        ],
        vec![dir_info(1, 2), dir_info(3, 1), dir_info(4, 1)],
    );
    with_gap.dirs[1] = dir_info(4, 1);
    with_gap.dirs[2] = dir_info(3, 1);
    assert_refused(&with_gap, "a gap before the children");
}

/// The category bytes are the index in `Category::ALL`. Reordering, removing or inserting a category changes
/// what a saved scan means, so it needs a new `SNAPSHOT_VERSION` (and then this list is updated).
#[test]
fn the_stored_category_bytes_are_pinned() {
    use Category::*;

    let pinned = [
        Video, Image, Audio, Document, Archive, Code, Build, Package, App, DiskImage, Data, Other,
    ];

    assert_eq!(Category::ALL, pinned);
    assert_eq!(SNAPSHOT_VERSION, 1);
    for (byte, category) in pinned.into_iter().enumerate() {
        assert_eq!(category.index(), byte);
        assert_eq!(category_from(byte as u8).unwrap(), category);
    }
    assert!(category_from(pinned.len() as u8).is_err());
}
