use crate::dto::NodeDto;
use crate::error::QueryError;
use crate::model::{NodeId, Tree};

/// Describes one node.
pub fn node(tree: &Tree, id: NodeId) -> Result<NodeDto, QueryError> {
    ensure(tree, id)?;
    Ok(node_dto(tree, id))
}

pub(crate) fn ensure(tree: &Tree, id: NodeId) -> Result<(), QueryError> {
    if tree.contains(id) {
        Ok(())
    } else {
        Err(QueryError::NotFound(id))
    }
}

/// Builds the DTO for a node known to exist.
pub(crate) fn node_dto(tree: &Tree, id: NodeId) -> NodeDto {
    NodeDto {
        id,
        name: tree.name_lossy(id).into_owned(),
        kind: tree.kind(id),
        allocated: tree.allocated(id),
        apparent: tree.apparent(id),
        files: tree.files(id),
        has_children: tree.has_children(id),
        unreadable: tree.is_unreadable(id),
        skipped: tree.skip_reason(id),
        category: tree.category(id),
        mtime: wire_mtime(tree.mtime(id)),
    }
}

/// Latest `mtime` the UI can represent: 9999-12-31T23:59:59Z in unix seconds.
const MAX_WIRE_MTIME: i64 = 253_402_300_799;

/// Clamps an `mtime` for the wire to 1970..=9999. The filesystem can report anything an `i64` holds,
/// but JavaScript's `Date` only represents about +-8.64e15 ms and the UI formats years with four digits;
/// an out-of-range value would turn into an invalid date there. The tree keeps the raw value (age
/// buckets and sorting use it); only the DTO is clamped.
pub(crate) fn wire_mtime(mtime: i64) -> i64 {
    mtime.clamp(0, MAX_WIRE_MTIME)
}
