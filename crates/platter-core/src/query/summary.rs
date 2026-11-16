use std::cmp::Reverse;

use super::{ensure, node_dto};
use crate::dto::{AgeBucket, AgeStat, CategoryStat, SummaryDto};
use crate::error::QueryError;
use crate::model::{Category, NodeId, NodeKind, Tree};

const LARGEST_DIRS: usize = 8;
const DAY: i64 = 86_400;

/// Age buckets in display order with the inclusive upper bound of their age, in seconds.
const BUCKETS: [(AgeBucket, i64); 5] = [
    (AgeBucket::Week, 7 * DAY),
    (AgeBucket::Month, 30 * DAY),
    (AgeBucket::Quarter, 90 * DAY),
    (AgeBucket::Year, 365 * DAY),
    (AgeBucket::TwoYears, 730 * DAY),
];

/// Breakdown of a subtree: bytes per category, bytes per age bucket (relative to scan time) and the
/// largest directories below the node (see [`largest_dirs`]).
///
/// Every byte of the subtree is attributed exactly once, so each breakdown sums to the node's
/// `allocated`: a file counts for its own category and mtime, a directory's own blocks count for the
/// directory's category and (aggregated) mtime. `files` counts only non-directory entries.
/// `byCategory` lists categories that have content, largest first; `byAge` always lists all six buckets.
pub fn summary(tree: &Tree, id: NodeId) -> Result<SummaryDto, QueryError> {
    ensure(tree, id)?;

    let mut by_category = [(0u64, 0u64); Category::COUNT];
    let mut by_age = [(0u64, 0u64); 6];
    let mut stack = vec![id];

    while let Some(current) = stack.pop() {
        let mut own = tree.allocated(current);
        let mut files = 0;

        match tree.kind(current) {
            NodeKind::Dir => {
                for child in tree.children(current) {
                    own = own.saturating_sub(tree.allocated(child));
                    stack.push(child);
                }
            }
            NodeKind::Mount | NodeKind::Collapsed => continue,
            _ => files = 1,
        }

        let category = &mut by_category[tree.category(current).index()];
        category.0 = category.0.saturating_add(own);
        category.1 += files;

        let age = &mut by_age[bucket_index(tree.scanned_at().saturating_sub(tree.mtime(current)))];
        age.0 = age.0.saturating_add(own);
        age.1 += files;
    }

    let mut categories: Vec<CategoryStat> = Category::ALL
        .iter()
        .zip(by_category)
        .filter(|(_, (allocated, files))| *allocated > 0 || *files > 0)
        .map(|(&category, (allocated, files))| CategoryStat {
            category,
            allocated,
            files,
        })
        .collect();
    categories.sort_by_key(|stat| Reverse(stat.allocated));

    let ages = (0..6)
        .map(|index| AgeStat {
            bucket: bucket_at(index),
            allocated: by_age[index].0,
            files: by_age[index].1,
        })
        .collect();

    Ok(SummaryDto {
        node: node_dto(tree, id),
        by_category: categories,
        by_age: ages,
        largest_dirs: largest_dirs(tree, id)
            .into_iter()
            .map(|dir| node_dto(tree, dir))
            .collect(),
    })
}

/// A negative age (mtime in the future) lands in the first bucket; a saturated huge age in `Older`.
fn bucket_index(age_seconds: i64) -> usize {
    BUCKETS
        .iter()
        .position(|&(_, limit)| age_seconds <= limit)
        .unwrap_or(BUCKETS.len())
}

fn bucket_at(index: usize) -> AgeBucket {
    BUCKETS.get(index).map_or(AgeBucket::Older, |&(bucket, _)| bucket)
}

/// A directory is replaced by its child directories when one of them holds at least this share of its
/// allocated bytes, so a path like `/home` > `/home/user` lists only the deeper levels.
const DOMINANT_SHARE_PERCENT: u128 = 80;

