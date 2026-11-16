//! Moving to the user's trash, in the freedesktop.org layout (`files/` and `info/*.trashinfo`), so file
//! managers can show and restore what Platter put there.
//!
//! The `.trashinfo` file is created first with `O_EXCL`, which reserves the name; the entry is then moved
//! with `renameat(RENAME_NOREPLACE)`, which never overwrites. An entry on the home trash's filesystem goes
//! there; one on another disk goes to that disk's `$topdir/.Trash-$uid` (the per-user fallback of the
//! spec; the shared `$topdir/.Trash/$uid` is not used), which must belong to the user and be closed to
//! others. Nothing is ever copied between filesystems.

use std::ffi::CString;
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use platter_core::EntryProbe;
use platter_core::ExpectedEntry;
use rustix::fs::{AtFlags, CWD, Mode, OFlags, mkdirat, openat, unlinkat};
use rustix::io::Errno;
use rustix::process::getuid;

use super::probe::LinuxEntryProbe;
use super::remove::{check, components, open_parent, rename_no_replace};
use super::stat::stat_fd;
use crate::localtime::local_timestamp;

const PATH_DIR: OFlags = OFlags::PATH
    .union(OFlags::NOFOLLOW)
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);

/// The longest file name on Linux filesystems.
const MAX_NAME_BYTES: usize = 255;

/// The first `max` bytes of `name`, cut so that a UTF-8 name stays valid UTF-8.
fn shorten(name: &[u8], max: usize) -> &[u8] {
    if name.len() <= max {
        return name;
    }
    let mut end = max;
    if std::str::from_utf8(name).is_ok() {
        while end > 0 && (name[end] & 0b1100_0000) == 0b1000_0000 {
            end -= 1;
        }
    }
    &name[..end]
}

/// How many names `name`, `name.2`, ... are tried before giving up.
const MAX_NAMES: u32 = 10_000;

pub(super) fn move_to_trash(path: &[u8], expected: &ExpectedEntry, home_trash: &Path) -> io::Result<Vec<u8>> {
    let (parent, name) = open_parent(path, expected.root_depth)?;
    let stat = check(&parent, &name, expected)?;

    let home = open_or_create_dir(home_trash.as_os_str().as_encoded_bytes())?;
    // The home trash must be the user's own, or what is moved there could be read by someone else.
    if stat_fd(&home)?.uid != getuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the trash folder belongs to someone else",
        ));
    }
    let home_files = child_dir(&home, c"files", None)?;
    if stat.meta.dev == stat_fd(&home_files)?.meta.dev {
        let info = child_dir(&home, c"info", None)?;
        let place = Place {
            files: home_files,
            info,
            location: home_trash.as_os_str().as_encoded_bytes().to_vec(),
            recorded_path: path.to_vec(),
        };
        return place.put(&parent, &name);
    }

    // Another disk: the trash of that disk's top folder, `.Trash-$uid` (freedesktop.org).
    let top = mount_top(path)?;
    let uid = getuid().as_raw();
    let top_fd = open_existing_dir(&top)?;
    let folder =
        CString::new(format!(".Trash-{uid}")).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let trash = child_dir(&top_fd, &folder, Some(uid))?;
    let files = child_dir(&trash, c"files", Some(uid))?;
    let info = child_dir(&trash, c"info", Some(uid))?;
    if stat.meta.dev != stat_fd(&files)?.meta.dev {
        return Err(io::Error::new(
            io::ErrorKind::CrossesDevices,
            "the trash of this disk is on another filesystem",
        ));
    }

    let mut location = top.clone();
    if location.last() != Some(&b'/') {
        location.push(b'/');
    }
    location.extend_from_slice(folder.to_bytes());
    let recorded_path = path
        .strip_prefix(top.as_slice())
        .map(|rest| rest.strip_prefix(b"/").unwrap_or(rest).to_vec())
        .unwrap_or_else(|| path.to_vec());

    Place {
        files,
        info,
        location,
        recorded_path,
    }
    .put(&parent, &name)
}

/// One trash folder, ready to take an entry.
struct Place {
    files: OwnedFd,
    info: OwnedFd,
    /// The trash folder's path, for the answer.
    location: Vec<u8>,
    /// The `Path=` of the record: absolute for the home trash, relative to the top folder otherwise.
    recorded_path: Vec<u8>,
}

