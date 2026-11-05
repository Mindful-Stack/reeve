use reeve_testdb::TestDb;
use sqlx::PgPool;

/// A database of its own, migrated by this crate's migrator (reeve-testdb: a clone on the run's server, or a
/// container of its own). Keep the `TestDb` alive as long as the pool.
pub async fn pg() -> (TestDb, PgPool) {
    let db = reeve_testdb::database(&reeve_activity::migrator()).await;
    let pool = db.pool.clone();
    (db, pool)
}
