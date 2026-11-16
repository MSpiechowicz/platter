use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::budget::Lease;
use crate::model::{EntryKind, ScanWarning, SkipReason};
use crate::ports::{DirListing, DirSource, EntryMeta, RawEntry, RootInfo, SourceOptions};

#[derive(Debug, Clone)]
struct FakeNode {
    name: String,
    parent: usize,
    meta: EntryMeta,
    children: Vec<usize>,
    unreadable: bool,
    withhold_child: bool,
    /// A mount root the source counts as the same filesystem as the scan root.
    same_filesystem: bool,
    /// A mount root that is never entered, with the reason the source gives.
    skipped_as: Option<SkipReason>,
    /// The directory lists this many generated files (see [`FakeFs::generated_dir`]) besides its children.
    generated: Option<Generated>,
    /// The listing fails after this many entries (see [`FakeFs::fail_listing_after`]).
    fail_after: Option<usize>,
}

/// Files a directory invents while it is listed, so a huge directory costs the fake no memory.
#[derive(Debug, Clone, Copy)]
struct Generated {
    count: usize,
    /// Length of every name: the number of the file, zero-padded.
    name_len: usize,
}

type ReadHook = Box<dyn Fn(&str) + Send + Sync>;

/// An in-memory filesystem implementing [`DirSource`].
///
/// Paths are absolute and `/`-separated; missing parent directories are created. Sizes of directories
/// default to 0 so test arithmetic stays simple; use [`FakeFs::set_size`] to change that. Directories
/// read are recorded (see [`FakeFs::read_dirs`]) and a hook can observe each read, e.g. to cancel.
pub struct FakeFs {
    nodes: Vec<FakeNode>,
    next_ino: u64,
    reads: Mutex<Vec<String>>,
    hook: Option<ReadHook>,
    /// The option of the latest `open_root`; like a real source, the fake applies it in `read_dir`.
    one_file_system: AtomicBool,
    warnings: Vec<ScanWarning>,
    /// Length of the longest listing `read_dir` returned (see [`FakeFs::largest_listing`]).
    largest_listing: AtomicUsize,
}

impl Default for FakeFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeFs {
    pub fn new() -> Self {
        let root = FakeNode {
            name: String::new(),
            parent: 0,
            meta: Self::meta(EntryKind::Dir, 0, 0, 1),
            children: Vec::new(),
            unreadable: false,
            withhold_child: false,
            same_filesystem: false,
            skipped_as: None,
            generated: None,
            fail_after: None,
        };
        Self {
            nodes: vec![root],
            next_ino: 2,
            reads: Mutex::new(Vec::new()),
            hook: None,
            one_file_system: AtomicBool::new(false),
            warnings: Vec::new(),
            largest_listing: AtomicUsize::new(0),
        }
    }

    fn meta(kind: EntryKind, allocated: u64, apparent: u64, ino: u64) -> EntryMeta {
        EntryMeta {
            kind,
            allocated,
            apparent,
            mtime: 0,
            dev: 1,
            ino,
            nlink: 1,
            is_mount_root: false,
        }
    }

    /// Adds a directory (and any missing parents).
    pub fn dir(&mut self, path: &str) -> &mut Self {
        self.insert(path, EntryKind::Dir, 0, 0);
        self
    }

    /// Adds a file whose allocated and apparent size are both `size`.
    pub fn file(&mut self, path: &str, size: u64) -> &mut Self {
        self.file_sized(path, size, size)
    }

    /// Adds a file with distinct allocated and apparent sizes (a sparse or compressed file).
    pub fn file_sized(&mut self, path: &str, allocated: u64, apparent: u64) -> &mut Self {
        self.insert(path, EntryKind::File, allocated, apparent);
        self
    }

    /// Adds a symlink; it is a leaf with its own size and its target is never visited.
    pub fn symlink(&mut self, path: &str, size: u64) -> &mut Self {
        self.insert(path, EntryKind::Symlink, size, size);
        self
    }

    /// Adds a socket, FIFO or device node.
    pub fn special(&mut self, path: &str) -> &mut Self {
        self.insert(path, EntryKind::Other, 0, 0);
        self
    }

    /// Adds `new` as another hardlink of the file at `existing` (same dev and inode, `nlink` updated).
    pub fn hardlink(&mut self, existing: &str, new: &str) -> &mut Self {
        let source = self.resolve(existing).expect("hardlink source exists");
        let meta = self.nodes[source].meta;

        let created = self.insert(new, meta.kind, meta.allocated, meta.apparent);
        self.nodes[created].meta.dev = meta.dev;
        self.nodes[created].meta.ino = meta.ino;

        let links = self
            .nodes
            .iter()
            .filter(|n| n.meta.ino == meta.ino && n.meta.dev == meta.dev)
            .count();
        for node in self
            .nodes
            .iter_mut()
            .filter(|n| n.meta.ino == meta.ino && n.meta.dev == meta.dev)
        {
            node.meta.nlink = links as u64;
        }
        self
    }

