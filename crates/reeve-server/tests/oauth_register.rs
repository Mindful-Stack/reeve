//! `POST /oauth/register` (GRA-668; kb/ADR-0029 R3, R11; RFC 7591): an app that publishes no metadata document
//! registers itself. Every step through the real router, on the harness's fixed clock; time moves only by
//! `clock.advance`, and nothing here contacts another host: a registered client causes no fetch.
#![cfg(feature = "static-oidc")]
mod common;
use axum::{body::Body, http::Request};
use common::{oauth::*, *};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const OTHER: &str = "https://other.example/cb";
/// The largest body the route reads.
const MAX_BODY: usize = 16 * 1024;

fn is_registered_shape(client_id: &str) -> bool {
    client_id.strip_prefix("dyn_").is_some_and(|hex| {
        hex.len() == 48 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// A valid registration padded to exactly `len` bytes with a member the route ignores.
fn padded(len: usize) -> String {
    let empty = json!({"client_name": "x", "redirect_uris": [CALLBACK], "pad": ""}).to_string();
    let body = json!({"client_name": "x", "redirect_uris": [CALLBACK], "pad": "a".repeat(len - empty.len())})
        .to_string();
    assert_eq!(body.len(), len);
    body
}

fn with_content_type(body: &str, content_type: Option<&str>) -> Request<Body> {
    let mut req = Request::post("/oauth/register");
    if let Some(content_type) = content_type {
        req = req.header("content-type", content_type);
    }
    req.body(Body::from(body.to_string())).unwrap()
}

/// Catches an answer that lacks a member RFC 7591 section 3.2.1 requires or carries one that was not registered,
/// a time read from the system clock, an id of another shape, and a registration stored as used.
#[tokio::test]
async fn a_registration_answers_exactly_this() {
    let f = registering_flow().await;
    let (status, headers, body) = register(&f, registration()).await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(headers["content-type"], "application/json");
    assert!(is_no_store(&headers));
    let client_id = body["client_id"].as_str().unwrap().to_string();
    assert!(is_registered_shape(&client_id), "{client_id}");
    assert_eq!(
        body,
        json!({
            "client_id": client_id,
            "client_id_issued_at": f.h.started.timestamp(),
            "client_name": "Example Assistant",
            "redirect_uris": [CALLBACK],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        })
    );
    let rows: Vec<(
        String,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as("SELECT client_id, created_at, last_used_at FROM registered_clients")
        .fetch_all(&f.h.state.pool)
        .await
        .unwrap();
    assert_eq!(rows, [(client_id, f.h.started, None)]);
}

/// One row per kind of answer. Catches a refusal in the API's envelope or without `no-store`, a refusal that
/// stores something, a wrong error code, a body bound that is off by one, a content type compared with its
/// parameters, and an app turned away for what it merely asks for (an authentication method this server
/// replaces by `none`, RFC 7591 section 2).
#[tokio::test]
async fn every_refusal_is_oauth_json_marked_no_store() {
    let f = registering_flow().await;
    let valid = registration().to_string();
    let edited = |edit: fn(&mut Value)| {
        let mut body = registration();
        edit(&mut body);
        register_request(&body.to_string())
    };
    let method = |method: &str| {
        Request::builder()
            .method(method)
            .uri("/oauth/register")
            .header("content-type", "application/json")
            .body(Body::from(valid.clone()))
            .unwrap()
    };
    let rows: Vec<(&str, Request<Body>, u16, Option<&str>)> = vec![
        (
            "an address the instance does not list",
            edited(|b| b["redirect_uris"] = json!([CALLBACK, OTHER])),
            400,
            Some("invalid_redirect_uri"),
        ),
        (
            "a name that is not text",
            edited(|b| b["client_name"] = json!(7)),
            400,
            Some("invalid_client_metadata"),
        ),
        (
            "no redirect address",
            edited(|b| b["redirect_uris"] = json!([])),
            400,
            Some("invalid_client_metadata"),
        ),
        (
            "not JSON",
            register_request("not json"),
            400,
            Some("invalid_client_metadata"),
        ),
        (
            "a form",
            with_content_type(&valid, Some("application/x-www-form-urlencoded")),
            400,
            Some("invalid_client_metadata"),
        ),
        (
            "no content type",
            with_content_type(&valid, None),
            400,
            Some("invalid_client_metadata"),
        ),
        (
            "a parameter after the type",
            with_content_type(&valid, Some("application/json; charset=utf-8")),
            201,
            None,
        ),
        (
            "one byte too many",
            register_request(&padded(MAX_BODY + 1)),
            413,
            Some("invalid_client_metadata"),
        ),
        (
            "exactly the bound",
            register_request(&padded(MAX_BODY)),
            201,
            None,
        ),
        (
            "another authentication method is replaced, not refused",
            edited(|b| b["token_endpoint_auth_method"] = json!("client_secret_basic")),
            201,
            None,
        ),
        (
            "no name: the host of the redirect address",
            edited(|b| {
                b.as_object_mut().unwrap().remove("client_name");
            }),
            201,
            None,
        ),
        ("GET", method("GET"), 405, None),
        ("PUT", method("PUT"), 405, None),
        ("DELETE", method("DELETE"), 405, None),
    ];
    for (what, request, expected, error) in rows {
        let before = registrations(&f.h).await;
        let (status, headers, text) = send(&f.h.app, request).await;
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        assert_eq!(status, expected, "{what}: {text}");
        assert!(is_no_store(&headers), "{what}");
        assert!(
            body.get("ok").is_none(),
            "{what}: not the API's envelope: {text}"
        );
        let stored = registrations(&f.h).await - before;
        match status {
            201 => {
                assert_eq!(stored, 1, "{what}");
                // Whatever was asked for, what is registered is a public client, and no secret is issued.
                assert_eq!(body["token_endpoint_auth_method"], "none", "{what}");
                assert!(
                    body.get("client_secret").is_none()
                        && body.get("client_secret_expires_at").is_none(),
                    "{what}: {text}"
                );
                if what.starts_with("no name") {
                    assert_eq!(body["client_name"], "client.example");
                }
            }
            405 => {
                assert_eq!(stored, 0, "{what}");
                assert_eq!(headers["allow"], "POST", "{what}");
                // Plain text, as the other OAuth routes answer a wrong method.
                assert_eq!(text, "this address answers POST only", "{what}");
            }
            _ => {
                assert_eq!(stored, 0, "{what}");
                assert_eq!(headers["content-type"], "application/json", "{what}");
                assert_eq!(body, json!({"error": error.unwrap()}), "{what}");
            }
        }
    }
}

/// A request whose body counts how often it is asked for bytes: an oversized body with the right content type,
/// which answers 413 once read.
fn counted_request() -> (Request<Body>, Arc<AtomicUsize>) {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let mut chunks = vec![padded(MAX_BODY + 1)].into_iter();
    let body = futures_util::stream::poll_fn(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        std::task::Poll::Ready(chunks.next().map(Ok::<_, std::io::Error>))
    });
    let request = Request::post("/oauth/register")
        .header("content-type", "application/json")
        .body(Body::from_stream(body))
        .unwrap();
    (request, polls)
}

/// With `[mcp]` off, or on with the setting off, the route answers 404 and has not asked the body for a single
/// byte: a 404 alone would only prove the answer. The control is the same request with the setting on, which is
/// read and refused for its size. Catches a switch below the read, and either switch missing.
#[tokio::test]
async fn off_the_route_answers_404_before_it_reads_anything() {
    let f = registering_flow().await;
    let apps = [
        ("no [mcp] section", harness().await.app),
        ("[mcp] on, the setting absent", flow().await.h.app),
        (
            "the setting on, [mcp] off",
            app_with(&f, |c| c.mcp.enabled = false),
        ),
        (
            "[mcp] on, the setting off",
            app_with(&f, |c| c.mcp.dynamic_registration = false),
        ),
    ];
    for (what, app) in apps {
        let (request, polls) = counted_request();
        let (status, headers, body) = send(&app, request).await;
        assert_eq!((status, body.as_str()), (404, "not found"), "{what}");
        assert!(is_no_store(&headers), "{what}");
        assert_eq!(
            polls.load(Ordering::SeqCst),
            0,
            "{what}: the body was not read"
        );
        // Whatever the method: a switched-off instance does not say which methods the route would answer.
        let (status, _, _) = send(
            &app,
            Request::get("/oauth/register").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, 404, "{what}");
    }
    let (request, polls) = counted_request();
    let (status, _, _) = send(&f.h.app, request).await;
    assert_eq!(status, 413, "the control: on, the same request is read");
    assert!(polls.load(Ordering::SeqCst) > 0, "the control");
    assert_eq!(registrations(&f.h).await, 0);
}

/// The line an operator reads to learn which address an app wants listed. Catches a line with the address's
/// query, user information or fragment, a line with the name the app gave itself, and a refusal logged twice.
#[tokio::test]
async fn a_refused_redirect_address_is_logged_once_with_its_host_and_path_only() {
    let f = registering_flow().await;
    let (logs, _guard) = Logs::capture();
    let (status, _, body) = register(
        &f,
        json!({
            "client_name": "NAMESENTINEL",
            "redirect_uris": [CALLBACK, "https://user:pw@wanted.example/cb/path?secret=SENTINEL#frag"],
        }),
    )
    .await;
    assert_eq!(
        (status, body),
        (400, json!({"error": "invalid_redirect_uri"}))
    );
    let text = logs.text();
    assert!(
        text.contains("wanted.example") && text.contains("/cb/path"),
        "{text}"
    );
    for absent in ["SENTINEL", "pw@", "frag"] {
        assert!(!text.contains(absent), "{absent}: {text}");
    }
    assert_eq!(
        text.matches("mcp client registration refused").count(),
        1,
        "{text}"
    );
}

/// Catches a secret, a software statement or a management token that is stored, answered or logged: only the
/// name and the redirect addresses of a request are read.
#[tokio::test]
async fn nothing_secret_is_stored_or_returned() {
    let f = registering_flow().await;
    let (logs, _guard) = Logs::capture();
    let mut body = registration();
    for member in [
        "client_secret",
        "software_statement",
        "registration_access_token",
    ] {
        body[member] = json!("SECRETSENTINEL");
    }
    let (status, _, answer) = register(&f, body).await;
    assert_eq!(status, 201, "{answer}");
    for member in [
        "client_secret",
        "client_secret_expires_at",
        "registration_access_token",
        "software_statement",
    ] {
        assert!(answer.get(member).is_none(), "{member}: {answer}");
    }
    assert!(!answer.to_string().contains("SECRETSENTINEL"));
    assert!(!logs.text().contains("SECRETSENTINEL"), "{}", logs.text());
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM registered_clients r WHERE r::text LIKE '%SECRETSENTINEL%'",
    )
    .fetch_one(&f.h.state.pool)
    .await
    .unwrap();
    assert_eq!((stored, registrations(&f.h).await), (0, 1));
}

/// Catches a table that grows past the cap, a full store that drops a registration somebody approved, one that
/// refuses while a never-used registration could go, and expired registrations that are kept.
#[tokio::test]
async fn the_cap_holds_and_never_drops_a_used_registration() {
    let f = registering_flow().await;
    let cap = reeve_identity::MAX_REGISTERED_CLIENTS;
    sqlx::query(
        "INSERT INTO registered_clients (client_id, client_name, redirect_uris, created_at, last_used_at) \
         SELECT 'dyn_' || lpad(to_hex(n), 48, '0'), 'used', ARRAY[$1::text], $2, $2 \
         FROM generate_series(1::bigint, $3) n",
    )
    .bind(CALLBACK)
    .bind(f.h.state.clock.now())
    .bind(cap)
    .execute(&f.h.state.pool)
    .await
    .unwrap();
    let used = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM registered_clients WHERE client_name = 'used'",
        )
        .fetch_one(&f.h.state.pool)
        .await
        .unwrap()
    };
    let (status, headers, body) = register(&f, registration()).await;
    assert_eq!(
        (status, body),
        (503, json!({"error": "temporarily_unavailable"}))
    );
    assert_eq!(headers["retry-after"], "3600");
    assert!(is_no_store(&headers));
    assert_eq!((registrations(&f.h).await, used().await), (cap, cap));

    // One of them was never used: it is the one that goes.
    let seventh = format!("dyn_{:048x}", 7);
    sqlx::query("UPDATE registered_clients SET last_used_at = NULL WHERE client_id = $1")
        .bind(&seventh)
        .execute(&f.h.state.pool)
        .await
        .unwrap();
    registered(&f, registration()).await;
    assert_eq!((registrations(&f.h).await, used().await), (cap, cap - 1));
    let gone: i64 =
        sqlx::query_scalar("SELECT count(*) FROM registered_clients WHERE client_id = $1")
            .bind(&seventh)
            .fetch_one(&f.h.state.pool)
            .await
            .unwrap();
    assert_eq!(gone, 0);

    // Ninety days on, the used ones and the one made above (never used, older than a day) have all expired.
    f.h.state.clock.advance(chrono::Duration::days(90));
    registered(&f, registration()).await;
    assert_eq!(registrations(&f.h).await, 1);
}

