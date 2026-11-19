//! A binary snapshot of a finished [`Tree`]: what lets an app keep the last scan across restarts.
//!
//! Layout, all integers little-endian (compression is left to the caller):
//!
//! ```text
//! magic "PLTRTREE" | version u32
//! scanned_at i64 | elapsed secs u64, nanos u32 | totals 5 x u64
//! root identity: dev u64, ino u64, mount id (flag u8, u64)
//! counts u64: name bytes, nodes, dirs, unreadable, skipped, entered mounts, shared
//! root path (u32 length, bytes) | warnings (count u8, one byte each)
//! names | nodes (40 bytes each) | dirs (12 bytes each) | unreadable ids
//! skipped (id u32, reason u8) | entered mounts (id u32, dev, ino, mount id, own filesystem u8) | shared ids
//! end marker "PLTREND\n"
//! ```
//!
//! Reading trusts nothing: a count is only believed as far as the stream holds that many bytes (nothing
//! is allocated up front for a count that was not backed yet), and the result is checked against every
//! invariant the scanner builds a tree with, so a damaged, truncated or foreign stream is an
//! `InvalidData` or `UnexpectedEof` error and never a panic or a tree that panics later.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use super::{Category, DirInfo, NO_DIR, NO_PARENT, Node, NodeKind, ScanWarning, ScannedMount, ScannedRoot};
use super::{SkipReason, Totals, Tree};
use crate::scan::MAX_DEPTH;

const MAGIC: &[u8; 8] = b"PLTRTREE";
const END: &[u8; 8] = b"PLTREND\n";

/// The layout above. A snapshot of another version is refused, not converted.
pub const SNAPSHOT_VERSION: u32 = 1;

const NODE_BYTES: usize = 40;
const DIR_BYTES: usize = 12;
const SKIPPED_BYTES: usize = 5;
const MOUNT_BYTES: usize = 4 + 8 + 8 + 9 + 1;
const ID_BYTES: usize = 4;

/// Longest root path a snapshot may hold (a path is far shorter in practice).
const MAX_PATH_BYTES: u64 = 1 << 16;
const MAX_WARNINGS: u64 = 64;

/// Bytes or records taken from the stream per step while a count is not yet backed by data.
const CHUNK_BYTES: usize = 1 << 20;
/// Bytes collected before they are handed to the writer.
const FLUSH_AT: usize = 1 << 16;

