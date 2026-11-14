//! How an MCP client finds where to sign in (GRA-655; kb/ADR-0029 R2, R11): the two metadata documents, the
//! pointer in the 401 from `/mcp`, and that nothing under `/oauth/` or `/.well-known/` is ever the web page.
mod common;
use axum::{body::Body, http::Request};
use common::*;

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
