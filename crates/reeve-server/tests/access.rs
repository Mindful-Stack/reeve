//! Sign-in access as the floor ∪ the database (kb/ADR-0015). Rows are written with SQL here wherever the test is about
//! what a row does, not about the API that writes it.
mod common;
use common::*;
use reeve_protocol::{TokenKind, Via};
use serde_json::{Value, json};
use std::sync::Arc;

async fn sql(h: &Harness, q: &str) {
    sqlx::query(q).execute(&h.state.pool).await.unwrap();
}

/// A token for `email`, issued against the current policy (the email must be allowed now).
async fn token_for(h: &Harness, email: &str, kind: TokenKind) -> String {
    let policy = h.state.access.load(&h.state.identity).await.unwrap();
    let log: Arc<dyn reeve_identity::IdentityActivity> = h.state.activity.clone();
    reeve_identity::issue_token(
        &h.state.pool,
        &*h.state.tokens,
        &*log,
        &policy,
        "test",
        Via::Interactive,
        email,
        kind,
        "t",
        None,
        chrono::Utc::now(),
    )
    .await
    .unwrap()
    .secret
}

#[tokio::test]
async fn a_database_entry_lets_its_email_in_and_its_removal_shuts_them_out_on_the_next_request() {
    let h = harness().await;
    sql(
        &h,
        "INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','guest@other.org','dan@example.com',now())",
    )
    .await;
    let t = token_for(&h, "guest@other.org", TokenKind::Interactive).await;
    assert_eq!(
        call(&h.app, Some(&t), "GET", "/api/v1/me", None).await.0,
        200
    );
    sql(&h, "DELETE FROM allowed_entries").await;
    assert_eq!(
        call(&h.app, Some(&t), "GET", "/api/v1/me", None).await.0,
        401
    );
    sql(
        &h,
        "INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','guest@other.org','dan@example.com',now())",
    )
    .await;
    assert_eq!(
        call(&h.app, Some(&t), "GET", "/api/v1/me", None).await.0,
        200,
        "the token was never revoked"
    );
}

#[tokio::test]
async fn a_database_admin_grant_is_admin_on_the_next_request_and_in_the_export() {
    let h = harness().await;
    let ada = token_for(&h, "ada@example.com", TokenKind::Interactive).await;
    assert_eq!(
        call(&h.app, Some(&ada), "GET", "/api/v1/users", None)
            .await
            .0,
        403
    );
    sql(
        &h,
        "INSERT INTO admin_grants (email, added_by, added_at) VALUES ('ada@example.com','dan@example.com',now())",
    )
    .await;
    let (status, me) = call(&h.app, Some(&ada), "GET", "/api/v1/me", None).await;
    assert_eq!((status, me["value"]["role"].as_str()), (200, Some("admin")));
    assert_eq!(
        call(&h.app, Some(&ada), "GET", "/api/v1/users", None)
            .await
            .0,
        200
    );
    let (_, users) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/users", None).await;
    let role = |v: &Value| {
        v["value"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["email"] == "ada@example.com")
            .unwrap()["role"]
            .clone()
    };
    assert_eq!(role(&users), "admin");
    let (_, export) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/export", None).await;
    let u = export["value"]["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == "ada@example.com")
        .unwrap()
        .clone();
    assert_eq!(u["role"], "admin");
}

#[tokio::test]
async fn a_story_owner_may_be_a_guest_allowed_only_by_the_database() {
    let h = harness().await;
    let (status, _) = call(&h.app, Some(&h.admin_token), "POST", "/api/v1/stories",
        Some(json!({"idempotency_key":"o1","title":"Owned","kind":"chore","criteria":[],"labels":[],"owner":"guest@other.org"}))).await;
    assert_eq!(status, 400);
    sql(
        &h,
        "INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','guest@other.org','dan@example.com',now())",
    )
    .await;
    let (status, body) = call(&h.app, Some(&h.admin_token), "POST", "/api/v1/stories",
        Some(json!({"idempotency_key":"o2","title":"Owned","kind":"chore","criteria":[],"labels":[],"owner":"guest@other.org"}))).await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn a_configured_admin_cannot_be_disabled_through_the_api() {
    let h = harness().await;
    sql(&h, "INSERT INTO admin_grants (email, added_by, added_at) VALUES ('ada@example.com','dan@example.com',now())").await;
    let ada = token_for(&h, "ada@example.com", TokenKind::Interactive).await;
    let (status, body) = call(
        &h.app,
        Some(&ada),
        "POST",
        "/api/v1/users/dan@example.com/disable",
        None,
    )
    .await;
    assert_eq!(
        (status, body["message"].as_str()),
        (400, Some("admin in reeve.toml; change it there"))
    );
}

#[tokio::test]
async fn disable_works_on_a_user_no_longer_allowed_and_refuses_one_never_seen() {
    let h = harness().await;
    sql(&h, "INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','guest@other.org','dan@example.com',now())").await;
    let guest = token_for(&h, "guest@other.org", TokenKind::Interactive).await;
    sql(&h, "DELETE FROM allowed_entries").await;
    let (status, _) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/users/guest@other.org/disable",
        None,
    )
    .await;
    assert_eq!(status, 200);
    sql(&h, "INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','guest@other.org','dan@example.com',now())").await;
    assert_eq!(
        call(&h.app, Some(&guest), "GET", "/api/v1/me", None)
            .await
            .0,
        401,
        "disable revoked the old token"
    );
    let (status, _) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/users/ghost@other.org/disable",
        None,
    )
    .await;
    assert_eq!(status, 404);
    // a database-only guest can be re-enabled
    let (status, _) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/users/guest@other.org/enable",
        None,
    )
    .await;
    assert_eq!(status, 200);
}
