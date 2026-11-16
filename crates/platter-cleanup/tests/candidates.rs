//! Which folders the local model may be asked about, on the in-memory filesystem.

use std::path::{Path, PathBuf};

use platter_cleanup::{CandidateLimits, Protected, label_candidates};
use platter_core::testing::{FakeFs, NOW};
use platter_core::{CancelToken, Environment, NodeId, Os, ScanConfig, Tree, scan};
use platter_rules::{RuleSet, matches, scan_is_kept};

const DAY: i64 = 86_400;

fn env_with(home: Option<&str>) -> Environment {
    Environment {
        home: home.map(PathBuf::from),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
    }
}

fn env() -> Environment {
    env_with(Some("/home/u"))
}

fn scan_at(fs: &FakeFs, root: &str) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    scan(fs, Path::new(root), &config, &CancelToken::new(), &|_| {}).expect("the fake scans")
}

fn limits(min_bytes: u64) -> CandidateLimits {
    CandidateLimits { min_bytes, max: 50 }
}

fn found(tree: &Tree, env: &Environment, limits: CandidateLimits) -> Vec<String> {
    label_candidates(tree, env, &RuleSet::builtin(), limits)
        .into_iter()
        .map(|node| String::from_utf8_lossy(&tree.path_bytes(node)).into_owned())
        .collect()
}

fn aged(fs: &mut FakeFs, path: &str, size: u64, days: i64) {
    fs.file(path, size).set_mtime(path, NOW - days * DAY);
}

#[test]
fn protected_refuses_names_the_place_a_path_hits_and_refuses_paths_it_cannot_read() {
    let protected = Protected::for_environment(&env()).unwrap();

    assert_eq!(
        protected.refuses(b"/home/u/.ssh/keys").as_deref(),
        Some("/home/u/.ssh")
    );
    assert_eq!(protected.refuses(b"/home/u").as_deref(), Some("/home/u"));
    assert_eq!(protected.refuses(b"/home/u/free/x"), None);
    assert_eq!(
        protected.refuses(b"/home/u/free/../.ssh").as_deref(),
        Some("/home/u/free/../.ssh")
    );
    assert!(protected.refuses(b"relative/path").is_some());
}

#[test]
fn the_default_limits_are_fifty_folders_of_at_least_256_mib() {
    assert_eq!(CandidateLimits::DEFAULT.max, 50);
    assert_eq!(CandidateLimits::DEFAULT.min_bytes, 256 * 1024 * 1024);
}

#[test]
fn the_largest_folders_no_rule_speaks_for_come_largest_first_and_small_ones_are_left_out() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/stuff/a", 1000);
    fs.file("/home/u/media/b", 600);
    fs.file("/home/u/tiny/c", 50);
    fs.file("/data/d", 800);
    let tree = scan_at(&fs, "/");

    assert_eq!(
        found(&tree, &env(), limits(100)),
        ["/home/u/stuff", "/data", "/home/u/media"]
    );
    assert_eq!(
        found(
            &tree,
            &env(),
            CandidateLimits {
                min_bytes: 100,
                max: 2
            }
        ),
        ["/home/u/stuff", "/data"],
        "the largest are kept"
    );
    assert!(found(&tree, &env(), limits(5000)).is_empty());
}

#[test]
fn a_candidate_is_the_outermost_folder_and_its_subtree_is_not_entered() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/stuff/big/a", 1000);
    fs.file("/home/u/stuff/other/b", 600);
    let tree = scan_at(&fs, "/");

    assert_eq!(found(&tree, &env(), limits(100)), ["/home/u/stuff"]);
}

#[test]
fn a_matched_folder_is_neither_a_candidate_nor_entered() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/app/blob", 900);
    fs.file("/home/u/.cache/app/deep/blob", 900);
    fs.file("/home/u/other/blob", 500);
    let tree = scan_at(&fs, "/");

    assert_eq!(found(&tree, &env(), limits(100)), ["/home/u/other"]);
}

#[test]
fn a_folder_holding_a_match_is_entered_so_what_lies_beside_the_match_can_be_chosen() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/work/code/rs/Cargo.toml", 1);
    aged(&mut fs, "/home/u/work/code/rs/target/x", 900, 90);
    fs.file("/home/u/work/notes/n", 500);
    fs.file("/home/u/work/code/rs/src/main.rs", 400);
    let tree = scan_at(&fs, "/");

    let all = found(&tree, &env(), limits(100));

    assert_eq!(all, ["/home/u/work/notes", "/home/u/work/code/rs/src"]);
}

