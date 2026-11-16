//! Read-only queries over a finished scan.
//!
//! Threading: `get_node` and `get_path` touch one node or one ancestor chain, so they run inline on the
//! async runtime. `get_children` (partial sort of a possibly huge directory), `get_lod` and
//! `get_summary` (walk a subtree) can take tens of milliseconds on big trees, so they run on the
//! blocking pool and never stall other commands or the webview's event delivery.

use std::sync::Arc;

use platter_core::dto::{ChildrenPage, LodNode, NodeDto, PathDto, ScanId, SortKey, SummaryDto};
use platter_core::{NodeId, QueryError, Tree, query};
use tauri::State;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Largest page `get_children` returns; a bigger `limit` is clamped.
pub const MAX_PAGE: u32 = 1000;
/// Most nodes `get_lod` returns; a bigger `maxNodes` is clamped.
pub const MAX_LOD_NODES: u32 = 5000;

#[tauri::command]
pub async fn get_node(state: State<'_, AppState>, scan: ScanId, node: NodeId) -> CommandResult<NodeDto> {
    let tree = state.scans.tree(scan)?;

    Ok(query::node(&tree, node)?)
}

/// `query`, when given, keeps only the children whose name contains it, ignoring case; `total` is then
/// the number of matches. An empty or blank query filters nothing.
#[tauri::command]
pub async fn get_children(
    state: State<'_, AppState>,
    scan: ScanId,
    node: NodeId,
    offset: u32,
    limit: u32,
    sort: SortKey,
    query: Option<String>,
) -> CommandResult<ChildrenPage> {
    let filter = match query.as_deref() {
        Some(text) if text.len() > query::MAX_NAME_FILTER_BYTES => {
            return Err(CommandError::invalid(format!(
                "the name filter is longer than {} bytes",
                query::MAX_NAME_FILTER_BYTES
            )));
        }
        Some(text) => query::NameFilter::new(text),
        None => None,
    };
    let tree = state.scans.tree(scan)?;
    let limit = clamp(limit, MAX_PAGE);

    blocking(tree, move |tree| {
        query::children_matching(tree, node, filter.as_ref(), offset as usize, limit, sort)
    })
    .await
}

#[tauri::command]
pub async fn get_lod(
    state: State<'_, AppState>,
    scan: ScanId,
    node: NodeId,
    max_nodes: u32,
    max_depth: u32,
) -> CommandResult<LodNode> {
    let tree = state.scans.tree(scan)?;
    let max_nodes = clamp(max_nodes, MAX_LOD_NODES);

    // `query::lod` itself caps the depth at `MAX_LOD_DEPTH`.
    blocking(tree, move |tree| {
        query::lod(tree, node, max_nodes, max_depth as usize)
    })
    .await
}

#[tauri::command]
pub async fn get_path(state: State<'_, AppState>, scan: ScanId, node: NodeId) -> CommandResult<PathDto> {
    let tree = state.scans.tree(scan)?;

    Ok(query::path(&tree, node)?)
}

#[tauri::command]
pub async fn get_summary(
    state: State<'_, AppState>,
    scan: ScanId,
    node: NodeId,
) -> CommandResult<SummaryDto> {
    let tree = state.scans.tree(scan)?;

    blocking(tree, move |tree| query::summary(tree, node)).await
}

/// Caps a client-supplied count at `max`.
pub fn clamp(requested: u32, max: u32) -> usize {
    requested.min(max) as usize
}

async fn blocking<T, F>(tree: Arc<Tree>, work: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Tree) -> Result<T, QueryError> + Send + 'static,
{
    let result = tauri::async_runtime::spawn_blocking(move || work(&tree)).await?;

    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_above_the_cap_are_clamped_and_others_pass_through() {
        assert_eq!(clamp(0, MAX_PAGE), 0);
        assert_eq!(clamp(50, MAX_PAGE), 50);
        assert_eq!(clamp(MAX_PAGE, MAX_PAGE), MAX_PAGE as usize);
        assert_eq!(clamp(u32::MAX, MAX_PAGE), MAX_PAGE as usize);
        assert_eq!(clamp(u32::MAX, MAX_LOD_NODES), MAX_LOD_NODES as usize);
    }
}
