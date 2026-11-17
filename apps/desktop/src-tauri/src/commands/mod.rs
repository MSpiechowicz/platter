//! IPC commands, one module per concern. Each is a few lines over `platter-core` / `platter-platform`.

pub mod advisor;
pub mod cleanup;
pub mod history;
pub mod query;
pub mod rules;
pub mod scan;
pub mod suggestions;
pub mod system;
pub mod volumes;
