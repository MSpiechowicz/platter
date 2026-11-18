//! Read-only queries over a finished scan.
//!
//! Threading: `get_node` and `get_path` touch one node or one ancestor chain, so they run inline on the
//! async runtime. `get_children` (partial sort of a possibly huge directory), `get_lod` and
//! `get_summary` (walk a subtree) can take tens of milliseconds on big trees, so they run on the
//! blocking pool and never stall other commands or the webview's event delivery. Each holds a slot of
//! a pool of its own for as long as it runs, so a page that calls in a loop cannot fill the blocking pool.

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
    children_in(&state, scan, node, offset, limit, sort, query).await
}

/// [`get_children`] for the state itself. It takes a slot of `AppState::queries` for as long as it runs
/// (`invalid` when all are taken).
pub async fn children_in(
    state: &AppState,
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
    let walk = state.queries.take()?;
    let limit = clamp(limit, MAX_PAGE);

    blocking(tree, move |tree| {
        let _walk = walk;
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
    lod_in(&state, scan, node, max_nodes, max_depth).await
}

/// [`get_lod`] for the state itself. It takes a slot of `AppState::queries` for as long as it runs.
pub async fn lod_in(
    state: &AppState,
    scan: ScanId,
    node: NodeId,
    max_nodes: u32,
    max_depth: u32,
) -> CommandResult<LodNode> {
    let tree = state.scans.tree(scan)?;
    let walk = state.queries.take()?;
    let max_nodes = clamp(max_nodes, MAX_LOD_NODES);

    // `query::lod` itself caps the depth at `MAX_LOD_DEPTH`.
    blocking(tree, move |tree| {
        let _walk = walk;
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
    summary_in(&state, scan, node).await
}

/// [`get_summary`] for the state itself. The summary walks the node's whole subtree, so at most a few run at
/// once (`invalid` beyond them, see `AppState::summaries`).
pub async fn summary_in(state: &AppState, scan: ScanId, node: NodeId) -> CommandResult<SummaryDto> {
    let tree = state.scans.tree(scan)?;
    let walk = state.summaries.take()?;

    blocking(tree, move |tree| {
        let _walk = walk;
        query::summary(tree, node)
    })
    .await
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

    #[test]
    fn a_summary_takes_a_slot_of_its_own_pool_and_searches_do_not_keep_it_out() {
        let state = AppState::for_test();
        let mut fs = platter_core::testing::FakeFs::new();
        fs.file("/d/a", 10);
        let (scan, _) = state.scans.begin().unwrap();
        state
            .scans
            .finish(scan, platter_core::testing::scan_fake(&fs, false))
            .unwrap();
        let root = state.scans.tree(scan).unwrap().root();
        let summary = || tauri::async_runtime::block_on(summary_in(&state, scan, root));

        // Every search slot taken: the summary still runs.
        let searches: Vec<_> = (0..crate::state::WalkSlots::DEFAULT_MAX)
            .map(|_| state.walks.take().unwrap())
            .collect();
        assert!(summary().is_ok());

        // Every summary slot taken: refused, and fine again once one is given back.
        let summaries: Vec<_> = (0..crate::state::WalkSlots::DEFAULT_MAX)
            .map(|_| state.summaries.take().unwrap())
            .collect();
        let refused = summary().expect_err("refused while four summaries run");
        assert_eq!(
            serde_json::to_value(&refused).unwrap()["message"],
            crate::state::BUSY_WALKING
        );
        drop((searches, summaries));
        assert!(summary().is_ok());
    }

    #[test]
    fn children_and_lod_take_a_slot_of_the_query_pool_and_no_other_pool_keeps_them_out() {
        let state = AppState::for_test();
        let mut fs = platter_core::testing::FakeFs::new();
        fs.file("/d/a", 10);
        let (scan, _) = state.scans.begin().unwrap();
        state
            .scans
            .finish(scan, platter_core::testing::scan_fake(&fs, false))
            .unwrap();
        let root = state.scans.tree(scan).unwrap().root();
        let children = || {
            tauri::async_runtime::block_on(children_in(&state, scan, root, 0, 10, SortKey::SizeDesc, None))
        };
        let lod = || tauri::async_runtime::block_on(lod_in(&state, scan, root, 100, 4));

        // Every other pool full: both still run.
        let others: Vec<_> = (0..crate::state::WalkSlots::DEFAULT_MAX)
            .flat_map(|_| [state.walks.take().unwrap(), state.summaries.take().unwrap()])
            .collect();
        assert!(children().is_ok());
        assert!(lod().is_ok());

        // 31 queries running: one more fits. All 32: both are refused, and run again once one ends.
        let mut queries: Vec<_> = (1..crate::state::MAX_QUERIES)
            .map(|_| state.queries.take().unwrap())
            .collect();
        assert!(children().is_ok());
        queries.push(state.queries.take().unwrap());
        for refused in [children().map(drop), lod().map(drop)] {
            let error = refused.expect_err("refused while 32 queries run");
            assert_eq!(
                serde_json::to_value(&error).unwrap()["message"],
                crate::state::BUSY_WALKING
            );
        }
        queries.pop();
        assert!(children().is_ok());
        assert!(lod().is_ok());

        // The queries that ran gave their slots back: with the held ones dropped, all 32 can be taken.
        drop((others, queries));
        let all: Vec<_> = (0..crate::state::MAX_QUERIES)
            .map(|_| state.queries.take().unwrap())
            .collect();
        assert_eq!(all.len(), crate::state::MAX_QUERIES);
    }
}