    /// Adds the root of another mounted filesystem (a directory flagged `is_mount_root`).
    pub fn mount(&mut self, path: &str) -> &mut Self {
        let index = self.insert(path, EntryKind::Dir, 0, 0);
        self.nodes[index].meta.is_mount_root = true;
        self.nodes[index].meta.dev = 2;
        self
    }

    /// Adds a mount root that the source counts as part of the scan root's filesystem (a btrfs
    /// subvolume of the same disk): it is entered even with `one_file_system`.
    pub fn same_filesystem_mount(&mut self, path: &str) -> &mut Self {
        self.mount(path);

        let index = self.resolve(path).expect("path exists");
        self.nodes[index].same_filesystem = true;
        self
    }

    /// Adds a directory whose contents cannot be read (`PermissionDenied`).
    pub fn unreadable_dir(&mut self, path: &str) -> &mut Self {
        let index = self.insert(path, EntryKind::Dir, 0, 0);
        self.nodes[index].unreadable = true;
        self
    }

    /// Makes the mount root at `path` a mount the source never enters, for `reason` (the reader knows why;
    /// without this the fake says "another disk" under `one_file_system` and nothing otherwise).
    pub fn skip_mount(&mut self, path: &str, reason: SkipReason) -> &mut Self {
        let index = self.resolve(path).expect("path exists");
        self.nodes[index].skipped_as = Some(reason);
        self
    }

    /// Makes the directory at `path` (created if missing) list `count` generated files of size 1 whose
    /// names are `name_len` characters long, in addition to its real children. The files exist only
    /// while a listing is made, so directories of a million entries need no memory.
    pub fn generated_dir(&mut self, path: &str, count: usize, name_len: usize) -> &mut Self {
        let index = self.insert(path, EntryKind::Dir, 0, 0);
        self.nodes[index].generated = Some(Generated { count, name_len });
        self
    }

    /// Makes listing the directory at `path` fail with an I/O error after it has admitted `entries`.
    pub fn fail_listing_after(&mut self, path: &str, entries: usize) -> &mut Self {
        let index = self.resolve(path).expect("path exists");
        self.nodes[index].fail_after = Some(entries);
        self
    }

    /// Makes the source report the directory without a child handle ("do not descend").
    pub fn withhold_handle(&mut self, path: &str) -> &mut Self {
        let index = self.resolve(path).expect("path exists");
        self.nodes[index].withhold_child = true;
        self
    }

    pub fn set_size(&mut self, path: &str, allocated: u64, apparent: u64) -> &mut Self {
        let index = self.resolve(path).expect("path exists");
        self.nodes[index].meta.allocated = allocated;
        self.nodes[index].meta.apparent = apparent;
        self
    }

    /// Overrides device, inode and link count (to build look-alike inodes on different devices).
    pub fn set_identity(&mut self, path: &str, dev: u64, ino: u64, nlink: u64) -> &mut Self {
        let index = self.resolve(path).expect("path exists");
        let meta = &mut self.nodes[index].meta;
        (meta.dev, meta.ino, meta.nlink) = (dev, ino, nlink);
        self
    }

    /// Device and inode of the entry at `path`.
    pub fn identity(&self, path: &str) -> (u64, u64) {
        let meta = &self.nodes[self.resolve(path).expect("path exists")].meta;
        (meta.dev, meta.ino)
    }

    pub fn set_mtime(&mut self, path: &str, mtime: i64) -> &mut Self {
        let index = self.resolve(path).expect("path exists");
        self.nodes[index].meta.mtime = mtime;
        self
    }

    /// Makes `open_root` report `warning`, like a source that could not read its mount table.
    pub fn warn(&mut self, warning: ScanWarning) -> &mut Self {
        self.warnings.push(warning);
        self
    }

    /// Length of the longest listing returned so far. Like a frugal real source, the fake stops listing
    /// when the lease refuses room for an entry, so this shows how far a huge directory was read.
    pub fn largest_listing(&self) -> usize {
        self.largest_listing.load(Ordering::Relaxed)
    }

    /// Calls `hook(path_of_directory)` at the start of every `read_dir`.
    pub fn on_read_dir(&mut self, hook: impl Fn(&str) + Send + Sync + 'static) -> &mut Self {
        self.hook = Some(Box::new(hook));
        self
    }

    /// Paths of the directories read so far, in call order.
    pub fn read_dirs(&self) -> Vec<String> {
        self.reads.lock().expect("reads lock").clone()
    }

