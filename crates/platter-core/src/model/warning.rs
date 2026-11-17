use serde::{Deserialize, Serialize};

/// Something that did not stop a scan but made its result less complete than it looks. A source reports
/// warnings when it opens the root; the scan counts each one as an error too and keeps them on the
/// finished tree, so a front end can tell the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanWarning {
    /// The list of mounted filesystems could not be read, so the source could not tell which mounts it
    /// may enter and left every mount point below the root out of the scan.
    MountTableUnavailable,
    /// Some folders the user leaves out of scans could not be found or read in time (moved, removed, on a disk
    /// that does not answer), so they were not left out of this scan.
    ExcludedUnresolved,
}

impl ScanWarning {
    /// One plain sentence for a person.
    pub fn message(self) -> &'static str {
        match self {
            ScanWarning::MountTableUnavailable => {
                "The list of mounted disks could not be read, so other disks mounted inside this folder were \
                 left out."
            }
            ScanWarning::ExcludedUnresolved => {
                "Some folders you leave out of scans could not be found, so they were scanned this time."
            }
        }
    }
}
