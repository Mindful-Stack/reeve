//! `/mcp` through the router: the order of the checks, then the methods (kb/ADR-0029 R1, R2, R6, R8).
mod common;
use axum::{body::Body, http::Request};
use common::*;
use serde_json::{Value, json};

const DISABLED: &str = "MCP is not enabled on this instance ([mcp] enabled)";

/// What every 401's `WWW-Authenticate` ends with (GRA-655); `tests/mcp_discovery.rs` pins the whole value and when
/// `error="invalid_token"` comes before it.
const POINTER: &str =
    r#"resource_metadata="http://127.0.0.1:7474/.well-known/oauth-protected-resource/mcp""#;

fn names_the_metadata(headers: &axum::http::HeaderMap) -> bool {
    headers["www-authenticate"]
        .to_str()
        .is_ok_and(|v| v.starts_with("Bearer ") && v.ends_with(POINTER))
}

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
        assert!(names_the_metadata(&headers));
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
        assert!(names_the_metadata(&headers), "{what}");
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
            "UPDATE tokens SET created_at = now() - interval '61 minutes', expires_at = now() - interval '1 minute' WHERE owner = $1",
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
    assert!(names_the_metadata(&headers));
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
    let rows: [Row; 14] = [
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
            200,
            Some(-32601),
        ),
        // An unknown method on the legacy path: the JSON-RPC error in an ordinary answer (404 there means an
        // expired session).
        (None, Some("2025-11-25"), "prompts/list", 200, Some(-32601)),
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
async fn discover_and_the_tool_list_have_the_2026_shapes() {
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
    let list = rpc(&body);
    assert_eq!((&list["jsonrpc"], &list["id"]), (&json!("2.0"), &json!(2)));
    let result = &list["result"];
    // What the tools are is the snapshot test's business; here, the envelope around them.
    assert!(
        result["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty())
    );
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["ttlMs"], 300000);
    assert_eq!(result["cacheScope"], "private");
    assert_eq!(
        result["_meta"],
        json!({"io.modelcontextprotocol/serverInfo": server_info()})
    );
    assert_eq!(
        result.as_object().unwrap().len(),
        5,
        "no other field: {result}"
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
    let answer = rpc(&body);
    assert_eq!(status, 200);
    assert_eq!(
        answer["result"].as_object().unwrap().len(),
        1,
        "only `tools`: {answer}"
    );
    assert!(answer["result"]["tools"].is_array());
    assert!(!headers.contains_key("mcp-session-id"));
}

#[tokio::test]
async fn the_legacy_path_needs_an_mcp_token_like_the_modern_one() {
    let h = mcp_harness().await;
    let init = json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}});
    let mut no_token = mcp_legacy("", Some(1), "initialize", init);
    no_token.headers_mut().remove("authorization");
    assert_eq!(send(&h.app, no_token).await.0, 401);
}

#[tokio::test]
async fn an_origin_that_is_not_one_allowed_value_is_403() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let (own, foreign) = ("http://127.0.0.1:7474", "https://evil.example");
    // Two Origin headers are refused whichever comes first.
    for (first, second) in [(own, foreign), (foreign, own)] {
        let mut req = knock(Some(&t), None);
        req.headers_mut().append("origin", first.parse().unwrap());
        req.headers_mut().append("origin", second.parse().unwrap());
        assert_eq!(send(&h.app, req).await.0, 403, "{first} then {second}");
    }
    for (what, value) in [
        ("null", axum::http::HeaderValue::from_static("null")),
        (
            "not text",
            axum::http::HeaderValue::from_bytes(b"http://127.0.0.1:7474\xff").unwrap(),
        ),
    ] {
        let mut req = knock(Some(&t), None);
        req.headers_mut().insert("origin", value);
        assert_eq!(send(&h.app, req).await.0, 403, "{what}");
    }
    // The control: no Origin passes.
    let (s, _, b) = send(&h.app, knock(Some(&t), None)).await;
    assert!(passed(s, &b), "{s} {b}");
}

