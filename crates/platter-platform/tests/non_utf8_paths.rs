//! A node whose path contains bytes that are not valid UTF-8 must map back to exactly that path, so
//! actions such as "show in file manager" hit the right file.

#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;

use platter_core::{CancelToken, NodeId, ScanConfig, Tree, scan};
use platter_platform::linux::LinuxDirSource;
use platter_platform::paths::node_path;

fn child_named(tree: &Tree, parent: NodeId, name: &[u8]) -> NodeId {
    tree.children(parent)
        .find(|&child| tree.name(child) == name)
        .expect("the entry was scanned")
}

#[test]
fn the_exact_path_of_a_non_utf8_name_round_trips_through_the_linux_reader() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let folder_name: &[u8] = b"dir-\xff\xfe";
    let file_name: &[u8] = b"caf\xe9.txt";
    let folder = dir.path().join(OsStr::from_bytes(folder_name));
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join(OsStr::from_bytes(file_name)), b"payload").unwrap();

    let tree = scan(
        &LinuxDirSource::new(),
        dir.path(),
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .unwrap();
    let folder_id = child_named(&tree, tree.root(), folder_name);
    let file_id = child_named(&tree, folder_id, file_name);

    let path = node_path(&tree, file_id);

    assert_eq!(path, folder.join(OsStr::from_bytes(file_name)));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"payload",
        "the path opens the real file"
    );
    assert_eq!(node_path(&tree, folder_id), folder);
    assert_eq!(node_path(&tree, tree.root()), dir.path());
    assert!(
        fs::metadata(tree.path_of(file_id)).is_err(),
        "the lossy path points at a file that does not exist"
    );
}
