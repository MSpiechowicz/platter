//! Helpers shared by the integration tests: scan a fake filesystem, build an environment, look things up.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use platter_core::testing::{FakeFs, NOW};
use platter_core::{CancelToken, NodeId, ScanConfig, Tree, scan};
use platter_rules::{Environment, Os, Rule, RuleSet, SuggestionDto, Suggestions, evaluate};

pub const DAY: i64 = 86_400;
pub const HOME: &str = "/home/u";

/// Scans the fake filesystem from `root` with the fixed clock `NOW`.
pub fn scan_at(fs: &FakeFs, root: &str, one_file_system: bool) -> Tree {
    let config = ScanConfig {
        one_file_system,
        now: Some(NOW),
        ..ScanConfig::default()
    };

    scan(fs, Path::new(root), &config, &CancelToken::new(), &|_| {}).expect("scan of the fake succeeds")
}

pub fn env_with_home(home: Option<&str>) -> Environment {
    Environment {
        home: home.map(PathBuf::from),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
        system_root: None,
    }
}

pub fn env() -> Environment {
    env_with_home(Some(HOME))
}

pub fn run(tree: &Tree, rules: Vec<Rule>) -> Suggestions {
    evaluate(tree, &env(), &RuleSet::new(rules))
}

pub fn get<'a>(suggestions: &'a Suggestions, id: &str) -> Option<&'a SuggestionDto> {
    suggestions.suggestions.iter().find(|s| s.id == id)
}

pub fn ids(suggestions: &Suggestions) -> Vec<&str> {
    suggestions.suggestions.iter().map(|s| s.id.as_str()).collect()
}

/// The node at `path`, given relative to the scan root.
pub fn find(tree: &Tree, path: &str) -> NodeId {
    let mut current = tree.root();

    for part in path.split('/').filter(|part| !part.is_empty()) {
        current = tree
            .children(current)
            .find(|&child| tree.name(child) == part.as_bytes())
            .unwrap_or_else(|| panic!("{path} exists in the tree"));
    }
    current
}

/// A file last modified `days` days before the fixed scan time.
pub fn aged_file(fs: &mut FakeFs, path: &str, size: u64, days: i64) {
    fs.file(path, size).set_mtime(path, NOW - days * DAY);
}
