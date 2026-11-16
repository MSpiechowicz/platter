use std::cmp::Ordering;

use super::{ensure, node_dto};
use crate::dto::{ChildrenPage, SortKey};
use crate::error::QueryError;
use crate::model::{NodeId, Tree};

/// One page of a node's children, ordered by `sort`.
///
/// Ties are broken by name (case-insensitive) and then id, so paging is stable. `total` is the number
/// of children, not the page length. Only the requested window is fully sorted.
pub fn children(
    tree: &Tree,
    parent: NodeId,
    offset: usize,
    limit: usize,
    sort: SortKey,
) -> Result<ChildrenPage, QueryError> {
    ensure(tree, parent)?;

    let mut ids: Vec<NodeId> = tree.children(parent).collect();
    let total = ids.len();
    let end = offset.saturating_add(limit).min(total);

    let order = |a: &NodeId, b: &NodeId| compare(tree, sort, *a, *b);
    if end < total {
        ids.select_nth_unstable_by(end, order);
        ids.truncate(end);
    }
    ids.sort_unstable_by(order);

    let items = ids.iter().skip(offset).map(|&id| node_dto(tree, id)).collect();
    Ok(ChildrenPage {
        parent_id: parent,
        total: total as u64,
        items,
    })
}

fn compare(tree: &Tree, sort: SortKey, a: NodeId, b: NodeId) -> Ordering {
    let primary = match sort {
        SortKey::SizeDesc => tree.allocated(b).cmp(&tree.allocated(a)),
        SortKey::SizeAsc => tree.allocated(a).cmp(&tree.allocated(b)),
        SortKey::NameAsc => Ordering::Equal,
        SortKey::FilesDesc => tree.files(b).cmp(&tree.files(a)),
        SortKey::MtimeDesc => tree.mtime(b).cmp(&tree.mtime(a)),
    };

    primary
        .then_with(|| name_order(tree.name(a), tree.name(b)))
        .then_with(|| a.cmp(&b))
}

fn name_order(a: &[u8], b: &[u8]) -> Ordering {
    a.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(b.iter().map(u8::to_ascii_lowercase))
}
