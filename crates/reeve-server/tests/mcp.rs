//! `/mcp` through the router: the order of the checks, then the methods (kb/ADR-0029 R1, R2, R6, R8).
mod common;
use axum::{body::Body, http::Request};
use common::*;
use serde_json::{Value, json};

const DISABLED: &str = "MCP is not enabled on this instance ([mcp] enabled)";

fn rpc(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text:?}"))
}

/// Every check before dispatch passed: the unknown method was looked up and not found.
fn passed(status: u16, body: &str) -> bool {
    status == 404 && serde_json::from_str::<Value>(body).is_ok_and(|v| v["error"]["code"] == -32601)
}

async fn admin_mcp(h: &Harness) -> String {
    issue_mcp_token(h, "dan@example.com").await.secret
}

/// A well-formed 2026-07-28 request for a method that does not exist, with `bearer` and `cookie` as given.
fn knock(bearer: Option<&str>, cookie: Option<&str>) -> Request<Body> {
    let mut req = mcp_modern(bearer.unwrap_or(""), 1, "nope/unknown", json!({}));
    if bearer.is_none() {
        req.headers_mut().remove("authorization");
    }
    if let Some(c) = cookie {
        req.headers_mut()
            .insert("cookie", format!("reeve_session={c}").parse().unwrap());
    }
    req
}

#[tokio::test]
async fn the_switch_comes_before_everything() {
    let h = harness().await; // no [mcp] section: off
    let t = admin_mcp(&h).await;
    for req in [
        Request::get("/mcp").body(Body::empty()).unwrap(),
        Request::post("/mcp").body(Body::empty()).unwrap(),
        Request::get("/mcp")
            .header("authorization", format!("Bearer {t}"))
            .body(Body::empty())
            .unwrap(),
        mcp_modern(&t, 1, "server/discover", json!({})),
    ] {
        let (status, headers, body) = send(&h.app, req).await;
        assert_eq!((status, body.as_str()), (404, DISABLED));
        assert!(!headers.contains_key("www-authenticate"));
    }
}

#[tokio::test]
async fn get_and_delete_are_405_whatever_else_they_carry() {
    let h = mcp_harness().await;
    for method in ["GET", "DELETE"] {
        // Nothing, a session id, and a foreign Origin (which a POST would be refused for with 403).
        for (name, value) in [
            ("x-nothing", "1"),
            ("mcp-session-id", "abc"),
            ("origin", "https://evil.example"),
        ] {
            let req = Request::builder()
                .method(method)
                .uri("/mcp")
                .header(name, value);
            let (status, headers, _) = send(&h.app, req.body(Body::empty()).unwrap()).await;
            assert_eq!(status, 405, "{method} with {name}");
            assert_eq!(headers["allow"], "POST");
        }
    }
}

#[tokio::test]
async fn a_foreign_origin_is_403_before_the_token_is_looked_at() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let with_origin = |origin: &str, bearer: Option<&str>| {
        let mut req = knock(bearer, None);
        req.headers_mut().insert("origin", origin.parse().unwrap());
        req
    };
    assert_eq!(
        send(&h.app, with_origin("https://evil.example", None))
            .await
            .0,
        403
    );
    assert_eq!(
        send(&h.app, with_origin("https://evil.example", Some(&t)))
            .await
            .0,
        403
    );
    // The controls: no Origin reaches the token check; the instance's own origin and no Origin pass.
    assert_eq!(send(&h.app, knock(None, None)).await.0, 401);
    let (s, _, b) = send(&h.app, with_origin("http://127.0.0.1:7474", Some(&t))).await;
    assert!(passed(s, &b), "{s} {b}");
    let (s, _, b) = send(&h.app, knock(Some(&t), None)).await;
    assert!(passed(s, &b), "{s} {b}");
}

#[tokio::test]
async fn without_a_token_the_answer_is_401_whatever_the_body() {
    let h = mcp_harness().await;
    let post = || Request::post("/mcp");
    for req in [
        post()
            .header("content-type", "application/json")
            .body(Body::from("{not json"))
            .unwrap(),
        post()
            .header("content-type", "text/plain")
            .body(Body::from("hello"))
            .unwrap(),
        post()
            .header("content-type", "application/json")
            .body(Body::from(vec![b' '; 2 * 1024 * 1024]))
            .unwrap(),
    ] {
        let (status, headers, _) = send(&h.app, req).await;
        assert_eq!(status, 401);
        assert_eq!(headers["www-authenticate"], "Bearer");
    }
}

