//! Chunk tree to arena, then one reverse pass for aggregates.

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::time::Duration;

use super::chunk::Chunk;
use crate::error::{ScanError, ScanLimit};
use crate::model::{
    Category, DirInfo, NO_DIR, NO_PARENT, Node, NodeKind, ScanWarning, SkipReason, Totals, Tree,
};
use crate::ports::EntryMeta;

/// Everything the assembler needs besides the chunk tree.
pub(crate) struct Assembly<'a> {
    pub root_path: &'a Path,
    pub root_meta: EntryMeta,
    pub errors: u64,
    pub scanned_at: i64,
    pub elapsed: Duration,
    /// Number of entries, directories and name bytes the walk collected. They size the arena exactly, so
    /// no vector grows by doubling (which keeps the old and the new buffer alive at once).
    pub expected_entries: usize,
    pub expected_dirs: usize,
    pub expected_name_bytes: u64,
    /// The entry limit in force, for the error when the arena overflows.
    pub entry_limit: u64,
    /// The name-byte limit in force, for the error when the name arena overflows.
    pub name_limit: u64,
    pub warnings: Vec<ScanWarning>,
}

struct Pending {
    id: u32,
    chunk: Chunk,
    /// Category imposed by a well-known ancestor directory, if any.
    inherited: Option<Category>,
}

pub(crate) fn assemble(root: Chunk, input: Assembly<'_>) -> Result<Tree, ScanError> {
    let mut tree = flatten(root, &input)?;
    tree.totals = aggregate(&mut tree.nodes, &mut tree.dirs);
    tree.totals.errors = input.errors;
    tree.warnings = input.warnings;
    Ok(tree)
}

/// Breadth-first copy of the chunks into the arena. Single-threaded and ordered, so ids, hardlink
/// attribution and everything derived from them are deterministic.
fn flatten(root: Chunk, input: &Assembly<'_>) -> Result<Tree, ScanError> {
    let root_name = root_name(input.root_path);
    let root_category = Category::of_directory(None, root_name.as_bytes(), &mut || false);

    // Exact sizes: the memory is reserved but not touched, so it only becomes resident as it is filled,
    // while the chunks it is filled from are freed one by one (see below).
    let name_bytes = usize::try_from(input.expected_name_bytes).unwrap_or(0) + root_name.len();
    let mut names = Vec::with_capacity(name_bytes);
    let mut nodes = Vec::with_capacity(input.expected_entries.saturating_add(1));
    let mut dirs = Vec::with_capacity(input.expected_dirs.saturating_add(1));
    let mut seen_links: HashSet<(u64, u64)> = HashSet::new();
    let mut unreadable: Vec<u32> = Vec::new();
    let mut skipped: Vec<(u32, SkipReason)> = Vec::new();
    let mut shared: Vec<u32> = Vec::new();

    names.extend_from_slice(root_name.as_bytes());
    nodes.push(Node {
        parent: NO_PARENT,
        name_off: 0,
        dir: 0,
        name_len: root_name.len().min(usize::from(u16::MAX)) as u16,
        kind: NodeKind::Dir,
        category: root_category.unwrap_or(Category::Other),
        allocated: input.root_meta.allocated,
        apparent: 0,
        mtime: input.root_meta.mtime,
    });
    dirs.push(DirInfo::default());

    let mut queue = VecDeque::new();
    queue.push_back(Pending {
        id: 0,
        chunk: root,
        inherited: root_category,
    });

    // Each chunk is popped, copied and dropped before the next one is looked at, so its names and
    // entries are freed as soon as they are in the arena; only the chunks still waiting in the queue
    // stay alive.
    while let Some(Pending {
        id,
        mut chunk,
        inherited,
    }) = queue.pop_front()
    {
        if chunk.unreadable {
            // Directories are copied in id order, so the list stays sorted.
            unreadable.push(id);
        }

        let parent_name = {
            let parent: &Node = &nodes[id as usize];
            let start = parent.name_off as usize;
            start..start + parent.name_len as usize
        };
        // Whether the directory being copied is a software project; worked out at most once, and only
        // when a name rule that depends on it comes up.
        let mut project_like: Option<bool> = None;

        let first_child = nodes.len();
        if first_child + chunk.entries.len() >= u32::MAX as usize {
            return Err(too_many_entries(input));
        }

        let mut subdirs = std::mem::take(&mut chunk.subdirs).into_iter();
        let mut links = chunk.links.iter().peekable();
        let mut skips = chunk.skips.iter().flat_map(|skips| skips.iter()).peekable();

        for (index, entry) in chunk.entries.iter().enumerate() {
            let name = &chunk.names[entry.name_range()];
            let name_off = u32::try_from(names.len()).map_err(|_| too_many_names(input))?;
            names.extend_from_slice(name);

            let node_id = nodes.len() as u32;

            let (mut allocated, mut apparent) = (entry.allocated, entry.apparent);
            if links.peek().is_some_and(|link| link.entry as usize == index) {
                let link = links.next().expect("peeked");
                shared.push(node_id);

                if !seen_links.insert((link.dev, link.ino)) {
                    allocated = 0;
                    apparent = 0;
                }
            }

            let (dir, category) = match entry.kind {
                NodeKind::Dir => {
                    let imposed = inherited.or_else(|| {
                        Category::of_directory(Some(&names[parent_name.clone()]), name, &mut || {
                            *project_like.get_or_insert_with(|| has_project_marker(&chunk))
                        })
                    });
                    dirs.push(DirInfo::default());

                    if entry.descended {
                        let sub = subdirs.next().unwrap_or_default();
                        queue.push_back(Pending {
                            id: node_id,
                            chunk: sub,
                            inherited: imposed,
                        });
                    }
                    ((dirs.len() - 1) as u32, imposed.unwrap_or(Category::Other))
                }
                NodeKind::Mount => {
                    if let Some(&&(_, reason)) = skips.peek().filter(|(entry, _)| *entry as usize == index) {
                        skipped.push((node_id, reason));
                        skips.next();
                    }
                    (NO_DIR, Category::Other)
                }
                _ => (NO_DIR, inherited.unwrap_or(entry.category)),
            };

            nodes.push(Node {
                parent: id,
                name_off,
                dir,
                name_len: entry.name_len,
                kind: entry.kind,
                category,
                allocated,
                apparent,
                mtime: entry.mtime,
            });
        }

        let dir = nodes[id as usize].dir as usize;
        dirs[dir] = DirInfo {
            first_child: first_child as u32,
            child_count: chunk.entries.len() as u32,
            files: 0,
        };
    }

    skipped.shrink_to_fit();
    shared.shrink_to_fit();

    Ok(Tree {
        names,
        nodes,
        dirs,
        root_path: input.root_path.to_path_buf(),
        scanned_at: input.scanned_at,
        elapsed: input.elapsed,
        totals: Totals::default(),
        warnings: Vec::new(),
        unreadable,
        skipped,
        shared,
    })
}

