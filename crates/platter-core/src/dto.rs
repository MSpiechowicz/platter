//! Wire types shared with the frontend. They serialize exactly as the TypeScript types in
//! `docs/architecture.md`: camelCase fields, `type`-tagged [`ScanEvent`], camelCase enum values.
//!
//! Names are lossy UTF-8; sizes are bytes as JSON numbers. Every size and count is clamped to
//! [`MAX_WIRE_INTEGER`] when it is serialized (see there); the tree keeps the exact value. The snapshot
//! tests at the bottom pin the wire format.

use serde::{Deserialize, Serialize, Serializer};

pub use crate::model::{Category, NodeId, NodeKind, Progress, ScanWarning, SkipReason, Totals, Volume};

use crate::error::{QueryError, ScanError};
use crate::model::Tree;

/// The largest integer a JavaScript number holds exactly (2^53 - 1, `Number.MAX_SAFE_INTEGER`). Sizes and
/// counts above it would reach the UI as a rounded, wrong number, so the wire carries this value
/// instead: an exabyte-scale figure only has to read as "huge". Like the `mtime` clamp, only the DTO is
/// clamped; the tree keeps the raw `u64`.
pub const MAX_WIRE_INTEGER: u64 = 9_007_199_254_740_991;

/// `serialize_with` for the `u64` sizes and counts of the wire types: clamps to [`MAX_WIRE_INTEGER`].
pub(crate) fn wire_u64<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u64((*value).min(MAX_WIRE_INTEGER))
}

/// Identifies a scan held by the backend.
pub type ScanId = u64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeDto {
    pub id: String,
    pub mount_point: String,
    pub label: String,
    pub device: String,
    pub fs_type: String,
    #[serde(serialize_with = "wire_u64")]
    pub total: u64,
    #[serde(serialize_with = "wire_u64")]
    pub available: u64,
    pub removable: bool,
}

