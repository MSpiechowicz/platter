//! How much stack the recursive walk really uses on a worker, in the worst shape this walker can meet.
//!
//! Every directory with several subdirectories forks them with a nested `par_iter`. A worker that waits
//! on a forked half (because another worker stole it) does not sleep: it steals further tasks and runs
//! them on top of its current frames, so frames of independent deep chains can pile up on one stack.
//! The test builds many deep chains that all fork at every level, scans them on many threads and
//! measures, per worker thread, how far the stack pointer travelled between its shallowest and deepest
//! observation (taken in `read_dir`, the deepest frame of every task).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::thread::ThreadId;
use std::time::Duration;

use crate::budget::EntryBudget;
use crate::ports::{DirSource, SourceOptions};
use crate::scan::progress::{PathChain, Reporter};
use crate::scan::walker::{
    MAX_DEPTH, MAX_STACK_FRAMES, Walker, frames_on_this_thread, set_frames_on_this_thread,
};
use crate::scan::{CancelToken, ScanConfig, WORKER_STACK_BYTES, scan};
use crate::testing::FakeFs;

/// Address of a local variable: a stand-in for the stack pointer (it needs no `unsafe`).
#[inline(never)]
fn stack_position() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

/// Deterministic xorshift generator, so the tree is the same on every run.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }
}

/// `chains` deep chains in which a side branch (itself a deep chain that may branch again) leaves with
/// probability `branch_per_mille / 1000` at every level. The side branch is named so that it sorts
/// either before or after the main chain, so the forking worker runs either the deep half or the short
/// one itself. Stops growing at `max_dirs` directories.
fn branching_chains(chains: usize, depth: usize, branch_per_mille: u64, max_dirs: usize) -> FakeFs {
    let mut fs = FakeFs::new();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut dirs = 0;

    let mut open: Vec<(String, usize)> = (0..chains).map(|chain| (format!("/c{chain}"), 1)).collect();
    while let Some((mut path, mut level)) = open.pop() {
        while level < depth {
            fs.dir(&path);
            dirs += 1;

            if dirs < max_dirs && rng.below(1000) < branch_per_mille {
                let side = if rng.below(2) == 0 { "a" } else { "z" };
                open.push((format!("{path}/{side}"), level + 1));
            }
            path.push_str("/m");
            level += 1;
        }
        fs.file(&format!("{path}/leaf"), 1);
    }
    fs
}

/// One chain of `depth` levels in which every level also has a `side` directory, so every directory
/// forks into two subdirectories: the shape with the biggest frames per level.
fn forking_chain(depth: usize) -> FakeFs {
    let mut fs = FakeFs::new();
    let mut path = String::from("/n");

    for _ in 0..depth {
        fs.file(&format!("{path}/side/f"), 1);
        path.push_str("/n");
    }
    fs.file(&format!("{path}/leaf"), 1);
    fs
}

/// The shape that piles frames up: `chains` chains that first run about `stem` levels (a little less,
/// varying per chain so workers finish at different times) with a single
/// subdirectory (no fork, so a worker gets deep cheaply) and then split into a short branch `a` and a
/// long forking branch `b` of `tail` levels. The worker that reaches the split runs `a` itself, finds `b`
/// stolen and still running, and waits by stealing the next chain, on top of its stem.
fn stems_with_a_long_tail(chains: usize, stem: usize, tail: usize) -> FakeFs {
    let mut fs = FakeFs::new();

    for chain in 0..chains {
        let mut base = format!("/c{chain}");
        for _ in 0..stem - chain * 37 % 90 {
            base.push_str("/s");
        }
        fs.file(&format!("{base}/a/f"), 1);

        let mut path = format!("{base}/b");
        for _ in 0..tail {
            fs.file(&format!("{path}/side/f"), 1);
            path.push_str("/n");
        }
        fs.file(&format!("{path}/leaf"), 1);
    }
    fs
}

/// Per thread: lowest and highest stack position seen, and the most `walk` frames seen.
type Seen = HashMap<ThreadId, (usize, usize, usize)>;

/// Scans `fs` and returns the largest stack travel seen on any worker, in bytes, and the most `walk`
/// frames seen on any worker's stack.
fn worst_stack_use(fs: &mut FakeFs, threads: usize) -> (usize, usize) {
    let seen: &'static Mutex<Seen> = Box::leak(Box::default());
    fs.on_read_dir(move |path| {
        let here = stack_position();
        let frames = frames_on_this_thread();

        // Directories that take a moment keep the other half of a fork "stolen and running" longer,
        // which is what makes a waiting worker go and steal more work.
        let _ = path;
        std::thread::sleep(std::time::Duration::from_micros(30));

        let mut seen = seen.lock().unwrap();
        let (low, high, most) = seen
            .entry(std::thread::current().id())
            .or_insert((here, here, frames));
        *low = (*low).min(here);
        *high = (*high).max(here);
        *most = (*most).max(frames);
    });

    let config = ScanConfig {
        threads: Some(threads),
        ..ScanConfig::default()
    };
    let tree = scan(fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).expect("scan succeeds");
    assert_eq!(tree.totals().errors, 0, "nothing was cut off by the depth limit");

    let seen = seen.lock().unwrap();
    let travel = seen.values().map(|(low, high, _)| high - low).max().unwrap_or(0);
    let frames = seen.values().map(|(_, _, most)| *most).max().unwrap_or(0);

    (travel, frames)
}