fn too_many_entries(input: &Assembly<'_>) -> ScanError {
    ScanError::TooLarge {
        limit: ScanLimit::Entries(input.entry_limit),
    }
}

fn too_many_names(input: &Assembly<'_>) -> ScanError {
    ScanError::TooLarge {
        limit: ScanLimit::Names(input.name_limit),
    }
}

/// Whether the listing in `chunk` contains a name that marks a software project.
fn has_project_marker(chunk: &Chunk) -> bool {
    chunk
        .entries
        .iter()
        .any(|entry| Category::is_project_marker(&chunk.names[entry.name_range()]))
}

/// Bytes the root's display name takes in the name arena.
pub(crate) fn root_name_len(path: &Path) -> u64 {
    root_name(path).len() as u64
}

/// Display name of the root: its last path component, or the whole path (`/`, `C:\`).
fn root_name(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

/// One reverse pass: children have larger ids than their parent, so when node `i` is reached its own
/// aggregates are final and can be added to its parent.
///
/// A directory's category is the one holding most of the allocated bytes of its files (ties go to the
/// lower category index); a directory without bytes keeps the category its name imposes, else `Other`.
fn aggregate(nodes: &mut [Node], dirs: &mut [DirInfo]) -> Totals {
    let mut by_category = vec![[0u64; Category::COUNT]; dirs.len()];
    let mut totals = Totals::default();

    for i in (0..nodes.len()).rev() {
        let node = nodes[i];

        let files = match node.kind {
            NodeKind::Dir => {
                if let Some(heaviest) = Category::heaviest(&by_category[node.dir as usize]) {
                    nodes[i].category = heaviest;
                }
                dirs[node.dir as usize].files
            }
            kind if kind.is_file_like() => 1,
            _ => 0,
        };

        if i == 0 {
            totals.allocated = node.allocated;
            totals.apparent = node.apparent;
            totals.files = u64::from(files);
            break;
        }
        if matches!(node.kind, NodeKind::Dir | NodeKind::Mount) {
            totals.dirs = totals.dirs.saturating_add(1);
        }

        let parent = node.parent as usize;
        let parent_dir = nodes[parent].dir as usize;

        if node.kind == NodeKind::Dir {
            let child_sums = by_category[node.dir as usize];
            for (total, add) in by_category[parent_dir].iter_mut().zip(child_sums) {
                *total = total.saturating_add(add);
            }
        } else if node.kind.is_file_like() {
            let slot = &mut by_category[parent_dir][node.category.index()];
            *slot = slot.saturating_add(node.allocated);
        }

        dirs[parent_dir].files = dirs[parent_dir].files.saturating_add(files);
        let parent_node = &mut nodes[parent];
        parent_node.allocated = parent_node.allocated.saturating_add(node.allocated);
        parent_node.apparent = parent_node.apparent.saturating_add(node.apparent);
        parent_node.mtime = parent_node.mtime.max(node.mtime);
    }

    totals
}