impl Tree {
    /// Writes the tree to `w`. The writer is used as given (wrap it in a `BufWriter` if it is a file).
    pub fn write_snapshot(&self, w: &mut impl Write) -> io::Result<()> {
        let mut out = Sink::new(w);

        out.put(MAGIC)?;
        out.put(&SNAPSHOT_VERSION.to_le_bytes())?;
        out.put(&self.scanned_at.to_le_bytes())?;
        out.put(&self.elapsed.as_secs().to_le_bytes())?;
        out.put(&self.elapsed.subsec_nanos().to_le_bytes())?;
        for number in [
            self.totals.allocated,
            self.totals.apparent,
            self.totals.files,
            self.totals.dirs,
            self.totals.errors,
        ] {
            out.put(&number.to_le_bytes())?;
        }
        out.put(&self.root_identity.dev.to_le_bytes())?;
        out.put(&self.root_identity.ino.to_le_bytes())?;
        out.optional(self.root_identity.mount_id)?;

        for count in [
            self.names.len(),
            self.nodes.len(),
            self.dirs.len(),
            self.unreadable.len(),
            self.skipped.len(),
            self.entered_mounts.len(),
            self.shared.len(),
        ] {
            out.put(&(count as u64).to_le_bytes())?;
        }

        let path = path_to_bytes(&self.root_path);
        let path_len = u32::try_from(path.len()).map_err(|_| invalid("the root path is too long"))?;
        out.put(&path_len.to_le_bytes())?;
        out.put(&path)?;

        let warnings = u8::try_from(self.warnings.len()).map_err(|_| invalid("too many warnings"))?;
        out.put(&[warnings])?;
        for warning in &self.warnings {
            out.put(&[warning_byte(*warning)])?;
        }

        out.put_large(&self.names)?;
        for node in &self.nodes {
            out.put(&node.parent.to_le_bytes())?;
            out.put(&node.name_off.to_le_bytes())?;
            out.put(&node.dir.to_le_bytes())?;
            out.put(&node.name_len.to_le_bytes())?;
            out.put(&[kind_byte(node.kind), node.category.index() as u8])?;
            out.put(&node.allocated.to_le_bytes())?;
            out.put(&node.apparent.to_le_bytes())?;
            out.put(&node.mtime.to_le_bytes())?;
        }
        for dir in &self.dirs {
            out.put(&dir.first_child.to_le_bytes())?;
            out.put(&dir.child_count.to_le_bytes())?;
            out.put(&dir.files.to_le_bytes())?;
        }
        for id in &self.unreadable {
            out.put(&id.to_le_bytes())?;
        }
        for (id, reason) in &self.skipped {
            out.put(&id.to_le_bytes())?;
            out.put(&[skip_byte(*reason)])?;
        }
        for (id, mount) in &self.entered_mounts {
            out.put(&id.to_le_bytes())?;
            out.put(&mount.dev.to_le_bytes())?;
            out.put(&mount.ino.to_le_bytes())?;
            out.optional(mount.mount_id)?;
            out.put(&[u8::from(mount.own_filesystem)])?;
        }
        for id in &self.shared {
            out.put(&id.to_le_bytes())?;
        }

        out.put(END)?;
        out.flush()
    }

    /// Reads a tree written by [`Tree::write_snapshot`].
    ///
    /// Anything that is not a whole, valid snapshot of this version is an error: wrong magic or version,
    /// a stream that ends early, a number or enum byte out of range, ids or offsets that point outside
    /// the tree, children that do not belong to their parent.
    pub fn read_snapshot(r: &mut impl Read) -> io::Result<Tree> {
        Self::read_snapshot_limited(r, u64::MAX)
    }