/// Candidates are at most this many levels below the summarized node (its children are level 1).
const MAX_DEPTH: u32 = 6;

/// A directory that may be listed, with its depth below the summarized node.
#[derive(Clone, Copy)]
struct Candidate {
    dir: NodeId,
    depth: u32,
}

/// Up to eight directories below `id` for the "largest folders" list, largest first.
///
/// 1. The candidates start as the directory children of `id`.
/// 2. A candidate with a child directory that holds at least [`DOMINANT_SHARE_PERCENT`] of its bytes is
///    replaced by its directory children (the dominated parent tells nothing its children do not), again
///    and again, down to [`MAX_DEPTH`] levels.
/// 3. While there are fewer than eight candidates, the largest candidate that can grow the list (it has at
///    least two directory children, and is not at the depth limit) is replaced by its directory children,
///    and step 2 is applied to the result. This stops when nothing can be expanded.
///
/// Empty directories and mount points are never candidates. The eight largest candidates are returned,
/// largest first; on equal size the lower id wins, so the result is deterministic.
fn largest_dirs(tree: &Tree, id: NodeId) -> Vec<NodeId> {
    let mut candidates: Vec<Candidate> = child_dirs(tree, id)
        .map(|dir| Candidate { dir, depth: 1 })
        .collect();

    settle_dominated(tree, &mut candidates);

    while candidates.len() < LARGEST_DIRS {
        let Some(position) = largest_expandable(tree, &candidates) else {
            break;
        };

        let parent = candidates.remove(position);
        candidates.extend(child_dirs(tree, parent.dir).map(|dir| Candidate {
            dir,
            depth: parent.depth + 1,
        }));
        settle_dominated(tree, &mut candidates);
    }

    // Largest first; on equal size the lower id wins.
    candidates.sort_by_key(|candidate| (Reverse(tree.allocated(candidate.dir)), candidate.dir.0));
    candidates.truncate(LARGEST_DIRS);
    candidates.into_iter().map(|candidate| candidate.dir).collect()
}

/// Directory children of `dir` that have content. Mounts and empty directories are skipped.
fn child_dirs(tree: &Tree, dir: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    tree.children(dir)
        .filter(|&child| tree.kind(child) == NodeKind::Dir && tree.allocated(child) > 0)
}

/// Replaces every candidate dominated by one of its child directories with its child directories,
/// repeatedly, as long as the depth limit allows.
fn settle_dominated(tree: &Tree, candidates: &mut Vec<Candidate>) {
    loop {
        let mut changed = false;
        let mut next = Vec::with_capacity(candidates.len());

        for &candidate in candidates.iter() {
            if candidate.depth < MAX_DEPTH && has_dominant_child(tree, candidate.dir) {
                next.extend(child_dirs(tree, candidate.dir).map(|dir| Candidate {
                    dir,
                    depth: candidate.depth + 1,
                }));
                changed = true;
            } else {
                next.push(candidate);
            }
        }

        *candidates = next;
        if !changed {
            return;
        }
    }
}

/// Position of the largest candidate whose expansion adds to the list: it is not at the depth limit and
/// has at least two directory children (one child would only swap a folder for a smaller one).
fn largest_expandable(tree: &Tree, candidates: &[Candidate]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.depth < MAX_DEPTH)
        .filter(|(_, candidate)| child_dirs(tree, candidate.dir).nth(1).is_some())
        .min_by_key(|(_, candidate)| (Reverse(tree.allocated(candidate.dir)), candidate.dir.0))
        .map(|(position, _)| position)
}

/// Whether some child directory of `dir` holds at least [`DOMINANT_SHARE_PERCENT`] of its bytes.
fn has_dominant_child(tree: &Tree, dir: NodeId) -> bool {
    let total = u128::from(tree.allocated(dir));
    if total == 0 {
        return false;
    }

    child_dirs(tree, dir)
        .any(|child| u128::from(tree.allocated(child)) * 100 >= total * DOMINANT_SHARE_PERCENT)
}
