use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use crate::model::Progress;
use crate::scan::{CancelToken, ScanConfig, scan};
use crate::testing::FakeFs;

/// Scans on one thread, pausing in every directory so a progress snapshot is taken for each.
fn scan_collecting_progress(fs: &mut FakeFs) -> (Vec<Progress>, crate::model::Totals) {
    fs.on_read_dir(|_| std::thread::sleep(Duration::from_millis(3)));
    let snapshots = Mutex::new(Vec::new());
    let config = ScanConfig {
        threads: Some(1),
        progress_interval: Duration::from_millis(1),
        ..ScanConfig::default()
    };

    let tree = scan(fs, Path::new("/"), &config, &CancelToken::new(), &|progress| {
        snapshots.lock().unwrap().push(progress.clone());
    })
    .unwrap();

    (snapshots.into_inner().unwrap(), tree.totals())
}

#[test]
fn progress_bytes_never_exceed_the_final_total_even_with_hardlinks() {
    let mut fs = FakeFs::new();
    fs.file("/a/data.bin", 1000)
        .hardlink("/a/data.bin", "/b/alias1.bin")
        .hardlink("/a/data.bin", "/c/alias2.bin")
        .file("/d/plain.bin", 10)
        .file("/e/plain.bin", 20)
        .file("/f/last.bin", 1);

    let (snapshots, totals) = scan_collecting_progress(&mut fs);

    assert_eq!(totals.allocated, 1000 + 10 + 20 + 1, "the inode is counted once");
    assert!(snapshots.len() >= 4, "{} snapshots", snapshots.len());
    for snapshot in &snapshots {
        assert!(
            snapshot.bytes <= totals.allocated,
            "progress {} exceeds the final {}",
            snapshot.bytes,
            totals.allocated
        );
    }
}

#[test]
fn progress_counters_only_grow() {
    let mut fs = FakeFs::new();
    for dir in ["a", "b", "c", "d"] {
        fs.file(&format!("/{dir}/f.bin"), 100);
    }

    let (snapshots, _) = scan_collecting_progress(&mut fs);

    for pair in snapshots.windows(2) {
        assert!(pair[0].files <= pair[1].files);
        assert!(pair[0].dirs <= pair[1].dirs);
        assert!(pair[0].bytes <= pair[1].bytes);
    }
}
