//! Connections on the API (GRA-669; kb/ADR-0029 R9 and its later observation): a person lists and revokes their
//! own, an admin anyone's, and a revoked connection stops at once. Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
mod common;
use axum::{body::Body, http::Request};
use chrono::Duration;
use common::{oauth::*, *};
use serde_json::{Value, json};

async fn list(h: &Harness, token: &str, owner: Option<&str>) -> (u16, Value) {
    let path = match owner {
        Some(o) => format!("/api/v1/connections?owner={o}"),
        None => "/api/v1/connections".to_string(),
    };
    call(&h.app, Some(token), "GET", &path, None).await
}

fn ids(body: &Value) -> Vec<i64> {
    body["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect()
}

fn revoke_request(token: &str, id: i64) -> Request<Body> {
    Request::delete(format!("/api/v1/connections/{id}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

async fn revoke(h: &Harness, token: &str, id: i64) -> u16 {
    answer_of(&h.app, revoke_request(token, id)).await.0
}

/// Catches a listing that shows another person's connections, a member who can ask for someone else's, a row
/// that lacks what R9 asks for (the client's name, once per connection), a "last refreshed" that is not the
/// refresh's instant, and a secret or a hash in the answer.
#[tokio::test]
async fn a_person_lists_their_own_connections_and_an_admin_anyones() {
    let f = flow().await;
    let h = &f.h;
    let bobs = connect(&f, &h.victim_token, "read").await;
    let dans = connect(&f, &h.admin_token, "write").await;
    let (_, before) = list(h, &h.victim_token, None).await;
    assert_eq!(
        before["value"][0]["refreshed_at"],
        Value::Null,
        "never refreshed yet"
    );
    // A refresh makes a second token in bob's connection: it is still listed once.
    h.state.clock.advance(Duration::minutes(7));
    assert_eq!(token(&f, &refresh_form(&bobs.refresh_token)).await.0, 200);

    let (status, own) = list(h, &h.victim_token, None).await;
    assert_eq!(status, 200, "{own}");
    assert_eq!(ids(&own), [bobs.connection_id]);
    let row = &own["value"][0];
    assert_eq!(
        (
            &row["owner"],
            &row["client_id"],
            &row["client_host"],
            &row["client_name"],
            &row["scope"]
        ),
        (
            &json!("bob@example.com"),
            &json!(f.client_id),
            &json!("127.0.0.1"),
            &json!("Example App"),
            &json!("read")
        )
    );
    assert_eq!(
        (&row["approved_at"], &row["ends_at"], &row["refreshed_at"]),
        (
            &json!(h.started),
            &json!(h.started + Duration::days(30)),
            &json!(h.started + Duration::minutes(7))
        )
    );
    assert_eq!(
        (&row["revoked_at"], &row["revoked_reason"]),
        (&Value::Null, &Value::Null)
    );
    for secret in [&bobs.access_token, &bobs.refresh_token] {
        assert!(!own.to_string().contains(secret.as_str()));
        assert!(
            !own.to_string()
                .contains(&reeve_identity::hash_secret(secret))
        );
    }
    // Asking for oneself by name is the same list; asking for someone else is the admin's.
    assert_eq!(
        ids(&list(h, &h.victim_token, Some("Bob@Example.com")).await.1),
        [bobs.connection_id]
    );
    assert_eq!(
        list(h, &h.victim_token, Some("dan@example.com")).await.0,
        403
    );
    assert_eq!(
        ids(&list(h, &h.admin_token, None).await.1),
        [dans.connection_id]
    );
    assert_eq!(
        ids(&list(h, &h.admin_token, Some("bob@example.com")).await.1),
        [bobs.connection_id]
    );
    assert_eq!(
        ids(&list(h, &h.bot_token, None).await.1),
        [dans.connection_id],
        "an automation token lists its owner's"
    );
    assert_eq!(
        ids(&list(h, &h.bot_token, Some("bob@example.com")).await.1),
        [bobs.connection_id],
        "an admin's automation token lists another person's"
    );
    assert_eq!(
        call(&h.app, None, "GET", "/api/v1/connections", None)
            .await
            .0,
        401
    );
}

/// kb/ADR-0029 R9: the owner revokes; the connection stops at the next call and cannot refresh; an entry is
/// written. Catches a revocation that only marks the row, one without its entry, a second entry for a second
/// revoke, and a member who can revoke (or even detect) another person's connection.
#[tokio::test]
async fn the_owner_revokes_a_connection_and_it_stops_at_once() {
    let f = flow().await;
    let h = &f.h;
    let c = connect(&f, &h.victim_token, "read").await;
    let dans = connect(&f, &h.admin_token, "read").await;
    assert_eq!(
        mcp_status(&f, &c.access_token).await,
        200,
        "the control: live before"
    );
    assert_eq!(
        revoke(h, &h.victim_token, dans.connection_id).await,
        404,
        "another person's: as if it did not exist"
    );
    assert_eq!(revoke(h, &h.victim_token, 999_999).await, 404);
    assert_eq!(mcp_status(&f, &dans.access_token).await, 200);

    h.state.clock.advance(Duration::minutes(3));
    assert_eq!(revoke(h, &h.victim_token, c.connection_id).await, 200);
    assert_eq!(mcp_status(&f, &c.access_token).await, 401);
    let (status, _, body) = token(&f, &refresh_form(&c.refresh_token)).await;
    assert_eq!(
        (status, &body["error"]),
        (400, &json!("invalid_grant")),
        "{body}"
    );
    let (_, listed) = list(h, &h.victim_token, None).await;
    let row = &listed["value"][0];
    assert_eq!(
        (&row["revoked_at"], &row["revoked_reason"]),
        (&json!(h.started + Duration::minutes(3)), &json!("revoked")),
        "still listed, with when and why"
    );
    let log = revocations(h).await;
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(
        (
            &log[0]["actor"],
            &log[0]["via"],
            &log[0]["entity_key"],
            &log[0]["payload"]
        ),
        (
            &json!("bob@example.com"),
            &json!("interactive"),
            &json!("bob@example.com"),
            &json!({"connection_id": c.connection_id, "client_id": f.client_id, "reason": "revoked"})
        )
    );
    assert_eq!(
        revoke(h, &h.victim_token, c.connection_id).await,
        404,
        "already revoked"
    );
    assert_eq!(revocations(h).await.len(), 1);
}

/// The later observation on kb/ADR-0029: an admin revokes any person's connection with either kind of token.
/// Catches an admin-only path that needs an interactive token (revoking only reduces access), and an entry that
/// names the owner as the actor.
#[tokio::test]
async fn an_admin_revokes_anyones_connection_with_either_kind_of_token() {
    let f = flow().await;
    let h = &f.h;
    let one = connect(&f, &h.victim_token, "read").await;
    let two = connect(&f, &h.victim_token, "write").await;
    assert_eq!(revoke(h, &h.admin_token, one.connection_id).await, 200);
    assert_eq!(
        revoke(h, &h.bot_token, two.connection_id).await,
        200,
        "dan's automation token"
    );
    for c in [&one, &two] {
        assert_eq!(mcp_status(&f, &c.access_token).await, 401);
    }
    let mut seen: Vec<(Value, Value, Value)> = revocations(h)
        .await
        .into_iter()
        .map(|e| {
            (
                e["payload"]["connection_id"].clone(),
                e["actor"].clone(),
                e["via"].clone(),
            )
        })
        .collect();
    seen.sort_by_key(|e| e.0.as_i64());
    assert_eq!(
        seen,
        [
            (
                json!(one.connection_id),
                json!("dan@example.com"),
                json!("interactive")
            ),
            (
                json!(two.connection_id),
                json!("dan@example.com"),
                json!("automation")
            ),
        ]
    );
}

/// Catches a listing that keeps ended and revoked connections for ever, or drops them at once: they show for
/// thirty days after they stopped. Fixed clock, exact boundaries. Also: an ended connection has nothing left to
/// revoke.
#[tokio::test]
async fn ended_and_revoked_connections_are_listed_for_thirty_days() {
    let f = flow().await;
    let h = &f.h;
    let ended = connect(&f, &h.victim_token, "read").await.connection_id;
    let cut = connect(&f, &h.victim_token, "read").await.connection_id;
    assert_eq!(revoke(h, &h.victim_token, cut).await, 200); // revoked at day 0; `ended` ends at day 30
    let listed = |h: &Harness| {
        let (app, token) = (h.app.clone(), h.victim_token.clone());
        async move {
            let mut got = ids(
                &call(&app, Some(&token), "GET", "/api/v1/connections", None)
                    .await
                    .1,
            );
            got.sort();
            got
        }
    };
    let both = {
        let mut b = vec![ended, cut];
        b.sort();
        b
    };
    let clock = &h.state.clock;
    assert_eq!(listed(h).await, both);
    clock.advance(Duration::days(30) - Duration::seconds(1));
    assert_eq!(
        listed(h).await,
        both,
        "one second before day 30: one revoked almost thirty days ago, one still live"
    );
    clock.advance(Duration::seconds(1));
    assert_eq!(
        listed(h).await,
        [ended],
        "day 30: the revoked one stopped thirty days ago; the other has just ended"
    );
    assert_eq!(
        revoke(h, &h.victim_token, ended).await,
        404,
        "an ended connection has nothing to revoke"
    );
    clock.advance(Duration::days(30) - Duration::seconds(1));
    assert_eq!(listed(h).await, [ended], "one second before day 60");
    clock.advance(Duration::seconds(1));
    assert_eq!(
        listed(h).await,
        Vec::<i64>::new(),
        "day 60: ended thirty days ago"
    );
}

/// The developer command's connection is listed like any other, by its fixed name and without a host.
#[tokio::test]
async fn the_developer_commands_connection_is_listed_and_revocable() {
    let h = mcp_harness().await;
    let developer = issue_mcp_token(&h, "dan@example.com").await;
    let (status, body) = list(&h, &h.admin_token, None).await;
    assert_eq!(status, 200, "{body}");
    let row = &body["value"][0];
    assert_eq!(
        (
            &row["client_id"],
            &row["client_host"],
            &row["client_name"],
            &row["scope"]
        ),
        (
            &json!("urn:reeve:developer-command"),
            &json!(""),
            &json!("Developer command"),
            &json!("write")
        )
    );
    let discover = || mcp_modern(&developer.secret, 1, "server/discover", json!({}));
    assert_eq!(
        send(&h.app, discover()).await.0,
        200,
        "the control: live before"
    );
    assert_eq!(
        revoke(&h, &h.admin_token, row["id"].as_i64().unwrap()).await,
        200
    );
    assert_eq!(send(&h.app, discover()).await.0, 401);
}

/// A revoke and a refresh of one connection never interleave: each takes the owner's row first. First the
/// owner's row is held, as a refresh in progress holds it, when a revoke arrives: the revoke waits, having
/// written nothing to the connection, and succeeds once the row is let go. Then a revoke is in progress on
/// another connection (a transaction holds the row and has marked the connection revoked, uncommitted) when a
/// refresh arrives: the refresh waits and then refuses, burning nothing. That second half stands the revoke in
/// with SQL, so it exercises the refresh side only. Catches a refresh that reads the connection without the
/// owner's row (it would see it live and issue a pair for a connection being revoked), and a revoke that writes
/// before it holds the row.
#[tokio::test]
async fn a_revoke_and_a_refresh_wait_for_each_other() {
    let f = flow().await;
    let h = &f.h;
    let c = connect(&f, &h.victim_token, "read").await;
    let hold_bob = || async {
        let mut tx = h.state.pool.begin().await.unwrap();
        sqlx::query("SELECT 1 FROM users WHERE email = 'bob@example.com' FOR UPDATE")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx
    };
    // A refresh in progress; the revoke waits and has not written the connection.
    let held = hold_bob().await;
    let revoking = start(&f, revoke_request(&h.victim_token, c.connection_id));
    until_waiting(h, 1).await;
    assert!(
        !locked(h, "connections", c.connection_id).await,
        "the revoke wrote before it held the owner's row"
    );
    held.rollback().await.unwrap();
    assert_eq!(revoking.await.unwrap().0, 200);

    // A revoke in progress on another connection; the refresh waits and then refuses.
    let d = connect(&f, &h.victim_token, "read").await;
    let mut revoke_in_progress = hold_bob().await;
    sqlx::query("UPDATE connections SET revoked_at = $2, revoked_reason = 'revoked' WHERE id = $1")
        .bind(d.connection_id)
        .bind(h.state.clock.now())
        .execute(&mut *revoke_in_progress)
        .await
        .unwrap();
    let refreshing = start(&f, token_request(&refresh_form(&d.refresh_token)));
    until_waiting(h, 1).await;
    revoke_in_progress.commit().await.unwrap();
    let (status, _, body) = refreshing.await.unwrap();
    assert_eq!(
        (status, &body["error"]),
        (400, &json!("invalid_grant")),
        "{body}"
    );
    let unused: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM refresh_tokens WHERE connection_id = $1 AND used_at IS NULL",
    )
    .bind(d.connection_id)
    .fetch_one(&h.state.pool)
    .await
    .unwrap();
    assert_eq!(unused, 1, "the refused refresh rolled its mark back");
}
