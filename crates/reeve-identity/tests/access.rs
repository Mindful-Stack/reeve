mod common;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reeve_identity::{
    AccessKind, AccessPolicy, AuthError, IdentityActivity, IdentityConfig, IdentityEvent,
    Principal, Role, TokenKind, allow_entry, grant_admin, make_access_store, remove_entry,
    revoke_admin,
};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

/// Records what identity logs; the transactional write itself is proven by reeve-activity's repository tests.
#[derive(Default)]
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
impl MemActivity {
    fn actions(&self) -> Vec<(String, &'static str, serde_json::Value)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|e| (e.subject.clone(), e.action, e.payload.clone()))
            .collect()
    }
}

fn floor() -> IdentityConfig {
    IdentityConfig {
        allowed_domains: vec!["example.com".into()],
        allowed_emails: vec!["svc@bots.dev".into()],
        admins: vec!["dan@example.com".into()],
    }
}
fn who(email: &str, role: Role, token_kind: TokenKind) -> Principal {
    Principal {
        email: email.into(),
        role,
        token_kind,
    }
}
fn dan() -> Principal {
    who("dan@example.com", Role::Admin, TokenKind::Interactive)
}
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 9, 9, 0, 0).unwrap()
}

struct Env {
    _db: reeve_testdb::TestDb,
    pool: PgPool,
    store: Arc<dyn reeve_identity::AccessStore>,
    log: MemActivity,
}
async fn env() -> Env {
    let (db, pool) = common::pg().await;
    Env {
        store: make_access_store(pool.clone()),
        pool,
        _db: db,
        log: MemActivity::default(),
    }
}
impl Env {
    async fn sql(&self, q: &str) {
        sqlx::query(q).execute(&self.pool).await.unwrap();
    }
    async fn policy(&self) -> AccessPolicy {
        self.store.load(&floor()).await.unwrap()
    }
    async fn allow(
        &self,
        by: &Principal,
        kind: AccessKind,
        v: &str,
    ) -> Result<AccessPolicy, AuthError> {
        allow_entry(
            &self.pool,
            &*self.store,
            &self.log,
            &floor(),
            by,
            kind,
            v,
            now(),
        )
        .await
    }
    async fn remove(
        &self,
        by: &Principal,
        kind: AccessKind,
        v: &str,
    ) -> Result<AccessPolicy, AuthError> {
        remove_entry(
            &self.pool,
            &*self.store,
            &self.log,
            &floor(),
            by,
            kind,
            v,
            now(),
        )
        .await
    }
    async fn grant(&self, by: &Principal, email: &str) -> Result<AccessPolicy, AuthError> {
        grant_admin(
            &self.pool,
            &*self.store,
            &self.log,
            &floor(),
            by,
            email,
            now(),
        )
        .await
    }
    async fn revoke(&self, by: &Principal, email: &str) -> Result<AccessPolicy, AuthError> {
        revoke_admin(
            &self.pool,
            &*self.store,
            &self.log,
            &floor(),
            by,
            email,
            now(),
        )
        .await
    }
}
fn validation(r: Result<AccessPolicy, AuthError>) -> String {
    match r {
        Err(AuthError::Validation(m)) => m,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn members_and_automation_tokens_cannot_write_and_nothing_is_written() {
    let e = env().await;
    for by in [
        who("bob@example.com", Role::Member, TokenKind::Interactive),
        who("dan@example.com", Role::Admin, TokenKind::Automation),
    ] {
        assert!(matches!(
            e.allow(&by, AccessKind::Email, "g@other.org").await,
            Err(AuthError::Forbidden)
        ));
        assert!(matches!(
            e.remove(&by, AccessKind::Email, "g@other.org").await,
            Err(AuthError::Forbidden)
        ));
        assert!(matches!(
            e.grant(&by, "ada@example.com").await,
            Err(AuthError::Forbidden)
        ));
        assert!(matches!(
            e.revoke(&by, "ada@example.com").await,
            Err(AuthError::Forbidden)
        ));
    }
    assert!(e.log.actions().is_empty());
    assert!(!e.policy().await.is_allowed("g@other.org"));
}

#[tokio::test]
async fn an_admin_whose_grant_is_gone_from_the_database_is_refused_even_with_an_admin_principal() {
    // The principal was resolved before the grant was revoked; the write decides on the locked policy (R3).
    let e = env().await;
    let stale = who("ada@example.com", Role::Admin, TokenKind::Interactive);
    assert!(matches!(
        e.allow(&stale, AccessKind::Email, "g@other.org").await,
        Err(AuthError::Forbidden)
    ));
}

#[tokio::test]
async fn allow_writes_the_entry_with_its_activity_and_refuses_duplicates_and_floor_entries() {
    let e = env().await;
    let p = e
        .allow(&dan(), AccessKind::Email, " G@Other.org ")
        .await
        .unwrap();
    assert!(p.is_allowed("g@other.org"));
    assert_eq!(
        e.log.actions(),
        [(
            "g@other.org".to_string(),
            "access_allowed",
            serde_json::json!({"kind": "email"})
        )]
    );
    // Copy out in a block: a guard held to the end of the test would deadlock `actions()` below (std Mutex).
    let (actor, via, at) = {
        let g = e.log.0.lock().unwrap();
        (g[0].actor.clone(), g[0].via, g[0].at)
    };
    assert_eq!(
        (actor.as_str(), via, at),
        ("dan@example.com", reeve_identity::Via::Interactive, now())
    );
    assert_eq!(
        validation(e.allow(&dan(), AccessKind::Email, "g@other.org").await),
        "already allowed"
    );
    assert_eq!(
        validation(e.allow(&dan(), AccessKind::Domain, "example.com").await),
        "already set in reeve.toml"
    );
    validation(e.allow(&dan(), AccessKind::Domain, "not a domain").await);
    validation(e.allow(&dan(), AccessKind::Email, "nope").await);
    assert_eq!(e.log.actions().len(), 1);
}

#[tokio::test]
async fn remove_targets_the_database_row_only() {
    let e = env().await;
    assert_eq!(
        validation(e.remove(&dan(), AccessKind::Domain, "example.com").await),
        "set in reeve.toml; change it there"
    );
    assert!(matches!(
        e.remove(&dan(), AccessKind::Email, "nobody@x.org").await,
        Err(AuthError::NotFound)
    ));
    // a database row duplicating the floor is listed separately and removable; the floor still allows
    e.sql("INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('domain','example.com','x',now())")
        .await;
    let p = e
        .remove(&dan(), AccessKind::Domain, "example.com")
        .await
        .unwrap();
    assert!(!p.in_table(AccessKind::Domain, "example.com") && p.is_allowed("ada@example.com"));
    assert_eq!(
        e.log.actions(),
        [(
            "example.com".to_string(),
            "access_removed",
            serde_json::json!({"kind": "domain"})
        )]
    );
}

#[tokio::test]
async fn grant_admin_allows_the_email_too_and_refuses_existing_admins() {
    let e = env().await;
    let p = e.grant(&dan(), "Guest@Other.org").await.unwrap();
    assert_eq!(p.role_for("guest@other.org"), Role::Admin);
    let acts: Vec<_> = e
        .log
        .actions()
        .into_iter()
        .map(|(s, a, _)| (s, a))
        .collect();
    assert_eq!(
        acts,
        [
            ("guest@other.org".to_string(), "access_allowed"),
            ("guest@other.org".to_string(), "admin_granted")
        ]
    );
    assert_eq!(
        validation(e.grant(&dan(), "guest@other.org").await),
        "already admin"
    );
    assert_eq!(
        validation(e.grant(&dan(), "dan@example.com").await),
        "already set in reeve.toml"
    );
    // an allowed email gets only the grant
    e.grant(&dan(), "ada@example.com").await.unwrap();
    assert_eq!(e.log.actions().last().unwrap().1, "admin_granted");
    assert_eq!(e.log.actions().len(), 3);
}

#[tokio::test]
async fn granting_a_guest_then_removing_their_email_entry_deletes_the_grant() {
    let e = env().await;
    e.grant(&dan(), "pat@partner.org").await.unwrap();
    let p = e
        .remove(&dan(), AccessKind::Email, "pat@partner.org")
        .await
        .unwrap();
    assert!(!p.has_admin_grant("pat@partner.org"));
    let acts = e.log.actions();
    assert_eq!(
        acts[acts.len() - 2..],
        [
            (
                "pat@partner.org".to_string(),
                "access_removed",
                serde_json::json!({"kind": "email"})
            ),
            (
                "pat@partner.org".to_string(),
                "admin_revoked",
                serde_json::json!({"reason": "access_removed"})
            ),
        ]
    );
}

#[tokio::test]
async fn revoke_admin_refuses_self_floor_and_absent_and_logs_the_revocation() {
    let e = env().await;
    e.grant(&dan(), "ada@example.com").await.unwrap();
    let ada = who("ada@example.com", Role::Admin, TokenKind::Interactive);
    assert_eq!(
        validation(e.revoke(&ada, "ada@example.com").await),
        "you cannot remove your own access"
    );
    assert_eq!(
        validation(e.revoke(&ada, "dan@example.com").await),
        "set in reeve.toml; change it there"
    );
    assert!(matches!(
        e.revoke(&dan(), "bob@example.com").await,
        Err(AuthError::NotFound)
    ));
    let p = e.revoke(&dan(), "ada@example.com").await.unwrap();
    assert_eq!(p.role_for("ada@example.com"), Role::Member);
    assert!(
        p.is_allowed("ada@example.com"),
        "a revoked admin keeps member access"
    );
    assert_eq!(e.log.actions().last().unwrap().1, "admin_revoked");
}

#[tokio::test]
async fn removing_an_entry_deletes_the_grants_it_leaves_unallowed_but_not_already_inert_ones() {
    let e = env().await;
    e.sql("INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('domain','partner.org','x',now())")
        .await;
    e.sql(
        "INSERT INTO admin_grants (email, added_by, added_at) VALUES ('pat@partner.org','x',now()), ('eve@gone.org','x',now())",
    )
    .await;
    let p = e
        .remove(&dan(), AccessKind::Domain, "partner.org")
        .await
        .unwrap();
    assert!(!p.has_admin_grant("pat@partner.org"));
    assert!(
        p.has_admin_grant("eve@gone.org"),
        "a grant inert before the removal is not this removal's to delete"
    );
    assert_eq!(
        e.log.actions(),
        [
            (
                "partner.org".to_string(),
                "access_removed",
                serde_json::json!({"kind": "domain"})
            ),
            (
                "pat@partner.org".to_string(),
                "admin_revoked",
                serde_json::json!({"reason": "access_removed"})
            ),
        ]
    );
}

#[tokio::test]
async fn an_admin_cannot_remove_an_entry_that_shuts_them_out_but_may_remove_one_that_overlaps() {
    let e = env().await;
    e.sql("INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','pat@partner.org','x',now()), ('domain','partner.org','x',now())").await;
    e.sql(
        "INSERT INTO admin_grants (email, added_by, added_at) VALUES ('pat@partner.org','x',now())",
    )
    .await;
    let pat = who("pat@partner.org", Role::Admin, TokenKind::Interactive);
    e.remove(&pat, AccessKind::Email, "pat@partner.org")
        .await
        .unwrap(); // still allowed through the domain
    assert_eq!(
        validation(e.remove(&pat, AccessKind::Domain, "partner.org").await),
        "you cannot remove your own access"
    );
    assert_eq!(e.policy().await.role_for("pat@partner.org"), Role::Admin);
}

#[tokio::test]
async fn two_concurrent_removals_that_would_together_shut_the_caller_out_cannot_both_succeed() {
    let e = env().await;
    e.sql("INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','pat@partner.org','x',now()), ('domain','partner.org','x',now())").await;
    e.sql(
        "INSERT INTO admin_grants (email, added_by, added_at) VALUES ('pat@partner.org','x',now())",
    )
    .await;
    let pat = who("pat@partner.org", Role::Admin, TokenKind::Interactive);
    let (a, b) = tokio::join!(
        e.remove(&pat, AccessKind::Email, "pat@partner.org"),
        e.remove(&pat, AccessKind::Domain, "partner.org")
    );
    assert_eq!(
        [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
        1,
        "{a:?} {b:?}"
    );
    assert!(e.policy().await.is_allowed("pat@partner.org"));
}

#[tokio::test]
async fn an_inert_grant_confers_nothing_is_revived_by_granting_and_is_deleted_by_revoking() {
    let e = env().await;
    e.sql("INSERT INTO admin_grants (email, added_by, added_at) VALUES ('eve@gone.org','x',now())")
        .await;
    assert_eq!(e.policy().await.role_for("eve@gone.org"), Role::Member);
    let p = e.grant(&dan(), "eve@gone.org").await.unwrap();
    assert_eq!(p.role_for("eve@gone.org"), Role::Admin);
    assert_eq!(
        e.log.actions().iter().map(|a| a.1).collect::<Vec<_>>(),
        ["access_allowed"],
        "the grant already existed"
    );
    e.sql("DELETE FROM allowed_entries").await; // a hand edit orphans it again
    e.revoke(&dan(), "eve@gone.org").await.unwrap();
    assert!(!e.policy().await.has_admin_grant("eve@gone.org"));
    assert_eq!(e.log.actions().last().unwrap().1, "admin_revoked");
}
