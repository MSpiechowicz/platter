use std::cmp::Reverse;
use std::path::{Component, Path, PathBuf};

use super::NameFilter;
use crate::error::QueryError;
use crate::model::{NodeId, NodeKind, Tree};

/// The longest query [`find_folders`] accepts, in bytes. Longer than any path component, and short
/// enough to bound one request.
pub const MAX_FIND_QUERY_BYTES: usize = 1024;

/// The most matches [`find_folders`] returns, whatever the caller asks for.
pub const MAX_FOLDER_MATCHES: usize = 20;

/// One folder a query found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderMatch {
    pub node: NodeId,
    /// The folder's path for people to read: the home directory shown as `~`, names that are not valid
    /// UTF-8 converted lossily.
    pub path: String,
    pub allocated: u64,
}

/// Finds folders of a scan by what the user typed, as a folder picker needs. A pure lookup in the tree:
/// nothing on disk is read.
///
/// The matches come in three tiers, each folder once. First the folder at exactly the typed path, when the
/// query starts with `~/` (the home directory) or `/`. Then folders whose name starts with the query, then
/// folders whose name contains it (case-insensitive). Inside a tier the largest folder comes first. Only
/// folders match: files, links and skipped mounts never do. The scan root matches only by its exact path.
///
/// `limit` is cut to [`MAX_FOLDER_MATCHES`]. A blank query finds nothing; a query longer than
/// [`MAX_FIND_QUERY_BYTES`] is an error.
pub fn find_folders(
    tree: &Tree,
    home: Option<&Path>,
    query: &str,
    limit: usize,
) -> Result<Vec<FolderMatch>, QueryError> {
    if query.len() > MAX_FIND_QUERY_BYTES {
        return Err(QueryError::QueryTooLong(MAX_FIND_QUERY_BYTES));
    }

    let limit = limit.min(MAX_FOLDER_MATCHES);
    let Some(filter) = NameFilter::new(query) else {
        return Ok(Vec::new());
    };

    let mut found: Vec<NodeId> = Vec::new();
    if let Some(node) = exact_folder(tree, home, query) {
        found.push(node);
    }

    let (prefix, inside) = by_name(tree, &filter, query, found.first().copied());
    found.extend(prefix);
    found.extend(inside);
    found.truncate(limit);

    Ok(found
        .into_iter()
        .map(|node| FolderMatch {
            node,
            path: display(&tree.path_of(node), home),
            allocated: tree.allocated(node),
        })
        .collect())
}

