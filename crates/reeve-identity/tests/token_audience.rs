//! Tokens carry an audience (kb/ADR-0029 R5, R6): the upgrade of existing rows, the storage rules, issuing an MCP
//! token, and the two authenticate functions.
//!
//! The upgrade test seeds six representative rows through identity's own migrations below 2004 and upgrades to
//! 2004, not further. It cannot reproduce production's rows or the server's composed migrator; 2004 touches only
//! `tokens`, which identity owns. Its last assertion runs today's code against the 2004 schema.
mod common;
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use reeve_identity::{
    AccessPolicy, AuthError, IdentityActivity, IdentityConfig, IdentityEvent, IssuedToken,
    TokenKind, Via, authenticate, hash_secret, issue_mcp_token, issue_token, make_token_store,
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

/// An interactive API token for dan@example.com.
async fn issue_api(pool: &PgPool) -> IssuedToken {
    let store = make_token_store(pool.clone());
    issue_token(
        pool,
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
    .unwrap()
}

/// An MCP token for `email` that expires at `expires_at`.
async fn try_issue_mcp(
    pool: &PgPool,
    email: &str,
    expires_at: DateTime<Utc>,
) -> Result<IssuedToken, AuthError> {
    let store = make_token_store(pool.clone());
    issue_mcp_token(
        pool,
        &*store,
        &NoLog,
        &policy(),
        "test",
        Via::Interactive,
        email,
        "t",
        expires_at,
        now(),
    )
    .await
}

/// An MCP token for dan@example.com with the longest life allowed.
async fn issue_mcp(pool: &PgPool) -> IssuedToken {
    try_issue_mcp(pool, "dan@example.com", now() + Duration::hours(1))
        .await
        .unwrap()
}

/// Identity's migrations whose version `keep` accepts, as a migrator.
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

type Row = (
    i64,
    String,
    String,
    String,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);
const ROWS: &str = "SELECT id, owner, kind, hash, label, created_at, expires_at, revoked_at FROM tokens ORDER BY id";

#[tokio::test]
async fn migration_2004_keeps_existing_tokens_and_makes_them_api_tokens() {
    assert!(
        reeve_identity::migrator()
            .migrations
            .iter()
            .any(|m| m.version == 2004),
        "identity has migration 2004: it is what the fixture holds back"
    );
    let db = reeve_testdb::empty_database().await;
    let pool = db.pool.clone();
    // The schema production has when this change deploys.
    migrator_where(|v| v < 2004).run(&pool).await.unwrap();
    let has_column: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns WHERE table_name = 'tokens' AND column_name = 'audience'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(has_column, 0, "the rows below are seeded before 2004");
    let t = now();
    sqlx::query("INSERT INTO users (email, created_at) VALUES ('dan@example.com', $1)")
        .bind(t)
        .execute(&pool)
        .await
        .unwrap();
    // (label, kind, expires_at, revoked_at): live, expired and revoked, of both kinds.
    let seeds = [
        ("live-i", "interactive", None, None),
        ("live-a", "automation", None, None),
        (
            "expired-i",
            "interactive",
            Some(t - Duration::hours(1)),
            None,
        ),
        (
            "expired-a",
            "automation",
            Some(t - Duration::hours(1)),
            None,
        ),
        (
            "revoked-i",
            "interactive",
            None,
            Some(t - Duration::minutes(5)),
        ),
        (
            "revoked-a",
            "automation",
            Some(t + Duration::days(1)),
            Some(t - Duration::minutes(5)),
        ),
    ];
    for (label, kind, expires, revoked) in seeds {
        sqlx::query(
            "INSERT INTO tokens (owner, kind, hash, label, created_at, expires_at, revoked_at) VALUES ('dan@example.com', $1, $2, $3, $4, $5, $6)",
        )
        .bind(kind)
        .bind(hash_secret(&format!("reeve_{label}")))
        .bind(label)
        .bind(t - Duration::days(1))
        .bind(expires)
        .bind(revoked)
        .execute(&pool)
        .await
        .unwrap();
    }
    let before: Vec<Row> = sqlx::query_as(ROWS).fetch_all(&pool).await.unwrap();
    assert_eq!(before.len(), 6);

    // The upgrade: up to 2004 and no further, so a later identity migration does not change what this test proves.
    let up_to_2004 = || migrator_where(|v| v <= 2004);
    up_to_2004().run(&pool).await.unwrap();

    let after: Vec<Row> = sqlx::query_as(ROWS).fetch_all(&pool).await.unwrap();
    assert_eq!(before, after, "2004 changes no existing column");
    let audiences: Vec<String> = sqlx::query_scalar("SELECT audience FROM tokens ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(audiences, vec!["api"; 6]);
    let store = make_token_store(pool.clone());
    for (label, ..) in seeds {
        let got = authenticate(&*store, &policy(), &format!("reeve_{label}"), t).await;
        assert_eq!(got.is_ok(), label.starts_with("live"), "{label}: {got:?}");
    }
    // A second run changes nothing.
    up_to_2004().run(&pool).await.unwrap();
    let again: Vec<Row> = sqlx::query_as(ROWS).fetch_all(&pool).await.unwrap();
    assert_eq!(after, again);
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(applied as usize, up_to_2004().migrations.len());
}

#[tokio::test]
async fn an_insert_without_an_audience_fails() {
    let (_db, pool) = common::pg().await;
    sqlx::query("INSERT INTO users (email, created_at) VALUES ('dan@example.com', now())")
        .execute(&pool)
        .await
        .unwrap();
    let err = sqlx::query(
        "INSERT INTO tokens (owner, kind, hash, created_at) VALUES ('dan@example.com', 'interactive', 'h1', now())",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(db_code(&err), "23502", "{err}");
}

#[tokio::test]
async fn the_table_refuses_an_mcp_token_of_kind_automation_and_an_unknown_audience() {
    let (_db, pool) = common::pg().await;
    sqlx::query("INSERT INTO users (email, created_at) VALUES ('dan@example.com', now())")
        .execute(&pool)
        .await
        .unwrap();
    let insert = "INSERT INTO tokens (owner, kind, audience, hash, created_at) VALUES ('dan@example.com', $1, $2, $3, now())";
    // The controls: the three legal pairs are accepted.
    for (kind, audience) in [
        ("interactive", "api"),
        ("automation", "api"),
        ("interactive", "mcp"),
    ] {
        sqlx::query(insert)
            .bind(kind)
            .bind(audience)
            .bind(format!("ok-{kind}-{audience}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    for (kind, audience) in [("automation", "mcp"), ("interactive", "web")] {
        let err = sqlx::query(insert)
            .bind(kind)
            .bind(audience)
            .bind(format!("bad-{kind}-{audience}"))
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(db_code(&err), "23514", "{kind}/{audience}: {err}");
    }
}

#[tokio::test]
async fn the_audience_round_trips_through_the_store() {
    let (_db, pool) = common::pg().await;
    let store = make_token_store(pool.clone());
    for (issued, text) in [
        (issue_api(&pool).await, "api"),
        (issue_mcp(&pool).await, "mcp"),
    ] {
        let row = store
            .find_by_hash(&hash_secret(&issued.secret))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.audience, text);
        let stored: String = sqlx::query_scalar("SELECT audience FROM tokens WHERE id = $1")
            .bind(issued.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, text);
    }
}

#[tokio::test]
async fn an_mcp_token_is_interactive_and_lives_at_most_one_hour() {
    let (_db, pool) = common::pg().await;
    // Exactly one hour is accepted, and is stored as an interactive token that expires then.
    let hour = now() + Duration::hours(1);
    let ok = try_issue_mcp(&pool, "dan@example.com", hour).await.unwrap();
    let (kind, expires): (String, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT kind, expires_at FROM tokens WHERE id = $1")
            .bind(ok.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), expires), ("interactive", Some(hour)));
    // A second more is refused, and so are an expiry that is not in the future and an email outside the allowlist.
    for (email, expires_at) in [
        ("dan@example.com", hour + Duration::seconds(1)),
        ("dan@example.com", now() + Duration::days(1)),
        ("dan@example.com", now()),
    ] {
        assert!(
            matches!(
                try_issue_mcp(&pool, email, expires_at).await,
                Err(AuthError::Validation(_))
            ),
            "{expires_at}"
        );
    }
    assert!(matches!(
        try_issue_mcp(&pool, "mallory@evil.example", hour).await,
        Err(AuthError::NotAllowed)
    ));
    let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tokens, 1, "a refused issue writes nothing");
}
