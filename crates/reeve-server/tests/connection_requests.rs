//! The approval page's API (GRA-657; kb/ADR-0029 R4, R9, R11): reading a request, approving and refusing it, and
//! every proof an answer needs. Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
mod common;
use axum::{body::Body, http::Request};
use common::{oauth::*, *};
use serde_json::{Value, json};

async fn entries(h: &Harness, action: &str) -> Vec<Value> {
    let (status, body) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/activity?action={action}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body["value"].as_array().unwrap().clone()
}

/// The status of an answer, and whether it says that no cache may store it.
async fn status_of(f: &Flow, req: Request<Body>) -> (u16, bool) {
    let (status, headers, _) = answer_of(&f.h.app, req).await;
    (status, is_no_store(&headers))
}

/// A person admitted by a database entry, not by the configured floor: the one kind of person a test can take
/// access away from without disabling them. Returns an interactive token's secret.
async fn admitted_by_an_entry(h: &Harness, email: &str) -> String {
    sqlx::query("INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email', $1, 'dan@example.com', $2)")
        .bind(email)
        .bind(h.started)
        .execute(&h.state.pool)
        .await
        .unwrap();
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = h.state.activity.clone();
    let policy = h.state.access.load(&h.state.identity).await.unwrap();
    reeve_identity::issue_token(
        &h.state.pool,
        &*h.state.tokens,
        &*log,
        &policy,
        "test",
        reeve_protocol::Via::Interactive,
        email,
        reeve_protocol::TokenKind::Interactive,
        "guest",
        None,
        h.started,
    )
    .await
    .unwrap()
    .secret
}

/// A second interactive token of dan@example.com (another browser's session).
async fn second_session(h: &Harness) -> String {
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = h.state.activity.clone();
    let policy = h.state.access.load(&h.state.identity).await.unwrap();
    reeve_identity::issue_token(
        &h.state.pool,
        &*h.state.tokens,
        &*log,
        &policy,
        "test",
        reeve_protocol::Via::Interactive,
        "dan@example.com",
        reeve_protocol::TokenKind::Interactive,
        "another browser",
        None,
        h.started,
    )
    .await
    .unwrap()
    .secret
}

/// Catches a read that works signed out, in a browser that did not start the request, or for another request's
/// cookie; a view that does not show what kb/ADR-0029 R4 names (the email, the client, the redirect host); and
/// an answer a cache may keep.
#[tokio::test]
async fn a_request_is_read_only_signed_in_and_in_the_browser_that_started_it() {
    let f = flow().await;
    let dan = f.h.admin_token.clone();
    let s = authorize(&f).await;
    let other = authorize(&f).await;
    let path = request_path(&s, "");
    let get = |session: Option<&str>, connect: Option<&str>| {
        answer_of(&f.h.app, page_request("GET", &path, session, connect, None))
    };
    assert_eq!(get(None, Some(&s.cookie)).await.0, 401, "signed out");
    let (status, headers, _) = get(Some(&dan), None).await;
    assert_eq!(status, 404, "another browser: no connect cookie");
    assert!(is_no_store(&headers), "a refusal is not stored either");
    assert_eq!(
        get(Some(&dan), Some(&other.cookie)).await.0,
        404,
        "another request's cookie"
    );
    assert_eq!(
        get(Some(&dan), Some(&format!("reeve_connect={}", s.id)))
            .await
            .0,
        404,
        "the id itself is not the cookie's value"
    );
    let (status, headers, body) = get(Some(&dan), Some(&s.cookie)).await;
    assert_eq!(status, 200, "{body}");
    assert!(is_no_store(&headers), "the answer holds an approval value");
    let v = &body["value"];
    assert_eq!(
        (
            &v["client_host"],
            &v["client_name"],
            &v["redirect_host"],
            &v["email"]
        ),
        (
            &json!("127.0.0.1"),
            &json!("Example App"),
            &json!("client.example"),
            &json!("dan@example.com")
        )
    );
    assert_eq!(v["approval"].as_str().unwrap().len(), 48, "{v}");
    assert_eq!(v["requested_at"], json!(f.h.started), "{v}");
    assert!(
        !body.to_string().contains(CHALLENGE),
        "the challenge is not the page's business"
    );
    assert!(
        entries(&f.h, "connection_approved").await.is_empty(),
        "reading approves nothing"
    );
}

