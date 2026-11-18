//! The scan's entry and memory budget is a shared reservation: what many workers read at the same time
//! never adds up to more than the limits allow. Everything is measured on counters, not on memory, and
//! on generated directories that cost the fake no memory (no test here creates a file).

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::budget::{BATCH_ENTRIES, BudgetLimits, ENTRY_BASE_COST_BYTES, EntryBudget, Lease, NAME_COPIES};
use crate::error::{ScanError, ScanLimit};
use crate::ports::{DirListing, DirSource, RootInfo, SourceOptions};
use crate::scan::chunk::Chunk;
use crate::scan::progress::{PathChain, Reporter};
use crate::scan::walker::Walker;
use crate::scan::{CancelToken, ScanConfig, scan};
use crate::testing::{FakeFs, NOW};

/// A source that counts the entries its listings returned, which is what the walker holds in memory.
struct Counting<'a> {
    inner: &'a FakeFs,
    materialised: AtomicU64,
}

impl<'a> Counting<'a> {
    fn new(inner: &'a FakeFs) -> Self {
        Self {
            inner,
            materialised: AtomicU64::new(0),
        }
    }

    fn materialised(&self) -> u64 {
        self.materialised.load(Ordering::SeqCst)
    }
}

impl DirSource for Counting<'_> {
    type Handle = usize;

    fn open_root(&self, path: &Path, options: SourceOptions) -> io::Result<RootInfo<usize>> {
        self.inner.open_root(path, options)
    }

    fn read_dir(&self, dir: &usize, lease: &mut Lease<'_>) -> io::Result<DirListing<usize>> {
        let listing = self.inner.read_dir(dir, lease)?;
        self.materialised
            .fetch_add(listing.entries.len() as u64, Ordering::SeqCst);

        Ok(listing)
    }
}

/// A source written before the lease existed: it lists everything and never asks for room.
struct Greedy<'a>(&'a FakeFs);

impl DirSource for Greedy<'_> {
    type Handle = usize;

    fn open_root(&self, path: &Path, options: SourceOptions) -> io::Result<RootInfo<usize>> {
        self.0.open_root(path, options)
    }

    fn read_dir(&self, dir: &usize, _lease: &mut Lease<'_>) -> io::Result<DirListing<usize>> {
        self.0.read_dir(dir, &mut EntryBudget::unlimited().lease())
    }
}

fn config(threads: usize, max_entries: Option<u64>, max_memory_bytes: Option<u64>) -> ScanConfig {
    ScanConfig {
        threads: Some(threads),
        now: Some(NOW),
        max_entries,
        max_name_bytes: None,
        max_memory_bytes,
        ..ScanConfig::default()
    }
}

fn run<S: DirSource>(source: &S, config: &ScanConfig) -> Result<crate::model::Tree, ScanError> {
    scan(source, Path::new("/"), config, &CancelToken::new(), &|_| {})
}

/// `directories` siblings, each with `entries` generated files, every read made to overlap the others.
fn sibling_directories(directories: usize, entries: usize, name_len: usize) -> FakeFs {
    let mut fs = FakeFs::new();
    for index in 0..directories {
        fs.generated_dir(&format!("/d{index:02}"), entries, name_len);
    }
    // All the listings start together, so every worker is reading when the first one reserves.
    fs.on_read_dir(|_| std::thread::sleep(Duration::from_millis(30)));
    fs
}

#[test]
fn sibling_directories_read_at_the_same_time_share_the_cap_instead_of_each_taking_all_of_it() {
    const CAP: u64 = 20_000;
    const WORKERS: usize = 32;
    // Thirteen siblings of CAP entries each: before the reservation each one was told the whole remaining
    // room, so thirteen times the cap was materialised at once.
    let fs = sibling_directories(13, CAP as usize, 8);
    let source = Counting::new(&fs);

    let result = run(&source, &config(WORKERS, Some(CAP), None));

    assert!(matches!(
        result,
        Err(ScanError::TooLarge {
            limit: ScanLimit::Entries(CAP)
        })
    ));
    assert!(
        source.materialised() <= CAP + WORKERS as u64 * BATCH_ENTRIES,
        "{} entries were materialised for a cap of {CAP}",
        source.materialised()
    );
    assert!(
        source.materialised() <= CAP,
        "a source that reserves never goes past the cap at all: {}",
        source.materialised()
    );
    assert!(
        source.materialised() >= CAP / 2,
        "the budget was actually used: {}",
        source.materialised()
    );
}

