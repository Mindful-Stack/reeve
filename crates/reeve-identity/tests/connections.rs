//! Connections (kb/ADR-0029 R5, R8, R9): migration 2005 on a database seeded after 2004, what the tables refuse,
//! the developer command's connection, and what `authenticate_mcp` requires of a token's connection.
//!
//! The migration test seeds representative rows through identity's own migrations up to 2004 and upgrades to
//! 2005, not further. It cannot reproduce production's rows.
mod common;
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use reeve_identity::{
    AccessPolicy, AuthError, IdentityActivity, IdentityConfig, IdentityEvent, IssuedToken, Role,
    Scope, TokenKind, Via, authenticate_api, authenticate_mcp, hash_secret, issue_mcp_token,
    issue_token, make_token_store,
};
use sqlx::PgPool;

struct NoLog;
#[async_trait]
impl IdentityActivity for NoLog {
    async fn record(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _e: IdentityEvent,
    ) -> Result<(), sqlx::Error> {
        Ok(())
    }
}

fn policy() -> AccessPolicy {
    AccessPolicy::floor_only(&IdentityConfig {
        allowed_domains: vec!["example.com".into()],
        allowed_emails: vec![],
        admins: vec!["dan@example.com".into()],
    })
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 14, 9, 0, 0).unwrap()
}

fn db_code(e: &sqlx::Error) -> String {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
        .unwrap_or_default()
}

fn migrator_where(keep: impl Fn(i64) -> bool) -> sqlx::migrate::Migrator {
    let mut m = reeve_identity::migrator();
    let kept: Vec<_> = m
        .migrations
        .iter()
        .filter(|m| keep(m.version))
        .cloned()
        .collect();
    m.migrations = std::borrow::Cow::Owned(kept);
    m
}

async fn developer_token(pool: &PgPool, email: &str) -> IssuedToken {
    let store = make_token_store(pool.clone());
    issue_mcp_token(
        pool,
        &*store,
        &NoLog,
        &policy(),
        "operator",
        Via::System,
        email,
        "mcp developer token",
        now() + Duration::hours(1),
        now(),
    )
    .await
    .unwrap()
}

type ApiRow = (
    String,
    String,
    String,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);
const API_ROWS: &str = "SELECT label, kind, hash, expires_at, revoked_at FROM tokens WHERE audience = 'api' ORDER BY id";

/// Catches a migration that keeps a developer MCP token (it would have no connection), one that touches an API
/// token, and code that cannot read an API token afterwards (its `connection_id` is NULL in a two-column
/// foreign key).
#[tokio::test]
async fn migration_2005_deletes_the_developer_mcp_tokens_and_leaves_api_tokens_alone() {
    assert!(
        reeve_identity::migrator()
            .migrations
            .iter()
            .any(|m| m.version == 2005)
    );
    let db = reeve_testdb::empty_database().await;
    let pool = db.pool.clone();
    migrator_where(|v| v <= 2004).run(&pool).await.unwrap();
    let t = now();
    sqlx::query("INSERT INTO users (email, created_at) VALUES ('dan@example.com', $1)")
        .bind(t)
        .execute(&pool)
        .await
        .unwrap();
    // (label, kind, audience, expires_at, revoked_at): live, expired and revoked, of both audiences.
    let seeds = [
        ("live-api", "interactive", "api", None, None),
        ("bot-api", "automation", "api", None, None),
        (
            "expired-api",
            "interactive",
            "api",
            Some(t - Duration::hours(1)),
            None,
        ),
        (
            "revoked-api",
            "interactive",
            "api",
            None,
            Some(t - Duration::minutes(5)),
        ),
        (
            "live-mcp",
            "interactive",
            "mcp",
            Some(t + Duration::minutes(30)),
            None,
        ),
        (
            "expired-mcp",
            "interactive",
            "mcp",
            Some(t - Duration::hours(1)),
            None,
        ),
        (
            "revoked-mcp",
            "interactive",
            "mcp",
            Some(t + Duration::minutes(30)),
            Some(t - Duration::minutes(5)),
        ),
    ];
    for (label, kind, audience, expires, revoked) in seeds {
        sqlx::query("INSERT INTO tokens (owner, kind, audience, hash, label, created_at, expires_at, revoked_at) VALUES ('dan@example.com', $1, $2, $3, $4, $5, $6, $7)")
            .bind(kind).bind(audience).bind(hash_secret(&format!("reeve_{label}"))).bind(label)
            .bind(t - Duration::days(1)).bind(expires).bind(revoked)
            .execute(&pool).await.unwrap();
    }
    let before: Vec<ApiRow> = sqlx::query_as(API_ROWS).fetch_all(&pool).await.unwrap();
    assert_eq!(before.len(), 4);

    migrator_where(|v| v <= 2005).run(&pool).await.unwrap();

    let after: Vec<ApiRow> = sqlx::query_as(API_ROWS).fetch_all(&pool).await.unwrap();
    assert_eq!(before, after, "2005 changes no API token");
    let (mcp, with_connection): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE audience = 'mcp'), count(*) FILTER (WHERE connection_id IS NOT NULL) FROM tokens",
    )
    .fetch_one(&pool).await.unwrap();
    assert_eq!(
        (mcp, with_connection),
        (0, 0),
        "the developer tokens are gone"
    );
    // Today's code reads the migrated rows: an API token with a NULL connection authenticates.
    let store = make_token_store(pool.clone());
    let who = authenticate_api(&*store, &policy(), "reeve_live-api", t)
        .await
        .unwrap();
    assert_eq!(
        (who.email.as_str(), who.token_kind),
        ("dan@example.com", TokenKind::Interactive)
    );
    assert!(
        authenticate_mcp(&*store, &policy(), "reeve_live-mcp", t)
            .await
            .is_err()
    );
    // And the command makes a new one that works.
    let fresh = developer_token(&pool, "dan@example.com").await;
    assert!(
        authenticate_mcp(&*store, &policy(), &fresh.secret, t)
            .await
            .is_ok()
    );
}