    /// [`Tree::read_snapshot`], refusing (`OutOfMemory`) a snapshot whose tree would hold more than
    /// `max_memory_bytes` (see [`Tree::memory_bytes`]) before anything of it is read.
    pub fn read_snapshot_limited(r: &mut impl Read, max_memory_bytes: u64) -> io::Result<Tree> {
        let mut src = Source { r };

        if src.take::<8>()? != *MAGIC {
            return Err(invalid("not a Platter scan"));
        }
        if src.u32()? != SNAPSHOT_VERSION {
            return Err(invalid("a scan saved by another version of Platter"));
        }

        let scanned_at = src.i64()?;
        let secs = src.u64()?;
        let nanos = src.u32()?;
        if nanos >= 1_000_000_000 {
            return Err(invalid("the scan time is not valid"));
        }
        let totals = Totals {
            allocated: src.u64()?,
            apparent: src.u64()?,
            files: src.u64()?,
            dirs: src.u64()?,
            errors: src.u64()?,
        };
        let root_identity = ScannedRoot {
            dev: src.u64()?,
            ino: src.u64()?,
            mount_id: src.optional()?,
        };

        let names_len = src.u64()?;
        let node_count = src.u64()?;
        let dir_count = src.u64()?;
        let unreadable_count = src.u64()?;
        let skipped_count = src.u64()?;
        let mounts_count = src.u64()?;
        let shared_count = src.u64()?;
        check_counts(
            &[names_len, node_count, dir_count],
            max_memory_bytes,
            &[unreadable_count, skipped_count, mounts_count, shared_count],
        )?;

        let path_len = u64::from(src.u32()?);
        if path_len > MAX_PATH_BYTES {
            return Err(invalid("the root path is too long"));
        }
        let root_path = path_from_bytes(src.bytes(path_len)?)?;

        let warning_count = u64::from(src.u8()?);
        if warning_count > MAX_WARNINGS {
            return Err(invalid("too many warnings"));
        }
        let mut warnings = Vec::new();
        for _ in 0..warning_count {
            warnings.push(warning_from(src.u8()?)?);
        }

        let names = src.bytes(names_len)?;
        let nodes = src.records(node_count, NODE_BYTES, |mut f| {
            Ok(Node {
                parent: f.u32(),
                name_off: f.u32(),
                dir: f.u32(),
                name_len: f.u16(),
                kind: kind_from(f.u8())?,
                category: category_from(f.u8())?,
                allocated: f.u64(),
                apparent: f.u64(),
                mtime: f.i64(),
            })
        })?;
        let dirs = src.records(dir_count, DIR_BYTES, |mut f| {
            Ok(DirInfo {
                first_child: f.u32(),
                child_count: f.u32(),
                files: f.u32(),
            })
        })?;
        let unreadable = src.records(unreadable_count, ID_BYTES, |mut f| Ok(f.u32()))?;
        let skipped = src.records(skipped_count, SKIPPED_BYTES, |mut f| {
            Ok((f.u32(), skip_from(f.u8())?))
        })?;
        let entered_mounts = src.records(mounts_count, MOUNT_BYTES, |mut f| {
            let id = f.u32();
            let dev = f.u64();
            let ino = f.u64();
            let mount_id = f.optional()?;
            let own_filesystem = f.flag()?;

            Ok((
                id,
                ScannedMount {
                    dev,
                    ino,
                    mount_id,
                    own_filesystem,
                },
            ))
        })?;
        let shared = src.records(shared_count, ID_BYTES, |mut f| Ok(f.u32()))?;

        if src.take::<8>()? != *END {
            return Err(invalid("the scan does not end where it should"));
        }

        let tree = Tree {
            names,
            nodes,
            dirs,
            root_path,
            scanned_at,
            elapsed: Duration::new(secs, nanos),
            totals,
            warnings,
            unreadable,
            skipped,
            entered_mounts,
            root_identity,
            shared,
        };
        check_tree(&tree)?;

        Ok(tree)
    }
}

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// The counts of a header against the caps of the format and the memory the caller allows.
fn check_counts(sizes: &[u64; 3], max_memory_bytes: u64, lists: &[u64; 4]) -> io::Result<()> {
    let [names, nodes, dirs] = *sizes;
    let [unreadable, skipped, mounts, shared] = *lists;

    if nodes == 0 || nodes >= u64::from(u32::MAX) {
        return Err(invalid("the number of entries is not valid"));
    }
    if names > u64::from(u32::MAX) {
        return Err(invalid("the names are too long"));
    }
    if dirs == 0 || dirs > nodes {
        return Err(invalid("the number of folders is not valid"));
    }
    if [unreadable, skipped, mounts, shared]
        .iter()
        .any(|&count| count > nodes)
    {
        return Err(invalid("a list is longer than the tree"));
    }

    let memory = names
        .saturating_add(nodes.saturating_mul(size_of::<Node>() as u64))
        .saturating_add(dirs.saturating_mul(size_of::<DirInfo>() as u64))
        .saturating_add(unreadable.saturating_mul(size_of::<u32>() as u64))
        .saturating_add(skipped.saturating_mul(size_of::<(u32, SkipReason)>() as u64))
        .saturating_add(mounts.saturating_mul(size_of::<(u32, ScannedMount)>() as u64))
        .saturating_add(shared.saturating_mul(size_of::<u32>() as u64));
    if memory > max_memory_bytes {
        return Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            "the scan needs more memory than may be used for it",
        ));
    }

    Ok(())
}

