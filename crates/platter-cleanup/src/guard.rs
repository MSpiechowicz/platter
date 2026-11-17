//! The guard: the only code that can make a [`VerifiedTarget`].

use std::fmt;
use std::io;

use platter_core::dto::Verdict;
use platter_core::{BirthTime, EntryKind, EntryProbe, NodeId, NodeKind, Tree};

use crate::kept::KeptFolders;
use crate::path::{PathStyle, display};
use crate::protect::Protected;

/// Why a matched node is not in the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No home directory is known, so personal folders cannot be protected.
    HomeUnknown,
    /// The rule labels it `keep`.
    Kept,
    /// A folder the user keeps could not be looked up in time (a link whose destination could not be
    /// followed, or the limit ran out before it was reached), so what is kept is not known and nothing
    /// is planned.
    KeptUnresolved,
    /// Platter does not clean what this rule finds.
    NotCleaned,
    /// It is the folder the scan started from.
    ScanRoot,
    /// It is not below the scan root.
    OutsideScan,
    /// It is, holds or lies inside a protected place (the path given).
    Protected(String),
    /// It is a symlink.
    Symlink,
    /// It is a mount, or a mount lies between the scan root and it.
    Mount,
    /// A folder inside it was left out of the scan by the user, and it was not looked at.
    Excluded,
    /// It, or something inside it, could not be read during the scan.
    Unreadable,
    /// Neither a file nor a folder (a socket, device or similar).
    NotFileOrFolder,
    /// It is gone, or no longer what the scan saw (the error or the difference).
    Changed(String),
    /// It lies inside another target of the plan.
    Nested,
    /// Its path cannot be read back exactly.
    BadPath,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::HomeUnknown => write!(f, "the home folder is not known, so nothing is cleaned"),
            Refusal::Kept => write!(f, "kept on purpose"),
            Refusal::KeptUnresolved => write!(
                f,
                "a folder you keep could not be looked up in time (a disk may not be answering), so nothing is planned. Try again."
            ),
            Refusal::NotCleaned => write!(f, "Platter does not clean this"),
            Refusal::ScanRoot => write!(f, "it is the scanned folder itself"),
            Refusal::OutsideScan => write!(f, "it is outside the scanned folder"),
            Refusal::Protected(place) => write!(f, "protected: {place}"),
            Refusal::Symlink => write!(f, "it is a link"),
            Refusal::Mount => write!(f, "it is, or lies below, another mounted filesystem"),
            Refusal::Excluded => write!(f, "it holds a folder you left out of scans"),
            Refusal::Unreadable => write!(f, "part of it could not be read"),
            Refusal::NotFileOrFolder => write!(f, "neither a file nor a folder"),
            Refusal::Changed(what) => write!(f, "changed since the scan: {what}"),
            Refusal::Nested => write!(f, "inside another item of the plan"),
            Refusal::BadPath => write!(f, "its path cannot be read back exactly"),
        }
    }
}

/// A target that passed every check of the guard when the plan was made. Its fields can be read, but
/// only the guard can make one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedTarget {
    node: NodeId,
    path: Vec<u8>,
    is_dir: bool,
    dev: u64,
    ino: u64,
    mount_id: Option<u64>,
    /// When the entry was created, as the plan saw it (when the filesystem records it).
    birth: Option<BirthTime>,
    /// Size and newest modification from the scan.
    allocated: u64,
    apparent: u64,
    mtime: i64,
    /// Files that made the rule match a folder (`Cargo.toml` next to `target`): every group must still
    /// hold one regular file when the target is acted on. Each inner list is a choice of paths.
    markers: Vec<Vec<Vec<u8>>>,
}

impl VerifiedTarget {
    /// A target that never saw a guard, for the tests of what acts on targets.
    #[cfg(test)]
    pub(crate) fn for_test(path: &[u8], is_dir: bool, identity: (u64, u64)) -> Self {
        Self {
            node: NodeId(0),
            path: path.to_vec(),
            is_dir,
            dev: identity.0,
            ino: identity.1,
            mount_id: None,
            birth: None,
            allocated: 1000,
            apparent: 1000,
            mtime: 0,
            markers: Vec::new(),
        }
    }