#[test]
fn a_kept_folder_is_neither_a_candidate_nor_entered() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/Documents/thesis/d", 900);
    fs.file("/home/u/.local/share/Steam/steamapps/common/G/g.pak", 900);
    fs.file("/home/u/other/blob", 500);
    let tree = scan_at(&fs, "/");

    assert_eq!(found(&tree, &env(), limits(100)), ["/home/u/other"]);
}

#[test]
fn protected_places_are_not_candidates_but_what_lies_beside_them_is() {
    let mut fs = FakeFs::new();
    fs.file("/etc/big/a", 900);
    fs.file("/usr/lib/b", 900);
    fs.file("/home/u/.config/app/c", 900);
    fs.file("/home/u/.ssh/d", 900);
    fs.file("/home/u/free/e", 500);
    fs.file("/mnt/disk/f", 400);
    let tree = scan_at(&fs, "/");

    assert_eq!(
        found(&tree, &env(), limits(100)),
        ["/home/u/free", "/mnt/disk"],
        "/mnt is only protected as a place, not inside it"
    );
}

#[test]
fn the_scan_root_is_never_a_candidate_and_a_scan_of_one_folder_chooses_inside_it() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/stuff/a/x", 1000);
    fs.file("/home/u/stuff/b/y", 600);
    let tree = scan_at(&fs, "/home/u/stuff");

    assert_eq!(
        found(&tree, &env(), limits(100)),
        ["/home/u/stuff/a", "/home/u/stuff/b"]
    );

    let only_files = scan_at(&fs, "/home/u/stuff/a");
    assert!(found(&only_files, &env(), limits(100)).is_empty());
}

#[test]
fn an_unreadable_folder_is_not_a_candidate_and_one_holding_such_a_part_is_entered_instead() {
    let mut fs = FakeFs::new();
    fs.unreadable_dir("/home/u/locked");
    fs.file("/home/u/partial/ok/a", 500);
    fs.unreadable_dir("/home/u/partial/locked");
    fs.file("/home/u/fine/b", 300);
    let tree = scan_at(&fs, "/");

    assert_eq!(
        found(&tree, &env(), limits(100)),
        ["/home/u/partial/ok", "/home/u/fine"]
    );
}

#[test]
fn links_mounts_and_files_are_not_candidates() {
    let mut fs = FakeFs::new();
    fs.symlink("/home/u/link", 900);
    fs.file("/home/u/file.bin", 900);
    fs.mount("/home/u/mnt");
    fs.file("/home/u/real/a", 500);
    let tree = scan_at(&fs, "/");

    assert_eq!(found(&tree, &env(), limits(100)), ["/home/u/real"]);
}

#[test]
fn without_a_known_home_or_inside_a_kept_folder_there_are_none() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/stuff/a", 1000);
    fs.file("/home/u/Documents/thesis/figures/f", 900);
    let tree = scan_at(&fs, "/");

    assert!(!found(&tree, &env(), limits(100)).is_empty());
    assert!(found(&tree, &env_with(None), limits(100)).is_empty());

    let inside_kept = scan_at(&fs, "/home/u/Documents/thesis");
    assert!(scan_is_kept(&inside_kept, &env(), &RuleSet::builtin()));
    assert!(found(&inside_kept, &env(), limits(1)).is_empty());
}

#[test]
fn a_scan_of_the_kept_folder_itself_has_no_candidates_either() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/Documents/thesis/d", 900);
    let tree = scan_at(&fs, "/home/u/Documents");

    assert!(found(&tree, &env(), limits(1)).is_empty());
}

mod property {
    use super::*;

    const ROUNDS: u64 = 300;
    const MAX_ENTRIES: usize = 120;

