//! `POST /oauth/token` (GRA-658; kb/ADR-0029 R4, R5, R6): the authorization code grant and the refresh grant,
//! each answer as RFC 6749 section 5 words it. Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
mod common;
use axum::{body::Body, http::Request};
use chrono::Duration;
use common::{oauth::*, *};
use serde_json::{Value, json};

async fn count(h: &Harness, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
}

/// One wrong request: what it is, its form, and the error it must be answered with.
type Case<'a> = (&'a str, Vec<(&'a str, &'a str)>, &'a str);

/// What every answer of the route must be: JSON that is not the API's envelope, never stored by a cache.
fn assert_oauth_answer(headers: &axum::http::HeaderMap, body: &Value, what: &str) {
    assert!(is_no_store(headers), "{what}");
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json"),
        "{what}"
    );
    assert!(
        body.is_object() && body.get("outcome").is_none(),
        "{what}: not an OAuth object: {body}"
    );
}

fn refused(answer: &(u16, axum::http::HeaderMap, Value), status: u16, error: &str, what: &str) {
    assert_eq!(
        (answer.0, &answer.2["error"]),
        (status, &json!(error)),
        "{what}: {}",
        answer.2
    );
    assert_oauth_answer(&answer.1, &answer.2, what);
    assert!(answer.2.get("access_token").is_none(), "{what}");
}

