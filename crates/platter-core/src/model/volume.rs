use std::path::PathBuf;

/// A mounted volume, as listed by a [`VolumeProvider`](crate::ports::VolumeProvider).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub id: String,
    pub mount_point: PathBuf,
    pub label: String,
    pub device: String,
    pub fs_type: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
}