/// A protocol header sent twice or not as text is refused, never read as absent: read as absent, it would move the
/// request onto the other revision's path.
#[tokio::test]
async fn a_protocol_header_sent_twice_or_not_as_text_is_a_header_mismatch() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let not_text = || axum::http::HeaderValue::from_bytes(b"\xff").unwrap();
    let mismatch = |status: u16, body: &str| {
        status == 400
            && serde_json::from_str::<Value>(body).is_ok_and(|v| v["error"]["code"] == -32020)
    };
    // The control: the request these are made from passes every check and reaches the tool.
    let call = || mcp_modern(&t, 1, "tools/call", json!({"name": "get_story"}));
    let (s, _, b) = send(&h.app, call()).await;
    assert!(s == 200 && rpc(&b)["result"].is_object(), "{s} {b}");
    for name in ["mcp-protocol-version", "mcp-method", "mcp-name"] {
        let mut twice = call();
        let value = twice.headers()[name].clone();
        twice.headers_mut().append(name, value);
        let (s, _, b) = send(&h.app, twice).await;
        assert!(mismatch(s, &b), "{name} twice: {s} {b}");
        let mut binary = call();
        binary.headers_mut().insert(name, not_text());
        let (s, _, b) = send(&h.app, binary).await;
        assert!(mismatch(s, &b), "{name} not text: {s} {b}");
    }
    // A method that does not read Mcp-Name still refuses one that is not text.
    let mut list = mcp_modern(&t, 2, "tools/list", json!({}));
    list.headers_mut().insert("mcp-name", not_text());
    let (s, _, b) = send(&h.app, list).await;
    assert!(mismatch(s, &b), "{s} {b}");
    // The handshake carries no version header; one that is not text is not "no header".
    let init = json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}});
    let mut handshake = mcp_legacy(&t, Some(3), "initialize", init.clone());
    handshake
        .headers_mut()
        .insert("mcp-protocol-version", not_text());
    let (s, _, b) = send(&h.app, handshake).await;
    assert!(mismatch(s, &b), "{s} {b}");
    assert_eq!(rpc(&b)["id"], 3);
    // The caller check still comes first.
    let mut unauthenticated = mcp_legacy(&t, Some(3), "initialize", init);
    unauthenticated.headers_mut().remove("authorization");
    unauthenticated
        .headers_mut()
        .insert("mcp-protocol-version", not_text());
    assert_eq!(send(&h.app, unauthenticated).await.0, 401);
}

#[tokio::test]
async fn an_authorization_header_that_is_not_bearer_is_401() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let mut req = knock(Some(&t), None);
    req.headers_mut()
        .insert("authorization", format!("Basic {t}").parse().unwrap());
    let (status, headers, _) = send(&h.app, req).await;
    assert_eq!(status, 401);
    assert!(names_the_metadata(&headers));
}

