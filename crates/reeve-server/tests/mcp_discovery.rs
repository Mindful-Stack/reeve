//! How an MCP client finds where to sign in (GRA-655; kb/ADR-0029 R2, R11): the two metadata documents, the
//! pointer in the 401 from `/mcp`, and that nothing under `/oauth/` or `/.well-known/` is ever the web page.
mod common;
use axum::{body::Body, http::Request};
use common::*;
use serde_json::{Value, json};

const PROTECTED_RESOURCE: &str = "/.well-known/oauth-protected-resource/mcp";
const AUTHORIZATION_SERVER: &str = "/.well-known/oauth-authorization-server";

fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

#[cfg(debug_assertions)]
const INDEX: &str = "<!doctype html><title>Reeve</title>";

/// With `[mcp]` off, each of the four public routes and every other path under the two prefixes answers 404,
/// for a read and for a write, and none is the web client's page.
#[cfg(debug_assertions)]
#[tokio::test]
async fn off_nothing_under_oauth_or_well_known_answers_and_none_is_the_web_page() {
    let h = harness().await; // no [mcp] section: off
    assert!(!h.state.config.mcp.enabled);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), INDEX).unwrap();
    let app = h.app_with_assets(dir.path());
    // The control: in this app a page path does fall back to index.html, so a 404 below is the rule at work and
    // not a missing build.
    let (status, _, body) = send(&app, request("GET", "/board")).await;
    assert_eq!((status, body.as_str()), (200, INDEX));
    for path in [
        PROTECTED_RESOURCE,
        AUTHORIZATION_SERVER,
        "/oauth/authorize",
        "/oauth/token",
        "/oauth",
        "/oauth/",
        "/oauth/nope",
        "/.well-known",
        "/.well-known/",
        "/.well-known/nope",
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp/more",
        "/.well-known/openid-configuration",
    ] {
        for method in ["GET", "POST"] {
            let (status, headers, body) = send(&app, request(method, path)).await;
            assert_eq!(status, 404, "{method} {path}");
            assert_ne!(body, INDEX, "{method} {path}");
            let content_type = headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert!(
                !content_type.starts_with("text/html"),
                "{method} {path}: {content_type}"
            );
        }
    }
}

/// The whole document, compared with text written out here and not built from the configuration: clients keep
/// these values, so a changed field, name or address must fail a test.
#[tokio::test]
async fn the_protected_resource_document_is_exactly_this() {
    let h = mcp_harness().await;
    let (status, headers, body) = send(&h.app, request("GET", PROTECTED_RESOURCE)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({
            "resource": "http://127.0.0.1:7474/mcp",
            "authorization_servers": ["http://127.0.0.1:7474"],
            "scopes_supported": ["read", "write"],
            "bearer_methods_supported": ["header"]
        })
    );
}

#[tokio::test]
async fn the_authorization_server_document_is_exactly_this() {
    let h = mcp_harness().await;
    let (status, headers, body) = send(&h.app, request("GET", AUTHORIZATION_SERVER)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({
            "issuer": "http://127.0.0.1:7474",
            "authorization_endpoint": "http://127.0.0.1:7474/oauth/authorize",
            "token_endpoint": "http://127.0.0.1:7474/oauth/token",
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "client_id_metadata_document_supported": true,
            "authorization_response_iss_parameter_supported": true,
            "scopes_supported": ["read", "write"]
        })
    );
}

