//! Tokens carry an audience (kb/ADR-0029 R5, R6): the upgrade of existing rows, the storage rules, issuing an MCP
//! token, and the two authenticate functions.
//!
//! The upgrade test seeds six representative rows through identity's own migrations below 2004 and upgrades to
//! 2004, not further. It cannot reproduce production's rows or the server's composed migrator; 2004 touches only
//! `tokens`, which identity owns. Its last assertion applies the rest of the migrations and runs today's code against
//! today's schema.
mod common;
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use reeve_identity::{
    AccessPolicy, AuthError, IdentityActivity, IdentityConfig, IdentityEvent, IssuedToken, Role,
    TokenKind, Via, authenticate_api, authenticate_mcp, disable_user, hash_secret, issue_mcp_token,
    issue_token, list_tokens, make_token_store, revoke_token,
};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

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

/// Records what identity logs, as the issuance tests in `tokens.rs` do.
struct MemActivity(Mutex<Vec<IdentityEvent>>);
#[async_trait]
impl IdentityActivity for MemActivity {
    async fn record(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        e: IdentityEvent,
    ) -> Result<(), sqlx::Error> {
        self.0.lock().unwrap().push(e);
        Ok(())
    }
}

/// An activity log whose write fails.
struct Failing;
#[async_trait]
impl IdentityActivity for Failing {
    async fn record(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _e: IdentityEvent,
    ) -> Result<(), sqlx::Error> {
        Err(sqlx::Error::Protocol("boom".into()))
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
    // A second run changes nothing.
    up_to_2004().run(&pool).await.unwrap();
    let again: Vec<Row> = sqlx::query_as(ROWS).fetch_all(&pool).await.unwrap();
    assert_eq!(after, again);
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(applied as usize, up_to_2004().migrations.len());
    // Today's code reads today's schema: the rest of the migrations, then the rows 2004 kept still authenticate.
    reeve_identity::migrator().run(&pool).await.unwrap();
    let store = make_token_store(pool.clone());
    for (label, ..) in seeds {
        let got = authenticate_api(&*store, &policy(), &format!("reeve_{label}"), t).await;
        assert_eq!(got.is_ok(), label.starts_with("live"), "{label}: {got:?}");
    }
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
    // An MCP token needs a connection (migration 2005), so the MCP rows get one: the refusal of `automation`/`mcp`
    // is then still the kind rule's.
    let connection: i64 = sqlx::query_scalar(
        "INSERT INTO connections (owner, client_id, client_name, scope, approved_at, ends_at) VALUES ('dan@example.com', 'urn:test', 'Test', 'read', now(), now() + interval '30 days') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let insert = "INSERT INTO tokens (owner, kind, audience, hash, created_at, connection_id) VALUES ('dan@example.com', $1, $2, $3, now(), $4)";
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
            .bind((audience == "mcp").then_some(connection))
            .execute(&pool)
            .await
            .unwrap();
    }
    for (kind, audience) in [("automation", "mcp"), ("interactive", "web")] {
        let err = sqlx::query(insert)
            .bind(kind)
            .bind(audience)
            .bind(format!("bad-{kind}-{audience}"))
            .bind((audience == "mcp").then_some(connection))
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(db_code(&err), "23514", "{kind}/{audience}: {err}");
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
    for expires_at in [
        hour + Duration::seconds(1),
        now() + Duration::days(1),
        now(),
    ] {
        assert!(
            matches!(
                try_issue_mcp(&pool, "dan@example.com", expires_at).await,
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

/// What the code logged while the guard is alive, as text.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
    type Writer = Logs;
    fn make_writer(&'a self) -> Logs {
        self.clone()
    }
}
impl Logs {
    fn capture() -> (Logs, tracing::subscriber::DefaultGuard) {
        let logs = Logs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .finish();
        (logs, tracing::subscriber::set_default(subscriber))
    }
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

#[tokio::test]
async fn each_audience_authenticates_only_through_its_own_function() {
    let (_db, pool) = common::pg().await;
    let store = make_token_store(pool.clone());
    let api = issue_api(&pool).await;
    let mcp = issue_mcp(&pool).await;
    let (logs, _guard) = Logs::capture();

    let p = authenticate_api(&*store, &policy(), &api.secret, now())
        .await
        .unwrap();
    assert_eq!(
        (p.email.as_str(), p.role, p.token_kind),
        ("dan@example.com", Role::Admin, TokenKind::Interactive)
    );
    let m = authenticate_mcp(&*store, &policy(), &mcp.secret, now())
        .await
        .unwrap();
    assert_eq!(
        (m.email.as_str(), m.role, m.client_id.as_str(), m.scope),
        (
            "dan@example.com",
            Role::Admin,
            "urn:reeve:developer-command",
            reeve_identity::Scope::Write
        )
    );
    assert!(
        !logs.text().contains("authentication refused"),
        "an accepted token logs no refusal: {}",
        logs.text()
    );

    assert!(matches!(
        authenticate_api(&*store, &policy(), &mcp.secret, now()).await,
        Err(AuthError::Unauthenticated)
    ));
    assert!(matches!(
        authenticate_mcp(&*store, &policy(), &api.secret, now()).await,
        Err(AuthError::Unauthenticated)
    ));
    let text = logs.text();
    assert_eq!(text.matches("wrong audience").count(), 2, "{text}");
    assert!(
        !text.contains(&api.secret) && !text.contains(&mcp.secret),
        "{text}"
    );
}

#[tokio::test]
async fn the_owners_listing_and_single_revocation_see_api_tokens_only_and_disabling_sees_all() {
    let (_db, pool) = common::pg().await;
    let store = make_token_store(pool.clone());
    let api = issue_api(&pool).await;
    let mcp = issue_mcp(&pool).await;
    let listed: Vec<i64> = list_tokens(&*store, "dan@example.com")
        .await
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(listed, vec![api.id]);
    let changed = revoke_token(
        &pool,
        &*store,
        &NoLog,
        "dan@example.com",
        Via::Interactive,
        mcp.id,
        now(),
    )
    .await
    .unwrap();
    assert!(
        !changed,
        "an MCP token is not found by the token page's revocation"
    );
    assert!(
        authenticate_mcp(&*store, &policy(), &mcp.secret, now())
            .await
            .is_ok()
    );
    // Disabling covers every audience (kb/ADR-0015 R6).
    disable_user(
        &pool,
        &*store,
        &NoLog,
        "operator",
        Via::System,
        "dan@example.com",
        now(),
    )
    .await
    .unwrap();
    assert!(
        authenticate_mcp(&*store, &policy(), &mcp.secret, now())
            .await
            .is_err()
    );
    assert!(
        authenticate_api(&*store, &policy(), &api.secret, now())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn issue_mcp_token_refuses_a_disabled_user_as_issue_token_does() {
    let (_db, pool) = common::pg().await;
    let store = make_token_store(pool.clone());
    disable_user(
        &pool,
        &*store,
        &NoLog,
        "operator",
        Via::System,
        "new@example.com",
        now(),
    )
    .await
    .unwrap();
    assert!(matches!(
        issue_token(
            &pool,
            &*store,
            &NoLog,
            &policy(),
            "operator",
            Via::System,
            "New@example.com",
            TokenKind::Interactive,
            "t",
            None,
            now(),
        )
        .await,
        Err(AuthError::Disabled)
    ));
    assert!(matches!(
        try_issue_mcp(&pool, "New@example.com", now() + Duration::hours(1)).await,
        Err(AuthError::Disabled)
    ));
    let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tokens, 0);
}

#[tokio::test]
async fn each_issue_records_token_issued_and_only_the_mcp_entry_names_an_audience() {
    let (_db, pool) = common::pg().await;
    let store = make_token_store(pool.clone());
    let log = MemActivity(Mutex::new(vec![]));
    let api = issue_token(
        &pool,
        &*store,
        &log,
        &policy(),
        "operator",
        Via::System,
        "Dan@Example.com",
        TokenKind::Automation,
        "ci",
        None,
        now(),
    )
    .await
    .unwrap();
    let mcp = issue_mcp_token(
        &pool,
        &*store,
        &log,
        &policy(),
        "operator",
        Via::System,
        "Dan@Example.com",
        "claude",
        now() + Duration::hours(1),
        now(),
    )
    .await
    .unwrap();
    let events = log.0.lock().unwrap();
    assert_eq!(events.len(), 2);
    for e in events.iter() {
        assert_eq!(
            (e.at, e.actor.as_str(), e.subject.as_str(), e.action),
            (now(), "operator", "dan@example.com", "token_issued")
        );
        assert!(matches!(e.via, Via::System));
    }
    assert_eq!(
        events[0].payload,
        serde_json::json!({"kind": "automation", "label": "ci", "token_id": api.id})
    );
    assert_eq!(
        events[1].payload,
        serde_json::json!({"kind": "interactive", "audience": "mcp", "label": "claude", "token_id": mcp.id})
    );
}

#[tokio::test]
async fn a_failed_activity_write_rolls_back_the_mcp_issue() {
    let (_db, pool) = common::pg().await;
    let store = make_token_store(pool.clone());
    assert!(
        issue_mcp_token(
            &pool,
            &*store,
            &Failing,
            &policy(),
            "operator",
            Via::Interactive,
            "dan@example.com",
            "t",
            now() + Duration::hours(1),
            now(),
        )
        .await
        .is_err()
    );
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 0);
    let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tokens, 0);
}