/// Catches an answer missing a field RFC 6749 section 5.1 names or the two cache headers, a token that does not
/// live exactly an hour, a refresh token stored as itself, a connection that does not record the client, the
/// scope the person chose or thirty days from the approval, and a code that works twice.
#[tokio::test]
async fn a_code_becomes_a_one_hour_token_and_a_refresh_token_once() {
    let f = flow().await;
    let code = code_for(&f, &f.h.admin_token, "read").await;
    assert_eq!(
        count(&f.h, "connections").await,
        0,
        "approving stores nothing yet"
    );
    let (status, headers, body) = token(&f, &code_form(&f, &code)).await;
    assert_eq!(status, 200, "{body}");
    assert_oauth_answer(&headers, &body, "success");
    assert_eq!(
        (&body["token_type"], &body["expires_in"]),
        (&json!("Bearer"), &json!(3600))
    );
    assert_eq!(
        body["scope"], "read",
        "the scope the person chose, not the one requested (read write)"
    );
    let access = body["access_token"].as_str().unwrap();
    let refresh = body["refresh_token"].as_str().unwrap();
    assert!(
        access.starts_with("reeve_") && refresh.starts_with("reeve_refresh_"),
        "{body}"
    );
    assert_eq!(mcp_status(&f, access).await, 200);

    type Row = (
        String,
        String,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let (owner, client_id, client_name, scope, approved_at, ends_at, revoked): Row =
        sqlx::query_as("SELECT owner, client_id, client_name, scope, approved_at, ends_at, revoked_at FROM connections")
            .fetch_one(&f.h.state.pool).await.unwrap();
    assert_eq!(
        (
            owner.as_str(),
            client_id.as_str(),
            client_name.as_str(),
            scope.as_str(),
            approved_at,
            ends_at,
            revoked
        ),
        (
            "dan@example.com",
            f.client_id.as_str(),
            "Example App",
            "read",
            f.h.started,
            f.h.started + Duration::days(30),
            None
        )
    );
    let (kind, audience, expires): (String, String, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as("SELECT kind, audience, expires_at FROM tokens WHERE hash = $1")
            .bind(reeve_identity::hash_secret(access))
            .fetch_one(&f.h.state.pool)
            .await
            .unwrap();
    assert_eq!(
        (kind.as_str(), audience.as_str(), expires),
        ("interactive", "mcp", f.h.started + Duration::hours(1))
    );
    let stored: Vec<(String, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as("SELECT hash, used_at FROM refresh_tokens")
            .fetch_all(&f.h.state.pool)
            .await
            .unwrap();
    assert_eq!(
        stored,
        [(reeve_identity::hash_secret(refresh), None)],
        "stored as its hash, unused"
    );

    // Once: the same correct request again finds nothing and makes nothing.
    refused(
        &token(&f, &code_form(&f, &code)).await,
        400,
        "invalid_grant",
        "replay",
    );
    assert_eq!(count(&f.h, "connections").await, 1);
    assert_eq!(
        mcp_status(&f, access).await,
        200,
        "a replayed code revokes nothing (the design's decision)"
    );
}

/// Each row is one wrong request with a live code; each is refused with the standard's error and changes
/// nothing, which the end shows by exchanging the very same code correctly, as an OAuth 2.1 client sends it (no
/// `redirect_uri`, no `resource`). Catches a code that anyone who saw it can spend, a wrong error code, an
/// accepted client secret, a repeated parameter the grant does not read slipping through, a lenient decoder, an
/// answer in the API's envelope or without `no-store`, and a required `redirect_uri`.
#[tokio::test]
async fn a_wrong_exchange_is_refused_as_the_standard_words_it_and_spends_nothing() {
    let f = flow().await;
    let code = code_for(&f, &f.h.admin_token, "write").await;
    let ok = code_form(&f, &code);
    let also = |name: &'static str, value: &'static str| {
        let mut t = ok.clone();
        t.push((name, value));
        t
    };
    let cases: Vec<Case> = vec![
        (
            "another client",
            form_with(
                ok.clone(),
                "client_id",
                Some("http://127.0.0.1:1/other.json"),
            ),
            "invalid_grant",
        ),
        (
            "another redirect address",
            form_with(
                ok.clone(),
                "redirect_uri",
                Some("https://client.example/other"),
            ),
            "invalid_grant",
        ),
        (
            "a wrong verifier",
            form_with(
                ok.clone(),
                "code_verifier",
                Some("a-wrong-verifier-of-43-characters-000000000"),
            ),
            "invalid_grant",
        ),
        (
            "a verifier that is not one",
            form_with(ok.clone(), "code_verifier", Some("short")),
            "invalid_grant",
        ),
        (
            "an unknown code",
            form_with(ok.clone(), "code", Some("reeve_code_000000")),
            "invalid_grant",
        ),
        (
            "another resource",
            form_with(ok.clone(), "resource", Some(ISSUER)),
            "invalid_target",
        ),
        (
            "the resource twice, both right",
            also("resource", RESOURCE),
            "invalid_target",
        ),
        (
            "no verifier",
            form_with(ok.clone(), "code_verifier", None),
            "invalid_request",
        ),
        (
            "no code",
            form_with(ok.clone(), "code", None),
            "invalid_request",
        ),
        (
            "no client_id",
            form_with(ok.clone(), "client_id", None),
            "invalid_request",
        ),
        (
            "no grant_type",
            form_with(ok.clone(), "grant_type", None),
            "invalid_request",
        ),
        (
            "the code twice",
            also("code", "reeve_code_other"),
            "invalid_request",
        ),
        (
            "scope twice, which this grant does not read",
            {
                let mut t = also("scope", "read");
                t.push(("scope", "write"));
                t
            },
            "invalid_request",
        ),
        (
            "another grant",
            form_with(ok.clone(), "grant_type", Some("password")),
            "unsupported_grant_type",
        ),
        (
            "a client secret",
            also("client_secret", "s3cret"),
            "invalid_client",
        ),
    ];
    for (what, form, error) in cases {
        refused(&token(&f, &form).await, 400, error, what);
    }
    // Raw bodies: what a form serialiser would never write.
    let good = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&ok)
        .finish();
    for (what, body) in [
        (
            "a percent escape that is not two hex digits",
            format!("{good}&x=%zz"),
        ),
        (
            "a value that is not UTF-8 once decoded",
            format!("{good}&x=%ff%fe"),
        ),
    ] {
        refused(
            &answer_of(&f.h.app, raw_token_request(&body)).await,
            400,
            "invalid_request",
            what,
        );
    }
    let huge = format!("{good}&x={}", "a".repeat(17 * 1024));
    refused(
        &answer_of(&f.h.app, raw_token_request(&huge)).await,
        413,
        "invalid_request",
        "a body over 16 KiB",
    );
    let as_json = Request::post("/oauth/token")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"grant_type": "authorization_code", "code": code}).to_string(),
        ))
        .unwrap();
    refused(
        &answer_of(&f.h.app, as_json).await,
        400,
        "invalid_request",
        "a JSON body",
    );
    // A request that tries to authenticate with a header: 401, naming the scheme it used (RFC 6749 section 5.2).
    for (sent, scheme) in [
        ("Basic Y2xpZW50OnNlY3JldA==", "Basic"),
        ("Bearer some-token", "Bearer"),
        ("\"odd\" x", "Basic"),
    ] {
        let mut req = token_request(&ok);
        req.headers_mut()
            .insert("authorization", sent.parse().unwrap());
        let answer = answer_of(&f.h.app, req).await;
        refused(&answer, 401, "invalid_client", sent);
        assert_eq!(
            answer.1["www-authenticate"],
            format!("{scheme} realm=\"reeve\""),
            "{sent}"
        );
    }

    assert_eq!(count(&f.h, "connections").await, 0, "nothing was issued");
    // Still unspent: an OAuth 2.1 client's request, without redirect_uri and resource, exchanges it.
    let bare = form_with(
        form_with(ok.clone(), "redirect_uri", None),
        "resource",
        None,
    );
    let (status, _, body) = token(&f, &bare).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["scope"], "write");
}

