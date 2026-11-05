//! The append-only activity log: written inside the caller's transaction, read by the feed.
mod domain;
mod repo;
pub use domain::{ActivityEntry, ActivityFilter, Via};
pub use repo::{ActivityError, ActivityReader, ActivityStore, ActivityWriter, make_activity_store};
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}