#[tokio::test]
async fn a_malformed_meta_is_refused_or_read_as_absent() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let post = |header: Option<&str>, meta: Value| {
        let mut req = Request::post("/mcp")
            .header("authorization", format!("Bearer {t}"))
            .header("content-type", "application/json")
            .header("mcp-method", "tools/list");
        if let Some(v) = header {
            req = req.header("mcp-protocol-version", v);
        }
        let body =
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": meta}});
        req.body(Body::from(body.to_string())).unwrap()
    };
    // A protocol version that is not a string is malformed, whatever the header says.
    let not_a_string = json!({
        "io.modelcontextprotocol/protocolVersion": 20260728,
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let (s, _, b) = send(&h.app, post(Some("2026-07-28"), not_a_string)).await;
    assert_eq!((s, rpc(&b)["error"]["code"].as_i64()), (400, Some(-32602)));
    // A `_meta` that is not an object carries no version, so the header rule decides.
    for (header, status, code) in [
        (Some("2025-11-25"), 200, None),
        (Some("2026-07-28"), 400, Some(-32020)),
        (None, 400, Some(-32020)),
    ] {
        let (s, _, b) = send(&h.app, post(header, json!("2026-07-28"))).await;
        assert_eq!(
            (s, rpc(&b)["error"]["code"].as_i64()),
            (status, code),
            "{header:?}: {b}"
        );
    }
}

#[tokio::test]
async fn a_refused_token_is_logged_once_and_never_with_its_secret() {
    let h = mcp_harness().await;
    let (logs, _guard) = Logs::capture();
    // An API token: a real secret, refused here for its audience.
    assert_eq!(send(&h.app, knock(Some(&h.admin_token), None)).await.0, 401);
    let text = logs.text();
    assert_eq!(text.matches("refused").count(), 1, "{text}");
    assert!(text.contains("wrong audience"), "{text}");
    assert!(!text.contains(&h.admin_token), "{text}");
    // A missing bearer header is the endpoint's own line, with what the request said about its protocol.
    let (logs, _guard) = Logs::capture();
    assert_eq!(send(&h.app, knock(None, None)).await.0, 401);
    let text = logs.text();
    assert_eq!(text.matches("refused").count(), 1, "{text}");
    assert!(
        text.contains("no bearer header")
            && text.contains("2026-07-28")
            && text.contains("nope/unknown"),
        "{text}"
    );
}

/// Steps 1 to 4 answer without reading the body: each request carries a body that fails the test when polled.
#[tokio::test]
async fn a_request_refused_before_step_5_never_has_its_body_read() {
    fn unread(mut req: Request<Body>) -> Request<Body> {
        *req.body_mut() = Body::from_stream(futures_util::stream::poll_fn(
            |_| -> std::task::Poll<Option<Result<axum::body::Bytes, std::io::Error>>> {
                panic!("the request body was polled")
            },
        ));
        req
    }
    let off = harness().await;
    let t = admin_mcp(&off).await;
    assert_eq!(send(&off.app, unread(knock(Some(&t), None))).await.0, 404);

    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let mut get = knock(Some(&t), None);
    *get.method_mut() = axum::http::Method::GET;
    assert_eq!(send(&h.app, unread(get)).await.0, 405);
    let mut foreign = knock(Some(&t), None);
    foreign
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    assert_eq!(send(&h.app, unread(foreign)).await.0, 403);
    assert_eq!(send(&h.app, unread(knock(None, None))).await.0, 401);
}

/// A story with `comments` comments under a ready epic, created through the API as the admin; returns its key.
async fn story_with_comments(h: &Harness, comments: usize) -> String {
    let epic = ready_epic(h, "mcp-e1").await;
    let (st, story) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key": "mcp-s1", "title": "Booking", "kind": "story", "epic": epic, "criteria": ["works"], "labels": []})),
    )
    .await;
    assert_eq!(st, 200, "{story}");
    let key = story["value"]["key"].as_str().unwrap().to_string();
    for n in 1..=comments {
        let (st, c) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            &format!("/api/v1/stories/{key}/comments"),
            Some(json!({"body": format!("comment {n}")})),
        )
        .await;
        assert_eq!(st, 200, "{c}");
    }
    key
}

/// The `value` of `GET /api/v1/stories/{key}/context`: what `reeve story context --json` prints.
async fn api_context(h: &Harness, key: &str) -> Value {
    let (st, v) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/stories/{key}/context"),
        None,
    )
    .await;
    assert_eq!(st, 200, "{v}");
    v["value"].clone()
}

/// `tools/call get_story` over 2026-07-28: the status and the JSON-RPC answer.
async fn call_get_story(h: &Harness, token: &str, arguments: Value) -> (u16, Value) {
    let params = json!({"name": "get_story", "arguments": arguments});
    let (status, _, body) = send(&h.app, mcp_modern(token, 9, "tools/call", params)).await;
    (status, rpc(&body))
}