/// Every invariant the scanner builds a tree with, checked in two linear passes: the entries one by one
/// (what each is and how deep and long its path is), then the layout (breadth-first, children contiguous).
///
/// What the queries and cleanup rely on is checked, not just what keeps them from panicking: a tree that
/// the scanner cannot produce (a chain deeper than it reads, names that are not names, a path of
/// megabytes, ids that are not breadth-first) is refused, as it would make a path or a walk over the
/// tree cost far more than any real tree does.
fn check_tree(tree: &Tree) -> io::Result<()> {
    let nodes = &tree.nodes;
    let len = nodes.len();

    let root = &nodes[0];
    if root.parent != NO_PARENT || root.kind != NodeKind::Dir || root.dir != 0 {
        return Err(invalid("the first entry is not the scan root"));
    }

    // The depth and the length of the path of every entry so far, each worked out from its parent's.
    // A node of the scan can be one level below the deepest folder that is read.
    let deepest = MAX_DEPTH + 1;
    let mut depths: Vec<u32> = Vec::with_capacity(len);
    let mut paths: Vec<u64> = Vec::with_capacity(len);

    // Folders are numbered in the order their nodes are, and each node of kind `dir` has its own.
    let mut next_dir = 0u32;
    for (id, node) in nodes.iter().enumerate() {
        let name_end = u64::from(node.name_off) + u64::from(node.name_len);
        if name_end > tree.names.len() as u64 {
            return Err(invalid("a name lies outside the names"));
        }

        match node.kind {
            NodeKind::Dir => {
                if node.dir != next_dir || next_dir as usize >= tree.dirs.len() {
                    return Err(invalid("a folder has the wrong folder record"));
                }
                next_dir += 1;
            }
            _ if node.dir != NO_DIR => return Err(invalid("a file has a folder record")),
            _ => {}
        }

        if id == 0 {
            depths.push(0);
            paths.push(tree.root_path_len());
            continue;
        }

        let parent = node.parent as usize;
        if parent >= id || nodes[parent].kind != NodeKind::Dir {
            return Err(invalid("an entry has no folder before it as its parent"));
        }

        let start = node.name_off as usize;
        if !is_name(&tree.names[start..start + usize::from(node.name_len)]) {
            return Err(invalid("an entry has a name that is not a file name"));
        }

        let depth = depths[parent] + 1;
        if depth > deepest {
            return Err(invalid("an entry is deeper than a scan reads"));
        }
        // The separator is counted for every name, a root that ends in one included: a bound, not the length.
        let path = paths[parent] + 1 + u64::from(node.name_len);
        if path > MAX_PATH_BYTES {
            return Err(invalid("the path of an entry is too long"));
        }
        depths.push(depth);
        paths.push(path);
    }
    if next_dir as usize != tree.dirs.len() {
        return Err(invalid("there are folder records no folder owns"));
    }

    // The layout the scanner makes: the folders in id order list their children one after the other, from
    // id 1 on, and each child names its folder. So the children of a folder are contiguous, ids are
    // breadth-first (a level is one run of ids, which is what the walks over a subtree count on), and every
    // entry but the root is listed by exactly one folder.
    let mut next_child = 1u64;
    for (id, node) in nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind == NodeKind::Dir)
    {
        let info = &tree.dirs[node.dir as usize];
        if info.child_count == 0 {
            continue;
        }

        let end = next_child + u64::from(info.child_count);
        if u64::from(info.first_child) != next_child || end > len as u64 {
            return Err(invalid(
                "the children of a folder are not where the scan puts them",
            ));
        }
        let children = next_child as usize..end as usize;
        if nodes[children].iter().any(|child| child.parent as usize != id) {
            return Err(invalid("a folder lists children that belong to another"));
        }
        next_child = end;
    }
    if next_child != len as u64 {
        return Err(invalid("an entry belongs to no folder"));
    }

    check_ids(&tree.unreadable, nodes, |kind| kind == NodeKind::Dir)?;
    check_ids(&tree.skipped, nodes, |kind| kind == NodeKind::Mount)?;
    check_ids(&tree.entered_mounts, nodes, |kind| kind == NodeKind::Dir)?;
    check_ids(&tree.shared, nodes, NodeKind::is_file_like)?;

    Ok(())
}

