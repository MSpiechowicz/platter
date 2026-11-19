//! The last finished scan, kept on disk so the next start can open it again (docs/architecture.md,
//! "Files on disk").
//!
//! Two files in the data folder, always replaced as a pair and never edited in place:
//! - `last-scan.bin`: the tree ([`Tree::write_snapshot`]), compressed with zstd;
//! - `last-scan.json`: [`Sidecar`], the options the scan ran with, when it ran, and whether a cleanup changed
//!   the disk since (`outdated`). The sidecar is small, so a cleanup rewrites it alone.
//!
//! Only one scan is kept: each scan that finishes replaces the pair. The files are written in a background
//! thread, each to a temporary file in the same folder and then renamed, the sidecar last (the old sidecar is
//! removed first), so a crash leaves either the old pair, the new pair or no sidecar, and a pair that does not
//! belong together (its scan times differ) is not opened. A save whose scan was followed by another one
//! that finished meanwhile does not write anything, so the older scan never replaces the newer. Temporary
//! files are made new, with a name of their own (process and a random part), so two running copies of the app
//! never write into each other's, and one is only swept when it is old.
//!
//! The files are opened like the rules file is: no link, and nothing but a regular file, in a data folder that
//! is the user's. The lock around the files is held only to open or replace them, never while a tree is
//! written or read: a file that is open stays the file it was when it was replaced.
//!
//! Nothing here fails a scan: a save that cannot be written is dropped, and a pair that cannot be read is
//! the same as no pair.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use platter_core::dto::ScanOptions;
use platter_core::{Tree, dto};
use serde::{Deserialize, Serialize};

use crate::commands::scan::check_excluded;

const TREE_FILE: &str = "last-scan.bin";
const SIDECAR_FILE: &str = "last-scan.json";
/// Temporary files are `last-scan.<what>.<process>-<random>.tmp`.
const TEMP_PREFIX: &str = "last-scan.";
const TEMP_SUFFIX: &str = ".tmp";

/// A temporary file this old belongs to a run that was stopped while writing; a younger one may be a
/// write of another running copy of the app.
const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// The version of the sidecar and of what it goes with. Another version is not opened.
const VERSION: u32 = 1;

/// A fast level: a save runs while the user looks at the result, and a tree is mostly names and numbers.
const ZSTD_LEVEL: i32 = 1;

/// The largest sidecar read (it holds a few paths).
const MAX_SIDECAR_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    pub version: u32,
    pub options: ScanOptions,
    /// Unix seconds at which the scan started ([`Tree::scanned_at`]); the tree in `last-scan.bin` has the
    /// same, which is how a pair is told to belong together.
    pub scanned_at: i64,
    /// A cleanup changed the disk after the scan.
    pub outdated: bool,
}

/// What [`LastScanStore::load`] found.
pub struct LastScan {
    pub tree: Tree,
    pub options: ScanOptions,
    pub outdated: bool,
}

/// What the writers agree on, behind the lock that also makes the file changes one at a time.
struct Saved {
    /// Counts the saves that began, so that only the newest one writes.
    generation: u64,
    /// Whether the newest scan is outdated now (a cleanup may run while it is written).
    outdated: bool,
    /// The temporary files being written by this process now.
    live: HashSet<PathBuf>,
}

pub struct LastScanStore {
    /// `None` when the data folder is not known (and in tests that keep nothing): nothing is saved or found.
    dir: Option<PathBuf>,
    saved: Mutex<Saved>,
}