/// Where authorize sends the browser for `client_id` and `redirect_uri` on `app`.
async fn went(f: &Flow, app: &axum::Router, client_id: &str, redirect_uri: &str) -> String {
    let params = with(
        with(authorize_params(f), "client_id", Some(client_id)),
        "redirect_uri",
        Some(redirect_uri),
    );
    let res = call_raw(app, authorize_request(&params)).await;
    assert_eq!(res.status(), 303);
    res.headers()["location"].to_str().unwrap().to_string()
}

/// One row per reason a registered client may not start a connect; the first row is the one that may. Catches
/// a client sent to an address it did not register or the instance no longer lists, an id this server never
/// gave out, a dropped or an expired registration that still works, and an instance that accepts registered
/// clients with the setting off. A refusal never goes to the client's address.
#[tokio::test]
async fn a_registered_client_starts_a_connect_only_while_it_may() {
    let f = registering_flow_with(&[CALLBACK, OTHER]).await;
    let id = registered(&f, registration()).await;
    let invalid = "/connect?problem=invalid_request";
    let not_enabled = "/connect?problem=app_not_enabled";

    let to = went(&f, &f.h.app, &id, CALLBACK).await;
    assert!(to.starts_with("/connect?request="), "{to}");

    let refusals = [
        (
            "an address the instance lists and the client did not register",
            went(&f, &f.h.app, &id, OTHER).await,
            invalid,
        ),
        (
            "an id this server did not give out",
            went(&f, &f.h.app, &format!("dyn_{}", "0".repeat(48)), CALLBACK).await,
            invalid,
        ),
        (
            "the address left the instance's list",
            went(
                &f,
                &app_with(&f, |c| c.mcp.redirect_uris = vec![OTHER.into()]),
                &id,
                CALLBACK,
            )
            .await,
            not_enabled,
        ),
        (
            "the setting is off: the remedy is the admin's",
            went(
                &f,
                &app_with(&f, |c| c.mcp.dynamic_registration = false),
                &id,
                CALLBACK,
            )
            .await,
            not_enabled,
        ),
    ];
    for (what, to, expected) in &refusals {
        assert_eq!(to, expected, "{what}");
        assert!(!to.contains("client.example"), "{what}: {to}");
    }

    // Dropped.
    sqlx::query("DELETE FROM registered_clients WHERE client_id = $1")
        .bind(&id)
        .execute(&f.h.state.pool)
        .await
        .unwrap();
    assert_eq!(went(&f, &f.h.app, &id, CALLBACK).await, invalid, "dropped");

    // Expired: the row is still stored (nothing deletes it until the next registration), and is refused.
    let fresh = registered(&f, registration()).await;
    let one_second = chrono::Duration::seconds(1);
    f.h.state
        .clock
        .advance(chrono::Duration::hours(24) - one_second);
    let to = went(&f, &f.h.app, &fresh, CALLBACK).await;
    assert!(to.starts_with("/connect?request="), "a second short: {to}");
    f.h.state.clock.advance(one_second);
    assert_eq!(registrations(&f.h).await, 1, "still stored");
    assert_eq!(
        went(&f, &f.h.app, &fresh, CALLBACK).await,
        invalid,
        "expired"
    );
}

