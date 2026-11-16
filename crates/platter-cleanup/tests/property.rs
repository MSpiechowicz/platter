//! Random trees, random scan roots, every built-in rule selected: whatever the layout, a plan never holds
//! a path outside the scan root, the scan root itself, a protected place, a link, or one target inside
//! another. Seeded, so a failure repeats.

use std::path::{Path, PathBuf};

use platter_cleanup::plan_cleanup;
use platter_core::testing::{FakeFs, NOW};
use platter_core::{CancelToken, EntryKind, EntryProbe, Environment, Os, ScanConfig, scan};
use platter_rules::RuleSet;

const ROUNDS: u64 = 400;
const MAX_ENTRIES: usize = 120;

/// Names that make rules match, protected names, and plain ones.
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
    "Desktop",
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
    "journal",
    "etc",
    "usr",
    "lib",
    "docker",
    ".local",
    "share",
    "Trash",
    "src",
    "a",
    "b",
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
    let mut dirs = vec![String::from("/home/u"), String::from("/var/cache/pacman")];
    fs.dir("/home/u").dir("/var/cache/pacman");

    for _ in 0..rng.below(MAX_ENTRIES) {
        let parent = dirs[rng.below(dirs.len())].clone();
        let name = NAMES[rng.below(NAMES.len())];
        let path = format!("{parent}/{name}");
        let days = [1, 40, 90, 400, 900][rng.below(5)];

        match rng.below(14) {
            10 => {
                fs.file(&format!("{path}/Cargo.toml"), 1);
                fs.file(&format!("{path}/target/x"), 50)
                    .set_mtime(&format!("{path}/target/x"), NOW - days * 86_400);
                dirs.push(format!("{path}/target"));
            }
            11 => {
                fs.file(&format!("{path}/package.json"), 1)
                    .file(&format!("{path}/pnpm-lock.yaml"), 1);
                fs.file(&format!("{path}/node_modules/d/i.js"), 50)
                    .set_mtime(&format!("{path}/node_modules/d/i.js"), NOW - days * 86_400);
                dirs.push(format!("{path}/node_modules"));
            }
            12 => {
                fs.file(&format!("{path}/__pycache__/m.pyc"), 5);
            }
            13 => {
                fs.file("/home/u/.cache/x/blob", 7)
                    .file("/var/cache/pacman/pkg/p", 9);
            }
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
            3..=5 => {
                fs.file(&path, 1 + rng.below(1000) as u64)
                    .set_mtime(&path, NOW - days * 86_400);
            }
            _ => {
                fs.dir(&path);
                dirs.push(path);
            }
        }
    }

    (fs, dirs)
}

fn parts(path: &str) -> Vec<&str> {
    path.split('/').filter(|part| !part.is_empty()).collect()
}

fn within(inner: &[&str], outer: &[&str]) -> bool {
    inner.len() >= outer.len() && inner[..outer.len()] == *outer
}

#[test]
fn no_plan_holds_anything_outside_the_scan_root_or_in_a_protected_place() {
    let env = Environment {
        home: Some(PathBuf::from("/home/u")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
    };
    let rules = RuleSet::builtin();
    let ids: Vec<&str> = rules.rules().iter().map(|rule| rule.id).collect();
    let protected_inside = [
        "/home/u/Documents",
        "/home/u/Pictures",
        "/home/u/Desktop",
        "/home/u/.ssh",
        "/home/u/.config",
        "/etc",
        "/usr",
    ];
    let protected_or_above = [
        "/",
        "/home",
        "/home/u",
        "/var",
        "/var/cache",
        "/var/lib",
        "/var/log",
    ];
    let mut planned = 0;

    for seed in 1..=ROUNDS {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (fs, dirs) = random_tree(&mut rng);
        let root = dirs[rng.below(dirs.len())].clone();
        let config = ScanConfig {
            now: Some(NOW),
            ..ScanConfig::default()
        };
        let Ok(tree) = scan(&fs, Path::new(&root), &config, &CancelToken::new(), &|_| {}) else {
            continue;
        };

        let plan = plan_cleanup(&tree, &env, &rules, &ids, &fs);
        let root_parts = parts(&root);
        let targets: Vec<String> = plan.targets().map(|t| t.display_path()).collect();
        planned += targets.len();

        for target in &targets {
            let target_parts = parts(target);
            let context = format!("seed {seed}, root {root}, target {target}");

            assert!(
                within(&target_parts, &root_parts),
                "outside the scan root: {context}"
            );
            assert!(
                target_parts.len() > root_parts.len(),
                "the scan root itself: {context}"
            );
            for place in protected_inside {
                assert!(!within(&target_parts, &parts(place)), "inside {place}: {context}");
            }
            for place in protected_or_above {
                assert!(
                    !within(&parts(place), &target_parts),
                    "{place} or above: {context}"
                );
            }

            let now = fs.probe(target.as_bytes()).expect("a planned target exists");
            assert_ne!(
                now.last().unwrap().meta.kind,
                EntryKind::Symlink,
                "a link: {context}"
            );
            assert!(
                now[root_parts.len()..]
                    .iter()
                    .all(|entry| !entry.meta.is_mount_root),
                "a mount: {context}"
            );
        }

        for (i, a) in targets.iter().enumerate() {
            for b in targets.iter().skip(i + 1) {
                assert!(
                    !within(&parts(a), &parts(b)) && !within(&parts(b), &parts(a)),
                    "nested targets {a} and {b} (seed {seed})"
                );
            }
        }
    }

    assert!(
        planned > 50,
        "the random trees must give the guard something to plan, got {planned}"
    );
}
