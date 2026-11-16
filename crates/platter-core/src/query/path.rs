use super::ensure;
use crate::dto::{PathDto, PathSegment};
use crate::error::QueryError;
use crate::model::{NodeId, Tree};

/// Full filesystem path of a node plus the id/name of every ancestor from the root down to the node.
pub fn path(tree: &Tree, id: NodeId) -> Result<PathDto, QueryError> {
    ensure(tree, id)?;

    let segments = tree
        .ancestry(id)
        .into_iter()
        .map(|segment| PathSegment {
            id: segment,
            name: tree.name_lossy(segment).into_owned(),
        })
        .collect();

    Ok(PathDto {
        path: tree.path_of(id).to_string_lossy().into_owned(),
        segments,
    })
}
