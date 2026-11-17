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

/// The most matches [`find_entries`] returns, whatever the caller asks for.
pub const MAX_ENTRY_MATCHES: usize = 20;

/// One folder or file a query found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMatch {
    pub node: NodeId,
    /// The entry's path for people to read, written like [`FolderMatch::path`].
    pub path: String,
    pub allocated: u64,
    /// Only [`NodeKind::Dir`] or [`NodeKind::File`].
    pub kind: NodeKind,
}

/// Finds folders and files of a scan by what the user typed, as the advisor's autocomplete needs. A pure
/// lookup in the tree: nothing on disk is read.
///
/// A query that starts with `~` or `/` is a path. The text up to its last `/` must be a folder of the
/// scan (a bare `~` means `~/`), and the matches are that folder's children whose name fits the text after
/// it. An empty rest lists every child. Any other query is a name, looked for in the whole tree.
///
/// Either way the matches come in three tiers: names equal to the text, names that start with it, then
/// names that contain it (all case-insensitive). Inside a tier the largest entry comes first. Only folders
/// and files match: links and skipped mounts never do, and neither does the scan root. While it walks the
/// tree only the best `limit` of each tier are kept, so a common text costs no more than a rare one.
///
/// `limit` is cut to [`MAX_ENTRY_MATCHES`]. A blank query or a limit of 0 finds nothing. A query longer
/// than [`MAX_FIND_QUERY_BYTES`] is an error.
pub fn find_entries(
    tree: &Tree,
    home: Option<&Path>,
    query: &str,
    limit: usize,
) -> Result<Vec<EntryMatch>, QueryError> {
    find_entries_until(tree, home, query, limit, || false)
}

/// How many entries [`find_entries_until`] walks between two looks at `stop`.
const STOP_CHECK_INTERVAL: usize = 4096;

/// [`find_entries`] that gives up early. `stop` is asked before the walk starts and then every few thousand
/// entries, and once it says `true` the walk ends and nothing is found (an empty list, not the best of
/// what was seen so far). A caller that has moved on to a newer query uses it so that an old walk over a
/// large tree does not run to its end.
pub fn find_entries_until(
    tree: &Tree,
    home: Option<&Path>,
    query: &str,
    limit: usize,
    stop: impl Fn() -> bool,
) -> Result<Vec<EntryMatch>, QueryError> {
    if query.len() > MAX_FIND_QUERY_BYTES {
        return Err(QueryError::QueryTooLong(MAX_FIND_QUERY_BYTES));
    }

    let limit = limit.min(MAX_ENTRY_MATCHES);
    if limit == 0 || query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut best = BestPerTier::new(limit);
    if is_path(query) {
        let query = if query == "~" { "~/" } else { query };
        let (parent, rest) = query.split_at(query.rfind('/').map_or(0, |slash| slash + 1));

        let Some(parent) = entry_at(tree, home, parent).filter(|&node| tree.kind(node) == NodeKind::Dir)
        else {
            return Ok(Vec::new());
        };

        let rest = Rest::new(rest);
        for (walked, node) in tree.children(parent).enumerate() {
            if walked % STOP_CHECK_INTERVAL == 0 && stop() {
                return Ok(Vec::new());
            }
            best.offer(tree, &rest, node);
        }
    } else {
        let rest = Rest::new(query);
        for index in 1..tree.len() {
            if (index - 1) % STOP_CHECK_INTERVAL == 0 && stop() {
                return Ok(Vec::new());
            }
            best.offer(tree, &rest, NodeId(index as u32));
        }
    }

    Ok(best
        .into_ranked()
        .into_iter()
        .take(limit)
        .map(|node| EntryMatch {
            node,
            path: display(&tree.path_of(node), home),
            allocated: tree.allocated(node),
            kind: tree.kind(node),
        })
        .collect())
}

