//! Read-only queries over a finished [`Tree`](crate::model::Tree). All return wire DTOs and fail only
//! with [`QueryError::NotFound`](crate::error::QueryError) for an unknown node id.

mod children;
mod find;
mod lod;
mod node;
mod path;
mod summary;

pub use children::{MAX_NAME_FILTER_BYTES, NameFilter, children, children_matching};
pub use find::{FolderMatch, MAX_FIND_QUERY_BYTES, MAX_FOLDER_MATCHES, find_folders};
pub use lod::{MAX_LOD_DEPTH, lod};
pub use node::node;
pub use path::path;
pub use summary::summary;

pub(crate) use node::{ensure, node_dto, wire_mtime};

#[cfg(test)]
mod tests;
