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

async fn access(
    h: &Harness,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    call(
        &h.app,
        Some(token),
        method,
        &format!("/api/v1/access{path}"),
        body,
    )
    .await
}
fn values(dto: &Value, list: &str) -> Vec<(String, String)> {
    dto["value"][list]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["value"].as_str().unwrap().to_string(),
                r["source"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn admins_list_add_and_remove_entries_and_everyone_else_is_forbidden() {
    let h = harness().await;
    let (s, dto) = access(&h, &h.admin_token, "GET", "", None).await;
    assert_eq!(s, 200);
    assert_eq!(
        values(&dto, "domains"),
        [("example.com".into(), "config".into())]
    );
    let (s, dto) = access(
        &h,
        &h.admin_token,
        "POST",
        "/entries",
        Some(json!({"kind":"email","value":"Guest@Other.org"})),
    )
    .await;
    assert_eq!(s, 200, "{dto}");
    let row = &dto["value"]["emails"][0];
    assert_eq!(
        (
            row["value"].as_str(),
            row["source"].as_str(),
            row["added_by"].as_str()
        ),
        (
            Some("guest@other.org"),
            Some("database"),
            Some("dan@example.com")
        )
    );
    assert!(row["added_at"].is_string());
    // the automation token may read but not write; a member may do neither
    assert_eq!(access(&h, &h.bot_token, "GET", "", None).await.0, 200);
    assert_eq!(
        access(
            &h,
            &h.bot_token,
            "POST",
            "/entries",
            Some(json!({"kind":"domain","value":"x.org"}))
        )
        .await
        .0,
        403
    );
    assert_eq!(access(&h, &h.victim_token, "GET", "", None).await.0, 403);
    assert_eq!(
        access(
            &h,
            &h.victim_token,
            "DELETE",
            "/entries/email/guest@other.org",
            None
        )
        .await
        .0,
        403
    );
    // floor locked, absent not found, duplicate refused
    // `AuthError::Validation` displays as "invalid request: {0}" and error.rs sends that text as is; the wire keeps the
    // prefix every identity validation error already carries.
    let (s, b) = access(
        &h,
        &h.admin_token,
        "DELETE",
        "/entries/domain/example.com",
        None,
    )
    .await;
    assert_eq!(
        (s, b["message"].as_str()),
        (
            400,
            Some("invalid request: set in reeve.toml; change it there")
        )
    );
    assert_eq!(
        access(
            &h,
            &h.admin_token,
            "DELETE",
            "/entries/email/nobody@x.org",
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        access(
            &h,
            &h.admin_token,
            "POST",
            "/entries",
            Some(json!({"kind":"email","value":"guest@other.org"}))
        )
        .await
        .0,
        400
    );
    let (s, dto) = access(
        &h,
        &h.admin_token,
        "DELETE",
        "/entries/email/guest@other.org",
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert!(dto["value"]["emails"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn admin_grants_through_the_api_and_their_activity() {
    let h = harness().await;
    let (s, dto) = access(
        &h,
        &h.admin_token,
        "POST",
        "/admins",
        Some(json!({"email":"pat@partner.org"})),
    )
    .await;
    assert_eq!(s, 200, "{dto}");
    assert_eq!(
        values(&dto, "emails"),
        [("pat@partner.org".into(), "database".into())]
    );
    let admins = values(&dto, "admins");
    assert_eq!(
        admins,
        [
            ("dan@example.com".into(), "config".into()),
            ("pat@partner.org".into(), "database".into())
        ]
    );
    assert_eq!(dto["value"]["admins"][1]["allowed"], true);
    let pat = token_for(&h, "pat@partner.org", TokenKind::Interactive).await;
    assert_eq!(
        call(&h.app, Some(&pat), "GET", "/api/v1/users", None)
            .await
            .0,
        200,
        "admin on the next request"
    );
    let (s, b) = access(&h, &pat, "DELETE", "/admins/pat@partner.org", None).await;
    assert_eq!(
        (s, b["message"].as_str()),
        (
            400,
            Some("invalid request: you cannot remove your own access")
        )
    );
    // removing the email entry takes the grant with it, logged
    let (s, dto) = access(
        &h,
        &h.admin_token,
        "DELETE",
        "/entries/email/pat@partner.org",
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(values(&dto, "admins").len(), 1);
    let (_, log) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/activity?limit=10",
        None,
    )
    .await;
    let acts: Vec<_> = log["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap().to_string())
        .collect();
    for a in [
        "access_allowed",
        "admin_granted",
        "access_removed",
        "admin_revoked",
    ] {
        assert!(acts.iter().any(|x| x == a), "{a} missing from {acts:?}");
    }
}

#[tokio::test]
async fn a_database_row_duplicating_the_floor_lists_twice_and_its_database_row_is_removable() {
    let h = harness().await;
    sql(&h, "INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('domain','example.com','x',now())").await;
    let (_, dto) = access(&h, &h.admin_token, "GET", "", None).await;
    assert_eq!(
        values(&dto, "domains"),
        [
            ("example.com".into(), "config".into()),
            ("example.com".into(), "database".into())
        ]
    );
    assert_eq!(dto["value"]["domains"][1]["also_in_config"], true);
    assert_eq!(
        access(
            &h,
            &h.admin_token,
            "DELETE",
            "/entries/domain/example.com",
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None)
            .await
            .0,
        200,
        "the floor still allows bob"
    );
}

/// The router a restarted server would serve: the same database, this configuration.
async fn restarted(
    h: &Harness,
    edit: impl FnOnce(&mut reeve_server::ServerConfig),
) -> axum::Router {
    let mut cfg = (*h.state.config).clone();
    edit(&mut cfg);
    reeve_server::app(
        reeve_server::build_state(cfg, h.state.pool.clone())
            .await
            .unwrap(),
    )
}

async fn activity_count(h: &Harness) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM activity")
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_grant_orphaned_by_the_configuration_is_kept_inert_and_counts_again_when_the_configuration_is_restored()
 {
    let h = harness().await;
    assert_eq!(
        access(
            &h,
            &h.admin_token,
            "POST",
            "/admins",
            Some(json!({"email":"ada@example.com"}))
        )
        .await
        .0,
        200
    );
    let ada = token_for(&h, "ada@example.com", TokenKind::Interactive).await;
    let before = activity_count(&h).await;
    // the floor loses example.com; dan stays allowed by email so the configuration still validates
    let narrowed = restarted(&h, |c| {
        c.allowed_domains = vec![];
        c.allowed_emails = vec!["dan@example.com".into()];
    })
    .await;
    assert_eq!(
        activity_count(&h).await,
        before,
        "a server start writes no activity"
    );
    assert_eq!(
        call(&narrowed, Some(&ada), "GET", "/api/v1/me", None)
            .await
            .0,
        401,
        "ada is no longer allowed"
    );
    let (_, dto) = call(
        &narrowed,
        Some(&h.admin_token),
        "GET",
        "/api/v1/access",
        None,
    )
    .await;
    let row = dto["value"]["admins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["value"] == "ada@example.com")
        .unwrap()
        .clone();
    assert_eq!(
        (row["source"].as_str(), row["allowed"].as_bool()),
        (Some("database"), Some(false))
    );
    // restored: the same grant counts again
    let restored = restarted(&h, |_| {}).await;
    assert_eq!(
        call(&restored, Some(&ada), "GET", "/api/v1/users", None)
            .await
            .0,
        200
    );
}

#[tokio::test]
async fn revoking_an_inert_grant_deletes_it_and_granting_its_email_revives_it() {
    let h = harness().await;
    sql(
        &h,
        "INSERT INTO admin_grants (email, added_by, added_at) VALUES ('eve@gone.org','x',now())",
    )
    .await;
    let (_, dto) = access(&h, &h.admin_token, "GET", "", None).await;
    let eve = dto["value"]["admins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["value"] == "eve@gone.org")
        .expect("an inert grant is listed")
        .clone();
    assert_eq!(eve["allowed"], false);
    // granting admin to an inert grant's email inserts the email entry and does not refuse
    let (s, dto) = access(
        &h,
        &h.admin_token,
        "POST",
        "/admins",
        Some(json!({"email":"eve@gone.org"})),
    )
    .await;
    assert_eq!(s, 200, "{dto}");
    let eve = dto["value"]["admins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["value"] == "eve@gone.org")
        .unwrap()
        .clone();
    assert_eq!(eve["allowed"], true);
    assert!(
        dto["value"]["emails"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["value"] == "eve@gone.org" && r["source"] == "database"),
        "{dto}"
    );
    sql(
        &h,
        "DELETE FROM allowed_entries WHERE value = 'eve@gone.org'",
    )
    .await;
    // granting the email's access revives the grant
    access(
        &h,
        &h.admin_token,
        "POST",
        "/entries",
        Some(json!({"kind":"email","value":"eve@gone.org"})),
    )
    .await;
    let (_, dto) = access(&h, &h.admin_token, "GET", "", None).await;
    let eve = dto["value"]["admins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["value"] == "eve@gone.org")
        .unwrap()
        .clone();
    assert_eq!(eve["allowed"], true);
    // inert again, then revoked: the row is deleted
    sql(
        &h,
        "DELETE FROM allowed_entries WHERE value = 'eve@gone.org'",
    )
    .await;
    let (s, dto) = access(&h, &h.admin_token, "DELETE", "/admins/eve@gone.org", None).await;
    assert_eq!(s, 200);
    assert!(
        dto["value"]["admins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["value"] != "eve@gone.org")
    );
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM admin_grants WHERE email = 'eve@gone.org'")
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn an_open_event_stream_ends_at_its_next_batch_after_the_owner_is_removed() {
    use axum::body::Body;
    use futures_util::StreamExt;
    let h = harness_patched(|c| c.events.poll_ms = 50).await;
    access(
        &h,
        &h.admin_token,
        "POST",
        "/entries",
        Some(json!({"kind":"email","value":"guest@other.org"})),
    )
    .await;
    let guest = token_for(&h, "guest@other.org", TokenKind::Interactive).await;
    let res = call_raw(
        &h.app,
        axum::http::Request::get("/api/v1/events")
            .header("authorization", format!("Bearer {guest}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    let mut body = res.into_body().into_data_stream();
    let _cursor = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
        .await
        .expect("cursor");
    access(
        &h,
        &h.admin_token,
        "DELETE",
        "/entries/email/guest@other.org",
        None,
    )
    .await;
    let end = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while body.next().await.is_some() {}
    })
    .await;
    assert!(
        end.is_ok(),
        "the stream did not end after the owner's removal"
    );
}

#[tokio::test]
async fn remove_then_disable_then_re_add_leaves_the_old_tokens_revoked() {
    let h = harness().await;
    let entry = json!({"kind":"email","value":"guest@other.org"});
    access(&h, &h.admin_token, "POST", "/entries", Some(entry.clone())).await;
    let guest = token_for(&h, "guest@other.org", TokenKind::Interactive).await;
    access(
        &h,
        &h.admin_token,
        "DELETE",
        "/entries/email/guest@other.org",
        None,
    )
    .await;
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/users/guest@other.org/disable",
            None
        )
        .await
        .0,
        200
    );
    access(&h, &h.admin_token, "POST", "/entries", Some(entry)).await;
    call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/users/guest@other.org/enable",
        None,
    )
    .await;
    assert_eq!(
        call(&h.app, Some(&guest), "GET", "/api/v1/me", None)
            .await
            .0,
        401
    );
}

#[tokio::test]
async fn a_database_only_guest_can_issue_their_own_token() {
    let h = harness().await;
    access(
        &h,
        &h.admin_token,
        "POST",
        "/entries",
        Some(json!({"kind":"email","value":"guest@other.org"})),
    )
    .await;
    let guest = token_for(&h, "guest@other.org", TokenKind::Interactive).await;
    let (s, b) = call(
        &h.app,
        Some(&guest),
        "POST",
        "/api/v1/tokens",
        Some(json!({"kind":"automation","label":"ci"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
}

#[tokio::test]
async fn disable_refuses_a_configured_admin_whatever_the_spelling() {
    let h = harness().await;
    sql(&h, "INSERT INTO admin_grants (email, added_by, added_at) VALUES ('ada@example.com','dan@example.com',now())").await;
    let ada = token_for(&h, "ada@example.com", TokenKind::Interactive).await;
    for path in [
        "/api/v1/users/DAN@Example.COM/disable",
        "/api/v1/users/%20dan@example.com%20/disable",
    ] {
        let (status, body) = call(&h.app, Some(&ada), "POST", path, None).await;
        assert_eq!(
            (status, body["message"].as_str()),
            (400, Some("admin in reeve.toml; change it there")),
            "{path}: {body}"
        );
    }
    assert_eq!(
        call(&h.app, Some(&h.admin_token), "GET", "/api/v1/me", None)
            .await
            .0,
        200
    );
}