/// Finds the files of a scan whose whole name is `name` (case-insensitive), largest first. Unlike
/// [`find_entries`] a folder of that name never takes a place, so any number of them cannot push a small
/// file out. A pure lookup in the tree: nothing on disk is read, and while it walks the tree only the best
/// `limit` files are kept.
///
/// `limit` is cut to [`MAX_ENTRY_MATCHES`]. A blank name or a limit of 0 finds nothing. A name longer than
/// [`MAX_FIND_QUERY_BYTES`] is an error.
pub fn find_files_named(
    tree: &Tree,
    home: Option<&Path>,
    name: &str,
    limit: usize,
) -> Result<Vec<EntryMatch>, QueryError> {
    if name.len() > MAX_FIND_QUERY_BYTES {
        return Err(QueryError::QueryTooLong(MAX_FIND_QUERY_BYTES));
    }

    let limit = limit.min(MAX_ENTRY_MATCHES);
    if limit == 0 || name.trim().is_empty() {
        return Ok(Vec::new());
    }

    let rest = Rest::new(name);
    let mut best: Vec<(Reverse<u64>, NodeId)> = Vec::new();
    for index in 1..tree.len() {
        let node = NodeId(index as u32);
        if tree.kind(node) == NodeKind::File && rest.tier(tree.name(node)) == Some(0) {
            keep_best(&mut best, limit, tree, node);
        }
    }

    Ok(best
        .into_iter()
        .map(|(_, node)| EntryMatch {
            node,
            path: display(&tree.path_of(node), home),
            allocated: tree.allocated(node),
            kind: NodeKind::File,
        })
        .collect())
}

/// Whether the query is typed as a path: the home directory or the file system root, then names.
fn is_path(query: &str) -> bool {
    query == "~" || query.starts_with("~/") || query.starts_with('/')
}

/// The text a name is matched against, already lower-cased.
struct Rest {
    lowered: String,
    /// The substring test. `None` for an empty or blank text, which the filter does not take.
    filter: Option<NameFilter>,
}

impl Rest {
    fn new(text: &str) -> Self {
        Self {
            lowered: text.to_lowercase(),
            filter: NameFilter::new(text),
        }
    }

    /// The tier of `name`: 0 for an exact name, 1 for a name that starts with the text and 2 for one that
    /// only contains it. `None` when it does not fit at all. An empty text puts every name in tier 1.
    fn tier(&self, name: &[u8]) -> Option<usize> {
        let contains = match &self.filter {
            Some(filter) => filter.matches(name),
            None => String::from_utf8_lossy(name)
                .to_lowercase()
                .contains(&self.lowered),
        };
        if !contains {
            return None;
        }

        if starts_with_ignoring_case(name, &self.lowered) {
            return Some(if equals_ignoring_case(name, &self.lowered) {
                0
            } else {
                1
            });
        }
        Some(2)
    }
}

/// The best few entries of each of the three tiers, kept sorted while the tree is walked.
struct BestPerTier {
    limit: usize,
    /// Each tier sorted by (largest first, then id), at most `limit` long.
    tiers: [Vec<(Reverse<u64>, NodeId)>; 3],
}

impl BestPerTier {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            tiers: [Vec::new(), Vec::new(), Vec::new()],
        }
    }

    /// Files `node` in its tier when it is a folder or file that fits and ranks among the best.
    fn offer(&mut self, tree: &Tree, rest: &Rest, node: NodeId) {
        if !matches!(tree.kind(node), NodeKind::Dir | NodeKind::File) {
            return;
        }
        let Some(tier) = rest.tier(tree.name(node)) else {
            return;
        };

        keep_best(&mut self.tiers[tier], self.limit, tree, node);
    }

    /// The tiers one after the other, each in rank order.
    fn into_ranked(self) -> Vec<NodeId> {
        self.tiers.into_iter().flatten().map(|(_, node)| node).collect()
    }
}