/// A registered client's id names no host, and everything else it says of itself is its own claim. Catches a
/// connection that is listed by the name alone, an approval entry that names the app by its id only, and a
/// change recorded as made through `dyn_…`: each names the host of the redirect address, kept with the
/// connection.
#[tokio::test]
async fn a_registered_clients_connection_and_its_changes_name_its_redirect_host() {
    let mut f = registering_flow().await;
    f.client_id = registered(&f, registration()).await;
    let h = &f.h;
    let connected = connect(&f, &h.admin_token, "write").await;

    let listed = api(h, &h.admin_token, "/api/v1/connections").await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(
        json!([
            listed[0]["client_id"],
            listed[0]["client_host"],
            listed[0]["client_name"]
        ]),
        json!([f.client_id, "client.example", "Example Assistant"])
    );
    let approvals = api(
        h,
        &h.admin_token,
        "/api/v1/activity?action=connection_approved",
    )
    .await;
    assert_eq!(
        approvals[0]["payload"],
        json!({
            "client_id": f.client_id,
            "client_host": "client.example",
            "client_name": "Example Assistant",
            "scope": "write",
        })
    );

    let epic = ready_epic(h, "reg-e1").await;
    answer(
        h,
        &connected.access_token,
        "add_comment",
        json!({"item": epic, "idempotency_key": "c1", "body": "Through a registered app."}),
    )
    .await;
    let entries = api(
        h,
        &h.admin_token,
        &format!("/api/v1/activity?entity_key={epic}"),
    )
    .await;
    let entry = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "commented")
        .unwrap_or_else(|| panic!("no comment entry: {entries}"));
    assert_eq!(entry["payload"]["client"], "client.example", "{entry}");
}