/// Catches a read by an automation token (kb/ADR-0015 R7), which would hand it an approval value and mark the
/// request read. The control: an interactive token still reads it afterwards, and approves with that value.
#[tokio::test]
async fn an_automation_token_cannot_read_a_request() {
    let f = flow().await;
    let s = authorize(&f).await;
    let (status, headers, body) = read(&f, &f.h.bot_token, &s).await;
    assert_eq!((status, is_no_store(&headers)), (403, true), "{body}");
    assert_eq!(body["value"]["approval"], Value::Null, "{body}");
    let dan = f.h.admin_token.clone();
    let value = approval(&f, &dan, &s).await;
    assert_eq!(approve(&f, &dan, &s, &value, "read").await.0, 200);
}

/// Catches an approval that sends the browser anywhere but the validated address, one without `state` or `iss`,
/// one a cache may keep, one that leaves the connect cookie or the request alive, and a missing, wrong or
/// uncommitted activity entry (it is read back through the API, on another connection).
#[tokio::test]
async fn approving_answers_the_clients_address_with_a_code_state_and_iss_and_writes_the_entry() {
    let f = flow().await;
    let dan = f.h.admin_token.clone();
    let s = authorize(&f).await;
    let value = approval(&f, &dan, &s).await;
    let (status, headers, body) = approve(&f, &dan, &s, &value, "read").await;
    assert_eq!(status, 200, "{body}");
    assert!(is_no_store(&headers), "the answer holds a code");
    let to = url::Url::parse(body["value"]["redirect_to"].as_str().unwrap()).unwrap();
    assert_eq!(without_query(&to), CALLBACK);
    let code = param(&to, "code").expect("a code");
    assert!(code.starts_with("reeve_code_"), "{code}");
    assert_eq!(param(&to, "state").as_deref(), Some("s-1"));
    assert_eq!(param(&to, "iss").as_deref(), Some(ISSUER));
    assert_eq!(param(&to, "error"), None);
    let cleared = headers["set-cookie"].to_str().unwrap();
    assert!(
        cleared.starts_with("reeve_connect=;") && cleared.contains("Max-Age=0"),
        "{cleared}"
    );
    let log = entries(&f.h, "connection_approved").await;
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(
        (
            &log[0]["actor"],
            &log[0]["via"],
            &log[0]["entity_key"],
            &log[0]["payload"]
        ),
        (
            &json!("dan@example.com"),
            &json!("interactive"),
            &json!("dan@example.com"),
            &json!({"client_id": f.client_id, "scope": "read"})
        )
    );
    assert!(
        !log[0].to_string().contains(&code),
        "an entry never holds a code"
    );
    // Answered: a second answer and a read find nothing, and nothing more is written.
    assert_eq!(approve(&f, &dan, &s, &value, "read").await.0, 404);
    assert_eq!(refuse(&f, &dan, &s, &value).await.0, 404);
    assert_eq!(read(&f, &dan, &s).await.0, 404);
    assert_eq!(entries(&f.h, "connection_approved").await.len(), 1);
}

