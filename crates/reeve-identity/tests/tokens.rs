mod common;
use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use reeve_identity::{
    AccessPolicy, AuthError, IdentityActivity, IdentityConfig, IdentityEvent, Role, TokenKind, Via,
    authenticate_api, disable_user, hash_secret, issue_token, revoke_all_for,
};
use std::sync::Mutex;

/// Records what identity logs; the transactional write itself is proven by reeve-activity's repository tests.
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

fn cfg() -> IdentityConfig {
    IdentityConfig {
        allowed_domains: vec!["example.com".into()],
        allowed_emails: vec!["guest@other.org".into()],
        admins: vec!["dan@example.com".into()],
    }
}

#[tokio::test]
async fn issue_then_authenticate_resolves_role_and_kind() {
    let (_c, pool) = common::pg().await;
    let store = reeve_identity::make_token_store(pool.clone());
    let log = MemActivity(Mutex::new(vec![]));
    let now = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    let issued = issue_token(
        &pool,
        &*store,
        &log,
        &AccessPolicy::floor_only(&cfg()),
        "operator",
        Via::Interactive,
        "dan@example.com",
        TokenKind::Automation,
        "lorekeeper",
        None,
        now,
    )
    .await
    .unwrap();
    assert_eq!(log.0.lock().unwrap()[0].action, "token_issued");
    assert!(
        issued.secret.starts_with("reeve_"),
        "secrets carry a recognisable prefix"
    );
    let p = authenticate_api(
        &*store,
        &AccessPolicy::floor_only(&cfg()),
        &issued.secret,
        now,
    )
    .await
    .unwrap();
    assert_eq!(p.email, "dan@example.com");
    assert_eq!(p.role, Role::Admin);
    assert_eq!(p.token_kind, TokenKind::Automation);
    let member = issue_token(
        &pool,
        &*store,
        &log,
        &AccessPolicy::floor_only(&cfg()),
        "operator",
        Via::Interactive,
        "guest@other.org",
        TokenKind::Interactive,
        "cli",
        None,
        now,
    )
    .await
    .unwrap();
    assert_eq!(
        authenticate_api(
            &*store,
            &AccessPolicy::floor_only(&cfg()),
            &member.secret,
            now
        )
        .await
        .unwrap()
        .role,
        Role::Member
    );
}

#[tokio::test]
async fn refuses_unknown_secret_expired_revoked_and_disallowed() {
    let (_c, pool) = common::pg().await;
    let store = reeve_identity::make_token_store(pool.clone());
    let log = MemActivity(Mutex::new(vec![]));
    let now = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    assert!(matches!(
        authenticate_api(
            &*store,
            &AccessPolicy::floor_only(&cfg()),
            "reeve_nope",
            now
        )
        .await,
        Err(AuthError::Unauthenticated)
    ));
    let short = issue_token(
        &pool,
        &*store,
        &log,
        &AccessPolicy::floor_only(&cfg()),
        "operator",
        Via::Interactive,
        "dan@example.com",
        TokenKind::Interactive,
        "cli",
        Some(now + Duration::hours(1)),
        now,
    )
    .await
    .unwrap();
    assert!(matches!(
        authenticate_api(
            &*store,
            &AccessPolicy::floor_only(&cfg()),
            &short.secret,
            now + Duration::hours(2)
        )
        .await,
        Err(AuthError::Unauthenticated)
    ));
    revoke_all_for(
        &pool,
        &*store,
        &log,
        "operator",
        Via::Interactive,
        "dan@example.com",
        now,
    )
    .await
    .unwrap();
    assert!(matches!(
        authenticate_api(
            &*store,
            &AccessPolicy::floor_only(&cfg()),
            &short.secret,
            now
        )
        .await,
        Err(AuthError::Unauthenticated)
    ));
    assert!(matches!(
        issue_token(
            &pool,
            &*store,
            &log,
            &AccessPolicy::floor_only(&cfg()),
            "operator",
            Via::Interactive,
            "mallory@evil.example",
            TokenKind::Interactive,
            "x",
            None,
            now
        )
        .await,
        Err(AuthError::NotAllowed)
    ));
    let stored: String = sqlx::query_scalar("SELECT hash FROM tokens WHERE id = $1")
        .bind(short.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, hash_secret(&short.secret));
    assert_eq!(stored.len(), 64);
    assert_ne!(stored, short.secret, "secrets are hashed at rest");
}