/// The startup line counts the tools `tools/list` answers, from the same built state; and says so only when the
/// endpoint is on.
#[tokio::test]
async fn the_startup_line_says_whether_and_where_mcp_is_served_and_counts_the_listed_tools() {
    let off = harness().await;
    assert_eq!(
        reeve_server::mcp_startup_line(&off.state),
        "mcp endpoint disabled"
    );

    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let (status, _, body) = send(&h.app, mcp_modern(&t, 1, "tools/list", json!({}))).await;
    assert_eq!(status, 200, "{body}");
    let listed = rpc(&body)["result"]["tools"].as_array().unwrap().len();
    assert!(listed > 1, "the control: {listed} tools are listed");
    assert_eq!(
        reeve_server::mcp_startup_line(&h.state),
        format!(
            "mcp endpoint enabled at http://127.0.0.1:7474/mcp (protocol 2026-07-28 and 2025-11-25, {listed} tools)"
        )
    );
}

/// The last guard and the bounded echo, through the dispatch: neither is a tool's own doing.
#[tokio::test]
async fn an_answer_too_large_to_send_is_a_tool_error_and_an_echoed_key_is_bounded() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    // Forty-five criteria of exactly 2 000 characters: none is cut, and together they are 90 000.
    let criterion = "a".repeat(2_000);
    let (st, story) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key": "mcp-big", "title": "Big", "kind": "story", "criteria": vec![criterion; 45], "labels": []})),
    )
    .await;
    assert_eq!(st, 200, "{story}");
    let key = story["value"]["key"].as_str().unwrap();
    // The control: the API route answers it, whole.
    let api = api_context(&h, key).await;
    assert_eq!(api["story"]["criteria"].as_array().unwrap().len(), 45);

    let (status, answer) = call_get_story(&h, &t, json!({"key": key})).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["result"]["isError"], true);
    assert!(answer["result"].get("structuredContent").is_none());
    let text = answer["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("The answer is too large to send (9")
            && text.ends_with(" bytes). For a list, ask for a narrower one. A single item this large cannot be read here."),
        "{text}"
    );

    // The guard counts bytes, not characters: twenty criteria of 2 000 two-byte characters are about 41 000
    // characters, under the limit, and about 81 000 bytes, over it.
    let (st, story) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key": "mcp-wide", "title": "Wide", "kind": "story", "criteria": vec!["å".repeat(2_000); 20], "labels": []})),
    )
    .await;
    assert_eq!(st, 200, "{story}");
    let key = story["value"]["key"].as_str().unwrap();
    let whole = api_context(&h, key).await.to_string();
    assert!(
        whole.chars().count() < 70_000 && whole.len() > 70_000,
        "the control: {} characters, {} bytes",
        whole.chars().count(),
        whole.len()
    );
    let (_, answer) = call_get_story(&h, &t, json!({"key": key})).await;
    assert_eq!(answer["result"]["isError"], true, "{answer}");
    let text = answer["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("The answer is too large to send (8"),
        "{text}"
    );

    // A key of 300 characters comes back as its first 200.
    let (_, answer) = call_get_story(&h, &t, json!({"key": "k".repeat(300)})).await;
    assert_eq!(
        answer["result"]["content"][0]["text"],
        json!(format!(
            "No story with key \"{}[cut: 100 more characters]\". Keys look like TST-123.",
            "k".repeat(200)
        ))
    );
}

fn has_ref(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.contains_key("$ref") || m.values().any(has_ref),
        Value::Array(a) => a.iter().any(has_ref),
        _ => false,
    }
}