impl Place {
    fn put(&self, parent: &OwnedFd, name: &CString) -> io::Result<Vec<u8>> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX));
        let record = trash_info(&self.recorded_path, &local_timestamp(now));

        // A name plus a number and `.trashinfo` must fit in a file name (255 bytes): shorten long ones.
        let base = shorten(name.to_bytes(), MAX_NAME_BYTES - b".trashinfo".len() - 6);
        for number in 1..=MAX_NAMES {
            let mut candidate = base.to_vec();
            if number > 1 {
                candidate.extend_from_slice(format!(".{number}").as_bytes());
            }
            let mut info_name = candidate.clone();
            info_name.extend_from_slice(b".trashinfo");
            let candidate =
                CString::new(candidate).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            let info_name =
                CString::new(info_name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;

            let flags = OFlags::WRONLY
                .union(OFlags::CREATE)
                .union(OFlags::EXCL)
                .union(OFlags::NOFOLLOW)
                .union(OFlags::CLOEXEC);
            let fd = match openat(&self.info, info_name.as_c_str(), flags, Mode::RUSR | Mode::WUSR) {
                Ok(fd) => fd,
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            };
            if let Err(error) = std::fs::File::from(fd).write_all(record.as_bytes()) {
                let _ = unlinkat(&self.info, info_name.as_c_str(), AtFlags::empty());
                return Err(error);
            }

            match rename_no_replace(parent, name, &self.files, &candidate) {
                Ok(()) => {
                    let mut to = self.location.clone();
                    to.extend_from_slice(b"/files/");
                    to.extend_from_slice(candidate.to_bytes());
                    return Ok(to);
                }
                Err(error) => {
                    let _ = unlinkat(&self.info, info_name.as_c_str(), AtFlags::empty());
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        Err(io::Error::other("no free name in the trash"))
    }
}

/// The top folder of the filesystem holding `path` (see the comment inside), or `/`.
fn mount_top(path: &[u8]) -> io::Result<Vec<u8>> {
    let parts = components(path)?;
    let found = LinuxEntryProbe.probe(path)?;
    // The top of the filesystem holding the entry: the deepest folder above it that starts a mount, or
    // whose device differs from its parent's (a nested btrfs subvolume, which is not a mount but cannot
    // be renamed out of).
    let ancestors = &found[..found.len().saturating_sub(1)];
    let depth = (0..ancestors.len())
        .rev()
        .find(|&index| {
            let entry = &ancestors[index];
            entry.meta.is_mount_root || (index > 0 && entry.meta.dev != ancestors[index - 1].meta.dev)
        })
        .map_or(0, |index| index + 1);

    let mut top = Vec::new();
    for part in &parts[..depth] {
        top.push(b'/');
        top.extend_from_slice(part);
    }
    if top.is_empty() {
        top.push(b'/');
    }
    Ok(top)
}

/// Opens an existing folder without following links and without creating anything.
fn open_existing_dir(path: &[u8]) -> io::Result<OwnedFd> {
    let mut dir = openat(CWD, c"/", PATH_DIR, Mode::empty())?;
    for part in components(path)? {
        let name = CString::new(part).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        dir = openat(&dir, name.as_c_str(), PATH_DIR, Mode::empty())?;
    }
    Ok(dir)
}

/// Moves the entry at `trashed` (`<trash>/files/<name>`) back to `original` and removes its record
/// (`<trash>/info/<name>.trashinfo`). Mounts on the way do not matter here (the trash and the original
/// folder may be on any mount), but no link is followed and nothing is overwritten.
pub(super) fn restore(trashed: &[u8], expected: &ExpectedEntry, original: &[u8]) -> io::Result<()> {
    // `<trash>/files/<name>`: the record is `<trash>/info/<name>.trashinfo`.
    let files_at = trashed
        .iter()
        .rposition(|&byte| byte == b'/')
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let trash_at = trashed[..files_at]
        .iter()
        .rposition(|&byte| byte == b'/')
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    if &trashed[trash_at + 1..files_at] != b"files" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not an entry of a trash folder",
        ));
    }
    let trash = &trashed[..trash_at];
    let name = &trashed[files_at + 1..];
    let mut info = trash.to_vec();
    info.extend_from_slice(b"/info/");
    info.extend_from_slice(name);
    info.extend_from_slice(b".trashinfo");

    // The record must still say this entry came from `original`: after the trash was emptied, a
    // newer entry could carry the same name (and even a reused inode).
    let (info_dir, info_name) = open_parent(&info, usize::MAX)?;
    let recorded = recorded_path(&info_dir, &info_name)?;
    let top = &trash[..trash.iter().rposition(|&byte| byte == b'/').unwrap_or(0)];
    let from_record = if recorded.first() == Some(&b'/') {
        recorded
    } else {
        let mut joined = if top.is_empty() { Vec::new() } else { top.to_vec() };
        joined.push(b'/');
        joined.extend_from_slice(&recorded);
        joined
    };
    if from_record != original {
        return Err(io::Error::other(
            "changed since the plan: the trash record names another place",
        ));
    }

    let (files, entry) = open_parent(trashed, usize::MAX)?;
    check(&files, &entry, expected)?;
    let (folder, original_name) = open_parent(original, usize::MAX)?;

    rename_no_replace(&files, &entry, &folder, &original_name)?;
    let _ = unlinkat(&info_dir, info_name.as_c_str(), AtFlags::empty());
    Ok(())
}

