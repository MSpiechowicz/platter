//! Removing the files of the model Platter no longer uses (Qwen3.5-2B, see [`RETIRED_FILES`]): its disk
//! space is the user's, and nothing else would ever delete it.
//!
//! Only Platter's own models folder is touched, only the exact names in [`RETIRED_FILES`], and only
//! when the name is a regular file: a link of that name is left alone (never followed, never removed).
//! The step runs under the folder's lock, like every other change there. A file that cannot be removed
//! is left for the next start; nothing is reported, because the folder is the user's data.

use std::fs;
use std::path::Path;

use crate::lock::ModelLock;
use crate::spec::RETIRED_FILES;

/// Removes the retired model's files from the models folder `dir`. Does nothing, and creates nothing
/// (not even the lock file), when none is there, when the folder is missing, or when another process
/// holds the lock.
pub fn remove_retired(dir: &Path) {
    if !RETIRED_FILES.iter().any(|name| is_regular_file(&dir.join(name))) {
        return;
    }

    let Ok(Some(_lock)) = ModelLock::try_acquire(dir) else {
        return;
    };

    for name in RETIRED_FILES {
        let path = dir.join(name);

        // Looked at again under the lock; `remove_file` unlinks the name and never follows a link.
        if is_regular_file(&path) {
            let _ = fs::remove_file(&path);
        }
    }
}

/// A regular file by `lstat`: a link, a folder or anything else is not.
fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "Qwen3.5-2B-Q4_K_M.gguf";
    const OLD_PARTIAL: &str = "Qwen3.5-2B-Q4_K_M.gguf.partial";

    fn write(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"data").unwrap();
    }

    #[test]
    fn both_old_files_go_and_everything_else_stays() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            OLD,
            OLD_PARTIAL,
            "Qwen3.5-4B-Q4_K_M.gguf",
            "Qwen3.5-4B-Q4_K_M.gguf.partial",
            "Qwen3.5-2B-Q4_K_M.gguf.bak",
            "notes.txt",
        ] {
            write(dir.path(), name);
        }

        remove_retired(dir.path());

        assert!(!dir.path().join(OLD).exists());
        assert!(!dir.path().join(OLD_PARTIAL).exists());
        for kept in [
            "Qwen3.5-4B-Q4_K_M.gguf",
            "Qwen3.5-4B-Q4_K_M.gguf.partial",
            "Qwen3.5-2B-Q4_K_M.gguf.bak",
            "notes.txt",
        ] {
            assert!(dir.path().join(kept).exists(), "{kept}");
        }
    }

    #[test]
    fn one_old_file_alone_is_removed_too() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), OLD_PARTIAL);

        remove_retired(dir.path());

        assert!(!dir.path().join(OLD_PARTIAL).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_of_the_old_name_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = elsewhere.path().join("precious.txt");
        fs::write(&target, b"precious").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(OLD)).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(OLD_PARTIAL)).unwrap();

        remove_retired(dir.path());

        assert_eq!(fs::read(&target).unwrap(), b"precious");
        assert!(elsewhere.path().exists());
    }

    #[test]
    fn a_folder_of_the_old_name_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(OLD)).unwrap();
        write(&dir.path().join(OLD), "inside.txt");

        remove_retired(dir.path());

        assert!(dir.path().join(OLD).join("inside.txt").exists());
    }

    #[test]
    fn nothing_to_remove_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();

        remove_retired(dir.path());
        remove_retired(&dir.path().join("missing"));

        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            0,
            "not even the lock file"
        );
    }

    #[test]
    fn a_folder_locked_by_another_holder_is_left_for_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), OLD);
        let held = ModelLock::try_acquire(dir.path()).unwrap().unwrap();

        remove_retired(dir.path());
        assert!(dir.path().join(OLD).exists());

        drop(held);
        remove_retired(dir.path());
        assert!(!dir.path().join(OLD).exists());
    }
}
