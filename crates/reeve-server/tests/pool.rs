//! A cancelled `begin` must not leave a pooled connection inside a transaction.
//!
//! sqlx 0.8.6's `PgTransactionManager::begin` sends `BEGIN …` and only then, after the reply, counts the transaction
//! (`transaction_depth += 1`). Its cancellation guard rolls back only when that depth is above zero, so a `begin` future
//! dropped while it waits for the reply (an axum handler cancelled because the browser aborted the request) returns the
//! connection to the pool with the transaction still open and sqlx unaware of it. When the statement was
//! `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` (story readiness), the next statement on that connection fixes a
//! snapshot (PostgreSQL 16 docs, 13.2.2: "a snapshot as of the start of the first non-transaction-control statement in
//! the transaction") and every later read on it keeps that snapshot: a session token issued afterwards does not exist
//! there, so `authenticate_api` answers 401 for one request in N while its siblings on other connections succeed. This
//! is the sporadic smoke 401 of CI run 40285287399.
use reeve_server::pool_options;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;

async fn pg() -> (reeve_testdb::TestDb, String) {
    let db = reeve_testdb::database(&reeve_server::composed_migrator()).await;
    let url = db.url.clone();
    (db, url)
}

/// Story readiness's BEGIN, then a statement that keeps the server busy after the transaction has started. sqlx's
/// `begin_with` sends its statement as one simple-protocol Query, which may hold several statements, and waits for
/// the final ReadyForQuery, so the reply it waits for arrives only after the sleep.
const SLOW_READINESS_BEGIN: &str =
    "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SELECT pg_sleep(0.5)";

/// Drops a `begin_with` of the readiness BEGIN on `app` after the server has run the BEGIN but before sqlx has read
/// the reply. The BEGIN is followed by a half-second sleep, and the future is dropped after 100 ms (the handler is
/// cancelled there): the server has started the transaction and is still sleeping, so sqlx never counted it. No race:
/// the window is half a second wide. Then waits, through `admin`, until the server has finished the sleep and the
/// connection sits idle in that transaction, which is the state a cancelled request leaves behind.
async fn cancel_begin_after_it_ran(app: &PgPool, admin: &PgPool) {
    sqlx::query("SELECT 1").execute(app).await.unwrap(); // one idle connection
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(app)
        .await
        .unwrap();
    let begin = tokio::time::timeout(
        Duration::from_millis(100),
        app.begin_with(SLOW_READINESS_BEGIN),
    )
    .await;
    assert!(
        begin.is_err(),
        "the begin must still be waiting for its reply"
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM pg_stat_activity WHERE pid = $1 AND query = $2")
                .bind(pid)
                .bind(SLOW_READINESS_BEGIN)
                .fetch_optional(admin)
                .await
                .unwrap()
                .flatten();
        if state.as_deref() == Some("idle in transaction") {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the cancelled begin's connection never went idle in transaction (state {state:?})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Poisons `app`'s only connection with a cancelled readiness BEGIN, lets one ordinary read fix the snapshot (a
/// request authenticating with an older session), signs in a new session through another connection, and returns
/// what `authenticate_api` answers for that new session on `app`.
async fn new_session_after_a_cancelled_begin(
    app: PgPool,
    admin: PgPool,
) -> Result<reeve_identity::Principal, reeve_identity::AuthError> {
    let cfg = reeve_identity::IdentityConfig {
        allowed_domains: vec!["example.com".into()],
        ..Default::default()
    };
    let admin_store = reeve_identity::make_token_store(admin.clone());
    let app_store = reeve_identity::make_token_store(app.clone());
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> =
        reeve_activity::make_activity_store(admin.clone());
    let who = || reeve_identity::VerifiedIdentity {
        email: "dan@example.com".into(),
        email_verified: true,
        name: None,
    };
    let now = chrono::Utc::now();
    let old = reeve_identity::sign_in(
        &admin,
        &*admin_store,
        &*log,
        &reeve_identity::AccessPolicy::floor_only(&cfg),
        who(),
        30,
        now,
    )
    .await
    .unwrap();

    cancel_begin_after_it_ran(&app, &admin).await;

    // An earlier request on the same connection: it succeeds, and (on a poisoned connection) fixes the snapshot.
    reeve_identity::authenticate_api(
        &*app_store,
        &reeve_identity::AccessPolicy::floor_only(&cfg),
        &old.secret,
        now,
    )
    .await
    .expect("the older session authenticates");
    let new = reeve_identity::sign_in(
        &admin,
        &*admin_store,
        &*log,
        &reeve_identity::AccessPolicy::floor_only(&cfg),
        who(),
        30,
        now,
    )
    .await
    .unwrap();
    reeve_identity::authenticate_api(
        &*app_store,
        &reeve_identity::AccessPolicy::floor_only(&cfg),
        &new.secret,
        now,
    )
    .await
}

#[tokio::test]
async fn a_cancelled_begin_does_not_hide_a_new_session_from_a_pooled_connection() {
    let (_c, url) = pg().await;
    let admin = PgPool::connect(&url).await.unwrap();
    let app = pool_options()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let got = new_session_after_a_cancelled_begin(app.clone(), admin).await;
    assert!(
        got.is_ok(),
        "a session signed in after a cancelled BEGIN must authenticate on every pooled connection: {got:?}"
    );
    // The connection the cancelled BEGIN left behind is not in a transaction any more: a write commits on its own.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens")
        .fetch_one(&app)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

/// The control: on a pool without the guard the same sequence answers 401. This proves the test above reaches the
/// window it is about. If sqlx fixes its `begin` cancellation (the guard counts the transaction before sending BEGIN,
/// or rolls back on status), this assertion starts failing: the guard in `pool_options` can then go.
#[tokio::test]
async fn control_sqlx_alone_leaves_the_transaction_open_and_the_new_session_unseen() {
    let (_c, url) = pg().await;
    let admin = PgPool::connect(&url).await.unwrap();
    let app = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let got = new_session_after_a_cancelled_begin(app, admin).await;
    assert!(
        matches!(got, Err(reeve_identity::AuthError::Unauthenticated)),
        "expected sqlx 0.8.6 to leave the snapshot behind (see the module docs): {got:?}"
    );
}

/// The guard costs no connection in the ordinary case: a clean connection is reused, not replaced.
#[tokio::test]
async fn a_clean_connection_is_reused() {
    let (_c, url) = pg().await;
    let app = pool_options()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let pid = || async {
        sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&app)
            .await
            .unwrap()
    };
    let first = pid().await;
    let mut tx = app.begin().await.unwrap();
    sqlx::query("SELECT 1").execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(pid().await, first);
    assert_eq!(pid().await, first);
}
