//! Disabling a person and their MCP connections (kb/ADR-0029 R9, kb/ADR-0015 R6), and the lock rule of every
//! transaction that touches a person's connections: the `users` row first, the activity entries last. Compiled
//! only with `static-oidc`.
#![cfg(feature = "static-oidc")]
mod common;
use axum::{body::Body, http::Request};
use common::{oauth::*, *};
use serde_json::{Value, json};

const BOB: &str = "bob@example.com";

fn disable_bob_request(h: &Harness) -> Request<Body> {
    Request::post("/api/v1/users/bob@example.com/disable")
        .header("authorization", format!("Bearer {}", h.admin_token))
        .body(Body::empty())
        .unwrap()
}

/// kb/ADR-0029 R9: "disabling the user MUST revoke it". Catches a connection that survives the disable (it would
/// work again once the person is enabled), a token that still answers, a missing entry or one that does not name
/// who disabled, and another person's connection revoked with it.
#[tokio::test]
async fn disabling_a_person_revokes_their_connections_and_enabling_revives_nothing() {
    let f = flow().await;
    let h = &f.h;
    let one = connect(&f, &h.victim_token, "read").await;
    let two = connect(&f, &h.victim_token, "write").await;
    let dans = connect(&f, &h.admin_token, "read").await;
    assert_eq!(
        mcp_status(&f, &one.access_token).await,
        200,
        "the control: live before"
    );

    assert_eq!(answer_of(&h.app, disable_bob_request(h)).await.0, 200);
    for c in [&one, &two] {
        assert_eq!(
            mcp_status(&f, &c.access_token).await,
            401,
            "at the next call"
        );
        assert_eq!(
            revoked(h, c.connection_id).await.as_deref(),
            Some("user_disabled")
        );
        let (status, _, body) = token(&f, &refresh_form(&c.refresh_token)).await;
        assert_eq!(
            (status, &body["error"]),
            (400, &json!("invalid_grant")),
            "{body}"
        );
    }
    let mut log: Vec<(Value, Value, Value, Value)> = revocations(h)
        .await
        .into_iter()
        .map(|e| {
            (
                e["actor"].clone(),
                e["via"].clone(),
                e["entity_key"].clone(),
                e["payload"].clone(),
            )
        })
        .collect();
    log.sort_by_key(|e| e.3["connection_id"].as_i64());
    let entry = |c: &Connected| {
        (
            json!("dan@example.com"),
            json!("interactive"),
            json!(BOB),
            json!({"connection_id": c.connection_id, "client_id": f.client_id, "reason": "user_disabled"}),
        )
    };
    assert_eq!(
        log,
        [entry(&one), entry(&two)],
        "one entry per connection, by who disabled"
    );
    assert!(revoked(h, dans.connection_id).await.is_none());
    assert_eq!(mcp_status(&f, &dans.access_token).await, 200);

    // Enabling the person again revives nothing: they connect again.
    let (status, body) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/users/bob@example.com/enable",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(mcp_status(&f, &one.access_token).await, 401);
    assert_eq!(token(&f, &refresh_form(&one.refresh_token)).await.0, 400);
}

/// The design's "a refresh racing a disable, and an exchange racing a disable, end with nothing live", made
/// deterministic: a disable is in progress (its transaction has set the flag and not committed) when a refresh
/// and an exchange arrive. Both must wait for it and then refuse. Catches a flow that reads the flag without
/// locking the row (it would see `false` and issue tokens to a person being disabled).
#[tokio::test]
async fn a_refresh_and_an_exchange_wait_for_a_disable_in_progress_and_then_issue_nothing() {
    let f = flow().await;
    let h = &f.h;
    let c = connect(&f, &h.victim_token, "read").await;
    let code = code_for(&f, &h.victim_token, "read").await;
    let mut disable = h.state.pool.begin().await.unwrap();
    sqlx::query("UPDATE users SET disabled = true WHERE email = $1")
        .bind(BOB)
        .execute(&mut *disable)
        .await
        .unwrap();

    let refresh = start(&f, token_request(&refresh_form(&c.refresh_token)));
    until_waiting(h, 1).await;
    let exchange = start(&f, token_request(&code_form(&f, &code)));
    until_waiting(h, 2).await;
    disable.commit().await.unwrap();

    for (what, answered) in [("refresh", refresh), ("exchange", exchange)] {
        let (status, _, body) = answered.await.unwrap();
        assert_eq!(
            (status, &body["error"]),
            (400, &json!("invalid_grant")),
            "{what}: {body}"
        );
    }
    // Nothing new is live: no second connection, no second token, and the refresh token's mark was rolled back.
    let (connections, tokens, unused): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM connections), (SELECT count(*) FROM tokens WHERE audience = 'mcp'), (SELECT count(*) FROM refresh_tokens WHERE used_at IS NULL)",
    )
    .fetch_one(&h.state.pool).await.unwrap();
    assert_eq!((connections, tokens, unused), (1, 1, 1));
}

/// The first lock of a refresh and of a disable is the person's `users` row. While the test holds it, both wait,
/// and neither has written the connection, the token or the refresh token yet. Catches either of the two writing
/// any of them before it holds the user's row (two such flows can deadlock, and a write can slip past a disable).
/// The exchange is probed only against a disable in progress (above); the developer command and
/// `connections-revoke-all` are not probed.
#[tokio::test]
async fn a_refresh_and_a_disable_take_the_users_row_before_anything_else() {
    let f = flow().await;
    let h = &f.h;
    let c = connect(&f, &h.victim_token, "read").await;
    let (token_row, refresh_row) = rows_of(h, &c).await;
    let mut held = h.state.pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM users WHERE email = $1 FOR UPDATE")
        .bind(BOB)
        .execute(&mut *held)
        .await
        .unwrap();

    let refresh = start(&f, token_request(&refresh_form(&c.refresh_token)));
    until_waiting(h, 1).await;
    let disable = start(&f, disable_bob_request(h));
    until_waiting(h, 2).await;
    for (table, id) in [
        ("connections", c.connection_id),
        ("tokens", token_row),
        ("refresh_tokens", refresh_row),
    ] {
        assert!(
            !locked(h, table, id).await,
            "{table} is written before the user's row is held"
        );
    }
    held.commit().await.unwrap();

    let refreshed = refresh.await.unwrap().0;
    assert!(refreshed == 200 || refreshed == 400, "{refreshed}");
    assert_eq!(disable.await.unwrap().0, 200);
    // Whichever went first, nothing is live at the end.
    assert_eq!(
        revoked(h, c.connection_id).await.as_deref(),
        Some("user_disabled")
    );
    assert_eq!(mcp_status(&f, &c.access_token).await, 401);
}

/// The activity entries are written last (the architecture standard: the writer's advisory lock serialises only
/// the commit tail). While the test holds that lock, a disable waits for it, and by then it has already written
/// the connection and the token. Catches an entry recorded before the change it describes.
#[tokio::test]
async fn the_activity_entries_are_written_after_the_rows() {
    let f = flow().await;
    let h = &f.h;
    let c = connect(&f, &h.victim_token, "read").await;
    let (token_row, _) = rows_of(h, &c).await;
    let mut held = h.state.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_activity'))")
        .execute(&mut *held)
        .await
        .unwrap();

    let disable = start(&f, disable_bob_request(h));
    until_waiting(h, 1).await;
    assert!(
        locked(h, "connections", c.connection_id).await,
        "the connection is not written yet"
    );
    assert!(
        locked(h, "tokens", token_row).await,
        "the token is not written yet"
    );
    held.commit().await.unwrap();
    assert_eq!(disable.await.unwrap().0, 200);
}