/// What the tables refuse, each with the database's own error code, and each beside a control that is accepted
/// so that a refusal is the rule's and not a broken statement's.
#[tokio::test]
async fn the_tables_refuse_what_the_rules_forbid() {
    let (_db, pool) = common::pg().await;
    for email in ["dan@example.com", "bob@example.com"] {
        sqlx::query("INSERT INTO users (email, created_at) VALUES ($1, $2)")
            .bind(email)
            .bind(now())
            .execute(&pool)
            .await
            .unwrap();
    }
    let connection = "INSERT INTO connections (owner, client_id, client_name, scope, approved_at, ends_at) VALUES ('dan@example.com', 'https://client.example/app.json', 'Example App', $1, $2, $3) RETURNING id";
    let insert = |scope: &'static str, ends: DateTime<Utc>| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(connection)
                .bind(scope)
                .bind(now())
                .bind(ends)
                .fetch_one(&pool)
                .await
        }
    };
    let dans = insert("read", now() + Duration::days(30)).await.unwrap();
    assert_eq!(
        db_code(
            &insert("admin", now() + Duration::days(30))
                .await
                .unwrap_err()
        ),
        "23514",
        "scope"
    );
    assert_eq!(
        db_code(&insert("read", now()).await.unwrap_err()),
        "23514",
        "ends_at must be after approved_at"
    );

    // A token: `mcp` exactly when it has a connection, and the connection is its owner's.
    let token = "INSERT INTO tokens (owner, kind, audience, hash, created_at, connection_id) VALUES ($1, $2, $3, $4, $5, $6)";
    let try_token = |owner: &'static str,
                     kind: &'static str,
                     audience: &'static str,
                     hash: &'static str,
                     conn: Option<i64>| {
        let pool = pool.clone();
        async move {
            sqlx::query(token)
                .bind(owner)
                .bind(kind)
                .bind(audience)
                .bind(hash)
                .bind(now())
                .bind(conn)
                .execute(&pool)
                .await
        }
    };
    try_token("dan@example.com", "interactive", "api", "ok-api", None)
        .await
        .unwrap();
    try_token(
        "dan@example.com",
        "interactive",
        "mcp",
        "ok-mcp",
        Some(dans),
    )
    .await
    .unwrap();
    for (what, owner, kind, audience, conn, code) in [
        (
            "an MCP token without a connection",
            "dan@example.com",
            "interactive",
            "mcp",
            None,
            "23514",
        ),
        (
            "an API token with a connection",
            "dan@example.com",
            "interactive",
            "api",
            Some(dans),
            "23514",
        ),
        (
            "an automation MCP token",
            "dan@example.com",
            "automation",
            "mcp",
            Some(dans),
            "23514",
        ),
        (
            "a token on another person's connection",
            "bob@example.com",
            "interactive",
            "mcp",
            Some(dans),
            "23503",
        ),
    ] {
        let err = try_token(owner, kind, audience, "bad", conn)
            .await
            .unwrap_err();
        assert_eq!(db_code(&err), code, "{what}: {err}");
    }

    // One live refresh token per connection; used ones do not count.
    let refresh = "INSERT INTO refresh_tokens (connection_id, hash, created_at, used_at) VALUES ($1, $2, $3, $4)";
    let try_refresh = |hash: &'static str, used: Option<DateTime<Utc>>| {
        let pool = pool.clone();
        async move {
            sqlx::query(refresh)
                .bind(dans)
                .bind(hash)
                .bind(now())
                .bind(used)
                .execute(&pool)
                .await
        }
    };
    try_refresh("r-used", Some(now())).await.unwrap();
    try_refresh("r-live", None).await.unwrap();
    assert_eq!(
        db_code(&try_refresh("r-live-2", None).await.unwrap_err()),
        "23505"
    );

    // Revoked at and why are set together or not at all, and the reason is one of four.
    for (what, set) in [
        ("only revoked_at", "revoked_at = '2026-11-14T09:00:00Z'"),
        ("only a reason", "revoked_reason = 'revoked'"),
        (
            "an unknown reason",
            "revoked_at = '2026-11-14T09:00:00Z', revoked_reason = 'bored'",
        ),
    ] {
        let err = sqlx::query(&format!("UPDATE connections SET {set} WHERE id = $1"))
            .bind(dans)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(db_code(&err), "23514", "{what}: {err}");
    }
    for reason in [
        "revoked",
        "refresh_token_reused",
        "user_disabled",
        "restored",
    ] {
        sqlx::query("UPDATE connections SET revoked_at = $2, revoked_reason = $3 WHERE id = $1")
            .bind(dans)
            .bind(now())
            .bind(reason)
            .execute(&pool)
            .await
            .unwrap();
    }

    // Deleting a connection deletes its tokens and refresh tokens, and nothing else.
    sqlx::query("DELETE FROM connections WHERE id = $1")
        .bind(dans)
        .execute(&pool)
        .await
        .unwrap();
    let left: Vec<String> = sqlx::query_scalar("SELECT hash FROM tokens ORDER BY hash")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, ["ok-api"]);
    let refresh_left: i64 = sqlx::query_scalar("SELECT count(*) FROM refresh_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(refresh_left, 0);
}

