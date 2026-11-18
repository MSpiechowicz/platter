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
/// allocated bytes, so a path like `/home` > `/home/user` lists only the deeper levels. A directory that
/// itself holds at least this share of the summarized node is replaced too when its non-empty child
/// directories hold at least [`SUBFOLDER_SHARE_PERCENT`] of it (a list headed by one folder tells nothing).
const DOMINANT_SHARE_PERCENT: u128 = 80;
/// The share of a folder that its non-empty child directories must hold for a folder holding most of the
/// node to be listed through them. A folder that is mostly loose files stays.
const SUBFOLDER_SHARE_PERCENT: u128 = 50;

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
/// 2. A candidate is replaced by its directory children, again and again, down to [`MAX_DEPTH`] levels,
///    when it has a child directory that holds at least [`DOMINANT_SHARE_PERCENT`] of its bytes (the
///    dominated parent tells nothing its children do not), or when it holds at least that share of the
///    bytes of `id` and its non-empty child directories hold at least [`SUBFOLDER_SHARE_PERCENT`] of it
///    (it is most of what is being summarized, so its contents are what the list should show). A big
///    folder that is mostly loose files is kept, with or without a small subfolder.
/// 3. While there are fewer than eight candidates, the largest candidate that can grow the list (it has at
///    least two directory children, and is not at the depth limit) is replaced by its directory children,
///    and step 2 is applied to the result. This stops when nothing can be expanded.
///
/// Empty directories and mount points are never candidates. The eight largest candidates are returned,
/// largest first; on equal size the lower id wins, so the result is deterministic.
fn largest_dirs(tree: &Tree, id: NodeId) -> Vec<NodeId> {
    let whole = u128::from(tree.allocated(id));
    let mut candidates: Vec<Candidate> = child_dirs(tree, id)
        .map(|dir| Candidate { dir, depth: 1 })
        .collect();

    settle_dominated(tree, &mut candidates, whole);

    while candidates.len() < LARGEST_DIRS {
        let Some(position) = largest_expandable(tree, &candidates) else {
            break;
        };

        let parent = candidates.remove(position);
        candidates.extend(child_dirs(tree, parent.dir).map(|dir| Candidate {
            dir,
            depth: parent.depth + 1,
        }));
        settle_dominated(tree, &mut candidates, whole);
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

/// Replaces every candidate that is dominated by one of its child directories, or that holds at least
/// [`DOMINANT_SHARE_PERCENT`] of `whole` (the bytes of the summarized node) with its non-empty child
/// directories holding [`SUBFOLDER_SHARE_PERCENT`] of it, with its child directories, repeatedly, as long
/// as the depth limit allows.
fn settle_dominated(tree: &Tree, candidates: &mut Vec<Candidate>, whole: u128) {
    loop {
        let mut changed = false;
        let mut next = Vec::with_capacity(candidates.len());

        for &candidate in candidates.iter() {
            if candidate.depth < MAX_DEPTH && is_settled_away(tree, candidate.dir, whole) {
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

/// Whether `dir` is listed through its child directories instead of itself.
fn is_settled_away(tree: &Tree, dir: NodeId, whole: u128) -> bool {
    if has_dominant_child(tree, dir) {
        return true;
    }

    // A folder holding most of what is summarized is listed through its folders, unless it is mostly
    // loose files: then its folders are less than half of it and it stays (a 100 GB folder of videos with
    // one empty subfolder is not replaced by that subfolder).
    let size = u128::from(tree.allocated(dir));
    let holds_most_of_whole = size * 100 >= whole * DOMINANT_SHARE_PERCENT;
    let in_folders: u128 = child_dirs(tree, dir)
        .map(|child| u128::from(tree.allocated(child)))
        .sum();

    holds_most_of_whole && in_folders > 0 && in_folders * 100 >= size * SUBFOLDER_SHARE_PERCENT
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
