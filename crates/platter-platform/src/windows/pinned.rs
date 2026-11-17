//! Opening the model file so that what was checked is what is loaded.
//!
//! Linux hands the loader `/proc/self/fd/N`, a name that follows a descriptor. Windows has no such name, so
//! the file is held with a share mode instead, and the loader opens it by its ordinary path:
//!
//! - [`open_pinned`] opens the file with `FILE_SHARE_READ` only. While that handle is open nobody else can
//!   open the file for writing (the share mode forbids `FILE_SHARE_WRITE`) and nobody can rename or delete
//!   it (that needs `DELETE` access, which `FILE_SHARE_DELETE` would allow). A process that already had the
//!   file open for writing makes the open fail with a sharing violation, so the file is not open for writing
//!   anywhere when the handle is made. The bytes read through the handle (to hash them) are therefore the
//!   bytes at the path for as long as the handle stays open.
//! - The folder the file is in is held too, with `FILE_SHARE_READ | FILE_SHARE_WRITE` and no
//!   `FILE_SHARE_DELETE`: it cannot be renamed or removed (which would let another file appear under the same
//!   path), and files can still be made and renamed inside it (the download and the lock file live there).
//!   Folders above it cannot be renamed either while a handle below them is open (Windows refuses to move a
//!   folder that has open files in it). That last rule is Windows's, not this crate's, and the CI test
//!   `a_folder_above_the_folder_holding_a_pinned_file_cannot_be_renamed` is its proof.
//!
//! The pin is a [`Pin`] (the folder's handle and the path the file was opened by). The caller keeps the pin and
//! the file for as long as the helper may read the model: through the `load`. Only the last name of the path and
//! of its folder are looked at (a reparse point at either is refused); a link further up is followed, as on
//! Linux, where the models folder is the user's own.
//!
//! [`open_nofollow`] is the plain open for the user's rules file and the small logs: no share restrictions.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::identity::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_ALL, identity_of,
};

/// `FILE_SHARE_READ`: others may read the file, nobody may write, rename or delete it.
const FILE_SHARE_READ: u32 = 0x0000_0001;
/// `FILE_SHARE_READ | FILE_SHARE_WRITE`: a folder others may use but not rename or remove.
const FILE_SHARE_READ_WRITE: u32 = 0x0000_0003;
/// `FILE_READ_ATTRIBUTES`: enough to hold a folder open.
const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;

/// What keeps the path of a model file naming the file that was checked, besides the open file itself.
#[derive(Debug)]
pub struct Pin {
    /// The folder the file is in, held open so that it cannot be renamed or removed.
    _folder: Option<File>,
    /// The absolute path the file was opened by: what the loader is given.
    path: PathBuf,
}

impl Pin {
    /// The path that names `file` (the file this pin was made with) for as long as both are held.
    pub fn path(&self, _file: &File) -> Option<PathBuf> {
        Some(self.path.clone())
    }
}

/// Opens `path` for reading with `FILE_SHARE_READ` only, refusing a reparse point at the name, a folder and
/// anything else that is not a regular file, and holds the folder it is in. See the module's text for what this
/// keeps. A relative `path` is made absolute first (the helper runs somewhere else).
pub fn open_pinned(path: &Path) -> io::Result<(File, Pin)> {
    let path = std::path::absolute(path)?;

    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)?;
    let identity = identity_of(&file)?;
    if identity.is_reparse_point() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is a link and Platter does not follow it", path.display()),
        ));
    }
    if identity.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }

    // Held after the file, so a file that is refused does not hold a folder.
    let folder = path.parent().map(hold_folder).transpose()?;

    Ok((
        file,
        Pin {
            _folder: folder,
            path,
        },
    ))
}

/// Opens `folder` so that it cannot be renamed or removed while the handle lives (and can be used otherwise).
fn hold_folder(folder: &Path) -> io::Result<File> {
    let held = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(folder)?;

    // A folder that is a link (a junction) could be pointed somewhere else after all.
    if identity_of(&held)?.is_reparse_point() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is a link and Platter does not follow it", folder.display()),
        ));
    }

    Ok(held)
}

/// Opens `path` for reading, refusing a reparse point at the path (symbolic link, junction) and anything
/// that is not a regular file. Only the last name is checked. Nobody is kept from using the file.
pub fn open_nofollow(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_ALL)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let identity = identity_of(&file)?;

    if identity.is_reparse_point() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is a link and Platter does not follow it", path.display()),
        ));
    }
    if identity.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }

    Ok(file)
}