/// The tool list is a deliberate interface (kb/ADR-0029 R12): accept a change with UPDATE_SNAPSHOTS=1, as for the
/// OpenAPI snapshot. The harness's key prefix (TST) is in the snapshot's examples.
#[tokio::test]
async fn the_tool_list_matches_its_snapshot_and_every_schema_is_a_self_contained_object() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let (status, _, body) = send(&h.app, mcp_modern(&t, 1, "tools/list", json!({}))).await;
    assert_eq!(status, 200, "{body}");
    let result = rpc(&body)["result"].clone();
    let tools = result["tools"].as_array().unwrap();
    let get_story = tools
        .iter()
        .find(|tool| tool["name"] == "get_story")
        .unwrap_or_else(|| panic!("no get_story in {result}"));
    assert_eq!(get_story["title"], "Read a story");
    assert_eq!(get_story["annotations"]["readOnlyHint"], true);
    assert_eq!(get_story["annotations"]["openWorldHint"], false);
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(!has_ref(&tool["inputSchema"]), "{tool}");
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{tool}"
        );
        assert!(
            tool["title"].as_str().is_some_and(|t| !t.is_empty()),
            "{tool}"
        );
        for hint in [
            "readOnlyHint",
            "destructiveHint",
            "idempotentHint",
            "openWorldHint",
        ] {
            assert!(tool["annotations"][hint].is_boolean(), "{hint}: {tool}");
        }
        assert!(tool.get("outputSchema").is_none(), "{tool}");
    }
    assert_eq!(
        (&result["resultType"], &result["cacheScope"]),
        (&json!("complete"), &json!("private"))
    );
    // 2025-11-25 lists the same tools, without the 2026 fields.
    let (_, _, body) = send(&h.app, mcp_legacy(&t, Some(2), "tools/list", json!({}))).await;
    assert_eq!(rpc(&body)["result"], json!({"tools": tools}));

    let doc = serde_json::to_string_pretty(&result["tools"]).unwrap();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/mcp_tools.json");
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::write(&path, &doc).unwrap();
        return;
    }
    let stored =
        std::fs::read_to_string(&path).expect("snapshot exists; run with UPDATE_SNAPSHOTS=1 once");
    assert_eq!(
        stored, doc,
        "the MCP tool list changed; if intended, rerun with UPDATE_SNAPSHOTS=1"
    );
}

#[tokio::test]
async fn get_story_answers_what_the_api_answers_in_both_revisions() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let key = story_with_comments(&h, 2).await;
    let mut want = api_context(&h, &key).await;
    assert_eq!(want["comments"].as_array().unwrap().len(), 2, "the control");
    want["comments_omitted"] = json!(0);

    let (status, answer) = call_get_story(&h, &t, json!({"key": key})).await;
    assert_eq!(status, 200, "{answer}");
    let result = &answer["result"];
    assert_eq!(result["structuredContent"], want);
    assert_eq!(result["isError"], false);
    assert_eq!(result["resultType"], "complete");
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    let text = content[0]["text"].as_str().unwrap();
    assert_eq!(rpc(text), want, "the text block is the same JSON");
    assert!(!text.contains('\n'), "compact");

    let params = json!({"name": "get_story", "arguments": {"key": key}});
    let (status, _, body) = send(&h.app, mcp_legacy(&t, Some(4), "tools/call", params)).await;
    let legacy = rpc(&body)["result"].clone();
    assert_eq!(status, 200, "{body}");
    assert_eq!(legacy["structuredContent"], want);
    assert_eq!(legacy["content"], result["content"]);
    assert!(
        legacy.get("resultType").is_none() && legacy.get("_meta").is_none(),
        "{legacy}"
    );
}

#[tokio::test]
async fn get_story_sends_the_twenty_most_recent_comments_and_counts_the_rest() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let key = story_with_comments(&h, 25).await;
    let api = api_context(&h, &key).await;
    let all = api["comments"].as_array().unwrap();
    assert_eq!(all.len(), 25, "the control: the API sends every comment");
    let (status, answer) = call_get_story(&h, &t, json!({"key": key})).await;
    assert_eq!(status, 200, "{answer}");
    let sent = &answer["result"]["structuredContent"];
    assert_eq!(sent["comments_omitted"], 5);
    let kept = sent["comments"].as_array().unwrap();
    assert_eq!(
        &kept[..],
        &all[5..],
        "the twenty most recent, in the API's order"
    );
    assert_eq!(kept[19]["body"], "comment 25");
    assert_eq!(kept[0]["author"], "dan@example.com");
    assert!(kept[0]["at"].is_string());
}