#[test]
fn one_huge_directory_is_read_only_as_far_as_the_budget_goes() {
    const CAP: u64 = 10_000;
    let mut fs = FakeFs::new();
    fs.generated_dir("/huge", 5_000_000, 12);
    let source = Counting::new(&fs);

    let result = run(&source, &config(4, Some(CAP), None));

    assert!(matches!(result, Err(ScanError::TooLarge { .. })));
    assert!(source.materialised() <= CAP, "{}", source.materialised());
    assert!(fs.largest_listing() <= CAP as usize);
    assert!(
        source.materialised() >= CAP - 2,
        "root and /huge are listed up to the cap: {}",
        source.materialised()
    );
}

#[test]
fn long_names_reach_the_memory_budget_long_before_the_entry_cap() {
    // 708 bytes each (the name counts twice). A budget of 2000 of them is 1.4 MB; the entry cap alone
    // would allow millions.
    let per_entry = ENTRY_BASE_COST_BYTES + NAME_COPIES * 250;
    let budget_bytes = 2_000 * per_entry;
    let mut fs = FakeFs::new();
    fs.generated_dir("/long", 100_000, 250);
    let source = Counting::new(&fs);

    let result = run(&source, &config(8, Some(10_000_000), Some(budget_bytes)));

    assert!(matches!(
        result,
        Err(ScanError::TooLarge {
            limit: ScanLimit::Memory(limit)
        }) if limit == budget_bytes
    ));
    assert!(
        source.materialised() <= 2_000,
        "{} entries of 708 bytes do not fit {budget_bytes}",
        source.materialised()
    );
    assert!(source.materialised() >= 1_500, "{}", source.materialised());

    // The same directory with short names fits the same budget many times over.
    let mut short = FakeFs::new();
    short.generated_dir("/short", 3_000, 6);
    assert!(run(&short, &config(8, Some(10_000_000), Some(budget_bytes))).is_ok());
}

#[test]
fn a_tree_exactly_at_the_entry_cap_is_scanned_and_one_entry_more_is_not() {
    let mut fs = FakeFs::new();
    fs.generated_dir("/a", 5_000, 8).generated_dir("/b", 3_000, 8);
    // `/a`, `/b` and their 8000 files.
    let entries = 2 + 8_000;

    let at = run(&fs, &config(1, Some(entries), None)).expect("exactly at the cap");
    assert_eq!(at.len() as u64, entries + 1, "the root is not counted");

    let below = run(&fs, &config(1, Some(entries - 1), None));
    assert!(matches!(
        below,
        Err(ScanError::TooLarge {
            limit: ScanLimit::Entries(limit)
        }) if limit == entries - 1
    ));
}

#[test]
fn a_tree_exactly_at_the_name_byte_limit_is_scanned_and_one_byte_less_is_not() {
    let mut fs = FakeFs::new();
    fs.generated_dir("/a", 500, 20);
    // 500 names of 20 bytes and the name `a`.
    let total = 500 * 20 + 1;
    let with_limit = |limit| ScanConfig {
        max_name_bytes: Some(limit),
        ..config(1, None, None)
    };

    assert!(run(&fs, &with_limit(total)).is_ok());
    assert!(matches!(
        run(&fs, &with_limit(total - 1)),
        Err(ScanError::TooLarge {
            limit: ScanLimit::Names(limit)
        }) if limit == total - 1
    ));
}

#[test]
fn a_budget_that_is_not_reached_does_not_fail_the_scan() {
    let mut fs = FakeFs::new();
    fs.generated_dir("/a", 1_000, 10);

    let tree = run(
        &fs,
        &config(
            8,
            Some(1_001),
            Some(1_001 * (ENTRY_BASE_COST_BYTES + 2 * 10 + 400) * 2),
        ),
    )
    .unwrap();

    assert_eq!(tree.len(), 1 + 1 + 1_000);
}

#[test]
fn the_scan_ends_with_too_large_exactly_once_whichever_workers_hit_a_limit() {
    let fs = sibling_directories(20, 5_000, 8);
    let failures = AtomicU64::new(0);

    for _ in 0..3 {
        let result = run(&fs, &config(16, Some(30_000), Some(1 << 40)));
        if let Err(ScanError::TooLarge { limit }) = result {
            failures.fetch_add(1, Ordering::SeqCst);
            assert_eq!(limit, ScanLimit::Entries(30_000), "one consistent limit");
        } else {
            panic!("a scan of 100 000 entries has to fail under a cap of 30 000");
        }
    }
    assert_eq!(failures.into_inner(), 3);
}