    pub fn node(&self) -> NodeId {
        self.node
    }
    /// The exact path, as bytes.
    pub fn path(&self) -> &[u8] {
        &self.path
    }
    /// The path for people to read.
    pub fn display_path(&self) -> String {
        display(&self.path)
    }
    pub fn is_dir(&self) -> bool {
        self.is_dir
    }
    /// Device and inode seen at plan time; acting code compares them again right before it acts.
    pub fn identity(&self) -> (u64, u64) {
        (self.dev, self.ino)
    }
    pub fn mount_id(&self) -> Option<u64> {
        self.mount_id
    }
    pub fn birth(&self) -> Option<BirthTime> {
        self.birth
    }
    /// Bytes the scan counted for it. What deleting frees can differ (hard links, shared blocks).
    pub fn allocated(&self) -> u64 {
        self.allocated
    }
    pub fn mtime(&self) -> i64 {
        self.mtime
    }
    /// File length from the scan (0 for a hard link whose bytes the scan charged elsewhere).
    pub fn apparent(&self) -> u64 {
        self.apparent
    }
    pub fn markers(&self) -> &[Vec<Vec<u8>>] {
        &self.markers
    }
}

/// What the guard needs to know about the scan and the machine.
pub(crate) struct Guard<'a, P: EntryProbe> {
    /// The syntax of the machine's paths, from its environment.
    pub style: PathStyle,
    pub tree: &'a Tree,
    pub root: Vec<Vec<u8>>,
    pub protected: &'a Protected,
    /// The folders the user keeps, by path and identity.
    pub kept: &'a KeptFolders,
    pub probe: &'a P,
}