/// kb/ADR-0029 R4: a code lives at most one minute. The clock is fixed, so the boundary is exact: 59 seconds
/// after the approval the code works, 60 seconds after it does not.
#[tokio::test]
async fn a_code_works_for_one_minute() {
    let f = flow().await;
    let late = code_for(&f, &f.h.admin_token, "read").await;
    f.h.state.clock.advance(Duration::seconds(60));
    refused(
        &token(&f, &code_form(&f, &late)).await,
        400,
        "invalid_grant",
        "60 s after the approval",
    );
    let in_time = code_for(&f, &f.h.admin_token, "read").await;
    f.h.state.clock.advance(Duration::seconds(59));
    assert_eq!(
        token(&f, &code_form(&f, &in_time)).await.0,
        200,
        "59 s after the approval"
    );
}

/// Two correct exchanges of one code at once: one connection. That a code is spent once is by construction
/// (`redeem` removes the request under the store's lock, unit-tested sequentially); this shows the route adds no
/// second path. The assertion holds for either order.
#[tokio::test]
async fn two_correct_exchanges_at_once_make_one_connection() {
    let f = flow().await;
    let code = code_for(&f, &f.h.admin_token, "read").await;
    let form = code_form(&f, &code);
    let (a, b) = tokio::join!(token(&f, &form), token(&f, &form));
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 400]);
    assert_eq!(count(&f.h, "connections").await, 1);
}

/// kb/ADR-0029 R5 and kb/ADR-0015 R5: the person is checked when the code is exchanged, not only when they
/// approved, and the attempt that fails spends the code. Catches a connection for a person disabled in between,
/// and a code that survives a failed exchange.
#[tokio::test]
async fn a_person_disabled_between_approval_and_exchange_gets_nothing_and_the_code_is_spent() {
    let f = flow().await;
    let code = code_for(&f, &f.h.victim_token, "read").await;
    let admin = |path: &'static str| call(&f.h.app, Some(&f.h.admin_token), "POST", path, None);
    assert_eq!(admin("/api/v1/users/bob@example.com/disable").await.0, 200);
    refused(
        &token(&f, &code_form(&f, &code)).await,
        400,
        "invalid_grant",
        "disabled",
    );
    assert_eq!(
        (
            count(&f.h, "connections").await,
            count(&f.h, "refresh_tokens").await
        ),
        (0, 0)
    );
    assert_eq!(admin("/api/v1/users/bob@example.com/enable").await.0, 200);
    refused(
        &token(&f, &code_form(&f, &code)).await,
        400,
        "invalid_grant",
        "the failed attempt spent the code",
    );
}

/// The same for a person who lost access: a guest admitted by a database entry approves, the entry is removed,
/// and the exchange issues nothing. Catches an exchange that trusts the approval's moment.
#[tokio::test]
async fn a_person_who_lost_access_between_approval_and_exchange_gets_nothing() {
    let f = flow().await;
    let guest = guest(&f.h).await;
    let code = code_for(&f, &guest, "read").await;
    set_guest_allowed(&f.h, false).await;
    refused(
        &token(&f, &code_form(&f, &code)).await,
        400,
        "invalid_grant",
        "no longer allowed",
    );
    assert_eq!(count(&f.h, "connections").await, 0);
    set_guest_allowed(&f.h, true).await;
    let fresh = code_for(&f, &guest, "read").await;
    assert_eq!(
        token(&f, &code_form(&f, &fresh)).await.0,
        200,
        "the control: allowed again, a new code works"
    );
}

