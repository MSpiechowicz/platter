//! Default skip list of the Windows reader, expressed as data.
//!
//! The rules name entries of a drive's root folder only: the paging and hibernation files, which the
//! system holds and which are not data the user can act on, and the two folders no program should
//! walk (`System Volume Information` is closed to everybody but the system, `$Recycle.Bin` is the
//! shell's). A skipped folder is still listed, as a `mount` node of size 0 that is never entered, like
//! an excluded mount on Linux. A skipped file is left out of the listing.
//!
//! The rules never apply to the scan root itself, and not below the drive's root: the reader only asks
//! about the children of a drive root that it lists as a child of the scan root or below. Names are
//! compared ASCII case-insensitively, as Windows does for these.

use std::ffi::OsStr;

/// What to skip in the root folder of a drive.
#[derive(Debug, Clone, Copy)]
pub struct ExclusionRules {
    /// Files left out of the listing.
    pub files: &'static [&'static str],
    /// Folders listed but never entered.
    pub folders: &'static [&'static str],
}

impl ExclusionRules {
    pub const DEFAULT: ExclusionRules = ExclusionRules {
        files: &["pagefile.sys", "hiberfil.sys", "swapfile.sys"],
        folders: &["System Volume Information", "$Recycle.Bin"],
    };

    /// No exclusions at all.
    pub const NONE: ExclusionRules = ExclusionRules {
        files: &[],
        folders: &[],
    };

    /// Whether the file `name` in a drive's root folder is left out.
    pub fn excludes_file(&self, name: &OsStr) -> bool {
        matches_any(self.files, name)
    }

    /// Whether the folder `name` in a drive's root folder is left out.
    pub fn excludes_folder(&self, name: &OsStr) -> bool {
        matches_any(self.folders, name)
    }
}

impl Default for ExclusionRules {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn matches_any(names: &[&str], name: &OsStr) -> bool {
    names.iter().any(|excluded| name.eq_ignore_ascii_case(excluded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_files_and_folders_of_a_drive_root_are_skipped_in_any_case() {
        let rules = ExclusionRules::DEFAULT;

        for name in ["pagefile.sys", "HIBERFIL.SYS", "SwapFile.sys"] {
            assert!(rules.excludes_file(OsStr::new(name)), "{name}");
            assert!(!rules.excludes_folder(OsStr::new(name)), "{name} is a file");
        }
        for name in ["System Volume Information", "$recycle.bin", "$RECYCLE.BIN"] {
            assert!(rules.excludes_folder(OsStr::new(name)), "{name}");
            assert!(!rules.excludes_file(OsStr::new(name)), "{name} is a folder");
        }
    }

    #[test]
    fn other_names_and_near_misses_are_scanned() {
        let rules = ExclusionRules::DEFAULT;

        for name in [
            "Users",
            "Windows",
            "pagefile.sys.bak",
            "pagefile",
            "System Volume",
            "$Recycle.Bin2",
        ] {
            assert!(!rules.excludes_file(OsStr::new(name)), "{name}");
            assert!(!rules.excludes_folder(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn no_rules_skip_nothing() {
        assert!(!ExclusionRules::NONE.excludes_file(OsStr::new("pagefile.sys")));
        assert!(!ExclusionRules::NONE.excludes_folder(OsStr::new("$Recycle.Bin")));
    }
}