/// Ids in a list that the tree keeps sorted by id (strictly ascending, so binary search finds them), each
/// an entry of an accepted kind.
fn check_ids<T: HasId>(list: &[T], nodes: &[Node], accepts: impl Fn(NodeKind) -> bool) -> io::Result<()> {
    let mut previous: Option<u32> = None;

    for item in list {
        let id = item.id();
        if previous.is_some_and(|before| before >= id) {
            return Err(invalid("a list of entries is not in order"));
        }
        previous = Some(id);

        match nodes.get(id as usize) {
            Some(node) if accepts(node.kind) => {}
            _ => {
                return Err(invalid(
                    "a list names an entry of the wrong kind or outside the tree",
                ));
            }
        }
    }

    Ok(())
}

trait HasId {
    fn id(&self) -> u32;
}

impl HasId for u32 {
    fn id(&self) -> u32 {
        *self
    }
}

impl<T> HasId for (u32, T) {
    fn id(&self) -> u32 {
        self.0
    }
}

/// Whether `name` can be the name of an entry in a folder: not empty, not `.` or `..`, and holding no
/// separator (`/`, and `\` where that is one too) and no NUL.
fn is_name(name: &[u8]) -> bool {
    if name.is_empty() || name == b"." || name == b".." {
        return false;
    }

    // `\` is an ordinary character of a name on Unix.
    !name
        .iter()
        .any(|&byte| byte == b'/' || byte == 0 || (cfg!(windows) && byte == b'\\'))
}

fn kind_byte(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Dir => 0,
        NodeKind::File => 1,
        NodeKind::Symlink => 2,
        NodeKind::Mount => 3,
        NodeKind::Other => 4,
        NodeKind::Collapsed => 5,
    }
}

fn kind_from(byte: u8) -> io::Result<NodeKind> {
    match byte {
        0 => Ok(NodeKind::Dir),
        1 => Ok(NodeKind::File),
        2 => Ok(NodeKind::Symlink),
        3 => Ok(NodeKind::Mount),
        4 => Ok(NodeKind::Other),
        // The synthetic kind of a level-of-detail query is never part of a scanned tree.
        _ => Err(invalid("an entry has an unknown kind")),
    }
}

fn category_from(byte: u8) -> io::Result<Category> {
    Category::ALL
        .get(usize::from(byte))
        .copied()
        .ok_or_else(|| invalid("an entry has an unknown category"))
}

fn skip_byte(reason: SkipReason) -> u8 {
    match reason {
        SkipReason::OtherDisk => 0,
        SkipReason::System => 1,
        SkipReason::Duplicate => 2,
        SkipReason::Excluded => 3,
        SkipReason::Unknown => 4,
    }
}

fn skip_from(byte: u8) -> io::Result<SkipReason> {
    match byte {
        0 => Ok(SkipReason::OtherDisk),
        1 => Ok(SkipReason::System),
        2 => Ok(SkipReason::Duplicate),
        3 => Ok(SkipReason::Excluded),
        4 => Ok(SkipReason::Unknown),
        _ => Err(invalid("a mount has an unknown reason")),
    }
}

fn warning_byte(warning: ScanWarning) -> u8 {
    match warning {
        ScanWarning::MountTableUnavailable => 0,
        ScanWarning::ExcludedUnresolved => 1,
    }
}

fn warning_from(byte: u8) -> io::Result<ScanWarning> {
    match byte {
        0 => Ok(ScanWarning::MountTableUnavailable),
        1 => Ok(ScanWarning::ExcludedUnresolved),
        _ => Err(invalid("the scan has an unknown warning")),
    }
}

/// The exact bytes of the path where the platform has them (Unix); otherwise its text, lossy.
fn path_to_bytes(path: &std::path::Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}

fn path_from_bytes(bytes: Vec<u8>) -> io::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        let text = String::from_utf8(bytes).map_err(|_| invalid("the root path is not text"))?;

        Ok(PathBuf::from(text))
    }
}

/// Collects what is written and hands it to the writer in larger pieces.
struct Sink<'a, W: Write> {
    w: &'a mut W,
    buf: Vec<u8>,
}