#[tokio::test]
async fn a_tool_failure_is_a_result_the_model_can_read_and_a_malformed_call_is_a_protocol_error() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let key = story_with_comments(&h, 0).await;

    let (status, answer) = call_get_story(&h, &t, json!({"key": "TST-999999"})).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["result"]["isError"], true);
    assert_eq!(
        answer["result"]["content"][0]["text"],
        "No story with key \"TST-999999\". Keys look like TST-123."
    );
    assert!(answer["result"].get("structuredContent").is_none());

    for (arguments, names) in [
        (json!({"key": 5}), "key"),
        (json!({"key": key, "extra": true}), "extra"),
        (json!({}), "key"),
        // `arguments` must be a JSON object: a string and an array are argument errors, not protocol errors.
        (json!("TST-1"), "JSON object"),
        (json!(["TST-1"]), "JSON object"),
    ] {
        let (status, answer) = call_get_story(&h, &t, arguments.clone()).await;
        assert_eq!(status, 200, "{arguments}: {answer}");
        assert_eq!(answer["result"]["isError"], true, "{arguments}");
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(names), "{arguments}: {text}");
        // The message carries the tool's input schema as `tools/list` shows it, with this instance's example.
        assert!(text.contains("TST-596"), "{arguments}: {text}");
    }
    // 2025-11-25 answers an argument failure the same way: a tool result, HTTP 200.
    let bad = json!({"name": "get_story", "arguments": {"key": 5}});
    let (status, _, body) = send(&h.app, mcp_legacy(&t, Some(6), "tools/call", bad)).await;
    let legacy = rpc(&body)["result"].clone();
    assert_eq!((status, &legacy["isError"]), (200, &json!(true)), "{body}");
    let text = legacy["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("TST-596"), "{text}");
    assert!(legacy.get("resultType").is_none(), "{legacy}");
    // `arguments` left out altogether reads as no arguments.
    let (status, _, body) = send(
        &h.app,
        mcp_modern(&t, 1, "tools/call", json!({"name": "get_story"})),
    )
    .await;
    assert_eq!(
        (status, &rpc(&body)["result"]["isError"]),
        (200, &json!(true))
    );

    // Protocol errors: an unknown tool, and a call without a name, in either revision.
    let unknown = json!({"name": "nope", "arguments": {}});
    let (status, _, body) = send(&h.app, mcp_modern(&t, 1, "tools/call", unknown)).await;
    let answer = rpc(&body);
    assert_eq!(
        (status, answer["error"]["code"].as_i64()),
        (400, Some(-32602)),
        "{body}"
    );
    assert_eq!(answer["error"]["message"], "Unknown tool: nope");
    let (status, _, body) = send(&h.app, mcp_modern(&t, 1, "tools/call", json!({}))).await;
    assert_eq!(
        (status, rpc(&body)["error"]["code"].as_i64()),
        (400, Some(-32602)),
        "{body}"
    );
    // 2025-11-25 carries the same JSON-RPC errors in an ordinary answer: HTTP 200.
    let (status, _, body) = send(&h.app, mcp_legacy(&t, Some(1), "tools/call", json!({}))).await;
    assert_eq!(
        (status, rpc(&body)["error"]["code"].as_i64()),
        (200, Some(-32602)),
        "{body}"
    );
    let unknown = json!({"name": "nope", "arguments": {}});
    let (status, _, body) = send(&h.app, mcp_legacy(&t, Some(1), "tools/call", unknown)).await;
    assert_eq!(
        (status, rpc(&body)["error"]["code"].as_i64()),
        (200, Some(-32602)),
        "{body}"
    );
}