/// Windows has no path that follows a handle: the path is the [`Pin`]'s.
pub fn pinned_path(_file: &File) -> Option<PathBuf> {
    None
}

/// Whether the advisor's model can be held open and handed to the runtime by path: yes, see [`open_pinned`].
pub const fn model_runtime_supported() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn model(dir: &Path) -> PathBuf {
        let path = dir.join("model.gguf");
        std::fs::write(&path, b"weights").unwrap();
        path
    }

    #[test]
    fn a_pinned_file_reads_and_the_pin_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = model(dir.path());

        let (mut file, pin) = open_pinned(&path).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();

        assert_eq!(text, "weights");
        assert_eq!(pin.path(&file), Some(path));
    }

    #[test]
    fn a_pinned_file_cannot_be_written_renamed_or_deleted_but_can_be_read_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = model(dir.path());
        let (_file, _pin) = open_pinned(&path).unwrap();

        let write = OpenOptions::new().write(true).open(&path);
        let append = std::fs::write(&path, b"swapped");
        let rename = std::fs::rename(&path, dir.path().join("moved.gguf"));
        let delete = std::fs::remove_file(&path);

        assert!(write.is_err(), "no one may open it for writing");
        assert!(append.is_err(), "no one may overwrite it");
        assert!(rename.is_err(), "no one may rename it");
        assert!(delete.is_err(), "no one may delete it");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"weights",
            "still the same bytes, and readable by name"
        );
    }

    #[test]
    fn a_pin_that_is_let_go_of_frees_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = model(dir.path());

        drop(open_pinned(&path).unwrap());

        std::fs::rename(&path, dir.path().join("moved.gguf")).unwrap();
    }

    #[test]
    fn a_file_that_is_open_for_writing_cannot_be_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let path = model(dir.path());
        let writer = OpenOptions::new().write(true).open(&path).unwrap();

        let error = open_pinned(&path).unwrap_err();

        assert!(error.raw_os_error().is_some(), "a sharing violation: {error}");
        drop(writer);
        assert!(open_pinned(&path).is_ok());
    }

    #[test]
    fn a_folder_holding_a_pinned_file_cannot_be_renamed_or_removed_but_takes_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        std::fs::create_dir(&models).unwrap();
        let path = model(&models);
        let (_file, _pin) = open_pinned(&path).unwrap();

        let renamed = std::fs::rename(&models, dir.path().join("elsewhere"));
        let removed = std::fs::remove_dir_all(&models);
        let made = std::fs::write(models.join("other.partial"), b"x");

        assert!(renamed.is_err(), "the folder cannot be renamed");
        assert!(removed.is_err() && path.exists(), "the folder cannot be removed");
        assert!(made.is_ok(), "files can still be made in it");
        std::fs::rename(models.join("other.partial"), models.join("other.gguf")).unwrap();
    }

    #[test]
    fn a_folder_above_the_folder_holding_a_pinned_file_cannot_be_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let platter = dir.path().join("platter");
        let models = platter.join("models");
        std::fs::create_dir_all(&models).unwrap();
        let (_file, _pin) = open_pinned(&model(&models)).unwrap();

        let renamed = std::fs::rename(&platter, dir.path().join("elsewhere"));

        assert!(
            renamed.is_err(),
            "Windows does not move a folder with open files in it"
        );
    }

    #[test]
    fn a_link_a_folder_and_a_missing_file_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = model(dir.path());
        let link = dir.path().join("link.gguf");
        // A symbolic link needs a privilege (or developer mode) on Windows: when it cannot be made, the
        // folder and the missing file are all there is to check.
        let linked = std::os::windows::fs::symlink_file(&target, &link).is_ok();

        if linked {
            assert_eq!(
                open_pinned(&link).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert!(open_pinned(dir.path()).is_err());
        assert_eq!(
            open_pinned(&dir.path().join("missing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(open_pinned(&target).is_ok());
    }

    #[test]
    fn a_path_with_letters_outside_ascii_is_pinned_and_read_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Użytkownik żółć 模型");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("model żółw.gguf");
        std::fs::write(&path, b"weights").unwrap();

        let (_file, pin) = open_pinned(&path).unwrap();

        assert_eq!(pin.path(&_file), Some(path.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), b"weights");
        assert!(std::fs::rename(&path, folder.join("moved.gguf")).is_err());
    }
}
