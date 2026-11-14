//! The write tools of `/mcp` (GRA-661), through the router, with real tokens from the sign-in flow: each tool
//! writes what its API route writes, answers a small receipt, records the client, needs the `write` scope, counts
//! against the write limit, and can never override the readiness gate.
#![cfg(feature = "static-oidc")]
mod common;
use common::oauth::flow;
use common::*;
use serde_json::json;

const DAN: &str = "dan@example.com";

/// How many rows `table` has.
async fn rows(h: &Harness, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
}

/// The reserved prefix, at the API: the four routes that store a caller's idempotency key in `stories` or
/// `comments` refuse a key in the MCP callers' space, and write nothing. The control is the same request with a
/// key outside it.
#[tokio::test]
async fn the_api_refuses_an_idempotency_key_in_the_mcp_callers_space() {
    let f = flow().await;
    let h = &f.h;
    let epic = ready_epic(h, "rk-e1").await;
    let story = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "rk-s1", "title": "Booking", "kind": "story", "epic": epic, "criteria": [], "labels": []}),
    )
    .await);
    let decision = post(
        h,
        "/api/v1/decisions",
        json!({"idempotency_key": "rk-d1", "on": epic, "title": "Which queue", "decider": DAN}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    let reserved = "mcp:bob@example.com:k1";
    let story_body = |idem: &str| json!({"idempotency_key": idem, "title": "Planted", "kind": "story", "criteria": [], "labels": []});
    let comment_body = |idem: &str| json!({"body": "planted", "idempotency_key": idem});
    let routes = [
        (
            "/api/v1/stories".to_string(),
            story_body(reserved),
            story_body("mcpx:k1"),
        ),
        (
            format!("/api/v1/items/{story}/comments"),
            comment_body(reserved),
            comment_body("mcpx:k2"),
        ),
        (
            format!("/api/v1/stories/{story}/comments"),
            comment_body(reserved),
            comment_body("mcpx:k3"),
        ),
        (
            format!("/api/v1/decisions/{decision}/comments"),
            comment_body(reserved),
            comment_body("mcpx:k4"),
        ),
    ];
    for (path, refused, control) in routes {
        let before = (rows(h, "stories").await, rows(h, "comments").await);
        let (status, body) = call(&h.app, Some(&h.admin_token), "POST", &path, Some(refused)).await;
        assert_eq!(
            (status, body["code"].as_str(), body["message"].as_str()),
            (
                400,
                Some("validation"),
                Some("an idempotency key must not start with mcp: (reserved for MCP clients)")
            ),
            "{path}"
        );
        assert_eq!(
            (rows(h, "stories").await, rows(h, "comments").await),
            before,
            "{path}: nothing written"
        );
        let (status, body) = call(&h.app, Some(&h.admin_token), "POST", &path, Some(control)).await;
        assert_eq!(status, 200, "the control on {path}: {body}");
    }
}