/// The planner limits no text. The tool cuts the story's own description at 10 000 characters and every other text
/// (each acceptance criterion, the epic's and the initiative's brief, a blocker's description, a comment's body) at
/// 2 000, and says how many characters it removed. The API route still answers every text whole.
#[tokio::test]
async fn get_story_cuts_long_text_and_the_api_route_does_not() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let initiative_brief = "é".repeat(2_003);
    let epic_brief = "ø".repeat(2_002);
    let criterion = "ñ".repeat(2_004);
    let (st, initiative) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key": "mcp-i2", "title": "Long", "horizon": "Now", "brief": initiative_brief})),
    )
    .await;
    assert_eq!(st, 200, "{initiative}");
    let (st, epic) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key": "mcp-e2", "title": "Long", "initiative": initiative["value"]["key"], "brief": epic_brief})),
    )
    .await;
    assert_eq!(st, 200, "{epic}");
    let epic = epic["value"]["key"].as_str().unwrap().to_string();
    // Two-byte characters: a cut that counted bytes would stop at half of them.
    let description = "å".repeat(10_010);
    let blocker_description = "ü".repeat(2_005);
    let mut keys = Vec::new();
    for (idem, text) in [("mcp-s2", &description), ("mcp-s3", &blocker_description)] {
        let (st, story) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/stories",
            Some(json!({"idempotency_key": idem, "title": "Long", "kind": "story", "epic": epic, "description": text, "criteria": ["works", criterion], "labels": []})),
        )
        .await;
        assert_eq!(st, 200, "{story}");
        keys.push(story["value"]["key"].as_str().unwrap().to_string());
    }
    let (key, blocker) = (&keys[0], &keys[1]);
    let (st, edge) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{key}/blocks"),
        Some(json!({"blocker": blocker})),
    )
    .await;
    assert_eq!(st, 200, "{edge}");
    let body = "ö".repeat(2_001);
    for text in ["short", body.as_str()] {
        let (st, c) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            &format!("/api/v1/stories/{key}/comments"),
            Some(json!({"body": text})),
        )
        .await;
        assert_eq!(st, 200, "{c}");
    }

    // The control: the API route answers every one of them whole.
    let api = api_context(&h, key).await;
    assert_eq!(api["story"]["description"], json!(description));
    assert_eq!(api["comments"][1]["body"], json!(body));
    assert_eq!(
        api["blockers"][0]["description"],
        json!(blocker_description)
    );
    assert_eq!(api["blockers"][0]["criteria"][1]["text"], json!(criterion));
    assert_eq!(api["epic"]["brief"], json!(epic_brief));
    assert_eq!(api["initiative"]["brief"], json!(initiative_brief));
    assert_eq!(api["story"]["criteria"][1]["text"], json!(criterion));

    let (status, answer) = call_get_story(&h, &t, json!({"key": key})).await;
    assert_eq!(status, 200);
    let sent = &answer["result"]["structuredContent"];
    assert_eq!(
        sent["story"]["description"],
        json!(format!("{}[cut: 10 more characters]", "å".repeat(10_000)))
    );
    assert_eq!(sent["comments"][0]["body"], "short");
    assert_eq!(
        sent["comments"][1]["body"],
        json!(format!("{}[cut: 1 more characters]", "ö".repeat(2_000)))
    );
    assert_eq!(
        sent["blockers"][0]["description"],
        json!(format!("{}[cut: 5 more characters]", "ü".repeat(2_000)))
    );
    assert_eq!(sent["blockers"][0]["criteria"][0]["text"], "works");
    assert_eq!(
        sent["blockers"][0]["criteria"][1]["text"],
        json!(format!("{}[cut: 4 more characters]", "ñ".repeat(2_000)))
    );
    assert_eq!(
        sent["epic"]["brief"],
        json!(format!("{}[cut: 2 more characters]", "ø".repeat(2_000)))
    );
    assert_eq!(
        sent["initiative"]["brief"],
        json!(format!("{}[cut: 3 more characters]", "é".repeat(2_000)))
    );
    assert_eq!(sent["story"]["criteria"][0]["text"], "works");
    assert_eq!(
        sent["story"]["criteria"][1]["text"],
        json!(format!("{}[cut: 4 more characters]", "ñ".repeat(2_000)))
    );
    let text = answer["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(&rpc(text), sent, "the text block carries the same cut text");
}

#[tokio::test]
async fn an_internal_error_in_a_tool_is_500_without_detail() {
    let h = mcp_harness().await;
    let t = admin_mcp(&h).await;
    let key = story_with_comments(&h, 0).await;
    sqlx::query("ALTER TABLE comments RENAME TO comments_gone")
        .execute(&h.state.pool)
        .await
        .unwrap();
    let (status, answer) = call_get_story(&h, &t, json!({"key": key})).await;
    assert_eq!(status, 500);
    assert_eq!(
        answer,
        json!({"jsonrpc": "2.0", "id": 9, "error": {"code": -32603, "message": "Internal error"}})
    );
}