impl<'a, W: Write> Sink<'a, W> {
    fn new(w: &'a mut W) -> Self {
        Self {
            w,
            buf: Vec::with_capacity(FLUSH_AT + NODE_BYTES),
        }
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.buf.extend_from_slice(bytes);

        if self.buf.len() >= FLUSH_AT {
            self.drain()?;
        }

        Ok(())
    }

    /// A piece too large to copy into the buffer.
    fn put_large(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.drain()?;

        self.w.write_all(bytes)
    }

    fn optional(&mut self, value: Option<u64>) -> io::Result<()> {
        self.put(&[u8::from(value.is_some())])?;

        self.put(&value.unwrap_or(0).to_le_bytes())
    }

    fn drain(&mut self) -> io::Result<()> {
        self.w.write_all(&self.buf)?;
        self.buf.clear();

        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain()?;

        self.w.flush()
    }
}

/// Reads the fields of the stream, never trusting a length further than the data that backs it.
struct Source<'a, R: Read> {
    r: &'a mut R,
}

impl<R: Read> Source<'_, R> {
    fn take<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut bytes = [0u8; N];
        self.r.read_exact(&mut bytes)?;

        Ok(bytes)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn i64(&mut self) -> io::Result<i64> {
        Ok(i64::from_le_bytes(self.take()?))
    }

    fn optional(&mut self) -> io::Result<Option<u64>> {
        let present = match self.u8()? {
            0 => false,
            1 => true,
            _ => return Err(invalid("a flag is neither 0 nor 1")),
        };
        let value = self.u64()?;

        Ok(present.then_some(value))
    }

    /// `len` bytes, read a chunk at a time so that a length the stream cannot back fails at the end of
    /// the stream, not in the allocator.
    fn bytes(&mut self, len: u64) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut left = len;

        while left > 0 {
            let step = left.min(CHUNK_BYTES as u64) as usize;
            let before = out.len();
            reserve(&mut out, step)?;
            out.resize(before + step, 0);
            self.r.read_exact(&mut out[before..])?;
            left -= step as u64;
        }
        out.shrink_to_fit();

        Ok(out)
    }

    /// `count` records of `size` bytes each, decoded one chunk at a time.
    fn records<T>(
        &mut self,
        count: u64,
        size: usize,
        decode: impl Fn(Fields<'_>) -> io::Result<T>,
    ) -> io::Result<Vec<T>> {
        let per_chunk = (CHUNK_BYTES / size).max(1) as u64;
        let mut out = Vec::new();
        let mut buf = Vec::new();
        let mut left = count;

        while left > 0 {
            let step = left.min(per_chunk) as usize;
            buf.resize(step * size, 0);
            self.r.read_exact(&mut buf)?;

            reserve(&mut out, step)?;
            for record in buf.chunks_exact(size) {
                out.push(decode(Fields(record))?);
            }
            left -= step as u64;
        }
        out.shrink_to_fit();

        Ok(out)
    }
}

fn reserve<T>(vec: &mut Vec<T>, extra: usize) -> io::Result<()> {
    vec.try_reserve(extra)
        .map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "not enough memory to read the scan"))
}

/// The fields of one record, in the order they were written. A record is as long as its fields (the
/// reader cuts them to that length), so a field is never short.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        match self.0.split_first_chunk::<N>() {
            Some((head, rest)) => {
                self.0 = rest;
                *head
            }
            None => [0; N],
        }
    }

    fn u8(&mut self) -> u8 {
        self.take::<1>()[0]
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take())
    }

    fn i64(&mut self) -> i64 {
        i64::from_le_bytes(self.take())
    }

    fn flag(&mut self) -> io::Result<bool> {
        match self.u8() {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("a flag is neither 0 nor 1")),
        }
    }

    fn optional(&mut self) -> io::Result<Option<u64>> {
        let present = self.flag()?;
        let value = self.u64();

        Ok(present.then_some(value))
    }
}

#[cfg(test)]
mod tests;
