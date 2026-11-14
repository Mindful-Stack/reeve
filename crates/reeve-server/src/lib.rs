//! Composition root for the planner: configuration, state, auth, routes.
mod actions;
pub mod assets;
pub mod atomic_file;
pub mod auth;
pub mod config;
pub mod error;
pub mod export;
pub mod extract;
mod mcp;
pub mod ops;
pub mod routes;
pub mod state;
pub use atomic_file::write_atomically;
pub use auth::routes::MAX_PENDING;
pub use config::ServerConfig;
pub use export::{
    ExportSources, SnapshotId, check_migrations, connect_offline, export_document,
    export_document_in, export_text, offline_export, offline_options,
};
pub use mcp::mcp_startup_line;
pub use ops::{HEALTHCHECK_TIMEOUT, healthcheck, migration_listing};
pub use routes::{app, openapi_json};
pub use state::{
    AppState, PendingLogin, build_state, composed_migrator, pool_options, run_migrations,
};