/// Catches a refusal that hands out a code or writes an approval entry, and one without `state` or `iss`.
#[tokio::test]
async fn refusing_answers_access_denied_and_no_code() {
    let f = flow().await;
    let dan = f.h.admin_token.clone();
    let s = authorize(&f).await;
    let value = approval(&f, &dan, &s).await;
    let (status, headers, body) = refuse(&f, &dan, &s, &value).await;
    assert_eq!(status, 200, "{body}");
    let to = url::Url::parse(body["value"]["redirect_to"].as_str().unwrap()).unwrap();
    assert_eq!(without_query(&to), CALLBACK);
    assert_eq!(
        (
            param(&to, "error").as_deref(),
            param(&to, "state").as_deref(),
            param(&to, "iss").as_deref(),
            param(&to, "code")
        ),
        (Some("access_denied"), Some("s-1"), Some(ISSUER), None)
    );
    assert!(
        headers["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert!(is_no_store(&headers));
    assert_eq!(approve(&f, &dan, &s, &value, "read").await.0, 404);
    assert!(entries(&f.h, "connection_approved").await.is_empty());
}

/// kb/ADR-0029 R4 and R11, one proof at a time, for Approve and for Refuse: each request below lacks exactly
/// one, is refused with the status named in an answer no cache may store, and leaves the request answerable, which the last step shows by
/// approving it with every proof present. A refusal that passed only because an earlier one consumed the request
/// would fail that last step.
#[tokio::test]
async fn an_answer_lacking_any_one_proof_is_refused_and_consumes_nothing() {
    let f = flow().await;
    let dan = f.h.admin_token.clone();
    let s = authorize(&f).await;
    let other = authorize(&f).await;
    // A later read replaces the value, so the stale value is taken first.
    let stale = approval(&f, &dan, &s).await;
    let dan_elsewhere = second_session(&f.h).await;
    let current = approval(&f, &dan, &s).await;
    for action in ["/approve", "/refuse"] {
        let path = request_path(&s, action);
        let body = |value: &str| Some(json!({"approval": value, "scope": "write"}));
        let post = |session: Option<&str>, connect: Option<&str>, value: &str| {
            status_of(
                &f,
                page_request("POST", &path, session, connect, body(value)),
            )
        };
        // Every answer of these routes, an extractor's refusal included, may not be stored by a cache.
        let handled = (404, true);
        assert_eq!(
            post(Some(&dan), Some(&s.cookie), &stale).await,
            handled,
            "{action}: a value a later read replaced"
        );
        assert_eq!(
            post(Some(&dan_elsewhere), Some(&s.cookie), &current).await,
            handled,
            "{action}: the current value from a token that did not read the request"
        );
        assert_eq!(
            post(Some(&dan), None, &current).await,
            handled,
            "{action}: no connect cookie"
        );
        assert_eq!(
            post(Some(&dan), Some(&other.cookie), &current).await,
            handled,
            "{action}: another request's connect cookie"
        );
        assert_eq!(
            post(None, Some(&s.cookie), &current).await,
            (401, true),
            "{action}: signed out"
        );
        assert_eq!(
            post(Some(&f.h.bot_token), Some(&s.cookie), &current).await,
            (403, true),
            "{action}: an automation token"
        );
        let mut without_header =
            page_request("POST", &path, Some(&dan), Some(&s.cookie), body(&current));
        without_header.headers_mut().remove("x-reeve-client");
        assert_eq!(
            status_of(&f, without_header).await,
            (403, true),
            "{action}: csrf, a cookie write without the client header"
        );
        // The router's own answer, not a handler's: nothing of the request is in it, and the routes' middleware
        // marks it all the same.
        assert_eq!(
            status_of(
                &f,
                page_request("GET", &path, Some(&dan), Some(&s.cookie), None)
            )
            .await,
            (405, true),
            "{action}: a GET never answers"
        );
    }
    let bad_scope = Some(json!({"approval": current, "scope": "admin"}));
    assert_eq!(
        status_of(
            &f,
            page_request(
                "POST",
                &request_path(&s, "/approve"),
                Some(&dan),
                Some(&s.cookie),
                bad_scope
            )
        )
        .await,
        (400, true),
        "a scope that does not exist"
    );
    assert!(entries(&f.h, "connection_approved").await.is_empty());
    // Nothing was consumed: with every proof present the same value approves.
    let (status, _, answer) = approve(&f, &dan, &s, &current, "write").await;
    assert_eq!(status, 200, "{answer}");
}

/// Catches an approval by a person who was disabled after the page loaded (kb/ADR-0015 R5: checked on this
/// request).
#[tokio::test]
async fn a_person_disabled_after_reading_cannot_approve() {
    let f = flow().await;
    let bob = f.h.victim_token.clone();
    let s = authorize(&f).await;
    let value = approval(&f, &bob, &s).await;
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = f.h.state.activity.clone();
    reeve_identity::disable_user(
        &f.h.state.pool,
        &*f.h.state.tokens,
        &*log,
        "test",
        reeve_protocol::Via::System,
        "bob@example.com",
        f.h.state.clock.now(),
    )
    .await
    .unwrap();
    assert_eq!(approve(&f, &bob, &s, &value, "read").await.0, 401);
    assert!(entries(&f.h, "connection_approved").await.is_empty());
}

/// Catches an approval by a person who lost access after the page loaded without being disabled: their allowlist
/// entry was removed (kb/ADR-0015 R5, R6: access is recomputed on this request, and the token is not revoked).
/// The control shows the same person, token and proofs approve while the entry is there.
#[tokio::test]
async fn a_person_who_lost_access_after_reading_cannot_approve() {
    let f = flow().await;
    let guest = admitted_by_an_entry(&f.h, "guest@other.org").await;
    let control = authorize(&f).await;
    let value = approval(&f, &guest, &control).await;
    assert_eq!(
        approve(&f, &guest, &control, &value, "read").await.0,
        200,
        "the control: admitted"
    );
    let s = authorize(&f).await;
    let value = approval(&f, &guest, &s).await;
    sqlx::query("DELETE FROM allowed_entries WHERE value = 'guest@other.org'")
        .execute(&f.h.state.pool)
        .await
        .unwrap();
    let (status, headers, _) = approve(&f, &guest, &s, &value, "read").await;
    assert_eq!((status, is_no_store(&headers)), (401, true));
    assert_eq!(refuse(&f, &guest, &s, &value).await.0, 401);
    assert_eq!(
        entries(&f.h, "connection_approved").await.len(),
        1,
        "the control's entry only"
    );
}

/// A request lives ten minutes on the server's clock, for the read and for both answers: one second before, the
/// control approves; at ten minutes each of the three is the 404 of an unknown request. Catches a handler that
/// reads the system time instead of the state's clock: the harness's clock is fixed at the instant the test
/// started and moved by `advance`, so on the system time the request would still be seconds old.
#[tokio::test]
async fn a_request_ends_ten_minutes_after_authorize_on_the_servers_clock() {
    let f = flow().await;
    let dan = f.h.admin_token.clone();
    let control = authorize(&f).await;
    let s = authorize(&f).await;
    let control_value = approval(&f, &dan, &control).await;
    let value = approval(&f, &dan, &s).await;
    f.h.state
        .clock
        .advance(chrono::Duration::minutes(10) - chrono::Duration::seconds(1));
    assert_eq!(
        approve(&f, &dan, &control, &control_value, "read").await.0,
        200,
        "ten minutes minus one second"
    );
    f.h.state.clock.advance(chrono::Duration::seconds(1));
    assert_eq!(read(&f, &dan, &s).await.0, 404, "read");
    assert_eq!(
        approve(&f, &dan, &s, &value, "read").await.0,
        404,
        "approve"
    );
    assert_eq!(refuse(&f, &dan, &s, &value).await.0, 404, "refuse");
    assert_eq!(
        entries(&f.h, "connection_approved").await.len(),
        1,
        "the control's entry only"
    );
}

/// Approve and Refuse sent at once for one request: exactly one answers. That the two cannot both win is by
/// construction (each is one step under the store's one lock, unit-tested sequentially in identity); this shows
/// the router adds no second path. The assertion holds for either order.
#[tokio::test]
async fn approve_and_refuse_at_once_give_exactly_one_answer() {
    let f = flow().await;
    let dan = f.h.admin_token.clone();
    let s = authorize(&f).await;
    let value = approval(&f, &dan, &s).await;
    let (a, r) = tokio::join!(
        approve(&f, &dan, &s, &value, "read"),
        refuse(&f, &dan, &s, &value)
    );
    let mut statuses = [a.0, r.0];
    statuses.sort();
    assert_eq!(statuses, [200, 404]);
    assert_eq!(
        entries(&f.h, "connection_approved").await.len(),
        usize::from(a.0 == 200)
    );
}

/// With `[mcp]` off the three routes answer 404 (not stored either) even for a request that is live in the
/// store, before the caller is looked at and before any extractor runs: a path that is not UTF-8, which the path
/// extractor refuses with 400, is a 404 too. The request is made with the feature on; a second router over the
/// same state has it off. The end shows the request was live all along, so the 404s are the switch's.
#[tokio::test]
async fn off_the_three_routes_answer_404_first_even_for_a_live_request() {
    let f = flow().await;
    let dan = f.h.admin_token.clone();
    let s = authorize(&f).await;
    let value = approval(&f, &dan, &s).await;
    let mut state = f.h.state.clone();
    let mut config = (*state.config).clone();
    config.mcp.enabled = false;
    state.config = std::sync::Arc::new(config);
    let off = reeve_server::app(state);
    for (method, action, body) in [
        ("GET", "", None),
        (
            "POST",
            "/approve",
            Some(json!({"approval": value, "scope": "read"})),
        ),
        ("POST", "/refuse", Some(json!({"approval": value}))),
    ] {
        let path = request_path(&s, action);
        let signed_in = page_request(method, &path, Some(&dan), Some(&s.cookie), body.clone());
        let (status, headers, _) = answer_of(&off, signed_in).await;
        assert_eq!(
            (status, is_no_store(&headers)),
            (404, true),
            "{method} {action}"
        );
        let signed_out = page_request(method, &path, None, Some(&s.cookie), body.clone());
        let (status, headers, _) = answer_of(&off, signed_out).await;
        assert_eq!(
            (status, is_no_store(&headers)),
            (404, true),
            "{method} {action}: the switch comes before the caller"
        );
        let invalid = page_request(
            method,
            &format!("/api/v1/connection-requests/%FF{action}"),
            Some(&dan),
            Some(&s.cookie),
            body.clone(),
        );
        let (status, headers, _) = answer_of(&off, invalid).await;
        assert_eq!(
            (status, is_no_store(&headers)),
            (404, true),
            "{method} {action}: the switch comes before the path"
        );
        let (status, headers, body) = answer_of(
            &f.h.app,
            page_request(
                method,
                &format!("/api/v1/connection-requests/%FF{action}"),
                Some(&dan),
                Some(&s.cookie),
                body,
            ),
        )
        .await;
        assert_eq!(
            (status, is_no_store(&headers)),
            (400, true),
            "{method} {action}: on, the path extractor's refusal is not stored either: {body}"
        );
    }
    assert_eq!(
        approve(&f, &dan, &s, &value, "read").await.0,
        200,
        "the control: live, with the feature on"
    );
}

/// On an https instance the connect cookie is `__Host-` prefixed and `Secure`, and a request that carries its
/// value under the unprefixed name is another browser. Catches a cookie a sibling subdomain or a plain-http
/// answer could plant.
#[tokio::test]
async fn an_https_instance_sets_and_reads_only_the_host_prefixed_connect_cookie() {
    let f = https_flow().await;
    let params = with(
        authorize_params(&f),
        "resource",
        Some("https://reeve.example.com/mcp"),
    );
    let res = call_raw(&f.h.app, authorize_request(&params)).await;
    let set = cookies_of(&res).join("\n");
    assert!(set.starts_with("__Host-reeve_connect="), "{set}");
    for part in ["Secure", "HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(set.contains(part), "{set} lacks {part}");
    }
    assert!(!set.contains("Domain"), "{set}");
    let s = started(res);
    let value = s
        .cookie
        .strip_prefix("__Host-reeve_connect=")
        .expect(&s.cookie);
    let get = |connect: String| {
        let req = Request::get(request_path(&s, ""))
            .header(
                "cookie",
                format!("__Host-reeve_session={}; {connect}", f.h.admin_token),
            )
            .body(Body::empty())
            .unwrap();
        status_of(&f, req)
    };
    assert_eq!(
        get(format!("reeve_connect={value}")).await.0,
        404,
        "the unprefixed name is not read"
    );
    assert_eq!(get(s.cookie.clone()).await.0, 200);
}