/// Connecting deletes the person's own connections that stopped more than thirty days ago, with their tokens and
/// refresh tokens, and nothing else. Rows are backdated by SQL against the fixed clock's instant. Catches a
/// cleanup that never runs, one that takes the end date where an earlier revocation is what stopped the
/// connection, one that deletes a connection still inside its thirty days, and one that touches another person.
#[tokio::test]
async fn connecting_clears_the_persons_own_long_stopped_connections_and_nothing_else() {
    let f = flow().await;
    let h = &f.h;
    let dan = h.admin_token.clone();
    let ended = connect(&f, &dan, "read").await.connection_id;
    let revoked = connect(&f, &dan, "read").await.connection_id;
    let recent = connect(&f, &dan, "read").await.connection_id;
    let bobs = connect(&f, &h.victim_token, "read").await.connection_id;
    let now = h.state.clock.now();
    let end_at = |ids: Vec<i64>, days_ago: i64| {
        let pool = h.state.pool.clone();
        async move {
            sqlx::query("UPDATE connections SET approved_at = $2, ends_at = $3 WHERE id = ANY($1)")
                .bind(ids)
                .bind(now - Duration::days(days_ago + 30))
                .bind(now - Duration::days(days_ago))
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    end_at(vec![ended, bobs], 31).await;
    end_at(vec![recent], 29).await;
    // Revoked 31 days ago, though its end date is still ahead.
    sqlx::query("UPDATE connections SET approved_at = $2, revoked_at = $3, revoked_reason = 'revoked' WHERE id = $1")
        .bind(revoked).bind(now - Duration::days(40)).bind(now - Duration::days(31))
        .execute(&h.state.pool).await.unwrap();

    let new = connect(&f, &dan, "read").await.connection_id;
    let mut left: Vec<i64> = sqlx::query_scalar("SELECT id FROM connections")
        .fetch_all(&h.state.pool)
        .await
        .unwrap();
    left.sort();
    let mut expected = vec![recent, bobs, new];
    expected.sort();
    assert_eq!(left, expected);
    let orphans: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM tokens WHERE connection_id = ANY($1)), (SELECT count(*) FROM refresh_tokens WHERE connection_id = ANY($1))",
    )
    .bind(vec![ended, revoked]).fetch_one(&h.state.pool).await.unwrap();
    assert_eq!(
        orphans,
        (0, 0),
        "their tokens and refresh tokens went with them"
    );
}

/// The route answers POST only, in the OAuth object, and with `[mcp]` off it is a plain 404 before anything is
/// read (also for GET: `tests/mcp_discovery.rs`).
#[tokio::test]
async fn the_token_route_answers_post_only_and_nothing_when_off() {
    let f = flow().await;
    let get = Request::get("/oauth/token").body(Body::empty()).unwrap();
    let answer = answer_of(&f.h.app, get).await;
    refused(&answer, 405, "invalid_request", "GET");
    assert_eq!(answer.1["allow"], "POST");
    let off = harness().await;
    let (status, headers, body) =
        send(&off.app, raw_token_request("grant_type=authorization_code")).await;
    assert_eq!((status, body.as_str()), (404, "not found"));
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
}

/// The connection's `revoked_reason`; `None` while it is not revoked.
async fn revoked(h: &Harness, connection_id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT revoked_reason FROM connections WHERE id = $1")
        .bind(connection_id)
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
}

async fn revocations(h: &Harness) -> Vec<Value> {
    let (status, body) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/activity?action=connection_revoked",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body["value"].as_array().unwrap().clone()
}