/// A read answers; anything else is 405 in plain text with `Allow`, never the API's JSON envelope (which the
/// router's own 405 would be).
#[tokio::test]
async fn the_documents_answer_reads_only_and_never_in_the_api_envelope() {
    let h = mcp_harness().await;
    for path in [PROTECTED_RESOURCE, AUTHORIZATION_SERVER] {
        let (_, get_headers, _) = send(&h.app, request("GET", path)).await;
        let (status, headers, body) = send(&h.app, request("HEAD", path)).await;
        assert_eq!((status, body.as_str()), (200, ""), "HEAD {path}");
        assert_eq!(
            headers["content-type"], get_headers["content-type"],
            "HEAD {path}"
        );
        for method in ["POST", "PUT", "DELETE"] {
            let (status, headers, _) = send(&h.app, request(method, path)).await;
            assert_eq!(status, 405, "{method} {path}");
            assert_eq!(headers["allow"], "GET, HEAD", "{method} {path}");
            assert!(
                headers["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with("text/plain"),
                "{method} {path}"
            );
        }
    }
}

/// On, a path under either prefix that no route answers is still a 404: only the documents and the sign-in
/// routes answer. The off test cannot show this: with the switch off a route that matched too much (the root form, a longer path) would answer 404
/// from its handler all the same.
#[tokio::test]
async fn on_an_unknown_path_under_the_prefixes_is_still_a_404() {
    let h = mcp_harness().await;
    for path in [
        "/oauth/nope",
        "/.well-known/nope",
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp/more",
        "/.well-known/openid-configuration",
    ] {
        assert_eq!(send(&h.app, request("GET", path)).await.0, 404, "{path}");
    }
}

const ISSUER: &str = "http://127.0.0.1:7474";
const POINTER: &str =
    r#"resource_metadata="http://127.0.0.1:7474/.well-known/oauth-protected-resource/mcp""#;

fn post_mcp(authorization: Option<&str>) -> Request<Body> {
    let mut req = Request::post("/mcp").header("content-type", "application/json");
    if let Some(value) = authorization {
        req = req.header("authorization", value);
    }
    req.body(Body::from("{}")).unwrap()
}

/// Every 401 carries the pointer; `error="invalid_token"` is there exactly when a bearer token was presented and
/// refused.
#[tokio::test]
async fn the_401_points_at_the_metadata_and_says_invalid_token_only_for_a_refused_token() {
    let h = mcp_harness().await;
    let live = issue_mcp_token(&h, "dan@example.com").await.secret;
    let expired = issue_mcp_token(&h, "e@example.com").await.secret;
    sqlx::query("UPDATE tokens SET created_at = now() - interval '61 minutes', expires_at = now() - interval '1 minute' WHERE owner = $1")
        .bind("e@example.com")
        .execute(&h.state.pool)
        .await
        .unwrap();
    let plain = format!("Bearer {POINTER}");
    let refused = format!(r#"Bearer error="invalid_token", {POINTER}"#);
    for (what, authorization, expected) in [
        ("no Authorization header", None, &plain),
        ("another scheme", Some(format!("Basic {live}")), &plain),
        // A bearer header without a token presents none, so there is no token to call invalid.
        ("an empty bearer token", Some("Bearer ".to_string()), &plain),
        (
            "a bearer token of spaces",
            Some("Bearer    ".to_string()),
            &plain,
        ),
        (
            "an unknown token",
            Some("Bearer reeve_0000".to_string()),
            &refused,
        ),
        (
            "an API token",
            Some(format!("Bearer {}", h.admin_token)),
            &refused,
        ),
        (
            "an expired MCP token",
            Some(format!("Bearer {expired}")),
            &refused,
        ),
    ] {
        let (status, headers, _) = send(&h.app, post_mcp(authorization.as_deref())).await;
        assert_eq!(status, 401, "{what}");
        assert_eq!(
            headers["www-authenticate"].to_str().unwrap(),
            expected.as_str(),
            "{what}"
        );
    }
    // The control: a live MCP token gets past the caller check, so the 401s above are about the credential.
    let (status, headers, _) = send(&h.app, post_mcp(Some(&format!("Bearer {live}")))).await;
    assert_ne!(status, 401);
    assert!(!headers.contains_key("www-authenticate"));
}

/// The address in the header is one this server answers, with the document for this endpoint: the pointer and the
/// route cannot drift apart.
#[tokio::test]
async fn the_address_the_401_names_serves_the_document_for_this_endpoint() {
    let h = mcp_harness().await;
    let (_, headers, _) = send(&h.app, post_mcp(None)).await;
    let challenge = headers["www-authenticate"].to_str().unwrap();
    let url = challenge
        .split('"')
        .nth(1)
        .expect("a quoted resource_metadata value");
    let path = url
        .strip_prefix(ISSUER)
        .expect("the pointer is on the issuer's origin");
    let (status, _, body) = send(&h.app, request("GET", path)).await;
    assert_eq!(status, 200, "{path}: {body}");
    let document: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(document["resource"], "http://127.0.0.1:7474/mcp");
}

/// A real file of the web build under `/.well-known/` (a `security.txt`, say) is served as any file is; only what
/// would otherwise fall back to the page is refused.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_real_file_under_well_known_is_served_and_an_unknown_path_there_is_not() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), INDEX).unwrap();
    std::fs::create_dir_all(dir.path().join(".well-known")).unwrap();
    std::fs::write(
        dir.path().join(".well-known/security.txt"),
        "Contact: mailto:security@example.com",
    )
    .unwrap();
    let app = h.app_with_assets(dir.path());
    let (status, headers, body) = send(&app, request("GET", "/.well-known/security.txt")).await;
    assert_eq!(
        (status, body.as_str()),
        (200, "Contact: mailto:security@example.com")
    );
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    for (method, path) in [
        ("GET", "/.well-known/other.txt"),
        ("GET", "/.well-known/security"),
        ("GET", "/.well-known/"),
        // The file answers reads only, and a path that climbs out of the directory is never looked up.
        ("POST", "/.well-known/security.txt"),
        ("GET", "/.well-known/../.well-known/security.txt"),
    ] {
        let (status, _, body) = send(&app, request(method, path)).await;
        assert_eq!(
            (status, body.as_str()),
            (404, "not found"),
            "{method} {path}"
        );
    }
}

/// Every address follows the configured `public_url`, here another host and port than the default the bind gives,
/// written with a trailing slash. The expectations are text written here: an address built from anything but the
/// configuration (the bind, a constant) fails.
#[tokio::test]
async fn every_address_follows_the_configured_public_url() {
    let h = harness_patched(|c| {
        c.public_url = Some("https://reeve.example.com:8443/".into());
        c.mcp.enabled = true;
    })
    .await;
    let (status, _, body) = send(&h.app, request("GET", PROTECTED_RESOURCE)).await;
    assert_eq!(status, 200, "{body}");
    let document: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(document["resource"], "https://reeve.example.com:8443/mcp");
    assert_eq!(
        document["authorization_servers"],
        json!(["https://reeve.example.com:8443"])
    );
    let (status, _, body) = send(&h.app, request("GET", AUTHORIZATION_SERVER)).await;
    assert_eq!(status, 200, "{body}");
    let document: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(document["issuer"], "https://reeve.example.com:8443");
    assert_eq!(
        document["authorization_endpoint"],
        "https://reeve.example.com:8443/oauth/authorize"
    );
    assert_eq!(
        document["token_endpoint"],
        "https://reeve.example.com:8443/oauth/token"
    );
    let (status, headers, _) = send(&h.app, post_mcp(None)).await;
    assert_eq!(status, 401);
    assert_eq!(
        headers["www-authenticate"],
        r#"Bearer resource_metadata="https://reeve.example.com:8443/.well-known/oauth-protected-resource/mcp""#
    );
}