const NOW: fn() -> chrono::DateTime<Utc> = || Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();

#[tokio::test]
async fn disable_normalises_email_revokes_and_records_events() {
    let (_c, pool) = common::pg().await;
    let store = reeve_identity::make_token_store(pool.clone());
    let log = MemActivity(Mutex::new(vec![]));
    let now = NOW();
    let t = issue_token(
        &pool,
        &*store,
        &log,
        &AccessPolicy::floor_only(&cfg()),
        "operator",
        Via::Interactive,
        "dan@example.com",
        TokenKind::Interactive,
        "cli",
        None,
        now,
    )
    .await
    .unwrap();
    disable_user(
        &pool,
        &*store,
        &log,
        "operator",
        Via::System,
        " Dan@Example.COM ",
        now,
    )
    .await
    .unwrap();
    assert!(matches!(
        authenticate_api(&*store, &AccessPolicy::floor_only(&cfg()), &t.secret, now).await,
        Err(AuthError::Unauthenticated)
    ));
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens WHERE revoked_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(live, 0);
    revoke_all_for(
        &pool,
        &*store,
        &log,
        "operator",
        Via::Automation,
        "dan@example.com",
        now,
    )
    .await
    .unwrap();
    let ev = log.0.lock().unwrap();
    assert_eq!(
        (ev[1].action, ev[1].via, ev[1].subject.as_str()),
        ("user_disabled", Via::System, "dan@example.com")
    );
    assert_eq!(
        (ev[2].action, ev[2].via),
        ("tokens_revoked", Via::Automation)
    );
}