    const NAMES: &[&str] = &[
        "target",
        "node_modules",
        "Cargo.toml",
        "package.json",
        "package-lock.json",
        "__pycache__",
        ".cache",
        ".npm",
        "Documents",
        "Pictures",
        ".ssh",
        ".config",
        "Downloads",
        "home",
        "u",
        "var",
        "cache",
        "pacman",
        "pkg",
        "log",
        "etc",
        "usr",
        ".local",
        "share",
        "Steam",
        "steamapps",
        "common",
        "src",
        "a",
        "b",
        "c",
    ];

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn random_tree(rng: &mut Rng) -> (FakeFs, Vec<String>) {
        let mut fs = FakeFs::new();
        let mut dirs = vec![
            String::from("/"),
            String::from("/home/u"),
            String::from("/var/cache"),
        ];
        fs.dir("/home/u").dir("/var/cache");

        for _ in 0..rng.below(MAX_ENTRIES) {
            let parent = dirs[rng.below(dirs.len())].clone();
            let path = format!(
                "{}/{}",
                parent.trim_end_matches('/'),
                NAMES[rng.below(NAMES.len())]
            );
            let days = [1, 40, 90, 400, 900][rng.below(5)];

            match rng.below(12) {
                0 => {
                    fs.symlink(&path, 1);
                }
                1 => {
                    fs.same_filesystem_mount(&path);
                    dirs.push(path);
                }
                2 => {
                    fs.unreadable_dir(&path);
                }
                3 => {
                    fs.file(&format!("{path}/Cargo.toml"), 1);
                    let inside = format!("{path}/target/x");
                    fs.file(&inside, 1 + rng.below(500) as u64)
                        .set_mtime(&inside, NOW - days * DAY);
                    dirs.push(format!("{path}/target"));
                }
                4..=6 => {
                    fs.file(&path, 1 + rng.below(500) as u64)
                        .set_mtime(&path, NOW - days * DAY);
                }
                _ => {
                    let inside = format!("{path}/f{}", rng.below(4));
                    fs.file(&inside, 1 + rng.below(500) as u64);
                    dirs.push(path);
                }
            }
        }

        (fs, dirs)
    }

    fn within(node: NodeId, of: NodeId, tree: &Tree) -> bool {
        tree.ancestry(node).contains(&of)
    }

    #[test]
    fn no_candidate_is_covered_by_holds_or_lies_in_a_match_or_a_protected_place() {
        let env = env();
        let rules = RuleSet::builtin();
        let protected = Protected::for_environment(&env).expect("the home is known");
        let mut chosen = 0;

        for seed in 1..=ROUNDS {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let (fs, dirs) = random_tree(&mut rng);
            let root = dirs[rng.below(dirs.len())].clone();
            let Ok(tree) = scan(
                &fs,
                Path::new(&root),
                &ScanConfig {
                    now: Some(NOW),
                    ..ScanConfig::default()
                },
                &CancelToken::new(),
                &|_| {},
            ) else {
                continue;
            };
            let limits = CandidateLimits {
                min_bytes: [1, 30, 150, 400][rng.below(4)],
                max: [3, 50][rng.below(2)],
            };

            let candidates = label_candidates(&tree, &env, &rules, limits);
            chosen += candidates.len();
            let all_matches: Vec<NodeId> = matches(&tree, &env, &rules)
                .into_iter()
                .flat_map(|rule| rule.nodes)
                .collect();
            let context = format!("seed {seed}, root {root}");

            assert!(candidates.len() <= limits.max, "{context}");
            if scan_is_kept(&tree, &env, &rules) {
                assert!(candidates.is_empty(), "inside a kept folder: {context}");
            }

            for (index, &node) in candidates.iter().enumerate() {
                let path = tree.path_bytes(node);
                let context = format!("{context}, candidate {}", String::from_utf8_lossy(&path));

                assert_ne!(node, tree.root(), "the scan root: {context}");
                assert_eq!(tree.kind(node), platter_core::NodeKind::Dir, "{context}");
                assert!(!tree.is_unreadable(node), "unreadable: {context}");
                assert!(tree.allocated(node) >= limits.min_bytes, "too small: {context}");
                assert_eq!(protected.refuses(&path), None, "protected: {context}");

                for &matched in &all_matches {
                    assert!(!within(node, matched, &tree), "covered by a match: {context}");
                    assert!(!within(matched, node, &tree), "holds a match: {context}");
                }
                for &other in &candidates[index + 1..] {
                    assert!(
                        !within(node, other, &tree) && !within(other, node, &tree),
                        "nested candidates: {context}"
                    );
                    assert!(
                        tree.allocated(other) <= tree.allocated(node),
                        "not largest first: {context}"
                    );
                }
            }
        }

        assert!(
            chosen > 100,
            "the trees give the property something to check: {chosen}"
        );
    }
}