/// kb/ADR-0029 R5: a refresh token rotates on use. Catches a refresh that needs `client_id` (the standards make
/// it optional here), one that hands back the same refresh token, one that leaves two live refresh tokens, one
/// that changes the scope, and one that revokes the previous access token (the design keeps it until its hour
/// ends: a call in flight must not fail).
#[tokio::test]
async fn a_refresh_issues_a_new_pair_and_retires_the_refresh_token() {
    let f = flow().await;
    let first = connect(&f, &f.h.admin_token, "write").await;
    f.h.state.clock.advance(Duration::minutes(5));
    let (status, headers, body) = token(&f, &refresh_form(&first.refresh_token)).await;
    assert_eq!(status, 200, "without client_id: {body}");
    assert_oauth_answer(&headers, &body, "refresh");
    let (access, refresh) = (
        body["access_token"].as_str().unwrap(),
        body["refresh_token"].as_str().unwrap(),
    );
    assert_ne!(access, first.access_token);
    assert_ne!(refresh, first.refresh_token);
    assert_eq!(
        (&body["token_type"], &body["scope"], &body["expires_in"]),
        (&json!("Bearer"), &json!("write"), &json!(3600))
    );
    assert_eq!(mcp_status(&f, access).await, 200);
    assert_eq!(
        mcp_status(&f, &first.access_token).await,
        200,
        "the previous access token runs out on its own"
    );
    let rows: Vec<(String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT hash, used_at FROM refresh_tokens WHERE connection_id = $1 ORDER BY id",
    )
    .bind(first.connection_id)
    .fetch_all(&f.h.state.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [
            (
                reeve_identity::hash_secret(&first.refresh_token),
                Some(f.h.started + Duration::minutes(5))
            ),
            (reeve_identity::hash_secret(refresh), None),
        ],
        "the old one used at the refresh's instant, the new one live"
    );
    assert_eq!(
        count(&f.h, "connections").await,
        1,
        "a refresh makes no connection"
    );
    assert!(revoked(&f.h, first.connection_id).await.is_none());
    // With its client's id it works as well.
    let with_id = form_with(refresh_form(refresh), "client_id", Some(&f.client_id));
    assert_eq!(token(&f, &with_id).await.0, 200);
}

/// kb/ADR-0029 R5: "using a rotated refresh token again MUST revoke the whole connection". Catches a reuse that
/// is merely refused, a revocation without its entry or with a person as its actor, tokens that keep working
/// after it, and a second entry for a second reuse.
#[tokio::test]
async fn a_used_refresh_token_presented_again_revokes_the_connection() {
    let f = flow().await;
    let first = connect(&f, &f.h.admin_token, "read").await;
    let (status, _, second) = token(&f, &refresh_form(&first.refresh_token)).await;
    assert_eq!(status, 200, "{second}");
    let (access2, refresh2) = (
        second["access_token"].as_str().unwrap(),
        second["refresh_token"].as_str().unwrap(),
    );
    assert!(
        revocations(&f.h).await.is_empty(),
        "the control: nothing is revoked yet"
    );

    refused(
        &token(&f, &refresh_form(&first.refresh_token)).await,
        400,
        "invalid_grant",
        "reuse",
    );
    assert_eq!(
        revoked(&f.h, first.connection_id).await.as_deref(),
        Some("refresh_token_reused")
    );
    let log = revocations(&f.h).await;
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(
        (
            &log[0]["actor"],
            &log[0]["via"],
            &log[0]["entity_key"],
            &log[0]["payload"]
        ),
        (
            &json!("system"),
            &json!("system"),
            &json!("dan@example.com"),
            &json!({"connection_id": first.connection_id, "client_id": f.client_id, "reason": "refresh_token_reused"})
        )
    );
    // Everything of the connection stops at once: both access tokens and the newest refresh token.
    assert_eq!(mcp_status(&f, &first.access_token).await, 401);
    assert_eq!(mcp_status(&f, access2).await, 401);
    refused(
        &token(&f, &refresh_form(refresh2)).await,
        400,
        "invalid_grant",
        "the newest refresh token",
    );
    // A further reuse changes nothing and writes nothing.
    assert_eq!(token(&f, &refresh_form(&first.refresh_token)).await.0, 400);
    assert_eq!(revocations(&f.h).await.len(), 1);
}