#[tokio::test]
async fn only_a_live_mcp_token_in_the_bearer_header_is_accepted() {
    let h = mcp_harness().await;
    let mcp = issue_mcp_token(&h, "dan@example.com").await;
    let (s, _, b) = send(&h.app, knock(Some(&mcp.secret), None)).await;
    assert!(passed(s, &b), "the control: {s} {b}");
    for (what, bearer, cookie) in [
        ("no token", None, None),
        ("the admin's API token", Some(h.admin_token.as_str()), None),
        ("a member's API token", Some(h.victim_token.as_str()), None),
        ("the automation token", Some(h.bot_token.as_str()), None),
        (
            "an MCP token in the session cookie",
            None,
            Some(mcp.secret.as_str()),
        ),
    ] {
        let (status, headers, _) = send(&h.app, knock(bearer, cookie)).await;
        assert_eq!(status, 401, "{what}");
        assert_eq!(headers["www-authenticate"], "Bearer", "{what}");
    }

    let pool = &h.state.pool;
    sqlx::query("INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','guest@other.org','dan@example.com',now())")
        .execute(pool)
        .await
        .unwrap();
    // Each state on a person and a token of its own: accepted before the change, 401 after it.
    for (what, email, change) in [
        (
            "revoked",
            "r@example.com",
            "UPDATE tokens SET revoked_at = now() WHERE owner = $1",
        ),
        (
            "expired",
            "e@example.com",
            "UPDATE tokens SET expires_at = now() - interval '1 minute' WHERE owner = $1",
        ),
        (
            "owner disabled",
            "d@example.com",
            "UPDATE users SET disabled = true WHERE email = $1",
        ),
        (
            "owner no longer allowed",
            "guest@other.org",
            "DELETE FROM allowed_entries WHERE value = $1",
        ),
    ] {
        let t = issue_mcp_token(&h, email).await;
        let (s, _, b) = send(&h.app, knock(Some(&t.secret), None)).await;
        assert!(passed(s, &b), "{what}, before: {s} {b}");
        let changed = sqlx::query(change).bind(email).execute(pool).await.unwrap();
        assert_eq!(changed.rows_affected(), 1, "{what}");
        assert_eq!(
            send(&h.app, knock(Some(&t.secret), None)).await.0,
            401,
            "{what}"
        );
    }
}

#[tokio::test]
async fn a_policy_that_cannot_be_loaded_is_500_without_detail() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    sqlx::query("DROP TABLE allowed_entries")
        .execute(&h.state.pool)
        .await
        .unwrap();
    let (status, _, body) = send(&h.app, knock(Some(&t), None)).await;
    assert_eq!(status, 500);
    assert_eq!(
        rpc(&body),
        json!({"jsonrpc": "2.0", "error": {"code": -32603, "message": "Internal error"}})
    );
}

#[tokio::test]
async fn the_body_is_json_of_at_most_one_mebibyte() {
    const LIMIT: usize = 1024 * 1024;
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    // A valid request padded with spaces to `len` bytes, sent as two chunks with no Content-Length.
    let streamed = |len: usize| {
        let mut bytes = json!({"jsonrpc": "2.0", "id": 1, "method": "nope/unknown", "params": {"_meta": mcp_meta()}})
            .to_string()
            .into_bytes();
        assert!(bytes.len() < len / 2);
        bytes.resize(len, b' ');
        let tail = bytes.split_off(len / 2);
        let chunks = [bytes, tail].map(|c| Ok::<_, std::io::Error>(axum::body::Bytes::from(c)));
        let mut req = knock(Some(&t), None);
        *req.body_mut() = Body::from_stream(futures_util::stream::iter(chunks));
        req
    };
    let (s, _, b) = send(&h.app, streamed(LIMIT)).await;
    assert!(passed(s, &b), "exactly 1 MiB is accepted: {s} {b}");
    assert_eq!(send(&h.app, streamed(LIMIT + 1)).await.0, 413);
    // A body that breaks off while it is read is a bad request, not a too-large one.
    let mut broken = knock(Some(&t), None);
    *broken.body_mut() = Body::from_stream(futures_util::stream::iter([
        Ok(axum::body::Bytes::from_static(b"{")),
        Err(std::io::Error::other("the fixture's read error")),
    ]));
    let (status, _, body) = send(&h.app, broken).await;
    assert_eq!(
        (status, body.as_str()),
        (400, "the request body could not be read")
    );

    let with = |content_type: &str, body: &str| {
        let mut req = knock(Some(&t), None);
        req.headers_mut()
            .insert("content-type", content_type.parse().unwrap());
        *req.body_mut() = Body::from(body.to_string());
        req
    };
    assert_eq!(send(&h.app, with("text/plain", "{}")).await.0, 415);
    let (s, _, b) = send(&h.app, with("application/json; charset=utf-8", "{not json")).await;
    assert_eq!((s, rpc(&b)["error"]["code"].as_i64()), (400, Some(-32700)));
    let (s, _, b) = send(
        &h.app,
        with(
            "application/json",
            r#"[{"jsonrpc":"2.0","id":1,"method":"tools/list"}]"#,
        ),
    )
    .await;
    assert_eq!((s, rpc(&b)["error"]["code"].as_i64()), (400, Some(-32600)));
}