/// The entries of a chunk tree.
fn entries_in(chunk: &Chunk) -> u64 {
    chunk.entries.len() as u64 + chunk.subdirs.iter().map(entries_in).sum::<u64>()
}

/// Walks the fake from the root with `budget` and returns what was collected.
fn walk(fs: &impl DirSource<Handle = usize>, budget: &EntryBudget, threads: usize) -> Chunk {
    let root = fs.open_root(Path::new("/"), SourceOptions::default()).unwrap();
    let cancel = CancelToken::new();
    let reporter = Reporter::new(&|_| {}, "/".to_owned(), Duration::from_secs(1));
    let walker = Walker {
        source: fs,
        cancel: &cancel,
        reporter: &reporter,
        budget,
        excluded: &[],
    };

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    pool.install(|| {
        let chain = PathChain {
            parent: None,
            name: b"/",
            identity: (0, 0),
        };
        walker.walk(root.handle, 0, &chain)
    })
}

#[test]
fn after_a_clean_walk_what_is_reserved_is_exactly_what_was_collected() {
    let mut fs = FakeFs::new();
    for dir in 0..40 {
        for file in 0..25 {
            fs.file(&format!("/d{dir:02}/sub/file-{file:02}.txt"), 1);
        }
    }
    fs.generated_dir("/gen", 3_000, 9);

    for threads in [1, 8] {
        let budget = EntryBudget::new(BudgetLimits::UNLIMITED, threads);

        let chunk = walk(&fs, &budget, threads);

        let (reserved, name_bytes) = budget.used();
        assert_eq!(reserved, entries_in(&chunk), "{threads} threads");
        assert_eq!(
            name_bytes,
            chunk_names(&chunk),
            "{threads} threads: the name bytes too"
        );
        assert!(!budget.is_exhausted());
    }
}

fn chunk_names(chunk: &Chunk) -> u64 {
    chunk.names.len() as u64 + chunk.subdirs.iter().map(chunk_names).sum::<u64>()
}

#[test]
fn a_listing_that_fails_midway_gives_its_reservation_back_and_nothing_leaks() {
    let mut fs = FakeFs::new();
    fs.generated_dir("/ok", 2_000, 8)
        .generated_dir("/broken", 5_000, 8)
        .fail_listing_after("/broken", 3_000);
    let budget = EntryBudget::new(BudgetLimits::UNLIMITED, 4);

    let chunk = walk(&fs, &budget, 4);

    // `/broken` is one unreadable directory with no entries, and none of its 3000 admitted entries stay
    // reserved.
    let broken = chunk
        .subdirs
        .iter()
        .find(|sub| sub.unreadable)
        .expect("the failed listing is flagged");
    assert!(broken.entries.is_empty());
    assert_eq!(budget.used().0, entries_in(&chunk));
    assert_eq!(
        budget.used().0,
        2 + 2_000,
        "the root's two entries and /ok's files"
    );
    assert!(
        !budget.is_exhausted(),
        "a failed listing is an error, not a limit"
    );
}

#[test]
fn a_failed_listing_does_not_use_up_the_room_other_directories_need() {
    // 3000 entries are admitted in `/broken` before it fails; with a cap of 2005 the scan only fits if
    // that reservation went back.
    let mut fs = FakeFs::new();
    fs.generated_dir("/broken", 5_000, 8)
        .fail_listing_after("/broken", 1_500)
        .generated_dir("/ok", 2_000, 8);

    let tree = run(&fs, &config(1, Some(2_002), None)).expect("the failure freed its room");

    assert_eq!(tree.totals().errors, 1);
    assert_eq!(tree.len(), 1 + 2 + 2_000);
}

#[test]
fn a_source_that_never_asks_for_room_is_still_held_to_the_cap_after_each_listing() {
    let mut fs = FakeFs::new();
    for dir in 0..5 {
        fs.generated_dir(&format!("/d{dir}"), 1_000, 8);
    }

    let result = run(&Greedy(&fs), &config(1, Some(2_500), None));

    assert!(matches!(
        result,
        Err(ScanError::TooLarge {
            limit: ScanLimit::Entries(2_500)
        })
    ));
    assert!(
        run(&Greedy(&fs), &config(1, Some(5_005), None)).is_ok(),
        "exactly at the cap"
    );
}