impl LastScanStore {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            saved: Mutex::new(Saved {
                generation: 0,
                outdated: false,
                live: HashSet::new(),
            }),
        }
    }

    /// The store in the user's data folder.
    pub fn production() -> Self {
        Self::new(platter_platform::data_dir())
    }

    fn lock(&self) -> MutexGuard<'_, Saved> {
        // Nothing under the lock leaves `Saved` half-changed, so recover from a panic.
        self.saved.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Keeps `tree` as the last scan, replacing the earlier one, in a thread of its own: the call returns
    /// at once and a failure is not reported. `outdated` says whether a cleanup changed the disk since the
    /// scan read it; it is asked once the store is locked, so a cleanup that is marked in the meantime is not
    /// overwritten by an answer that was true before it. The handle is for tests; dropping it leaves the
    /// thread running.
    pub fn save(
        self: &Arc<Self>,
        tree: Arc<Tree>,
        options: ScanOptions,
        outdated: impl FnOnce() -> bool,
    ) -> Option<JoinHandle<()>> {
        let dir = self.dir.clone()?;

        let generation = {
            let mut saved = self.lock();
            saved.generation += 1;
            saved.outdated = outdated();
            saved.generation
        };

        let store = Arc::clone(self);
        thread::Builder::new()
            .name("platter-save-scan".to_owned())
            .spawn(move || {
                // Nobody is waiting for the answer, and the scan is on screen either way.
                let _ = store.write(&dir, generation, &tree, &options);
            })
            .ok()
    }

    fn write(&self, dir: &Path, generation: u64, tree: &Tree, options: &ScanOptions) -> io::Result<()> {
        let token = unique_token();
        let tree_temp = dir.join(temp_name("bin", &token));
        let sidecar_temp = dir.join(temp_name("json", &token));

        {
            // Nothing to do when another scan finished since: it is, or will be, the one kept.
            let mut saved = self.lock();
            if saved.generation != generation {
                return Ok(());
            }
            saved.live.insert(tree_temp.clone());
            saved.live.insert(sidecar_temp.clone());
        }

        let written = platter_platform::create_private_dir(dir)
            .and_then(|()| write_tree(&tree_temp, tree))
            .and_then(|()| self.replace_pair(dir, generation, &tree_temp, &sidecar_temp, tree, options));
        if written.is_err() {
            let _ = fs::remove_file(&tree_temp);
            let _ = fs::remove_file(&sidecar_temp);
        }

        let mut saved = self.lock();
        saved.live.remove(&tree_temp);
        saved.live.remove(&sidecar_temp);

        written
    }

    /// Makes the written tree the kept one, unless a newer save began meanwhile.
    fn replace_pair(
        &self,
        dir: &Path,
        generation: u64,
        tree_temp: &Path,
        sidecar_temp: &Path,
        tree: &Tree,
        options: &ScanOptions,
    ) -> io::Result<()> {
        let saved = self.lock();
        if saved.generation != generation {
            return fs::remove_file(tree_temp);
        }

        let sidecar = Sidecar {
            version: VERSION,
            options: options.clone(),
            scanned_at: tree.scanned_at(),
            outdated: saved.outdated,
        };
        write_sidecar(sidecar_temp, &sidecar)?;
        remove_leftovers(dir, &saved.live);

        // The old sidecar goes first and the new one comes last: in between there is no pair to open.
        remove_if_there(&dir.join(SIDECAR_FILE))?;
        fs::rename(tree_temp, dir.join(TREE_FILE))?;
        fs::rename(sidecar_temp, dir.join(SIDECAR_FILE))
    }

    /// A cleanup (or an undo) changed the disk, or is about to: the scan kept is outdated, and so is one that
    /// is being written now. Rewrites the sidecar only.
    pub fn mark_outdated(&self) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };

        let mut saved = self.lock();
        saved.outdated = true;

        let Some(mut sidecar) = read_sidecar(dir) else {
            return;
        };
        if sidecar.outdated {
            return;
        }
        sidecar.outdated = true;

        let temp = dir.join(temp_name("json", &format!("outdated-{}", unique_token())));
        let renamed = write_sidecar(&temp, &sidecar).and_then(|()| fs::rename(&temp, dir.join(SIDECAR_FILE)));
        if renamed.is_err() {
            let _ = fs::remove_file(&temp);
        }
    }

    /// Deletes the scan kept on disk (both files), when the user asks to forget it. A save that began before
    /// is dropped: it counts as older, so it never puts the forgotten scan back. The next finished scan is kept
    /// again as usual.
    pub fn forget(&self) -> io::Result<()> {
        let Some(dir) = self.dir.as_deref() else {
            return Ok(());
        };

        let mut saved = self.lock();
        saved.generation += 1;
        saved.outdated = false;

        // The sidecar first: without it the tree file is never read.
        remove_if_there(&dir.join(SIDECAR_FILE))?;
        remove_if_there(&dir.join(TREE_FILE))
    }

    /// The scan kept on disk, or `None` when there is none, it cannot be read (damaged, cut short, made by
    /// another version, too large for `max_memory_bytes`) or its two files do not belong together.
    pub fn load(&self, max_memory_bytes: u64) -> Option<LastScan> {
        let dir = self.dir.as_deref()?;

        // Both files are opened under the lock, so that they are the pair one save wrote; the tree is read
        // after it is let go, from the file that is open (a save that replaces the name meanwhile leaves
        // the open file as it was).
        let (sidecar, file) = {
            let _files = self.lock();
            open_pair(dir)?
        };

        let mut decoder = zstd::Decoder::new(file).ok()?;
        let tree = Tree::read_snapshot_limited(&mut decoder, max_memory_bytes).ok()?;
        // The frame must end where the tree does: reading its end is what checks its checksum.
        if decoder.read(&mut [0u8; 1]).ok()? != 0 {
            return None;
        }

        if tree.scanned_at() != sidecar.scanned_at || !options_are_valid(&sidecar.options) {
            return None;
        }

        Some(LastScan {
            tree,
            options: sidecar.options,
            outdated: sidecar.outdated,
        })
    }
}

