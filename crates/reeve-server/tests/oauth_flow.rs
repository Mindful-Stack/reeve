//! MCP sign-in end to end (GRA-655 to GRA-658), following only what the server publishes, and the isolation of
//! codes and refresh tokens from every authenticated surface (kb/ADR-0029 R6). Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
mod common;
use axum::{body::Body, http::Request};
use common::{oauth::*, *};
use serde_json::{Value, json};

async fn get_json(f: &Flow, path: &str) -> Value {
    let (status, _, body) = send(&f.h.app, Request::get(path).body(Body::empty()).unwrap()).await;
    assert_eq!(status, 200, "{path}: {body}");
    serde_json::from_str(&body).unwrap()
}

/// The path of an address the server published, which must be under its own issuer.
fn path_of(address: &Value) -> String {
    address
        .as_str()
        .and_then(|a| a.strip_prefix(ISSUER))
        .unwrap_or_else(|| panic!("not an address of this server: {address}"))
        .to_string()
}

/// One client's journey with nothing known in advance but the endpoint's address: the 401 and its pointer, the
/// two documents, authorize, the approval as a signed-in person, the exchange, a tool call, a refresh, and a tool
/// call with the new token. Catches a document that names an address that does not answer, and any step that
/// works only with knowledge a client does not have.
#[tokio::test]
async fn a_client_connects_using_only_what_the_server_publishes() {
    let f = flow().await;
    let h = &f.h;
    // 1. No token: 401, with the pointer.
    let bare = Request::post("/mcp")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, headers, _) = send(&h.app, bare).await;
    assert_eq!(status, 401);
    let challenge = headers["www-authenticate"].to_str().unwrap();
    let pointer = challenge
        .split("resource_metadata=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no pointer in {challenge}"));
    // 2. The protected resource's document, then its authorization server's.
    let resource_doc = get_json(&f, pointer.strip_prefix(ISSUER).expect(pointer)).await;
    assert_eq!(resource_doc["resource"], RESOURCE);
    let issuer = resource_doc["authorization_servers"][0].as_str().unwrap();
    assert_eq!(issuer, ISSUER);
    let server_doc = get_json(&f, "/.well-known/oauth-authorization-server").await;
    assert_eq!(server_doc["issuer"], issuer);
    let authorize_path = path_of(&server_doc["authorization_endpoint"]);
    let token_path = path_of(&server_doc["token_endpoint"]);
    // 3. Authorize at the published address, asking for every scope the server lists, as a client does.
    let scopes: Vec<&str> = resource_doc["scopes_supported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    let params = with(authorize_params(&f), "scope", Some(&scopes.join(" ")));
    let request = Request::get(format!("{authorize_path}?{}", query_of(&params)))
        .body(Body::empty())
        .unwrap();
    let s = started(call_raw(&h.app, request).await);
    // 4. The person approves on the page: read only.
    let value = approval(&f, &h.admin_token, &s).await;
    let (status, _, answer) = approve(&f, &h.admin_token, &s, &value, "read").await;
    assert_eq!(status, 200, "{answer}");
    let back = url::Url::parse(answer["value"]["redirect_to"].as_str().unwrap()).unwrap();
    assert_eq!(
        param(&back, "iss").as_deref(),
        Some(issuer),
        "the client checks iss against the document"
    );
    let code = param(&back, "code").unwrap();
    // 5. The exchange at the published address.
    let mut exchange = token_request(&code_form(&f, &code));
    *exchange.uri_mut() = token_path.parse().unwrap();
    let (status, _, pair) = answer_of(&h.app, exchange).await;
    assert_eq!(status, 200, "{pair}");
    assert_eq!(pair["scope"], "read");
    // 6. A tool call.
    let epic = ready_epic(h, "flow-epic").await;
    let story = json!({"idempotency_key": "flow-story", "title": "Flow", "kind": "story", "epic": epic, "criteria": ["works"], "labels": []});
    let (status, story) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(story),
    )
    .await;
    assert_eq!(status, 200, "{story}");
    let key = story["value"]["key"].as_str().unwrap().to_string();
    let tool = |access: &str| {
        mcp_modern(
            access,
            9,
            "tools/call",
            json!({"name": "get_story", "arguments": {"key": key}}),
        )
    };
    let (status, _, body) = send(&h.app, tool(pair["access_token"].as_str().unwrap())).await;
    assert_eq!(status, 200, "{body}");
    let result: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(result["result"]["isError"], false, "{body}");
    assert!(body.contains(&key), "{body}");
    // 7. A refresh at the published address, and the tool again with the new token.
    let mut refreshing = token_request(&form_with(
        refresh_form(pair["refresh_token"].as_str().unwrap()),
        "resource",
        Some(RESOURCE),
    ));
    *refreshing.uri_mut() = token_path.parse().unwrap();
    let (status, _, next) = answer_of(&h.app, refreshing).await;
    assert_eq!(status, 200, "{next}");
    assert_eq!(
        send(&h.app, tool(next["access_token"].as_str().unwrap()))
            .await
            .0,
        200
    );
}

/// kb/ADR-0029 R6: "a refresh token MUST authenticate nowhere"; a code neither. Each is presented as a bearer
/// token and as a session cookie on the API, the event stream, an attachment and `/mcp`. Statuses are read from
/// the response head, so an accepted secret on the stream is a 200 here and not a test that hangs. The controls
/// come last: the same code and the same refresh token then do what they are for, so they were live secrets when
/// they were refused.
#[tokio::test]
async fn a_code_and_a_refresh_token_are_credentials_nowhere() {
    let f = flow().await;
    let h = &f.h;
    let connected = connect(&f, &h.admin_token, "write").await;
    let code = code_for(&f, &h.admin_token, "read").await;
    let status = |req: Request<Body>| async move { call_raw(&h.app, req).await.status().as_u16() };
    for (what, secret) in [
        ("a code", code.as_str()),
        ("a refresh token", connected.refresh_token.as_str()),
    ] {
        for path in [
            "/api/v1/me",
            "/api/v1/events",
            "/api/v1/tokens",
            "/api/v1/attachments/00000000-0000-4000-8000-000000000000",
        ] {
            let bearer = Request::get(path)
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap();
            assert_eq!(status(bearer).await, 401, "{what} as bearer on {path}");
            let cookie = Request::get(path)
                .header("cookie", format!("reeve_session={secret}"))
                .body(Body::empty())
                .unwrap();
            assert_eq!(status(cookie).await, 401, "{what} as session on {path}");
        }
        assert_eq!(
            mcp_status(&f, secret).await,
            401,
            "{what} as bearer at /mcp"
        );
        let cookie = Request::post("/mcp")
            .header("content-type", "application/json")
            .header("cookie", format!("reeve_session={secret}"))
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(status(cookie).await, 401, "{what} as a cookie at /mcp");
    }
    // An access token is not a refresh token, and an API token is neither.
    for (what, secret) in [
        ("the access token", connected.access_token.as_str()),
        ("an API token", h.admin_token.as_str()),
    ] {
        let (status, _, body) = token(&f, &refresh_form(secret)).await;
        assert_eq!(
            (status, &body["error"]),
            (400, &json!("invalid_grant")),
            "{what}: {body}"
        );
    }
    // The controls.
    assert_eq!(token(&f, &code_form(&f, &code)).await.0, 200);
    assert_eq!(
        token(&f, &refresh_form(&connected.refresh_token)).await.0,
        200
    );
}