/// Bytes of stack one directory level costs in the forking shape, measured on one thread.
fn bytes_per_level() -> usize {
    let depth = MAX_DEPTH as usize - 2;
    let (travel, frames) = worst_stack_use(&mut forking_chain(depth), 1);

    assert!(frames > depth, "the chain was walked ({frames} frames)");
    travel / depth
}

#[test]
fn the_frame_limit_fits_the_worker_stack_with_room_to_spare() {
    let per_level = bytes_per_level();
    let worst = per_level * MAX_STACK_FRAMES;

    eprintln!(
        "{per_level} bytes per level; {MAX_STACK_FRAMES} frames = {worst} bytes = {:.1}% of the {WORKER_STACK_BYTES} byte stack",
        100.0 * worst as f64 / WORKER_STACK_BYTES as f64
    );
    assert!(worst < WORKER_STACK_BYTES * 3 / 4);
}

#[test]
fn deep_chains_that_wait_and_steal_never_pile_up_more_than_the_frame_limit() {
    // 300 chains of about 900 to 990 single levels and a 30 level tail: every nested steal adds about 1000 frames, so
    // without the limit a third one would go past it.
    let mut fs = stems_with_a_long_tail(150, 990, 30);

    let (travel, frames) = worst_stack_use(&mut fs, 32);

    eprintln!(
        "32 threads: most frames on a worker {frames} (limit {MAX_STACK_FRAMES}), travel {travel} bytes = {:.1}% of the stack",
        100.0 * travel as f64 / WORKER_STACK_BYTES as f64
    );
    assert!(frames <= MAX_STACK_FRAMES, "{frames} frames on one stack");
    assert!(travel < WORKER_STACK_BYTES * 3 / 4, "{travel} bytes of stack");
}

#[test]
fn branching_chains_stay_within_the_frame_limit_too() {
    let mut fs = branching_chains(32, MAX_DEPTH as usize - 2, 3, 40_000);

    let (travel, frames) = worst_stack_use(&mut fs, 32);

    eprintln!(
        "branching chains, 32 threads: most frames {frames}, travel {travel} bytes = {:.1}% of the stack",
        100.0 * travel as f64 / WORKER_STACK_BYTES as f64
    );
    assert!(frames <= MAX_STACK_FRAMES);
    assert!(travel < WORKER_STACK_BYTES * 3 / 4);
}

/// Walks a directory with eight subdirectories on a four thread pool, on a thread that already carries
/// `frames` walk frames, and returns how many distinct threads read directories.
fn threads_used_when_already_carrying(frames: usize) -> usize {
    let mut fs = FakeFs::new();
    for index in 0..8 {
        fs.file(&format!("/d{index}/f"), 1);
    }
    let readers: &'static Mutex<HashSet<ThreadId>> = Box::leak(Box::default());
    fs.on_read_dir(move |_| {
        readers.lock().unwrap().insert(std::thread::current().id());
        std::thread::sleep(Duration::from_millis(10));
    });

    let root = fs.open_root(Path::new("/"), SourceOptions::default()).unwrap();
    let cancel = CancelToken::new();
    let reporter = Reporter::new(&|_| {}, "/".to_owned(), Duration::from_secs(1));
    let budget = EntryBudget::unlimited();
    let walker = Walker {
        source: &fs,
        cancel: &cancel,
        reporter: &reporter,
        budget: &budget,
        excluded: &[],
    };

    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    pool.install(|| {
        set_frames_on_this_thread(frames);
        let chain = PathChain {
            parent: None,
            name: b"/",
            identity: (0, 0),
        };
        walker.walk(root.handle, 0, &chain);
        set_frames_on_this_thread(0);
    });

    readers.lock().unwrap().len()
}

#[test]
fn a_directory_forks_while_the_stack_has_room_and_reads_serially_once_it_does_not() {
    assert!(
        threads_used_when_already_carrying(0) > 1,
        "with room to spare the subdirectories are spread over the workers"
    );
    assert_eq!(
        threads_used_when_already_carrying(MAX_STACK_FRAMES - MAX_DEPTH as usize),
        1,
        "with no room for another maximum-depth task nothing forks (so nothing waits and steals)"
    );
    assert!(
        threads_used_when_already_carrying(MAX_STACK_FRAMES - MAX_DEPTH as usize - 1) > 1,
        "with room for exactly one it still forks"
    );
}