/// Keeps `node` in `held` (sorted by largest first, then id) when it ranks among the best `limit`.
fn keep_best(held: &mut Vec<(Reverse<u64>, NodeId)>, limit: usize, tree: &Tree, node: NodeId) {
    let key = (Reverse(tree.allocated(node)), node);
    if held.len() == limit && held.last().is_some_and(|worst| *worst <= key) {
        return;
    }

    let at = held.partition_point(|entry| *entry < key);
    held.insert(at, key);
    held.truncate(limit);
}

/// The folder at the typed path, for a query that is a path.
fn exact_folder(tree: &Tree, home: Option<&Path>, query: &str) -> Option<NodeId> {
    entry_at(tree, home, query).filter(|&node| tree.kind(node) == NodeKind::Dir)
}

/// The folder or file at exactly the typed path (`~`, `~/...` or `/...`), or `None` when there is none:
/// the path is missing, is a link, a skipped mount or another kind of entry, or lies outside the scan
/// root. A pure lookup in the tree: nothing on disk is read.
pub fn entry_at(tree: &Tree, home: Option<&Path>, path: &str) -> Option<NodeId> {
    let typed = absolute(home, path)?;
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

    matches!(tree.kind(current), NodeKind::Dir | NodeKind::File).then_some(current)
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

/// Whether `name` equals `lowered`, a lower-cased query, in the same way as [`starts_with_ignoring_case`].
fn equals_ignoring_case(name: &[u8], lowered: &str) -> bool {
    if name.is_ascii() && lowered.is_ascii() {
        return name.eq_ignore_ascii_case(lowered.as_bytes());
    }

    String::from_utf8_lossy(name).to_lowercase() == lowered
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

    fn entry_paths(matches: &[EntryMatch]) -> Vec<&str> {
        matches.iter().map(|found| found.path.as_str()).collect()
    }

    fn entries(tree: &Tree, query: &str, limit: usize) -> Vec<EntryMatch> {
        find_entries(tree, Some(home()), query, limit).unwrap()
    }

    #[test]
    fn files_named_finds_only_files_with_exactly_that_name() {
        let tree = fixture();

        let found = find_files_named(&tree, Some(home()), "APP", 20).unwrap();

        assert_eq!(
            entry_paths(&found),
            ["~/app"],
            "the folders called app are not files"
        );
        assert_eq!((found[0].kind, found[0].allocated), (NodeKind::File, 9999));
        assert!(
            find_files_named(&tree, Some(home()), "ap", 20)
                .unwrap()
                .is_empty()
        );
        assert!(
            find_files_named(&tree, Some(home()), "app", 0)
                .unwrap()
                .is_empty()
        );
        assert!(
            find_files_named(&tree, Some(home()), "  ", 20)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            find_files_named(&tree, Some(home()), &"a".repeat(MAX_FIND_QUERY_BYTES + 1), 20),
            Err(QueryError::QueryTooLong(MAX_FIND_QUERY_BYTES))
        );
    }

    #[test]
    fn a_name_finds_folders_and_files_in_tiers_with_the_largest_first() {
        let tree = fixture();

        let found = entries(&tree, "APP", 20);

        assert_eq!(
            entry_paths(&found),
            [
                "~/app",
                "~/Projects/app",
                "/home/other/app",
                "~/Projects/Apple",
                "~/notes/appendix",
                "~/notes/pineapple",
            ],
            "exact names, then names that start with it, then names that hold it, never the link"
        );
        assert_eq!(found[0].kind, NodeKind::File);
        assert_eq!(found[0].allocated, 9999);
        assert_eq!(found[1].kind, NodeKind::Dir);
        assert_eq!(found[1].allocated, 5100);
    }

    #[test]
    fn a_path_lists_the_children_of_its_folder_with_a_prefix_before_a_substring() {
        let tree = fixture();

        assert_eq!(
            entry_paths(&entries(&tree, "~/p", 20)),
            ["~/Projects", "~/app"],
            "Projects starts with p, the larger app only holds one"
        );
        assert_eq!(
            entry_paths(&entries(&tree, "~/Projects/ap", 20)),
            ["~/Projects/app", "~/Projects/Apple"]
        );
        assert_eq!(
            entry_paths(&entries(&tree, "/home/u/notes/PINE", 20)),
            ["~/notes/pineapple"]
        );
        assert!(entries(&tree, "~/Projects/zzz", 20).is_empty());
    }

    #[test]
    fn an_exact_name_comes_before_a_larger_prefix_match() {
        let mut fs = FakeFs::new();
        fs.file("/home/u/Log", 10);
        fs.file("/home/u/log.old", 9000);
        fs.file("/home/u/blog", 90_000);
        let tree = scan_fake(&fs, false);

        assert_eq!(
            entry_paths(&entries(&tree, "~/log", 20)),
            ["~/Log", "~/log.old", "~/blog"]
        );
        assert_eq!(
            entry_paths(&entries(&tree, "log", 20)),
            ["~/Log", "~/log.old", "~/blog"]
        );
    }

    #[test]
    fn an_empty_rest_lists_every_folder_and_file_child_largest_first() {
        let tree = fixture();

        let listed = entries(&tree, "~/", 20);
        assert_eq!(entry_paths(&listed), ["~/app", "~/Projects", "~/notes"]);
        assert_eq!(listed[2].allocated, 750);

        assert_eq!(entries(&tree, "~", 20), listed, "a bare ~ is ~/");
        assert_eq!(
            entry_paths(&entries(&tree, "/home/u/Projects/app/", 20)),
            ["~/Projects/app/target", "~/Projects/app/src"]
        );
        assert_eq!(
            entry_paths(&find_entries(&tree, None, "/", 20).unwrap()),
            ["/home"],
            "the scan root is listed through its children, never itself"
        );
    }

    #[test]
    fn a_path_without_a_folder_to_list_finds_nothing() {
        let tree = fixture();

        for query in [
            "~/app/x",
            "~/app/",
            "~/missing/",
            "~/Projects/../notes/",
            "/elsewhere/",
            "/home/u/applink/",
        ] {
            assert!(entries(&tree, query, 20).is_empty(), "{query}");
        }

        assert!(
            find_entries(&tree, None, "~/", 20).unwrap().is_empty(),
            "no home, no `~`"
        );
    }

    #[test]
    fn a_path_outside_the_scan_root_finds_nothing() {
        let mut fs = FakeFs::new();
        fs.file("/home/u/Projects/app/a", 10);
        fs.file("/home/u/other/app/b", 10);
        let config = crate::ScanConfig {
            now: Some(crate::testing::NOW),
            ..crate::ScanConfig::default()
        };
        let tree = crate::scan(
            &fs,
            Path::new("/home/u/Projects"),
            &config,
            &crate::CancelToken::new(),
            &|_| {},
        )
        .unwrap();

        assert_eq!(
            entry_paths(&entries(&tree, "~/Projects/", 20)),
            ["~/Projects/app"]
        );
        assert!(entries(&tree, "~/other/", 20).is_empty());
        assert!(
            entries(&tree, "~/", 20).is_empty(),
            "the home holds the root, not the other way round"
        );
        assert!(entries(&tree, "/", 20).is_empty());
    }

    #[test]
    fn links_and_mounts_never_match_and_the_root_never_matches_by_name() {
        let mut fs = FakeFs::new();
        fs.file("/home/u/keep/a", 10);
        fs.symlink("/home/u/keep-link", 5);
        fs.mount("/home/u/keep-mount");
        fs.special("/home/u/keep-socket");
        fs.file("/keep/b", 1);
        let tree = scan_fake(&fs, true);

        assert_eq!(entry_paths(&entries(&tree, "keep", 20)), ["~/keep", "/keep"]);
        assert_eq!(entry_paths(&entries(&tree, "~/keep", 20)), ["~/keep"]);
        assert!(entries(&tree, "/", 20).iter().all(|found| found.path != "/"));
        assert!(entries(&tree, "root", 20).is_empty());
        assert_eq!(tree.kind(tree.root()), NodeKind::Dir);
    }

    #[test]
    fn the_limit_is_kept_and_cut_to_twenty_for_entries() {
        let mut fs = FakeFs::new();
        for index in 0..30 {
            fs.file(&format!("/d/item{index:02}"), 10 + index);
        }
        let tree = scan_fake(&fs, false);

        assert_eq!(entries(&tree, "item", 3).len(), 3);
        assert_eq!(entries(&tree, "item", 100).len(), 20);
        assert_eq!(entries(&tree, "/d/item", 100).len(), 20);
        assert!(entries(&tree, "item", 0).is_empty());
        assert_eq!(entries(&tree, "item", 1)[0].path, "/d/item29");
    }

    #[test]
    fn the_bounded_search_equals_a_full_sort_cut_to_the_limit() {
        let mut fs = FakeFs::new();
        for index in 0..70_u64 {
            // Sizes repeat on purpose, so the order inside a tier falls back to the node id.
            let size = (index * 7) % 13 + 1;
            fs.file(&format!("/d/item{index}"), size);
            fs.file(&format!("/d/x{index}/ITEM"), size + 1);
            fs.file(&format!("/d/y{index}/an-item-{index}"), size + 2);
            fs.file(&format!("/d/z{index}/other"), size);
        }
        let tree = scan_fake(&fs, false);

        let mut naive: Vec<(usize, Reverse<u64>, NodeId)> = (1..tree.len())
            .map(|index| NodeId(index as u32))
            .filter(|&node| matches!(tree.kind(node), NodeKind::Dir | NodeKind::File))
            .filter_map(|node| {
                let name = String::from_utf8_lossy(tree.name(node)).to_lowercase();
                let tier = if name == "item" {
                    0
                } else if name.starts_with("item") {
                    1
                } else if name.contains("item") {
                    2
                } else {
                    return None;
                };
                Some((tier, Reverse(tree.allocated(node)), node))
            })
            .collect();
        naive.sort();
        let expected: Vec<NodeId> = naive.into_iter().take(20).map(|(_, _, node)| node).collect();

        let found: Vec<NodeId> = entries(&tree, "Item", 20)
            .into_iter()
            .map(|found| found.node)
            .collect();

        assert_eq!(found, expected);
        assert!(
            naive_has_all_tiers(&tree),
            "the fixture really fills the tiers beyond the limit"
        );
    }

    fn naive_has_all_tiers(tree: &Tree) -> bool {
        let names: Vec<String> = (1..tree.len())
            .map(|index| String::from_utf8_lossy(tree.name(NodeId(index as u32))).to_lowercase())
            .collect();

        names.iter().filter(|name| *name == "item").count() > 20
            && names
                .iter()
                .filter(|name| name.starts_with("item") && *name != "item")
                .count()
                > 20
            && names
                .iter()
                .filter(|name| name.contains("item") && !name.starts_with("item"))
                .count()
                > 20
    }

    #[test]
    fn a_walk_told_to_stop_finds_nothing_in_a_name_and_in_a_path() {
        let tree = fixture();

        assert!(!entries(&tree, "app", 20).is_empty());
        assert!(!entries(&tree, "~/Projects/", 20).is_empty());

        for query in ["app", "~/Projects/"] {
            let found = find_entries_until(&tree, Some(home()), query, 20, || true).unwrap();

            assert!(found.is_empty(), "{query} found {found:?}");
        }
    }

    #[test]
    fn a_walk_never_told_to_stop_finds_what_find_entries_finds() {
        let tree = fixture();

        for query in ["app", "~/Projects/", "~/Projects/a", "/"] {
            assert_eq!(
                find_entries_until(&tree, Some(home()), query, 20, || false).unwrap(),
                entries(&tree, query, 20),
                "{query}"
            );
        }
    }

    #[test]
    fn a_walk_over_a_large_tree_stops_within_a_few_thousand_entries_of_being_told_to() {
        let mut fs = FakeFs::new();
        for number in 0..(3 * STOP_CHECK_INTERVAL) {
            fs.file(&format!("/d/file{number}"), 1);
        }
        let tree = scan_fake(&fs, true);

        // Not asked to stop until the second look: the walk ends there and does not go on to the third.
        let looks = std::cell::Cell::new(0);
        let found = find_entries_until(&tree, None, "file", 20, || {
            looks.set(looks.get() + 1);
            looks.get() >= 2
        })
        .unwrap();
        assert!(found.is_empty());
        assert_eq!(looks.get(), 2);

        // Never told to stop, it looks once per interval and finds the best of the whole tree.
        let looks = std::cell::Cell::new(0);
        let found = find_entries_until(&tree, None, "file", 20, || {
            looks.set(looks.get() + 1);
            false
        })
        .unwrap();
        assert_eq!(found.len(), 20);
        assert!(looks.get() >= 3, "looked {} times", looks.get());
    }

    #[test]
    fn a_blank_or_too_long_entry_query_is_handled() {
        let tree = fixture();

        assert!(entries(&tree, "", 20).is_empty());
        assert!(entries(&tree, "   ", 20).is_empty());

        let longest = "a".repeat(MAX_FIND_QUERY_BYTES);
        assert!(find_entries(&tree, None, &longest, 20).is_ok());

        let too_long = "a".repeat(MAX_FIND_QUERY_BYTES + 1);
        assert_eq!(
            find_entries(&tree, None, &too_long, 20),
            Err(QueryError::QueryTooLong(MAX_FIND_QUERY_BYTES))
        );
        assert_eq!(
            find_entries(&tree, None, &format!("/{too_long}"), 20),
            Err(QueryError::QueryTooLong(MAX_FIND_QUERY_BYTES))
        );
    }

    #[test]
    fn entries_with_other_alphabets_match_ignoring_case() {
        let mut fs = FakeFs::new();
        fs.file("/home/u/Żółć", 10);
        fs.file("/home/u/x-żółć", 20);
        let tree = scan_fake(&fs, false);

        assert_eq!(entry_paths(&entries(&tree, "żÓŁĆ", 20)), ["~/Żółć", "~/x-żółć"]);
        assert_eq!(entry_paths(&entries(&tree, "~/ŻÓŁ", 20)), ["~/Żółć", "~/x-żółć"]);
    }

    #[test]
    fn entry_at_finds_a_folder_or_a_file_and_nothing_else() {
        let tree = fixture();
        let at = |path: &str| entry_at(&tree, Some(home()), path);

        let folder = at("~/Projects/app").unwrap();
        assert_eq!(tree.kind(folder), NodeKind::Dir);

        let file = at("/home/u/Projects/app/src/main.rs").unwrap();
        assert_eq!(tree.kind(file), NodeKind::File);

        assert_eq!(at("~").map(|node| tree.path_of(node)), Some(home().to_path_buf()));
        assert_eq!(at("/"), Some(tree.root()));
        assert_eq!(at("~/Projects/./app"), Some(folder));

        assert_eq!(at("~/missing"), None);
        assert_eq!(at("~/applink"), None, "a link");
        assert_eq!(at("~/Projects/../notes"), None);
        assert_eq!(at("relative/path"), None);
        assert_eq!(entry_at(&tree, None, "~/app"), None, "no home, no `~`");
    }

    #[test]
    fn entry_at_does_not_return_a_mount() {
        let mut fs = FakeFs::new();
        fs.file("/home/u/a", 1);
        fs.mount("/home/u/disk");
        let tree = scan_fake(&fs, true);

        assert_eq!(entry_at(&tree, Some(home()), "~/disk"), None);
        assert!(entry_at(&tree, Some(home()), "~/a").is_some());
    }
}