/// Catches a developer token without a connection of its own, a connection that is not the fixed developer
/// client with scope write and thirty days, and a principal that does not carry them.
#[tokio::test]
async fn the_developer_command_makes_a_connection_and_the_principal_carries_it() {
    let (_db, pool) = common::pg().await;
    let store = make_token_store(pool.clone());
    let first = developer_token(&pool, "dan@example.com").await;
    let second = developer_token(&pool, "dan@example.com").await;
    type Row = (
        i64,
        String,
        String,
        String,
        String,
        DateTime<Utc>,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT c.id, c.owner, c.client_id, c.client_name, c.scope, c.approved_at, c.ends_at, c.revoked_at FROM connections c JOIN tokens t ON t.connection_id = c.id WHERE t.id = ANY($1) ORDER BY c.id",
    )
    .bind(vec![first.id, second.id]).fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 2, "each issue makes its own connection");
    assert_ne!(rows[0].0, rows[1].0);
    for r in &rows {
        assert_eq!(
            (
                r.1.as_str(),
                r.2.as_str(),
                r.3.as_str(),
                r.4.as_str(),
                r.5,
                r.6,
                r.7
            ),
            (
                "dan@example.com",
                "urn:reeve:developer-command",
                "Developer command",
                "write",
                now(),
                now() + Duration::days(30),
                None
            )
        );
    }
    let who = authenticate_mcp(&*store, &policy(), &first.secret, now())
        .await
        .unwrap();
    assert_eq!(
        (
            who.email.as_str(),
            who.role,
            who.connection_id,
            who.client_id.as_str(),
            who.scope
        ),
        (
            "dan@example.com",
            Role::Admin,
            rows[0].0,
            "urn:reeve:developer-command",
            Scope::Write
        )
    );
}

/// kb/ADR-0029 R8: the connection is checked on every call. Catches a token that outlives its connection's
/// revocation or its end, and an end boundary on the wrong side. Each refusal follows an accepted call with the
/// same token, so it is the connection's state that refuses.
#[tokio::test]
async fn a_token_stops_when_its_connection_is_revoked_or_has_ended() {
    let (_db, pool) = common::pg().await;
    let store = make_token_store(pool.clone());
    let auth = |secret: String, at: DateTime<Utc>| {
        let store = store.clone();
        async move { authenticate_mcp(&*store, &policy(), &secret, at).await }
    };
    let revoked = developer_token(&pool, "dan@example.com").await;
    assert!(auth(revoked.secret.clone(), now()).await.is_ok());
    sqlx::query("UPDATE connections SET revoked_at = $2, revoked_reason = 'revoked' WHERE id = (SELECT connection_id FROM tokens WHERE id = $1)")
        .bind(revoked.id).bind(now()).execute(&pool).await.unwrap();
    assert!(matches!(
        auth(revoked.secret.clone(), now()).await,
        Err(AuthError::Unauthenticated)
    ));

    // Ended: the connection ends in ten minutes although the token's own hour is not over.
    let ended = developer_token(&pool, "dan@example.com").await;
    sqlx::query("UPDATE connections SET approved_at = $2, ends_at = $3 WHERE id = (SELECT connection_id FROM tokens WHERE id = $1)")
        .bind(ended.id).bind(now() - Duration::days(30)).bind(now() + Duration::minutes(10)).execute(&pool).await.unwrap();
    assert!(
        auth(
            ended.secret.clone(),
            now() + Duration::minutes(10) - Duration::seconds(1)
        )
        .await
        .is_ok()
    );
    assert!(matches!(
        auth(ended.secret.clone(), now() + Duration::minutes(10)).await,
        Err(AuthError::Unauthenticated)
    ));
    // An API token is untouched by all of this.
    let api = issue_token(
        &pool,
        &*store,
        &NoLog,
        &policy(),
        "test",
        Via::Interactive,
        "dan@example.com",
        TokenKind::Interactive,
        "t",
        None,
        now(),
    )
    .await
    .unwrap();
    assert!(
        authenticate_api(&*store, &policy(), &api.secret, now())
            .await
            .is_ok()
    );
}