/// Each row is a refresh that must be refused without burning the token: the end shows the very same refresh
/// token still works. Catches a used mark that survives a refused request (the person would lose the connection
/// to a typo), a refresh that widens the scope, and one another client can use.
#[tokio::test]
async fn a_refused_refresh_rolls_back_and_the_token_still_works() {
    let f = flow().await;
    let c = connect(&f, &f.h.admin_token, "read").await;
    let ok = refresh_form(&c.refresh_token);
    let cases: Vec<Case> = vec![
        (
            "another client",
            form_with(
                ok.clone(),
                "client_id",
                Some("http://127.0.0.1:1/other.json"),
            ),
            "invalid_grant",
        ),
        (
            "more than was granted",
            form_with(ok.clone(), "scope", Some("read write")),
            "invalid_scope",
        ),
        (
            "a scope that does not exist",
            form_with(ok.clone(), "scope", Some("admin")),
            "invalid_scope",
        ),
        (
            "another resource",
            form_with(ok.clone(), "resource", Some(ISSUER)),
            "invalid_target",
        ),
        (
            "no refresh token",
            form_with(ok.clone(), "refresh_token", None),
            "invalid_request",
        ),
        (
            "an unknown refresh token",
            form_with(ok.clone(), "refresh_token", Some("reeve_refresh_0000")),
            "invalid_grant",
        ),
        (
            "the access token as refresh token",
            form_with(ok.clone(), "refresh_token", Some(&c.access_token)),
            "invalid_grant",
        ),
    ];
    for (what, form, error) in cases {
        refused(&token(&f, &form).await, 400, error, what);
    }
    assert!(
        revoked(&f.h, c.connection_id).await.is_none(),
        "none of these is a reuse"
    );
    // The same token, with what a client may send: the granted scope, offline_access and the resource.
    let full = form_with(
        form_with(ok.clone(), "scope", Some("read offline_access")),
        "resource",
        Some(RESOURCE),
    );
    let (status, _, body) = token(&f, &full).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["scope"], "read");
}

/// kb/ADR-0029 R5: "MUST stop working 30 days after the approval". Fixed clock, exact boundaries. Catches thirty
/// days counted from the exchange or from the last refresh, an access token that outlives the connection, and a
/// refresh at or after the end.
#[tokio::test]
async fn a_connection_ends_thirty_days_after_its_approval() {
    let f = flow().await;
    let clock = &f.h.state.clock;
    let code = code_for(&f, &f.h.admin_token, "read").await; // approved at the start
    clock.advance(Duration::seconds(50));
    let (status, _, body) = token(&f, &code_form(&f, &code)).await; // exchanged 50 s later
    assert_eq!(status, 200, "{body}");
    let refresh = body["refresh_token"].as_str().unwrap().to_string();
    // Thirty seconds before the thirtieth day after the approval.
    clock.advance(Duration::days(30) - Duration::seconds(80));
    let (status, _, body) = token(&f, &refresh_form(&refresh)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["expires_in"], 30, "the token ends with the connection");
    let access = body["access_token"].as_str().unwrap().to_string();
    let refresh = body["refresh_token"].as_str().unwrap().to_string();
    clock.advance(Duration::seconds(29));
    assert_eq!(
        mcp_status(&f, &access).await,
        200,
        "one second before the end"
    );
    clock.advance(Duration::seconds(1));
    assert_eq!(mcp_status(&f, &access).await, 401, "at the end");
    refused(
        &token(&f, &refresh_form(&refresh)).await,
        400,
        "invalid_grant",
        "at the end",
    );
}

/// kb/ADR-0029 R5 ("MUST re-check the person's access when used") and kb/ADR-0015 R6 (losing access revokes
/// nothing). Catches a refresh for a person who is no longer allowed, a connection revoked for it, and a used mark
/// left behind: once access is back, the same refresh token works.
#[tokio::test]
async fn a_person_who_lost_access_cannot_refresh_and_the_connection_is_left_as_it_is() {
    let f = flow().await;
    let guest = guest(&f.h).await;
    let c = connect(&f, &guest, "read").await;
    assert_eq!(mcp_status(&f, &c.access_token).await, 200);
    set_guest_allowed(&f.h, false).await;
    assert_eq!(
        mcp_status(&f, &c.access_token).await,
        401,
        "the access check of every call"
    );
    refused(
        &token(&f, &refresh_form(&c.refresh_token)).await,
        400,
        "invalid_grant",
        "no longer allowed",
    );
    assert!(
        revoked(&f.h, c.connection_id).await.is_none(),
        "left as it is"
    );
    set_guest_allowed(&f.h, true).await;
    assert_eq!(
        token(&f, &refresh_form(&c.refresh_token)).await.0,
        200,
        "the used mark was rolled back"
    );
}