/// A message is known as a notification only once step 6 has parsed it. Before that line a failure is answered as
/// for any request; after it a notification never gets a body.
#[tokio::test]
async fn a_notification_gets_no_body_once_it_has_been_parsed() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let notification = || mcp_legacy(&t, None, "notifications/initialized", json!({}));

    // Before the line. Step 4: no token; the ordinary 401 with its header and text.
    let mut unauthenticated = notification();
    unauthenticated.headers_mut().remove("authorization");
    let (status, headers, body) = send(&h.app, unauthenticated).await;
    assert_eq!((status, body.as_str()), (401, "unauthenticated"));
    assert_eq!(headers["www-authenticate"], "Bearer");
    // Step 6: a body without an id that is not a JSON-RPC message; the ordinary JSON-RPC error, without an id.
    let mut malformed = notification();
    *malformed.body_mut() = Body::from(r#"{"jsonrpc":"1.0","method":"notifications/initialized"}"#);
    let (status, _, body) = send(&h.app, malformed).await;
    let answer = rpc(&body);
    assert_eq!(
        (status, answer["error"]["code"].as_i64()),
        (400, Some(-32600))
    );
    assert!(answer.get("id").is_none(), "{answer}");

    // After the line. One that passes the checks: 202.
    let (status, _, body) = send(&h.app, notification()).await;
    assert_eq!((status, body.as_str()), (202, ""));
    // One that fails a later check (no version anywhere): that check's status, no body.
    let mut unversioned = notification();
    unversioned.headers_mut().remove("mcp-protocol-version");
    let (status, _, body) = send(&h.app, unversioned).await;
    assert_eq!((status, body.as_str()), (400, ""));
    // The control: the same failing message with an id is a request and gets the JSON-RPC error.
    let mut request = mcp_legacy(&t, Some(3), "tools/list", json!({}));
    request.headers_mut().remove("mcp-protocol-version");
    let (status, _, body) = send(&h.app, request).await;
    let answer = rpc(&body);
    assert_eq!(
        (status, answer["error"]["code"].as_i64()),
        (400, Some(-32020))
    );
    assert_eq!(answer["id"], 3);
}

fn server_info() -> Value {
    json!({"name": "reeve", "version": env!("CARGO_PKG_VERSION")})
}

#[tokio::test]
async fn every_row_of_the_classification_table_through_the_router() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    // (version in _meta, MCP-Protocol-Version, method) -> (status, JSON-RPC error code; None for a result)
    type Row = (
        Option<&'static str>,
        Option<&'static str>,
        &'static str,
        u16,
        Option<i64>,
    );
    let rows: [Row; 13] = [
        (
            Some("2026-07-28"),
            Some("2026-07-28"),
            "tools/list",
            200,
            None,
        ),
        (
            Some("2026-07-28"),
            Some("2026-07-28"),
            "initialize",
            404,
            Some(-32601),
        ),
        (Some("2026-07-28"), None, "tools/list", 400, Some(-32020)),
        (
            Some("2026-07-28"),
            Some("2025-11-25"),
            "tools/list",
            400,
            Some(-32020),
        ),
        (None, Some("2026-07-28"), "tools/list", 400, Some(-32020)),
        (None, Some("2025-11-25"), "tools/list", 200, None),
        (
            None,
            Some("2025-11-25"),
            "server/discover",
            404,
            Some(-32601),
        ),
        (None, None, "initialize", 200, None),
        (None, None, "tools/list", 400, Some(-32020)),
        (None, Some("2025-06-18"), "tools/list", 400, Some(-32022)),
        (
            Some("1900-01-01"),
            Some("1900-01-01"),
            "tools/list",
            400,
            Some(-32022),
        ),
        // Step 7 before step 8: an unsupported body version without the header is a mismatch first.
        (Some("1900-01-01"), None, "tools/list", 400, Some(-32020)),
        (
            Some("2025-11-25"),
            Some("2025-11-25"),
            "tools/list",
            400,
            Some(-32600),
        ),
    ];
    for (meta, header, method, status, code) in rows {
        let mut params = json!({});
        if let Some(v) = meta {
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": v,
                "io.modelcontextprotocol/clientCapabilities": {}
            });
        }
        let mut req = Request::post("/mcp")
            .header("authorization", format!("Bearer {t}"))
            .header("content-type", "application/json")
            .header("mcp-method", method);
        if let Some(v) = header {
            req = req.header("mcp-protocol-version", v);
        }
        let body = json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params});
        let (got, _, text) = send(&h.app, req.body(Body::from(body.to_string())).unwrap()).await;
        let answer = rpc(&text);
        let row = format!("_meta {meta:?}, header {header:?}, {method}: {text}");
        assert_eq!(
            (got, answer["error"]["code"].as_i64()),
            (status, code),
            "{row}"
        );
        assert_eq!(answer["id"], 7, "{row}");
        assert_eq!(answer.get("result").is_some(), code.is_none(), "{row}");
        if code == Some(-32022) {
            assert_eq!(
                answer["error"]["data"]["supported"],
                json!(["2026-07-28", "2025-11-25"]),
                "{row}"
            );
            assert_eq!(
                answer["error"]["data"]["requested"],
                json!(meta.or(header)),
                "{row}"
            );
        }
    }
}