    fn insert(&mut self, path: &str, kind: EntryKind, allocated: u64, apparent: u64) -> usize {
        let mut current = 0;

        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        for (position, part) in parts.iter().enumerate() {
            let last = position + 1 == parts.len();
            let existing = self.nodes[current]
                .children
                .iter()
                .copied()
                .find(|&c| self.nodes[c].name == *part);

            current = match existing {
                Some(index) if !last => index,
                Some(index) => {
                    self.nodes[index].meta.kind = kind;
                    self.nodes[index].meta.allocated = allocated;
                    self.nodes[index].meta.apparent = apparent;
                    index
                }
                None => {
                    let node_kind = if last { kind } else { EntryKind::Dir };
                    let (alloc, app) = if last { (allocated, apparent) } else { (0, 0) };
                    let ino = self.next_ino;
                    self.next_ino += 1;

                    self.nodes.push(FakeNode {
                        name: (*part).to_owned(),
                        parent: current,
                        meta: Self::meta(node_kind, alloc, app, ino),
                        children: Vec::new(),
                        unreadable: false,
                        withhold_child: false,
                        same_filesystem: false,
                        skipped_as: None,
                        generated: None,
                        fail_after: None,
                    });
                    let index = self.nodes.len() - 1;
                    self.nodes[current].children.push(index);
                    index
                }
            };
        }
        current
    }

    fn resolve(&self, path: &str) -> Option<usize> {
        let mut current = 0;

        for part in path.split('/').filter(|part| !part.is_empty()) {
            current = self.nodes[current]
                .children
                .iter()
                .copied()
                .find(|&c| self.nodes[c].name == part)?;
        }
        Some(current)
    }

    fn path_of(&self, index: usize) -> String {
        let mut names = Vec::new();
        let mut current = index;
        while current != 0 {
            names.push(self.nodes[current].name.as_str());
            current = self.nodes[current].parent;
        }
        names.reverse();
        format!("/{}", names.join("/"))
    }
}

impl DirSource for FakeFs {
    type Handle = usize;

    fn open_root(&self, path: &Path, options: SourceOptions) -> io::Result<RootInfo<usize>> {
        self.one_file_system
            .store(options.one_file_system, Ordering::Relaxed);

        let text = path.to_string_lossy();
        let index = self
            .resolve(&text)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;

        Ok(RootInfo {
            handle: index,
            meta: self.nodes[index].meta,
            warnings: self.warnings.clone(),
        })
    }

    fn read_dir(&self, dir: &usize, lease: &mut Lease<'_>) -> io::Result<DirListing<usize>> {
        let node = &self.nodes[*dir];
        let path = self.path_of(*dir);

        self.reads.lock().expect("reads lock").push(path.clone());
        if let Some(hook) = &self.hook {
            hook(&path);
        }
        if node.unreadable {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }

        let one_file_system = self.one_file_system.load(Ordering::Relaxed);
        let mut entries = Vec::new();

        for &child in &node.children {
            let entry = &self.nodes[child];
            if !self.admit(
                node,
                lease,
                entry.name.len(),
                entry.meta.kind == EntryKind::Dir,
                entries.len(),
            )? {
                break;
            }

            let is_dir = entry.meta.kind == EntryKind::Dir;
            let other_filesystem = entry.meta.is_mount_root && !entry.same_filesystem;
            let refused = entry.skipped_as.is_some() || (one_file_system && other_filesystem);
            let enter = is_dir && !entry.withhold_child && !refused;

            let skipped = entry
                .skipped_as
                .or_else(|| (one_file_system && other_filesystem).then_some(SkipReason::OtherDisk));
            entries.push(RawEntry {
                name: entry.name.clone().into_bytes(),
                meta: entry.meta,
                child: enter.then_some(child),
                skipped: skipped.filter(|_| !enter),
            });
        }

        if let Some(generated) = node.generated {
            for number in 0..generated.count {
                if !self.admit(node, lease, generated.name_len, false, entries.len())? {
                    break;
                }

                entries.push(RawEntry {
                    name: format!("{number:0width$}", width = generated.name_len).into_bytes(),
                    meta: Self::meta(EntryKind::File, 1, 1, number as u64 + 1),
                    child: None,
                    skipped: None,
                });
            }
        }

        self.largest_listing.fetch_max(entries.len(), Ordering::Relaxed);
        Ok(DirListing { entries, errors: 0 })
    }
}

impl FakeFs {
    /// Asks the lease for room for the next entry of a listing that has `listed` entries so far. `Err`
    /// when the directory is set to fail at this point, `Ok(false)` when the budget says stop.
    fn admit(
        &self,
        node: &FakeNode,
        lease: &mut Lease<'_>,
        name_len: usize,
        is_dir: bool,
        listed: usize,
    ) -> io::Result<bool> {
        if node.fail_after == Some(listed) {
            return Err(io::Error::other("the listing failed midway"));
        }

        Ok(lease.admit(name_len, is_dir))
    }
}
