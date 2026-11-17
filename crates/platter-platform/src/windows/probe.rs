//! The entry probe on Windows (W2).
//!
//! [`WindowsEntryProbe`] looks at a path component by component. Windows has no `openat`, so each
//! component is opened by the path up to it, as a reparse point (see [`identity`](super::identity)): a
//! reparse point (symbolic link, junction, mount point, placeholder) at the last name is described as a
//! link and never followed, and one on the way is `NotADirectory`. A change of volume between two
//! components marks a mount root.
//!
//! What stops a component from being swapped for a link after it was looked at is that every folder on
//! the way stays open while the next one is opened, and is opened *without* `FILE_SHARE_DELETE`, so it can
//! be neither deleted nor renamed while it is held. A plain look lets go of them when it answers. The
//! remover keeps them for the whole action (see [`walk`] and [`Hold`]).
//!
//! The paths are the bytes of UTF-8 text of the form `X:\a\b` (the WTF-8 of `OsStr` on Windows, which is
//! UTF-8 unless a name holds an unpaired surrogate: such a path is refused, because it cannot be turned
//! back into a path without `unsafe`). `/` is not a separator, and a name that Windows would read as
//! another one (a trailing dot or space, a stream `:`, a device name such as `NUL` or `COM¹`, a name
//! that ends in a `.{GUID}`, which the shell reads as a namespace) is refused.
//!
//! A look runs in its own thread and is given up after [`PROBE_TIMEOUT`], like the Linux probe: a share
//! that does not answer cannot hold cleanup. The thread is left to finish (or hang) on its own.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, UNIX_EPOCH};

use platter_core::{EntryKind, EntryMeta, EntryProbe, ProbedEntry};

use super::identity::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_ALL,
    FileIdentity, identity_of,
};

/// How long one look at a path may take before it is given up.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// `FILE_SHARE_READ | FILE_SHARE_WRITE`: everything but `FILE_SHARE_DELETE`. A folder held open like this
/// cannot be deleted or renamed by anybody else, which is what keeps a checked name from being swapped.
const FILE_SHARE_READ_WRITE: u32 = 0x0000_0003;

/// DOS device names: a file by one of these, with or without an extension, is the device.
const DEVICE_NAMES: &[&str] = &[
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "CONIN$",
    "CONOUT$",
    "COM0",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT0",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

/// [`EntryProbe`] on Windows handles.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsEntryProbe;

impl EntryProbe for WindowsEntryProbe {
    fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
        let path = path.to_vec();

        timed(move || walk(&path, Hold::Nothing).map(|held| held.entries))
    }

    /// The canonical path (every link followed, never verbatim), for finding where a protected or kept
    /// folder really is. Never used for a target.
    fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let parts = parse(path)?;
        let path = join(&parts, parts.len());

        timed(move || {
            let resolved = dunce::canonicalize(path)?;

            Ok(resolved.into_os_string().into_encoded_bytes())
        })
    }
}

/// How much of the way a [`walk`] keeps open after it has looked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hold {
    /// Nothing: the handles are let go when the look is done.
    Nothing,
    /// Every folder above the last name (the remover moves the last one itself, so it must not be held
    /// without `FILE_SHARE_DELETE`).
    Ancestors,
    /// Every name, the last included (a folder whose children are acted on).
    All,
}

/// What a [`walk`] saw, and the handles it kept.
#[derive(Debug)]
pub(super) struct Walk {
    /// One entry per component, the drive root first.
    pub entries: Vec<ProbedEntry>,
    /// The same, as the system reports them (the 64-bit file index is only meaningful with the serial).
    pub identities: Vec<FileIdentity>,
    /// The folders above the last name, when [`Hold::Ancestors`] or [`Hold::All`] asked for them.
    pub held: Vec<File>,
    /// The last name, opened so that nobody is kept from using it: let it go before the entry is moved.
    pub target: File,
}

/// Looks at `path` as it is now, component by component. See the module's text.
///
/// Errors: `InvalidInput` for a path that is not plainly `X:\...`, `NotADirectory` for a link or a file
/// above the last name, `NotFound` for a missing one, and whatever the system says (a sharing violation
/// when something else has a folder open in a way that does not allow [`Hold`]).
pub(super) fn walk(path: &[u8], hold: Hold) -> io::Result<Walk> {
    let parts = parse(path)?;

    let mut entries = Vec::with_capacity(parts.len());
    let mut identities: Vec<FileIdentity> = Vec::with_capacity(parts.len());
    let mut held: Vec<File> = Vec::new();
    let mut last: Option<File> = None;

    for position in 0..parts.len() {
        let is_last = position + 1 == parts.len();

        let strict = match hold {
            Hold::Nothing => false,
            Hold::Ancestors => !is_last,
            Hold::All => true,
        };
        let file = open_component(&join(&parts, position + 1), strict)?;
        let identity = identity_of(&file)?;

        if !is_last && (identity.is_reparse_point() || !identity.is_dir()) {
            return Err(io::Error::from(io::ErrorKind::NotADirectory));
        }

        let parent_serial = identities.last().map(|parent| parent.volume_serial);
        entries.push(describe(&file, &identity, parent_serial)?);
        identities.push(identity);

        if is_last {
            last = Some(file);
        } else if hold != Hold::Nothing {
            held.push(file);
        }
    }

    let target = last.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    Ok(Walk {
        entries,
        identities,
        held,
        target,
    })
}

