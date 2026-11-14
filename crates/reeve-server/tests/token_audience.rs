//! Every authenticated surface other than `/mcp` refuses an MCP token (kb/ADR-0029 R6); the token page neither
//! lists nor revokes one; disabling its owner revokes it (kb/ADR-0015 R6).
mod common;
use axum::{body::Body, http::Request};
use common::*;
use serde_json::json;

async fn live(h: &Harness, id: i64) -> bool {
    sqlx::query_scalar("SELECT revoked_at IS NULL FROM tokens WHERE id = $1")
        .bind(id)
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn every_api_surface_refuses_an_mcp_token() {
    let h = harness().await;
    let mcp = issue_mcp_token(&h, "dan@example.com").await;
    let t = Some(mcp.secret.as_str());
    let attachment = "/api/v1/attachments/00000000-0000-4000-8000-000000000000";
    // The controls: the same person's API token reaches each route.
    assert_eq!(
        call(&h.app, Some(&h.admin_token), "GET", "/api/v1/me", None)
            .await
            .0,
        200
    );
    assert_eq!(
        call(&h.app, Some(&h.admin_token), "GET", attachment, None)
            .await
            .0,
        404
    );

    assert_eq!(call(&h.app, t, "GET", "/api/v1/me", None).await.0, 401);
    assert_eq!(
        call(
            &h.app,
            t,
            "POST",
            "/api/v1/tokens",
            Some(json!({"kind": "interactive"}))
        )
        .await
        .0,
        401
    );
    assert_eq!(call(&h.app, t, "GET", "/api/v1/events", None).await.0, 401);
    assert_eq!(call(&h.app, t, "GET", attachment, None).await.0, 401);
    let as_cookie = call_raw(
        &h.app,
        Request::get("/api/v1/me")
            .header("cookie", format!("reeve_session={}", mcp.secret))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(as_cookie.status(), 401);
    assert!(live(&h, mcp.id).await);
}

#[tokio::test]
async fn logout_does_not_revoke_an_mcp_token() {
    let h = harness().await;
    let mcp = issue_mcp_token(&h, "dan@example.com").await;
    let logout = |secret: &str| {
        Request::post("/auth/logout")
            .header("cookie", format!("reeve_session={secret}"))
            .header("x-reeve-client", "web")
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(call_raw(&h.app, logout(&mcp.secret)).await.status(), 303);
    assert!(
        live(&h, mcp.id).await,
        "logout must leave an MCP token alone"
    );
    // The control: the same request with an API token revokes it.
    let admin_id: i64 = sqlx::query_scalar("SELECT id FROM tokens WHERE hash = $1")
        .bind(reeve_identity::hash_secret(&h.admin_token))
        .fetch_one(&h.state.pool)
        .await
        .unwrap();
    assert_eq!(call_raw(&h.app, logout(&h.admin_token)).await.status(), 303);
    assert!(!live(&h, admin_id).await);
}

#[tokio::test]
async fn the_token_page_neither_lists_nor_revokes_an_mcp_token_and_disabling_its_owner_revokes_it()
{
    let h = harness().await;
    let mcp = issue_mcp_token(&h, &h.victim_email).await;
    let (st, list) = call(&h.app, Some(&h.victim_token), "GET", "/api/v1/tokens", None).await;
    assert_eq!(st, 200);
    let ids: Vec<i64> = list["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ids.len(),
        1,
        "the control: bob's own API token is listed: {list}"
    );
    assert!(!ids.contains(&mcp.id), "{list}");
    let (st, _) = call(
        &h.app,
        Some(&h.victim_token),
        "DELETE",
        &format!("/api/v1/tokens/{}", mcp.id),
        None,
    )
    .await;
    assert_eq!(st, 404);
    assert!(live(&h, mcp.id).await);
    let (st, _) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/users/bob@example.com/disable",
        None,
    )
    .await;
    assert_eq!(st, 200);
    assert!(
        !live(&h, mcp.id).await,
        "disabling the user revokes every audience"
    );
}
