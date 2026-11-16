use std::borrow::Cow;
use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use super::{Category, NodeId, NodeKind, ScanWarning, SkipReason, Totals};

pub(crate) const NO_PARENT: u32 = u32::MAX;
pub(crate) const NO_DIR: u32 = u32::MAX;

/// Fixed-size record of one entry. Directory-only data lives in [`DirInfo`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Node {
    pub parent: u32,
    pub name_off: u32,
    /// Index into `Tree::dirs`, or `NO_DIR` for non-directories.
    pub dir: u32,
    pub name_len: u16,
    pub kind: NodeKind,
    pub category: Category,
    /// For directories: the whole subtree, including the directory's own blocks.
    pub allocated: u64,
    pub apparent: u64,
    /// For directories: the newest mtime in the subtree.
    pub mtime: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DirInfo {
    pub first_child: u32,
    pub child_count: u32,
    pub files: u32,
}

/// The finished result of a scan: an immutable arena of nodes.
///
/// Node ids are assigned breadth-first, so a parent always precedes its children and the children of
/// one directory are contiguous (and sorted by name). Names live in one byte buffer and are raw bytes;
/// they become lossy UTF-8 only when a DTO or path is built.
#[derive(Debug)]
pub struct Tree {
    pub(crate) names: Vec<u8>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) dirs: Vec<DirInfo>,
    pub(crate) root_path: PathBuf,
    pub(crate) scanned_at: i64,
    pub(crate) elapsed: Duration,
    pub(crate) totals: Totals,
    pub(crate) warnings: Vec<ScanWarning>,
    /// Ids of directories that could not be read, ascending. Kept apart from the nodes so a flag costs
    /// nothing per entry: a scan has few of them.
    pub(crate) unreadable: Vec<u32>,
    /// Why mount nodes were not entered, as `(id, reason)` ascending by id. Only reasons other than
    /// [`SkipReason::Unknown`] are listed; a mount node without an entry is unknown. Kept apart from the
    /// nodes like `unreadable`: a scan has few mounts.
    pub(crate) skipped: Vec<(u32, SkipReason)>,
    /// Ids of the file-like nodes with more than one hard link, ascending. The tree charges the bytes of
    /// such a file to the first link met, so this is what tells how far a total can be trusted for "what
    /// would be freed". Costs 4 bytes per hardlinked file and nothing for the other entries.
    pub(crate) shared: Vec<u32>,
}

impl Tree {
    pub fn root(&self) -> NodeId {
        NodeId::ROOT
    }