/// Opens one component to look at it: no access to its contents, a reparse point opened itself, a folder
/// openable. `strict` refuses `FILE_SHARE_DELETE` to everybody else (see the module's text).
fn open_component(path: &std::path::Path, strict: bool) -> io::Result<File> {
    let share = if strict {
        FILE_SHARE_READ_WRITE
    } else {
        FILE_SHARE_ALL
    };

    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(share)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn describe(file: &File, identity: &FileIdentity, parent_serial: Option<u64>) -> io::Result<ProbedEntry> {
    let kind = if identity.is_reparse_point() {
        EntryKind::Symlink
    } else if identity.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::File
    };

    Ok(ProbedEntry {
        meta: EntryMeta {
            kind,
            allocated: identity.size,
            apparent: identity.size,
            mtime: modified_seconds(file)?,
            dev: identity.volume_serial,
            ino: identity.file_index,
            nlink: identity.links,
            // A different volume than the folder above is a mount (a volume mounted in a folder is a
            // reparse point, which is refused on its own: this is the belt to that braces).
            is_mount_root: parent_serial.is_some_and(|serial| serial != identity.volume_serial),
        },
        mount_id: None,
        birth: identity.birth(),
    })
}

/// The modification time in unix seconds, as the scan's reader counts it (the whole seconds of
/// `Metadata::modified`).
fn modified_seconds(file: &File) -> io::Result<i64> {
    let modified = file.metadata()?.modified()?;

    Ok(match modified.duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    })
}

/// The components of `path`: the drive root (`C:\`), then the names. See the module's text for what is
/// refused.
pub(super) fn parse(path: &[u8]) -> io::Result<Vec<String>> {
    let invalid = || io::Error::from(io::ErrorKind::InvalidInput);

    let text = std::str::from_utf8(path).map_err(|_| invalid())?;
    let mut characters = text.chars();
    let (Some(letter), Some(':'), Some('\\')) = (characters.next(), characters.next(), characters.next())
    else {
        return Err(invalid());
    };
    if !letter.is_ascii_alphabetic() {
        return Err(invalid());
    }

    let mut parts = vec![text[..3].to_owned()];
    let rest = &text[3..];
    let rest = rest.strip_suffix('\\').unwrap_or(rest);
    if rest.is_empty() {
        return Ok(parts);
    }

    for name in rest.split('\\') {
        if !is_plain_name(name) {
            return Err(invalid());
        }
        parts.push(name.to_owned());
    }

    Ok(parts)
}

/// Whether `name` means what it says on Windows.
pub(super) fn is_plain_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return false;
    }

    // `NUL.txt` is the device too: what is before the first dot decides.
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');

    if DEVICE_NAMES
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
    {
        return false;
    }

    !ends_in_guid(name)
}