impl<P: EntryProbe> Guard<'_, P> {
    /// Checks one matched node. The order is cheapest first; the disk is looked at last. `by_tool` says
    /// that the owning tool cleans it, not Platter: then a part Platter could not read does not matter
    /// (the tool runs with the rights it needs, and Platter removes nothing there itself).
    pub fn verify(
        &self,
        node: NodeId,
        verdict: Verdict,
        by_tool: bool,
        markers: &[&[&str]],
    ) -> Result<VerifiedTarget, Refusal> {
        let tree = self.tree;

        if verdict == Verdict::Keep {
            return Err(Refusal::Kept);
        }
        if node == tree.root() || tree.parent(node).is_none() {
            return Err(Refusal::ScanRoot);
        }

        let is_dir = match tree.kind(node) {
            NodeKind::Dir => true,
            NodeKind::File => false,
            NodeKind::Symlink => return Err(Refusal::Symlink),
            NodeKind::Mount => return Err(Refusal::Mount),
            NodeKind::Other | NodeKind::Collapsed => return Err(Refusal::NotFileOrFolder),
        };
        if !by_tool && (tree.is_unreadable(node) || tree.has_unreadable(node)) {
            return Err(Refusal::Unreadable);
        }
        // A folder the user left out was not read, so it is not in the size this target was judged by.
        // Removing the target would remove it too, and so would a tool that cleans it (a journal vacuum, a
        // package cache clean): the user said never, so a tool target is refused as well.
        if is_dir && tree.holds_excluded(node) {
            return Err(Refusal::Excluded);
        }

        let path = tree.path_bytes(node);
        let style = self.style;
        let parts = style.components(&path).ok_or(Refusal::BadPath)?;
        if parts.len() <= self.root.len() || !style.is_within(&parts, &self.root) {
            return Err(Refusal::OutsideScan);
        }
        if let Some(place) = self.protected.hit_for(&parts, by_tool) {
            return Err(Refusal::Protected(place));
        }

        // A folder the user keeps, by path: the target is it, holds it or lies in it. The scan's own
        // view is not trusted for this, because a kept folder it did not enter (a skipped mount) is not
        // a folder node, and one below such a mount is not in the tree at all.
        if self.kept.touches(&parts) {
            return Err(Refusal::Kept);
        }

        let found = self.probe.probe(&path).map_err(|error| changed(&error))?;
        if found.len() != parts.len() {
            return Err(Refusal::Changed("the path leads somewhere else".to_owned()));
        }

        // The same protected places by identity: another path to them (a bind mount) does not count.
        let way: Vec<(u64, u64)> = found
            .iter()
            .map(|entry| (entry.meta.dev, entry.meta.ino))
            .collect();
        let tool_folder = by_tool && self.protected.is_tool_folder(&parts);
        if let Some(place) = self.protected.hit_by_identity(&way, tool_folder) {
            return Err(Refusal::Protected(place));
        }

        // A folder the user keeps, reached by another path (a bind mount): the rules matched it by its
        // path, so only the identity tells. Nothing at or below it may be a target.
        if way.iter().any(|identity| self.kept.has_identity(identity)) {
            return Err(Refusal::Kept);
        }

        // Below the scan root nothing may start a mount: not the target, not a folder on the way.
        if found[self.root.len()..]
            .iter()
            .any(|entry| entry.meta.is_mount_root)
        {
            return Err(Refusal::Mount);
        }

        let now = found[parts.len() - 1];
        match (now.meta.kind, is_dir) {
            (EntryKind::Dir, true) | (EntryKind::File, false) => {}
            (EntryKind::Symlink, _) => return Err(Refusal::Symlink),
            (EntryKind::Dir, false) => return Err(Refusal::Changed("it is now a folder".to_owned())),
            (EntryKind::File, true) => return Err(Refusal::Changed("it is now a file".to_owned())),
            (EntryKind::Other, _) => return Err(Refusal::NotFileOrFolder),
        }

        // A file is acted on only as the scan saw it: the rule judged that size and age.
        if !is_dir && !same_file(&now.meta, tree.mtime(node), tree.apparent(node)) {
            return Err(Refusal::Changed("it was modified after the scan".to_owned()));
        }

        let parent = style.parent(&path);
        let markers: Vec<Vec<Vec<u8>>> = markers
            .iter()
            .map(|choices| {
                choices
                    .iter()
                    .map(|name| style.child(parent, name.as_bytes()))
                    .collect()
            })
            .collect();
        markers_present(self.probe, style, &markers).map_err(Refusal::Changed)?;

        Ok(VerifiedTarget {
            node,
            path,
            is_dir,
            dev: now.meta.dev,
            ino: now.meta.ino,
            mount_id: now.mount_id,
            birth: now.birth,
            allocated: tree.allocated(node),
            apparent: tree.apparent(node),
            mtime: tree.mtime(node),
            markers,
        })
    }
}

/// Whether a file now (`meta`) is the file the scan saw: same modification time, and same length when
/// the scan recorded one (a second hard link carries none).
pub(crate) fn same_file(meta: &platter_core::EntryMeta, mtime: i64, apparent: u64) -> bool {
    meta.mtime == mtime && (apparent == 0 || meta.apparent == apparent)
}

/// Every group of `markers` still holds a regular file; otherwise what is missing.
pub(crate) fn markers_present(
    probe: &impl EntryProbe,
    style: PathStyle,
    markers: &[Vec<Vec<u8>>],
) -> Result<(), String> {
    for choices in markers {
        let present = choices.iter().any(|marker| {
            probe
                .probe(marker)
                .ok()
                .and_then(|found| found.last().copied())
                .is_some_and(|entry| entry.meta.kind == EntryKind::File)
        });
        if !present {
            let names: Vec<String> = choices
                .iter()
                .map(|marker| String::from_utf8_lossy(style.file_name(marker)).into_owned())
                .collect();
            return Err(format!(
                "the project file next to it is gone ({})",
                names.join(" or ")
            ));
        }
    }
    Ok(())
}

fn changed(error: &io::Error) -> Refusal {
    let what = match error.kind() {
        io::ErrorKind::NotFound => "it is gone".to_owned(),
        io::ErrorKind::NotADirectory => "a folder on the way is now a link or a file".to_owned(),
        _ => error.to_string(),
    };

    Refusal::Changed(what)
}