impl From<&Volume> for VolumeDto {
    fn from(volume: &Volume) -> Self {
        Self {
            id: volume.id.clone(),
            mount_point: volume.mount_point.to_string_lossy().into_owned(),
            label: volume.label.clone(),
            device: volume.device.clone(),
            fs_type: volume.fs_type.clone(),
            total: volume.total,
            available: volume.available,
            removable: volume.removable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOptions {
    pub root: String,
    pub one_file_system: bool,
    /// Absolute paths of folders the scan leaves out (at most [`MAX_EXCLUDED`], each at most
    /// [`MAX_EXCLUDED_PATH_BYTES`] bytes). A path that no longer exists is ignored. Omitted means none.
    #[serde(default)]
    pub excluded: Vec<String>,
}

/// Most folders a scan may be told to leave out.
pub const MAX_EXCLUDED: usize = 64;

/// Longest path, in bytes, of a folder a scan may be told to leave out (the usual `PATH_MAX`).
pub const MAX_EXCLUDED_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ScanEvent {
    /// Running counters. `bytes` is a lower bound of the final `allocated` (hardlinked files are left
    /// out, see [`Progress`]); once `finished` arrives, show its `totals` instead.
    Progress {
        #[serde(serialize_with = "wire_u64")]
        files: u64,
        #[serde(serialize_with = "wire_u64")]
        dirs: u64,
        #[serde(serialize_with = "wire_u64")]
        bytes: u64,
        #[serde(serialize_with = "wire_u64")]
        errors: u64,
        current_path: String,
    },
    Finished {
        scan_id: ScanId,
        root_id: NodeId,
        totals: Totals,
        #[serde(serialize_with = "wire_u64")]
        elapsed_ms: u64,
        /// Problems that made the result less complete than it looks (also counted in
        /// `totals.errors`), e.g. `"mountTableUnavailable"`. Omitted from the JSON when there are none,
        /// so a consumer that does not know the field sees the same events as before.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        warnings: Vec<ScanWarning>,
    },
    Cancelled,
    Failed {
        message: String,
    },
}

impl ScanEvent {
    pub fn finished(scan_id: ScanId, tree: &Tree) -> Self {
        ScanEvent::Finished {
            scan_id,
            root_id: tree.root(),
            totals: tree.totals(),
            elapsed_ms: u64::try_from(tree.elapsed().as_millis()).unwrap_or(u64::MAX),
            warnings: tree.warnings().to_vec(),
        }
    }

    pub fn failed(error: &ScanError) -> Self {
        match error {
            ScanError::Cancelled => ScanEvent::Cancelled,
            other => ScanEvent::Failed {
                message: other.to_string(),
            },
        }
    }
}

impl From<&Progress> for ScanEvent {
    fn from(progress: &Progress) -> Self {
        ScanEvent::Progress {
            files: progress.files,
            dirs: progress.dirs,
            bytes: progress.bytes,
            errors: progress.errors,
            current_path: progress.current_path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDto {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    #[serde(serialize_with = "wire_u64")]
    pub allocated: u64,
    #[serde(serialize_with = "wire_u64")]
    pub apparent: u64,
    /// Files in the subtree; 1 for a file-like node.
    #[serde(serialize_with = "wire_u64")]
    pub files: u64,
    pub has_children: bool,
    /// The scan could not read this directory (permission denied, a loop, too deep): its size is
    /// unknown rather than zero. Always present; false for everything else.
    pub unreadable: bool,
    /// Why a `mount` node was not entered: `"otherDisk"`, `"system"`, `"duplicate"` or `"unknown"` (see
    /// [`SkipReason`]). Present only for nodes of kind `mount`; omitted from the JSON for every other node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<SkipReason>,
    pub category: Category,
    /// Unix seconds; for a directory the newest mtime inside it.
    pub mtime: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortKey {
    SizeDesc,
    SizeAsc,
    NameAsc,
    FilesDesc,
    MtimeDesc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildrenPage {
    pub parent_id: NodeId,
    #[serde(serialize_with = "wire_u64")]
    pub total: u64,
    pub items: Vec<NodeDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LodNode {
    pub node: NodeDto,
    pub children: Vec<LodNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathSegment {
    pub id: NodeId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathDto {
    pub path: String,
    pub segments: Vec<PathSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgeBucket {
    Week,
    Month,
    Quarter,
    Year,
    TwoYears,
    Older,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryStat {
    pub category: Category,
    #[serde(serialize_with = "wire_u64")]
    pub allocated: u64,
    #[serde(serialize_with = "wire_u64")]
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgeStat {
    pub bucket: AgeBucket,
    #[serde(serialize_with = "wire_u64")]
    pub allocated: u64,
    #[serde(serialize_with = "wire_u64")]
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryDto {
    pub node: NodeDto,
    pub by_category: Vec<CategoryStat>,
    pub by_age: Vec<AgeStat>,
    pub largest_dirs: Vec<NodeDto>,
}

/// How a cleanup suggestion is labelled. Ordered from most to least removable, so sorting by verdict
/// puts `safe` first. Wire names are lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Regenerated or re-downloaded on demand.
    Safe,
    /// Probably removable, but only the user can tell.
    Review,
    /// Deliberately left alone.
    Keep,
}

/// Who made a suggestion's verdict. Wire names are lowercase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionSource {
    /// The built-in rule table.
    #[default]
    Rule,
    /// A rule the user added.
    User,
    /// The local model's labels.
    Model,
}

/// One rule's findings over a scan (read-only: nothing is changed or deleted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionDto {
    /// Stable rule id, e.g. `pacman-cache`.
    pub id: String,
    pub title: String,
    /// Who made the verdict. A suggestion without this field counts as `rule`.
    #[serde(default)]
    pub source: SuggestionSource,
    /// Display path or pattern: `/var/cache/pacman/pkg`, `~/.cache`, `~/Projekty/**/target`. A home
    /// directory is always shown as `~`.
    pub path: String,
    pub verdict: Verdict,
    /// One or two plain sentences; any count or age in it comes from the real matches.
    pub reason: String,
    /// Bytes this suggestion covers.
    #[serde(serialize_with = "wire_u64")]
    pub allocated: u64,
    /// How many folders (or files, for file rules) matched.
    #[serde(serialize_with = "wire_u64")]
    pub items: u64,
    /// How many files inside the matched nodes have more than one hard link. The scan charges such a
    /// file to the first link it met, which may lie outside the matched nodes, so `allocated` can differ
    /// from what removing the nodes frees, in either direction. 0 when there are none.
    #[serde(serialize_with = "wire_u64")]
    pub shared_files: u64,
    /// Up to 20 largest matched nodes, largest first.
    pub nodes: Vec<NodeId>,
}

/// Bytes covered per verdict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerdictTotals {
    #[serde(serialize_with = "wire_u64")]
    pub safe: u64,
    #[serde(serialize_with = "wire_u64")]
    pub review: u64,
    #[serde(serialize_with = "wire_u64")]
    pub keep: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionsDto {
    /// Safe first, then review, then keep; largest first within each.
    pub suggestions: Vec<SuggestionDto>,
    pub totals: VerdictTotals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApiErrorCode {
    NotFound,
    Cancelled,
    Io,
    Invalid,
}

/// Error returned by every IPC command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<&ScanError> for ApiError {
    fn from(error: &ScanError) -> Self {
        let code = match error {
            ScanError::Cancelled => ApiErrorCode::Cancelled,
            ScanError::NotADirectory(_) | ScanError::TooLarge { .. } => ApiErrorCode::Invalid,
            ScanError::Root { .. } | ScanError::ThreadPool(_) => ApiErrorCode::Io,
        };
        ApiError::new(code, error.to_string())
    }
}

impl From<QueryError> for ApiError {
    fn from(error: QueryError) -> Self {
        let code = match error {
            QueryError::NotFound(_) => ApiErrorCode::NotFound,
            QueryError::QueryTooLong(_) => ApiErrorCode::Invalid,
        };
        ApiError::new(code, error.to_string())
    }
}

#[cfg(test)]
mod tests;