    /// Number of nodes, root included.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn contains(&self, id: NodeId) -> bool {
        id.index() < self.nodes.len()
    }

    pub fn totals(&self) -> Totals {
        self.totals
    }

    /// Problems that made the scan less complete than it looks (each is also counted in
    /// `totals().errors`). Empty for a clean scan.
    pub fn warnings(&self) -> &[ScanWarning] {
        &self.warnings
    }

    /// Path of the scan root as given to the scanner.
    pub fn root_path(&self) -> &std::path::Path {
        &self.root_path
    }

    /// Unix seconds at which the scan started; reference point for age buckets.
    pub fn scanned_at(&self) -> i64 {
        self.scanned_at
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Approximate heap bytes held by the tree.
    pub fn memory_bytes(&self) -> usize {
        self.names.capacity()
            + self.nodes.capacity() * size_of::<Node>()
            + self.dirs.capacity() * size_of::<DirInfo>()
            + self.unreadable.capacity() * size_of::<u32>()
            + self.skipped.capacity() * size_of::<(u32, SkipReason)>()
            + self.shared.capacity() * size_of::<u32>()
    }

    pub fn name(&self, id: NodeId) -> &[u8] {
        let node = self.node(id);
        let start = node.name_off as usize;
        &self.names[start..start + node.name_len as usize]
    }

    pub fn name_lossy(&self, id: NodeId) -> Cow<'_, str> {
        String::from_utf8_lossy(self.name(id))
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        match self.node(id).parent {
            NO_PARENT => None,
            parent => Some(NodeId(parent)),
        }
    }

    pub fn kind(&self, id: NodeId) -> NodeKind {
        self.node(id).kind
    }

    pub fn category(&self, id: NodeId) -> Category {
        self.node(id).category
    }

    /// Whether the scan tried to read this directory and failed (listing refused, a loop, or too deep):
    /// its size is unknown, not zero. Always false for anything but a directory.
    pub fn is_unreadable(&self, id: NodeId) -> bool {
        self.unreadable.binary_search(&id.0).is_ok()
    }

    /// Why a mount node was not entered; `None` for everything that is not a `mount` node.
    pub fn skip_reason(&self, id: NodeId) -> Option<SkipReason> {
        if self.node(id).kind != NodeKind::Mount {
            return None;
        }

        let reason = match self.skipped.binary_search_by_key(&id.0, |&(node, _)| node) {
            Ok(index) => self.skipped[index].1,
            Err(_) => SkipReason::Unknown,
        };
        Some(reason)
    }

    /// How many files in the subtree (the file itself for a file-like node) have more than one hard link.
    /// Their bytes are charged to whichever link the scan met first, wherever that is.
    pub fn shared_files(&self, id: NodeId) -> u64 {
        if self.shared.is_empty() {
            return 0;
        }

        let node = self.node(id);
        if node.kind != NodeKind::Dir {
            return self.shared_in(id.0..id.0 + 1);
        }

        // Ids are breadth-first, so the descendants at one depth of a node are one run of ids, and so
        // are the children of that run: walk down one run at a time.
        let mut level = id.0..id.0 + 1;
        let mut count = 0;
        loop {
            count += self.shared_in(level.clone());

            match self.children_run(level) {
                Some(run) => level = run,
                None => return count,
            }
        }
    }

    /// Whether some directory inside the subtree (or the node itself) could not be listed. The sizes,
    /// counts and age of such a node only cover the part that was read.
    pub fn has_unreadable(&self, id: NodeId) -> bool {
        if self.unreadable.is_empty() || self.node(id).kind != NodeKind::Dir {
            return false;
        }

        // Same walk as `shared_files`: one run of ids per depth.
        let mut level = id.0..id.0 + 1;
        loop {
            let from = self.unreadable.partition_point(|&dir| dir < level.start);
            if self.unreadable.get(from).is_some_and(|&dir| dir < level.end) {
                return true;
            }

            match self.children_run(level) {
                Some(run) => level = run,
                None => return false,
            }
        }
    }

    /// The run of ids holding every child of the directories in `level`.
    fn children_run(&self, level: Range<u32>) -> Option<Range<u32>> {
        let mut next: Option<Range<u32>> = None;
        for index in level {
            let node = &self.nodes[index as usize];
            if node.kind != NodeKind::Dir {
                continue;
            }

            let dir = &self.dirs[node.dir as usize];
            if dir.child_count == 0 {
                continue;
            }

            let children = dir.first_child..dir.first_child + dir.child_count;
            next = Some(match next {
                Some(run) => run.start.min(children.start)..run.end.max(children.end),
                None => children,
            });
        }

        next
    }

    /// Hard-linked files among the ids in `ids`.
    fn shared_in(&self, ids: Range<u32>) -> u64 {
        let below = self.shared.partition_point(|&shared| shared < ids.start);
        let to = self.shared.partition_point(|&shared| shared < ids.end);

        (to - below) as u64
    }

    pub fn allocated(&self, id: NodeId) -> u64 {
        self.node(id).allocated
    }

    pub fn apparent(&self, id: NodeId) -> u64 {
        self.node(id).apparent
    }

    pub fn mtime(&self, id: NodeId) -> i64 {
        self.node(id).mtime
    }

    /// Files in the subtree; 1 for a file-like node, 0 for mounts.
    pub fn files(&self, id: NodeId) -> u64 {
        let node = self.node(id);

        match node.kind {
            NodeKind::Dir => u64::from(self.dirs[node.dir as usize].files),
            kind if kind.is_file_like() => 1,
            _ => 0,
        }
    }

    pub fn child_count(&self, id: NodeId) -> usize {
        self.dir_info(id).map_or(0, |dir| dir.child_count as usize)
    }

    pub fn has_children(&self, id: NodeId) -> bool {
        self.child_count(id) > 0
    }

    /// Children of a node, in name order. Empty for non-directories.
    pub fn children(&self, id: NodeId) -> impl ExactSizeIterator<Item = NodeId> + Clone + use<> {
        self.child_range(id).map(NodeId)
    }

    pub(crate) fn child_range(&self, id: NodeId) -> Range<u32> {
        match self.dir_info(id) {
            Some(dir) => dir.first_child..dir.first_child + dir.child_count,
            None => 0..0,
        }
    }

    /// Filesystem path of a node: the scan root joined with the names below it (lossy UTF-8).
    pub fn path_of(&self, id: NodeId) -> PathBuf {
        let mut path = self.root_path.clone();

        for segment in self.ancestry(id).into_iter().skip(1) {
            path.push(&*self.name_lossy(segment));
        }
        path
    }

    /// Exact filesystem path of a node as bytes: the scan root followed by the names below it, joined by
    /// the platform path separator (no extra separator after a root that already ends in one).
    ///
    /// Unlike [`Tree::path_of`] nothing is converted lossily. Names and the root are the
    /// `OsStr::as_encoded_bytes` form, which on Unix is exactly the bytes the kernel uses, so there the
    /// result round-trips to the original path even when a name is not valid UTF-8. On other platforms
    /// the encoding is an implementation detail of the standard library; the bytes are only meaningful to
    /// the same build, and an adapter without a lossless way back has to fall back to [`Tree::path_of`].
    pub fn path_bytes(&self, id: NodeId) -> Vec<u8> {
        let separator = std::path::MAIN_SEPARATOR as u8;
        let mut path = self.root_path.as_os_str().as_encoded_bytes().to_vec();

        for segment in self.ancestry(id).into_iter().skip(1) {
            if !path.is_empty() && path.last() != Some(&separator) {
                path.push(separator);
            }
            path.extend_from_slice(self.name(segment));
        }
        path
    }

    /// Ids from the root down to `id`, both included.
    pub fn ancestry(&self, id: NodeId) -> Vec<NodeId> {
        let mut chain = vec![id];
        let mut current = id;

        while let Some(parent) = self.parent(current) {
            chain.push(parent);
            current = parent;
        }
        chain.reverse();
        chain
    }

    pub(crate) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    fn dir_info(&self, id: NodeId) -> Option<&DirInfo> {
        match self.node(id).dir {
            NO_DIR => None,
            dir => Some(&self.dirs[dir as usize]),
        }
    }
}
