use std::cmp::Ordering;

use super::{ensure, node_dto};
use crate::dto::{ChildrenPage, SortKey};
use crate::error::QueryError;
use crate::model::{NodeId, Tree};

/// The longest name filter a caller should accept, in bytes. A file name is at most 255 bytes on the
/// platforms Platter runs on, so a longer filter could never match; refusing it bounds one request.
pub const MAX_NAME_FILTER_BYTES: usize = 1024;

/// A case-insensitive substring of a child's name, as the Explorer's search box means it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameFilter {
    needle: String,
}

impl NameFilter {
    /// The filter for `text`, or `None` when it filters nothing (empty or only spaces, as in the UI).
    pub fn new(text: &str) -> Option<Self> {
        if text.trim().is_empty() {
            return None;
        }

        Some(Self {
            needle: text.to_lowercase(),
        })
    }

    /// Whether `name` contains the filter, ignoring case. A name that is not valid UTF-8 is compared in
    /// the form it is shown in (invalid bytes become U+FFFD).
    pub fn matches(&self, name: &[u8]) -> bool {
        if name.is_ascii() && self.needle.is_ascii() {
            let needle = self.needle.as_bytes();
            return name
                .windows(needle.len())
                .any(|window| window.eq_ignore_ascii_case(needle));
        }

        String::from_utf8_lossy(name)
            .to_lowercase()
            .contains(&self.needle)
    }
}

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
    children_matching(tree, parent, None, offset, limit, sort)
}

/// Like [`children`], but only the children whose name matches `filter`; `total` is then the number of
/// matching children.
pub fn children_matching(
    tree: &Tree,
    parent: NodeId,
    filter: Option<&NameFilter>,
    offset: usize,
    limit: usize,
    sort: SortKey,
) -> Result<ChildrenPage, QueryError> {
    ensure(tree, parent)?;

    let mut ids: Vec<NodeId> = match filter {
        Some(filter) => tree
            .children(parent)
            .filter(|&id| filter.matches(tree.name(id)))
            .collect(),
        None => tree.children(parent).collect(),
    };
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