/// The decoded `Path=` of a `.trashinfo` record, read without following a link (at most 64 KiB).
fn recorded_path(dir: &OwnedFd, name: &std::ffi::CStr) -> io::Result<Vec<u8>> {
    use std::io::Read;

    let flags = OFlags::RDONLY
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK);
    let fd = openat(dir, name, flags, Mode::empty())?;
    let mut text = Vec::new();
    std::fs::File::from(fd).take(64 * 1024).read_to_end(&mut text)?;

    text.split(|&byte| byte == b'\n')
        .find_map(|line| line.strip_prefix(b"Path="))
        .and_then(percent_decode)
        .ok_or_else(|| io::Error::other("the trash record has no readable Path"))
}

fn percent_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.iter();
    while let Some(&byte) = bytes.next() {
        if byte == b'%' {
            let high = (*bytes.next()? as char).to_digit(16)?;
            let low = (*bytes.next()? as char).to_digit(16)?;
            out.push((high * 16 + low) as u8);
        } else {
            out.push(byte);
        }
    }
    Some(out)
}

/// Opens the folder at `path` without following links, creating missing components (mode 0700).
fn open_or_create_dir(path: &[u8]) -> io::Result<OwnedFd> {
    let mut dir = openat(CWD, c"/", PATH_DIR, Mode::empty())?;

    for part in components(path)? {
        let name = CString::new(part).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        dir = child_dir(&dir, &name, None)?;
    }

    Ok(dir)
}

/// Opens the folder `name` below `parent` without following a link, creating it (mode 0700) when
/// missing. With `owner`, the folder must belong to that user and be closed to everyone else, as a
/// shared disk's trash must be (another user could have made it first).
fn child_dir(parent: &OwnedFd, name: &std::ffi::CStr, owner: Option<u32>) -> io::Result<OwnedFd> {
    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }

    let dir = openat(parent, name, PATH_DIR, Mode::empty())?;
    if let Some(uid) = owner {
        let stat = stat_fd(&dir)?;
        if stat.uid != uid || stat.mode & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the disk's trash folder belongs to someone else or is open to others",
            ));
        }
    }
    Ok(dir)
}

/// The `.trashinfo` text: the original path, percent-encoded, and the deletion time in local time.
fn trash_info(path: &[u8], when: &str) -> String {
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={when}\n",
        percent_encode(path)
    )
}

fn percent_encode(path: &[u8]) -> String {
    let mut out = String::with_capacity(path.len());
    for &byte in path {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_names_are_shortened_without_splitting_a_letter() {
        assert_eq!(shorten(b"short", 10), b"short");
        assert_eq!(
            shorten("aż".as_bytes(), 2),
            b"a",
            "the two bytes of ż stay together"
        );
        assert_eq!(shorten(&[b'x'; 300], 239).len(), 239);
    }

    #[test]
    fn a_recorded_path_decodes_back_to_the_original_bytes() {
        let original = b"/home/u/a b/\xc5\xbc\xff.txt";
        let encoded = percent_encode(original);

        assert_eq!(percent_decode(encoded.as_bytes()).unwrap(), original);
        assert_eq!(percent_decode(b"%zz"), None);
        assert_eq!(percent_decode(b"%4"), None);
    }

    #[test]
    fn the_info_record_follows_the_spec() {
        assert_eq!(
            trash_info(b"/home/u/a b/\xc5\xbc.txt", "2025-11-16T11:46:40"),
            "[Trash Info]\nPath=/home/u/a%20b/%C5%BC.txt\nDeletionDate=2025-11-16T11:46:40\n"
        );
        assert_eq!(
            trash_info(b"photos/x.jpg", "t"),
            "[Trash Info]\nPath=photos/x.jpg\nDeletionDate=t\n"
        );
    }
}