/// Whether `name` ends in `.{` hex digits and dashes `}`: the shell reads such a folder as the namespace
/// the GUID names (a junction to it), whatever is inside.
fn ends_in_guid(name: &str) -> bool {
    let Some(inner) = name.strip_suffix('}') else {
        return false;
    };
    let Some(open) = inner.rfind(".{") else {
        return false;
    };
    let guid = &inner[open + 2..];

    !guid.is_empty() && guid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// The path of the first `count` components of `parts`.
pub(super) fn join(parts: &[String], count: usize) -> PathBuf {
    let mut text = String::new();
    for (position, part) in parts.iter().take(count).enumerate() {
        if position > 1 {
            text.push('\\');
        }
        text.push_str(part);
    }

    PathBuf::from(text)
}

/// The UTF-16 length of the longest path Platter hands to the shell: longer ones are refused, because
/// the shell would turn them into something else or fail in ways that are not told apart here.
pub(super) const MAX_PATH_UTF16: usize = 32_000;

/// Whether the path is short enough for the shell (see [`MAX_PATH_UTF16`]).
pub(super) fn is_short_enough(path: &std::path::Path) -> bool {
    path.as_os_str().encode_wide().count() < MAX_PATH_UTF16
}

/// Runs `work` in its own thread and waits [`PROBE_TIMEOUT`] for it. The thread is left to finish (or
/// hang) on its own: a stuck system call cannot be interrupted, only stopped waiting for.
pub(super) fn timed<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("platter-probe".to_owned())
        .spawn(move || {
            let _ = send.send(work());
        })?;

    receive.recv_timeout(PROBE_TIMEOUT).unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the path did not answer in time (a network share?)",
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_a_drive_root_and_names() {
        assert_eq!(parse(b"C:\\").unwrap(), ["C:\\"]);
        assert_eq!(parse(b"C:\\Users\\me\\").unwrap(), ["C:\\", "Users", "me"]);
        assert_eq!(
            join(&parse(b"d:\\a b\\c.d").unwrap(), 3),
            PathBuf::from("d:\\a b\\c.d")
        );
        assert_eq!(join(&parse(b"d:\\a\\b").unwrap(), 1), PathBuf::from("d:\\"));
    }

    #[test]
    fn anything_that_is_not_plainly_drive_absolute_is_refused() {
        for bad in [
            &b""[..],
            b"/",
            b"C:",
            b"C:a",
            b"C:/a",
            b"C:\\a/b",
            b"C:\\a\\\\b",
            b"C:\\a\\..\\b",
            b"\\\\host\\share\\a",
            b"\\\\?\\C:\\a",
            b"C:\\a.",
            b"C:\\a \\b",
            b"C:\\a:s",
            b"C:\\NUL",
            b"C:\\a\\con.txt",
            b"C:\\a\\CONIN$",
            b"C:\\a\\conout$.txt",
            "C:\\a\\COM\u{b9}".as_bytes(),
            "C:\\a\\lpt\u{b2}.log".as_bytes(),
            "C:\\a\\LPT\u{b3}".as_bytes(),
            b"C:\\Folder.{645FF040-5081-101B-9F08-00AA002F954E}",
            b"C:\\a\xed\xa0\x80",
        ] {
            let error = parse(bad).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
    }

    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dunce::canonicalize(dir.path()).unwrap();

        (dir, path)
    }

    fn bytes(path: &std::path::Path) -> Vec<u8> {
        path.as_os_str().as_encoded_bytes().to_vec()
    }

    #[test]
    fn every_component_is_described_and_the_last_is_the_entry() {
        let (_guard, base) = scratch();
        let file = base.join("sub").join("f.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"12345").unwrap();

        let looked = walk(&bytes(&file), Hold::Nothing).unwrap();

        let drive_and_names = file.components().count() - 1;
        assert_eq!(looked.entries.len(), drive_and_names);
        let entry = looked.entries.last().unwrap();
        assert_eq!(
            (entry.meta.kind, entry.meta.apparent, entry.meta.nlink),
            (EntryKind::File, 5, 1)
        );
        assert!(entry.birth.is_some());
        assert!(
            looked.entries[..looked.entries.len() - 1]
                .iter()
                .all(|entry| entry.meta.kind == EntryKind::Dir)
        );
        assert!(looked.entries.iter().all(|entry| !entry.meta.is_mount_root));
        assert!(looked.held.is_empty(), "nothing is held unless asked");
    }

    #[test]
    fn a_held_folder_cannot_be_renamed_or_deleted_until_it_is_let_go() {
        let (_guard, base) = scratch();
        let parent = base.join("parent");
        std::fs::create_dir_all(parent.join("child")).unwrap();
        let file = parent.join("child").join("f.txt");
        std::fs::write(&file, b"x").unwrap();

        let looked = walk(&bytes(&file), Hold::Ancestors).unwrap();
        assert_eq!(looked.held.len(), looked.entries.len() - 1);

        assert!(
            std::fs::rename(&parent, base.join("moved")).is_err(),
            "an ancestor is held"
        );
        assert!(
            std::fs::rename(parent.join("child"), parent.join("moved")).is_err(),
            "so is the folder directly above"
        );
        // The target is open too (it is not held against being moved, but its handle is still a handle):
        // let go of everything the look kept.
        drop(looked);
        std::fs::rename(&parent, base.join("moved")).unwrap();
    }

    #[test]
    fn a_link_on_the_way_is_not_a_folder_and_a_link_at_the_end_is_described_as_one() {
        let (_guard, base) = scratch();
        let real = base.join("real");
        std::fs::create_dir_all(real.join("inner")).unwrap();
        let link = base.join("link");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .output()
            .unwrap();
        assert!(made.status.success());

        let through = walk(&bytes(&link.join("inner")), Hold::Nothing).unwrap_err();
        let at_the_end = walk(&bytes(&link), Hold::Nothing).unwrap();

        assert_eq!(through.kind(), io::ErrorKind::NotADirectory);
        assert_eq!(at_the_end.entries.last().unwrap().meta.kind, EntryKind::Symlink);
    }

    #[test]
    fn a_missing_name_is_not_found_and_a_file_on_the_way_is_not_a_folder() {
        let (_guard, base) = scratch();
        std::fs::write(base.join("f"), b"x").unwrap();

        let missing = walk(&bytes(&base.join("nothing")), Hold::Nothing).unwrap_err();
        let below_a_file = walk(&bytes(&base.join("f").join("x")), Hold::Nothing).unwrap_err();

        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        assert_eq!(below_a_file.kind(), io::ErrorKind::NotADirectory);
    }
}