/// Two refreshes with one token at once: one pair is issued, and the loser is a reuse, which revokes the
/// connection (no grace period, kb/ADR-0029 R5 as written). Both hold the owner's row in turn, so the outcome is
/// the same for either order; the assertions hold for both. Catches two pairs from one refresh token.
#[tokio::test]
async fn two_refreshes_at_once_issue_one_pair_and_then_the_connection_is_revoked() {
    let f = flow().await;
    let c = connect(&f, &f.h.admin_token, "read").await;
    let form = refresh_form(&c.refresh_token);
    let (a, b) = tokio::join!(token(&f, &form), token(&f, &form));
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 400]);
    assert_eq!(
        revoked(&f.h, c.connection_id).await.as_deref(),
        Some("refresh_token_reused")
    );
    let winner = if a.0 == 200 { &a.2 } else { &b.2 };
    assert_eq!(
        mcp_status(&f, winner["access_token"].as_str().unwrap()).await,
        401
    );
}

/// Catches expired MCP token rows piling up: a refresh deletes the connection's expired ones, and only those.
#[tokio::test]
async fn a_refresh_deletes_the_connections_expired_token_rows() {
    let f = flow().await;
    let c = connect(&f, &f.h.admin_token, "read").await;
    let other = connect(&f, &f.h.admin_token, "read").await;
    f.h.state.clock.advance(Duration::hours(1));
    assert_eq!(token(&f, &refresh_form(&c.refresh_token)).await.0, 200);
    let tokens_of = |id: i64| {
        let pool = f.h.state.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tokens WHERE connection_id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        tokens_of(c.connection_id).await,
        1,
        "the expired row went, the new one stays"
    );
    assert_eq!(
        tokens_of(other.connection_id).await,
        1,
        "another connection's rows are not this refresh's"
    );
}

/// What the code logged while the guard is alive, as text (as in `tests/mcp.rs`).
#[derive(Clone, Default)]
struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
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

/// The operator gets one `warn` line per refusal and never a secret (the design, section 5). Catches a refusal
/// that is not logged or logged twice, and a line that holds the code, the verifier, a refresh token or the
/// client id the caller sent.
#[tokio::test]
async fn a_refused_token_request_is_logged_once_and_never_with_what_was_sent() {
    let f = flow().await;
    let code = code_for(&f, &f.h.admin_token, "read").await;
    let wrong_verifier = "a-wrong-verifier-of-43-characters-000000000";
    let sent_client = "http://127.0.0.1:1/sent-by-the-caller.json";
    let (logs, guard) = Logs::capture();
    let form = form_with(
        form_with(code_form(&f, &code), "code_verifier", Some(wrong_verifier)),
        "client_id",
        Some(sent_client),
    );
    assert_eq!(token(&f, &form).await.0, 400);
    let text = logs.text();
    assert_eq!(text.matches("token request refused").count(), 1, "{text}");
    for secret in [code.as_str(), wrong_verifier, sent_client, VERIFIER] {
        assert!(!text.contains(secret), "{secret} in: {text}");
    }
    drop(guard);

    let c = connect(&f, &f.h.admin_token, "read").await;
    let (status, _, next) = token(&f, &refresh_form(&c.refresh_token)).await;
    assert_eq!(status, 200);
    let (logs, _guard) = Logs::capture();
    let reuse = form_with(
        refresh_form(&c.refresh_token),
        "client_id",
        Some(sent_client),
    );
    assert_eq!(token(&f, &reuse).await.0, 400);
    let text = logs.text();
    assert_eq!(text.matches("token request refused").count(), 1, "{text}");
    for secret in [
        c.refresh_token.as_str(),
        next["refresh_token"].as_str().unwrap(),
        next["access_token"].as_str().unwrap(),
        sent_client,
    ] {
        assert!(!text.contains(secret), "{secret} in: {text}");
    }
}