/// The folder at the typed path, for a query that is a path.
fn exact_folder(tree: &Tree, home: Option<&Path>, query: &str) -> Option<NodeId> {
    let typed = absolute(home, query)?;
    let below_root = typed.strip_prefix(tree.root_path()).ok()?;

    let mut current = tree.root();
    for component in below_root.components() {
        match component {
            Component::Normal(name) => {
                let name = name.as_encoded_bytes();
                current = tree.children(current).find(|&child| tree.name(child) == name)?;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }

    (tree.kind(current) == NodeKind::Dir).then_some(current)
}

/// The typed path as an absolute one: `~` and `~/...` are below the home directory.
fn absolute(home: Option<&Path>, query: &str) -> Option<PathBuf> {
    if query == "~" {
        return home.map(Path::to_path_buf);
    }

    if let Some(rest) = query.strip_prefix("~/") {
        return Some(home?.join(rest.trim_start_matches('/')));
    }

    query.starts_with('/').then(|| PathBuf::from(query))
}

/// Folders whose name starts with the query, and those that only contain it; each largest first.
fn by_name(
    tree: &Tree,
    filter: &NameFilter,
    query: &str,
    exact: Option<NodeId>,
) -> (Vec<NodeId>, Vec<NodeId>) {
    let mut prefix = Vec::new();
    let mut inside = Vec::new();

    // A name never holds a separator, so a path query cannot match one.
    if query.contains('/') {
        return (prefix, inside);
    }

    let lowered = query.to_lowercase();
    for index in 1..tree.len() {
        let node = NodeId(index as u32);
        if tree.kind(node) != NodeKind::Dir || exact == Some(node) {
            continue;
        }

        let name = tree.name(node);
        if !filter.matches(name) {
            continue;
        }

        if starts_with_ignoring_case(name, &lowered) {
            prefix.push(node);
        } else {
            inside.push(node);
        }
    }

    for tier in [&mut prefix, &mut inside] {
        tier.sort_unstable_by_key(|&node| (Reverse(tree.allocated(node)), node));
    }
    (prefix, inside)
}

/// Whether `name` starts with `lowered`, a lower-cased query. A name that is not valid UTF-8 is compared
/// in the form it is shown in.
fn starts_with_ignoring_case(name: &[u8], lowered: &str) -> bool {
    if name.is_ascii() && lowered.is_ascii() {
        let needle = lowered.as_bytes();
        return name.len() >= needle.len() && name[..needle.len()].eq_ignore_ascii_case(needle);
    }

    String::from_utf8_lossy(name).to_lowercase().starts_with(lowered)
}

/// `path` with the home directory shown as `~`.
fn display(path: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return path.to_string_lossy().into_owned();
    };

    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeFs, scan_fake};

    fn fixture() -> Tree {
        let mut fs = FakeFs::new();
        fs.file("/home/u/Projects/app/src/main.rs", 100);
        fs.file("/home/u/Projects/app/target/big.bin", 5000);
        fs.file("/home/u/Projects/Apple/x", 300);
        fs.file("/home/u/notes/pineapple/y", 700);
        fs.file("/home/u/notes/appendix/z", 50);
        fs.file("/home/u/app", 9999);
        fs.file("/home/other/app/q", 10);
        fs.symlink("/home/u/applink", 1);
        scan_fake(&fs, false)
    }

    fn home() -> &'static Path {
        Path::new("/home/u")
    }

    fn paths(matches: &[FolderMatch]) -> Vec<&str> {
        matches.iter().map(|found| found.path.as_str()).collect()
    }

    #[test]
    fn name_prefix_comes_before_substring_and_the_largest_comes_first_in_a_tier() {
        let tree = fixture();

        let found = find_folders(&tree, Some(home()), "APP", 20).unwrap();

        assert_eq!(
            paths(&found),
            [
                "~/Projects/app",
                "~/Projects/Apple",
                "~/notes/appendix",
                "/home/other/app",
                "~/notes/pineapple",
            ],
            "a file and a link named app... never match"
        );
        assert_eq!(found[0].allocated, 5100);
    }

    #[test]
    fn an_exact_path_comes_first_and_is_not_repeated() {
        let tree = fixture();

        let by_home = find_folders(&tree, Some(home()), "~/Projects/app", 20).unwrap();
        assert_eq!(paths(&by_home), ["~/Projects/app"]);
        assert_eq!(by_home[0].allocated, 5100);

        let absolute = find_folders(&tree, Some(home()), "/home/other/app/", 20).unwrap();
        assert_eq!(paths(&absolute), ["/home/other/app"]);

        let home_itself = find_folders(&tree, Some(home()), "~", 20).unwrap();
        assert_eq!(paths(&home_itself), ["~"]);
    }

    #[test]
    fn a_path_that_is_not_a_folder_of_the_scan_finds_nothing() {
        let tree = fixture();

        for query in [
            "~/app",
            "~/nothing",
            "~/Projects/../notes",
            "/home/u/applink",
            "/elsewhere",
            "app/src",
        ] {
            assert!(
                find_folders(&tree, Some(home()), query, 20).unwrap().is_empty(),
                "{query}"
            );
        }

        assert!(
            find_folders(&tree, None, "~/Projects", 20).unwrap().is_empty(),
            "no home, no `~`"
        );
    }

    #[test]
    fn without_a_home_paths_are_shown_in_full() {
        let tree = fixture();

        let found = find_folders(&tree, None, "pineapple", 20).unwrap();

        assert_eq!(paths(&found), ["/home/u/notes/pineapple"]);
    }

    #[test]
    fn the_scan_root_matches_only_by_its_exact_path() {
        let tree = fixture();

        assert_eq!(paths(&find_folders(&tree, None, "/", 20).unwrap()), ["/"]);
        assert!(find_folders(&tree, None, "", 20).unwrap().is_empty());
        assert!(find_folders(&tree, None, "  ", 20).unwrap().is_empty());
    }

    #[test]
    fn a_scan_of_a_subfolder_finds_only_inside_it() {
        let tree = {
            let mut fs = FakeFs::new();
            fs.file("/home/u/Projects/app/a", 10);
            fs.file("/home/u/other/app/b", 10);
            let config = crate::ScanConfig {
                now: Some(crate::testing::NOW),
                ..crate::ScanConfig::default()
            };
            crate::scan(
                &fs,
                Path::new("/home/u/Projects"),
                &config,
                &crate::CancelToken::new(),
                &|_| {},
            )
            .unwrap()
        };

        assert_eq!(
            paths(&find_folders(&tree, Some(home()), "~/Projects/app", 20).unwrap()),
            ["~/Projects/app"]
        );
        assert!(
            find_folders(&tree, Some(home()), "~/other/app", 20)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            paths(&find_folders(&tree, Some(home()), "~/Projects", 20).unwrap()),
            ["~/Projects"],
            "the root, by its exact path"
        );
    }

    #[test]
    fn the_limit_is_kept_and_cut_to_twenty() {
        let mut fs = FakeFs::new();
        for index in 0..30 {
            fs.file(&format!("/d/item{index:02}/f"), 10 + index);
        }
        let tree = scan_fake(&fs, false);

        assert_eq!(find_folders(&tree, None, "item", 3).unwrap().len(), 3);
        assert_eq!(find_folders(&tree, None, "item", 100).unwrap().len(), 20);
        assert!(find_folders(&tree, None, "item", 0).unwrap().is_empty());
        assert_eq!(
            find_folders(&tree, None, "item", 1).unwrap()[0].path,
            "/d/item29",
            "the largest first"
        );
    }

    #[test]
    fn a_query_over_the_cap_is_an_error() {
        let tree = fixture();

        let longest = "a".repeat(MAX_FIND_QUERY_BYTES);
        assert!(find_folders(&tree, None, &longest, 20).is_ok());

        let too_long = "a".repeat(MAX_FIND_QUERY_BYTES + 1);
        assert_eq!(
            find_folders(&tree, None, &too_long, 20),
            Err(QueryError::QueryTooLong(MAX_FIND_QUERY_BYTES))
        );
    }

    #[test]
    fn names_with_other_alphabets_match_ignoring_case() {
        let mut fs = FakeFs::new();
        fs.file("/home/u/Żółć/a", 10);
        fs.file("/home/u/x-żółć/b", 20);
        let tree = scan_fake(&fs, false);

        let found = find_folders(&tree, Some(home()), "ŻÓŁ", 20).unwrap();

        assert_eq!(paths(&found), ["~/Żółć", "~/x-żółć"]);
    }
}