#[tokio::test]
async fn discover_and_an_empty_tool_list_have_the_2026_shapes() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let (status, headers, body) =
        send(&h.app, mcp_modern(&t, 1, "server/discover", json!({}))).await;
    assert_eq!(status, 200);
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(
        rpc(&body),
        json!({"jsonrpc": "2.0", "id": 1, "result": {
            "resultType": "complete",
            "supportedVersions": ["2026-07-28", "2025-11-25"],
            "capabilities": {"tools": {}},
            "ttlMs": 300000,
            "cacheScope": "public",
            "_meta": {"io.modelcontextprotocol/serverInfo": server_info()}
        }})
    );
    let (status, _, body) = send(&h.app, mcp_modern(&t, 2, "tools/list", json!({}))).await;
    assert_eq!(status, 200);
    assert_eq!(
        rpc(&body),
        json!({"jsonrpc": "2.0", "id": 2, "result": {
            "resultType": "complete",
            "tools": [],
            "ttlMs": 300000,
            "cacheScope": "public",
            "_meta": {"io.modelcontextprotocol/serverInfo": server_info()}
        }})
    );
    // Deliberately not implemented in 2026-07-28: no list changes at run time, and that revision has no ping.
    for method in ["subscriptions/listen", "ping", "initialize"] {
        let (status, _, body) = send(&h.app, mcp_modern(&t, 3, method, json!({}))).await;
        assert_eq!(
            (status, rpc(&body)["error"]["code"].as_i64()),
            (404, Some(-32601)),
            "{method}"
        );
    }
}

#[tokio::test]
async fn the_legacy_handshake_is_served_without_a_session() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    // Whatever version the client asks for, the answer is 2025-11-25 and the client decides.
    for asked in ["2026-07-28", "2025-11-25", "2024-11-05"] {
        let init = json!({"protocolVersion": asked, "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}});
        let (status, headers, body) =
            send(&h.app, mcp_legacy(&t, Some(1), "initialize", init)).await;
        assert_eq!(status, 200, "{asked}");
        assert!(!headers.contains_key("mcp-session-id"), "{asked}");
        assert_eq!(
            rpc(&body),
            json!({"jsonrpc": "2.0", "id": 1, "result": {
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "serverInfo": server_info()
            }}),
            "{asked}"
        );
    }
    let (status, _, body) = send(
        &h.app,
        mcp_legacy(&t, None, "notifications/initialized", json!({})),
    )
    .await;
    assert_eq!((status, body.as_str()), (202, ""));
    let (status, _, body) = send(&h.app, mcp_legacy(&t, Some(2), "ping", json!({}))).await;
    assert_eq!(
        (status, rpc(&body)),
        (200, json!({"jsonrpc": "2.0", "id": 2, "result": {}}))
    );
    // A session id a client sends is ignored, and none is issued.
    let mut list = mcp_legacy(&t, Some(3), "tools/list", json!({}));
    list.headers_mut()
        .insert("mcp-session-id", "abc".parse().unwrap());
    let (status, headers, body) = send(&h.app, list).await;
    assert_eq!(
        (status, rpc(&body)),
        (
            200,
            json!({"jsonrpc": "2.0", "id": 3, "result": {"tools": []}})
        )
    );
    assert!(!headers.contains_key("mcp-session-id"));
}

#[tokio::test]
async fn the_legacy_path_needs_an_mcp_token_like_the_modern_one() {
    let h = mcp_harness().await;
    let init = json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}});
    let mut no_token = mcp_legacy("", Some(1), "initialize", init.clone());
    no_token.headers_mut().remove("authorization");
    assert_eq!(send(&h.app, no_token).await.0, 401);
    assert_eq!(
        send(
            &h.app,
            mcp_legacy(&h.admin_token, Some(1), "initialize", init)
        )
        .await
        .0,
        401
    );
    let call = json!({"name": "get_story", "arguments": {"key": "TST-1"}});
    assert_eq!(
        send(
            &h.app,
            mcp_legacy(&h.admin_token, Some(2), "tools/call", call)
        )
        .await
        .0,
        401
    );
}
