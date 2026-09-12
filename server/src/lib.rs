//! kube-docs-mcp: MCP server over a read-only SQLite index of Kubernetes/CNCF schemas,
//! annotations and reference docs. See `docs/index-format.md`.

pub mod db;
pub mod error;
pub mod index_updater;
pub mod mcp;
pub mod metrics;
pub mod schema;
pub mod search;
pub mod validate;
pub mod web;

pub use error::AppError;