/// The sidecar and the open tree file of a data folder that is the user's.
fn open_pair(dir: &Path) -> Option<(Sidecar, File)> {
    // The folder is checked like the write path checks it (owner, and closed to others), not made.
    if !fs::metadata(dir).ok()?.is_dir() || platter_platform::create_private_dir(dir).is_err() {
        return None;
    }

    let sidecar = read_sidecar(dir)?;
    let file = platter_platform::open_nofollow(&dir.join(TREE_FILE)).ok()?;

    Some((sidecar, file))
}

/// The tree into `path`, a new file, compressed and on the disk. Not under the lock: it takes the longest.
fn write_tree(path: &Path, tree: &Tree) -> io::Result<()> {
    let file = create_private_new(path)?;

    let mut encoder = zstd::Encoder::new(BufWriter::new(file), ZSTD_LEVEL)?;
    encoder.include_checksum(true)?;
    tree.write_snapshot(&mut encoder)?;

    let file = encoder
        .finish()?
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?;

    file.sync_all()
}

/// Options a scan can be started with again: what the file says is not trusted.
fn options_are_valid(options: &ScanOptions) -> bool {
    Path::new(&options.root).is_absolute() && check_excluded(&options.excluded).is_ok()
}

fn temp_name(what: &str, token: &str) -> String {
    format!("{TEMP_PREFIX}{what}.{token}{TEMP_SUFFIX}")
}

/// A name part no other save has: this process, and a random number.
fn unique_token() -> String {
    let mut hasher = RandomState::new().build_hasher();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    hasher.write_u128(now.as_nanos());

    format!("{}-{:016x}", std::process::id(), hasher.finish())
}

/// A file that is made here and now (an existing one, or a link at the name, is an error), closed to others.
fn create_private_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }

    options.open(path)
}

/// Removes the temporary files that a run left when it was stopped while writing: those that are not being
/// written by this process and are older than [`STALE_AFTER`].
fn remove_leftovers(dir: &Path, live: &HashSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };

        let temporary = name.starts_with(TEMP_PREFIX) && name.ends_with(TEMP_SUFFIX);
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age >= STALE_AFTER));
        if temporary && stale && !live.contains(&entry.path()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn remove_if_there(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn write_sidecar(path: &Path, sidecar: &Sidecar) -> io::Result<()> {
    let bytes = serde_json::to_vec(sidecar).map_err(io::Error::other)?;

    let mut file = create_private_new(path)?;
    file.write_all(&bytes)?;

    file.sync_all()
}

/// The sidecar, when it is there, can be read and is of this version.
fn read_sidecar(dir: &Path) -> Option<Sidecar> {
    let file = platter_platform::open_nofollow(&dir.join(SIDECAR_FILE)).ok()?;

    let mut bytes = Vec::new();
    file.take(MAX_SIDECAR_BYTES).read_to_end(&mut bytes).ok()?;

    let sidecar: Sidecar = serde_json::from_slice(&bytes).ok()?;

    (sidecar.version == VERSION).then_some(sidecar)
}

/// How the wire writes the `elapsed` of a scan (see `ScanEvent::Finished`).
pub fn elapsed_ms(tree: &Tree) -> u64 {
    u64::try_from(tree.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .min(dto::MAX_WIRE_INTEGER)
}

/// When the scan finished, in unix seconds: it started at [`Tree::scanned_at`] and took [`Tree::elapsed`].
pub fn finished_at(tree: &Tree) -> i64 {
    let took = i64::try_from(tree.elapsed().as_secs()).unwrap_or(i64::MAX);

    tree.scanned_at().saturating_add(took)
}

#[cfg(test)]
mod tests;