#[tokio::test]
async fn disabling_an_unseen_email_blocks_later_issue() {
    let (_c, pool) = common::pg().await;
    let store = reeve_identity::make_token_store(pool.clone());
    let log = MemActivity(Mutex::new(vec![]));
    let now = NOW();
    disable_user(
        &pool,
        &*store,
        &log,
        "operator",
        Via::System,
        "new@example.com",
        now,
    )
    .await
    .unwrap();
    let disabled: bool =
        sqlx::query_scalar("SELECT disabled FROM users WHERE email = 'new@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(disabled);
    assert!(matches!(
        issue_token(
            &pool,
            &*store,
            &log,
            &AccessPolicy::floor_only(&cfg()),
            "operator",
            Via::System,
            "New@example.com",
            TokenKind::Interactive,
            "x",
            None,
            now
        )
        .await,
        Err(AuthError::Disabled)
    ));
}

#[tokio::test]
async fn expiry_boundary_config_changes_and_validation() {
    let (_c, pool) = common::pg().await;
    let store = reeve_identity::make_token_store(pool.clone());
    let log = MemActivity(Mutex::new(vec![]));
    let now = NOW();
    let t = issue_token(
        &pool,
        &*store,
        &log,
        &AccessPolicy::floor_only(&cfg()),
        "operator",
        Via::Interactive,
        "guest@other.org",
        TokenKind::Interactive,
        "cli",
        Some(now + Duration::hours(1)),
        now,
    )
    .await
    .unwrap();
    assert!(
        authenticate_api(
            &*store,
            &AccessPolicy::floor_only(&cfg()),
            &t.secret,
            now + Duration::minutes(59)
        )
        .await
        .is_ok()
    );
    assert!(matches!(
        authenticate_api(
            &*store,
            &AccessPolicy::floor_only(&cfg()),
            &t.secret,
            now + Duration::hours(1)
        )
        .await,
        Err(AuthError::Unauthenticated)
    ));
    let removed = IdentityConfig {
        allowed_emails: vec![],
        ..cfg()
    };
    assert!(matches!(
        authenticate_api(&*store, &AccessPolicy::floor_only(&removed), &t.secret, now).await,
        Err(AuthError::Unauthenticated)
    ));
    let promoted = IdentityConfig {
        admins: vec!["guest@other.org".into()],
        ..cfg()
    };
    assert_eq!(
        authenticate_api(&*store, &AccessPolicy::floor_only(&cfg()), &t.secret, now)
            .await
            .unwrap()
            .role,
        Role::Member
    );
    assert_eq!(
        authenticate_api(
            &*store,
            &AccessPolicy::floor_only(&promoted),
            &t.secret,
            now
        )
        .await
        .unwrap()
        .role,
        Role::Admin
    );
    assert!(matches!(
        issue_token(
            &pool,
            &*store,
            &log,
            &AccessPolicy::floor_only(&cfg()),
            "operator",
            Via::Interactive,
            "dan@example.com",
            TokenKind::Interactive,
            "x",
            Some(now),
            now
        )
        .await,
        Err(AuthError::Validation(_))
    ));
    assert!(!cfg().is_allowed("@example.com"));
    assert!(!cfg().is_allowed("a@b@example.com"));
}

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

#[tokio::test]
async fn failed_activity_write_rolls_back_the_issue() {
    let (_c, pool) = common::pg().await;
    let store = reeve_identity::make_token_store(pool.clone());
    let now = NOW();
    assert!(
        issue_token(
            &pool,
            &*store,
            &Failing,
            &AccessPolicy::floor_only(&cfg()),
            "operator",
            Via::Interactive,
            "dan@example.com",
            TokenKind::Interactive,
            "x",
            None,
            now
        )
        .await
        .is_err()
    );
    let users: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE email = 'dan@example.com'")
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

type Setup = (
    reeve_testdb::TestDb,
    sqlx::PgPool,
    std::sync::Arc<dyn reeve_identity::TokenStore>,
    std::sync::Arc<MemActivity>,
    IdentityConfig,
);

async fn setup() -> Setup {
    let (c, pool) = common::pg().await;
    let store = reeve_identity::make_token_store(pool.clone());
    (
        c,
        pool,
        store,
        std::sync::Arc::new(MemActivity(Mutex::new(vec![]))),
        cfg(),
    )
}

async fn issue(
    pool: &sqlx::PgPool,
    store: &dyn reeve_identity::TokenStore,
    log: &MemActivity,
    cfg: &IdentityConfig,
    email: &str,
    now: chrono::DateTime<Utc>,
) -> reeve_identity::IssuedToken {
    issue_token(
        pool,
        store,
        log,
        &AccessPolicy::floor_only(cfg),
        "test",
        Via::Interactive,
        email,
        TokenKind::Interactive,
        "t",
        None,
        now,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn sign_in_issues_a_session_token_only_for_verified_allowed_enabled_users() {
    let (_c, pool, store, log, cfg) = setup().await;
    // Postgres stores microseconds; a fixed instant keeps the expires_at equality exact.
    let now = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    let id = |email: &str, verified: bool| reeve_identity::VerifiedIdentity {
        email: email.into(),
        email_verified: verified,
        name: Some("Dan T".into()),
    };
    // baselines come before the first refusal, so both refusals are covered by the before/after check below
    let users_before = reeve_identity::list_users(&*store).await.unwrap().len();
    let tokens_before: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    // session_days outside 1..=365 -> Validation (0 would issue an already-expired session)
    assert!(matches!(
        reeve_identity::sign_in(
            &pool,
            &*store,
            &*log,
            &AccessPolicy::floor_only(&cfg),
            id("dan@example.com", true),
            0,
            now
        )
        .await,
        Err(reeve_identity::AuthError::Validation(_))
    ));
    // unverified -> refused, no user row, no token
    // `IssuedToken` carries a secret and does not implement Debug, so inspect the Err without unwrap_err
    assert!(matches!(
        reeve_identity::sign_in(
            &pool,
            &*store,
            &*log,
            &AccessPolicy::floor_only(&cfg),
            id("Dan@Example.com", false),
            30,
            now
        )
        .await,
        Err(reeve_identity::AuthError::NotAllowed)
    ));
    // outside allowlist -> refused
    assert!(matches!(
        reeve_identity::sign_in(
            &pool,
            &*store,
            &*log,
            &AccessPolicy::floor_only(&cfg),
            id("x@other.org", true),
            30,
            now
        )
        .await,
        Err(reeve_identity::AuthError::NotAllowed)
    ));
    // a refusal writes nothing: no user row, no token, no activity entry
    assert_eq!(
        reeve_identity::list_users(&*store).await.unwrap().len(),
        users_before
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tokens")
            .fetch_one(&pool)
            .await
            .unwrap(),
        tokens_before
    );
    assert!(
        log.0
            .lock()
            .unwrap()
            .iter()
            .all(|e| e.action != "signed_in")
    );
    // allowed -> interactive token labelled "web session", expiring in 30 days, email normalised, name stored
    let t = reeve_identity::sign_in(
        &pool,
        &*store,
        &*log,
        &AccessPolicy::floor_only(&cfg),
        id("Dan@Example.com", true),
        30,
        now,
    )
    .await
    .unwrap();
    assert!(t.secret.starts_with("reeve_"));
    {
        let events = log.0.lock().unwrap();
        let signed: Vec<_> = events.iter().filter(|e| e.action == "signed_in").collect();
        assert_eq!(signed.len(), 1);
        assert_eq!(signed[0].actor, "dan@example.com");
        assert_eq!(signed[0].subject, "dan@example.com");
        assert!(matches!(signed[0].via, Via::Interactive));
        assert_eq!(signed[0].payload["token_id"], t.id);
    }
    let p =
        reeve_identity::authenticate_api(&*store, &AccessPolicy::floor_only(&cfg), &t.secret, now)
            .await
            .unwrap();
    assert_eq!(p.email, "dan@example.com");
    assert_eq!(p.token_kind, reeve_protocol::TokenKind::Interactive);
    let mine = reeve_identity::list_tokens(&*store, "dan@example.com")
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].label, "web session");
    assert_eq!(mine[0].expires_at, Some(now + chrono::Duration::days(30)));
    let users = reeve_identity::list_users(&*store).await.unwrap();
    assert_eq!(users[0].display_name, "Dan T");
    // the session is the token: expired by the clock -> unauthenticated
    assert!(
        reeve_identity::authenticate_api(
            &*store,
            &AccessPolicy::floor_only(&cfg),
            &t.secret,
            now + chrono::Duration::days(31)
        )
        .await
        .is_err()
    );
    // disabled -> refused on the next sign-in
    reeve_identity::disable_user(
        &pool,
        &*store,
        &*log,
        "admin",
        reeve_protocol::Via::Interactive,
        "dan@example.com",
        now,
    )
    .await
    .unwrap();
    assert!(matches!(
        reeve_identity::sign_in(
            &pool,
            &*store,
            &*log,
            &AccessPolicy::floor_only(&cfg),
            id("dan@example.com", true),
            30,
            now
        )
        .await,
        Err(reeve_identity::AuthError::Disabled)
    ));
    // enable restores sign-in but not the revoked tokens
    reeve_identity::enable_user(
        &pool,
        &*store,
        &*log,
        &AccessPolicy::floor_only(&cfg),
        "admin",
        reeve_protocol::Via::Interactive,
        "dan@example.com",
        now,
    )
    .await
    .unwrap();
    {
        let events = log.0.lock().unwrap();
        let enabled: Vec<_> = events
            .iter()
            .filter(|e| e.action == "user_enabled")
            .collect();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].actor, "admin");
        assert_eq!(enabled[0].subject, "dan@example.com");
    }
    // enabling an already-enabled user changes nothing and records nothing
    reeve_identity::enable_user(
        &pool,
        &*store,
        &*log,
        &AccessPolicy::floor_only(&cfg),
        "admin",
        reeve_protocol::Via::Interactive,
        "dan@example.com",
        now,
    )
    .await
    .unwrap();
    assert_eq!(
        log.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.action == "user_enabled")
            .count(),
        1
    );
    assert!(
        reeve_identity::authenticate_api(&*store, &AccessPolicy::floor_only(&cfg), &t.secret, now)
            .await
            .is_err()
    );
    let t2 = reeve_identity::sign_in(
        &pool,
        &*store,
        &*log,
        &AccessPolicy::floor_only(&cfg),
        id("dan@example.com", true),
        30,
        now,
    )
    .await
    .unwrap();
    assert!(
        reeve_identity::authenticate_api(&*store, &AccessPolicy::floor_only(&cfg), &t2.secret, now)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn revoke_one_only_touches_the_owners_own_token() {
    let (_c, pool, store, log, cfg) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    let a = issue(&pool, &*store, &log, &cfg, "a@example.com", now).await;
    let b = issue(&pool, &*store, &log, &cfg, "b@example.com", now).await;
    // b cannot revoke a's token: returns Ok(false), token still works
    assert!(
        !reeve_identity::revoke_token(
            &pool,
            &*store,
            &*log,
            "b@example.com",
            reeve_protocol::Via::Interactive,
            a.id,
            now
        )
        .await
        .unwrap()
    );
    assert!(
        reeve_identity::authenticate_api(&*store, &AccessPolicy::floor_only(&cfg), &a.secret, now)
            .await
            .is_ok()
    );
    // a revokes it: Ok(true), token dead, activity entry "token_revoked" with token_id
    assert!(
        reeve_identity::revoke_token(
            &pool,
            &*store,
            &*log,
            "a@example.com",
            reeve_protocol::Via::Interactive,
            a.id,
            now
        )
        .await
        .unwrap()
    );
    assert!(
        reeve_identity::authenticate_api(&*store, &AccessPolicy::floor_only(&cfg), &a.secret, now)
            .await
            .is_err()
    );
    assert!(
        reeve_identity::authenticate_api(&*store, &AccessPolicy::floor_only(&cfg), &b.secret, now)
            .await
            .is_ok()
    );
    // reeve-identity must not depend on reeve-activity (it is the supplier); assert on the in-memory log instead.
    // The TOTAL count across all subjects: the no-op attempt by b must not have logged under b either.
    let revoked_events = || {
        log.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.action == "token_revoked")
            .map(|e| (e.subject.clone(), e.payload["token_id"].clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        revoked_events(),
        vec![("a@example.com".to_string(), a.id.into())]
    );
    // revoking it again as its owner changes nothing: Ok(false), still exactly one entry
    assert!(
        !reeve_identity::revoke_token(
            &pool,
            &*store,
            &*log,
            "a@example.com",
            reeve_protocol::Via::Interactive,
            a.id,
            now + Duration::hours(1)
        )
        .await
        .unwrap()
    );
    assert_eq!(revoked_events().len(), 1);
}

#[tokio::test]
async fn failed_activity_write_rolls_back_sign_in_revoke_and_enable() {
    let (_c, pool, store, log, cfg) = setup().await;
    let now = NOW();
    let who = reeve_identity::VerifiedIdentity {
        email: "dan@example.com".into(),
        email_verified: true,
        name: Some("Dan T".into()),
    };
    // sign_in: no user, no token, no display name persisted
    assert!(
        reeve_identity::sign_in(
            &pool,
            &*store,
            &Failing,
            &AccessPolicy::floor_only(&cfg),
            who,
            30,
            now
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
    // revoke_token: the token stays live
    let a = issue(&pool, &*store, &log, &cfg, "a@example.com", now).await;
    assert!(
        reeve_identity::revoke_token(
            &pool,
            &*store,
            &Failing,
            "a@example.com",
            reeve_protocol::Via::Interactive,
            a.id,
            now
        )
        .await
        .is_err()
    );
    assert!(
        reeve_identity::authenticate_api(&*store, &AccessPolicy::floor_only(&cfg), &a.secret, now)
            .await
            .is_ok()
    );
    // enable_user: the user stays disabled
    reeve_identity::disable_user(
        &pool,
        &*store,
        &*log,
        "admin",
        reeve_protocol::Via::Interactive,
        "a@example.com",
        now,
    )
    .await
    .unwrap();
    assert!(
        reeve_identity::enable_user(
            &pool,
            &*store,
            &Failing,
            &AccessPolicy::floor_only(&cfg),
            "admin",
            reeve_protocol::Via::Interactive,
            "a@example.com",
            now
        )
        .await
        .is_err()
    );
    let disabled: bool =
        sqlx::query_scalar("SELECT disabled FROM users WHERE email = 'a@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(disabled);
}
