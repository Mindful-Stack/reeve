use reeve_testdb::TestDb;
use sqlx::PgPool;

/// A database of its own, migrated by this crate's migrator. Keep the `TestDb` alive as long as the pool.
pub async fn pg() -> (TestDb, PgPool) {
    let db = reeve_testdb::database(&reeve_planning::migrator()).await;
    let pool = db.pool.clone();
    (db, pool)
}

/// An empty database of its own, for a test that runs a partial migrator itself.
// Unused in one of the two test binaries that include this module.
#[allow(dead_code)]
pub async fn pg_unmigrated() -> (TestDb, PgPool) {
    let db = reeve_testdb::empty_database().await;
    let pool = db.pool.clone();
    (db, pool)
}

/// Like `pg()`, with `pg_stat_statements` created in this database, for tests that count the statements a call
/// sends. Every server reeve-testdb uses preloads it with room for 100 000 entries; a server configured otherwise
/// fails here rather than undercounting.
// Unused in one of the two test binaries that include this module.
#[allow(dead_code)]
pub async fn pg_counted() -> (TestDb, PgPool) {
    let (db, pool) = pg().await;
    let loaded: String = sqlx::query_scalar("SHOW shared_preload_libraries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        loaded.contains("pg_stat_statements"),
        "pg_stat_statements must be preloaded, got {loaded:?}"
    );
    let max: String = sqlx::query_scalar("SHOW pg_stat_statements.max")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        max.parse::<i64>().unwrap() >= 100_000,
        "pg_stat_statements.max must leave room for every test database's entries, got {max}"
    );
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .unwrap();
    (db, pool)
}

/// The number of statements `f` sends to this database: reset, run, sum the calls. The reset's own entry is counted
/// on every measurement alike, so compare two measurements with each other, never with an absolute number.
/// `f` must unwrap its results itself (it returns `()`), and must end every transaction it opens (commit or
/// rollback): a dropped transaction's ROLLBACK is sent after `f` returns and may land in a later measurement.
/// The reset clears this database's entries only, so tests counting in other databases are undisturbed.
// Unused in one of the two test binaries that include this module.
#[allow(dead_code)]
pub async fn statements<F, Fut>(pool: &PgPool, f: F) -> i64
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    sqlx::query(
        "SELECT pg_stat_statements_reset(0::oid, \
         (SELECT oid FROM pg_database WHERE datname = current_database()), 0::bigint)",
    )
    .execute(pool)
    .await
    .unwrap();
    f().await;
    sqlx::query_scalar(
        "SELECT coalesce(sum(calls), 0)::bigint FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}
