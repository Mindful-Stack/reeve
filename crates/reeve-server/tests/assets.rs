//! The embedded client: SPA fallback, caching, CSP, and the paths that must never fall back. These tests read a fake
//! build from a temp directory through the debug-only `Assets::Dir` override, so they do not depend on `web/build`.
#![cfg(debug_assertions)]
mod common;
use axum::{body::Body, http::Request};
use base64::Engine;
use common::*;
use sha2::{Digest, Sha256};

const INDEX: &str = "<!doctype html><title>Reeve</title><script>window.x=1</script>";

async fn body_of(res: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn spa_fallback_serves_index_for_pages_but_never_for_api_or_auth() {
    let h = harness().await;
    // The test writes a fake build (index.html + _app/x.js) into a temp dir and points the server at it through the
    // `Assets::Dir` override (rust-embed reads from disk in debug builds, but the override keeps the test independent
    // of the repository's web/build contents).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), INDEX).unwrap();
    std::fs::create_dir_all(dir.path().join("_app/immutable")).unwrap();
    std::fs::write(dir.path().join("_app/x.js"), "// plain").unwrap();
    std::fs::write(dir.path().join("_app/immutable/x.js"), "// immutable").unwrap();
    std::fs::write(dir.path().join("robots.txt"), "User-agent: *").unwrap();
    let app = h.app_with_assets(dir.path());
    for p in [
        "/",
        "/index.html",
        "/board",
        "/initiatives/TST-1",
        "/auth/signin",
        "/auth/signin?reason=not_allowed",
    ] {
        let res = call_raw(&app, Request::get(p).body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), 200, "{p}");
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html"),
            "{p}"
        );
        assert_eq!(res.headers()["cache-control"], "no-cache");
        // the inline bootstrap script must be allowed by hash, or the app would never start
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest("window.x=1"));
        let csp = res.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains(&format!("'sha256-{hash}'")), "{p}: {csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
        assert!(csp.contains("base-uri 'self'"), "{csp}");
        assert!(csp.contains("form-action 'self'"), "{csp}");
        assert!(!csp.contains("https:"), "{csp}");
        assert_eq!(res.headers()["x-content-type-options"], "nosniff", "{p}");
        assert_eq!(res.headers()["referrer-policy"], "no-referrer", "{p}");
        assert_eq!(body_of(res).await, INDEX, "{p}");
    }
    let res = call_raw(
        &app,
        Request::get("/_app/immutable/x.js")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert!(
        res.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    assert_eq!(res.headers()["referrer-policy"], "no-referrer");
    assert_eq!(body_of(res).await, "// immutable");
    // a plain asset is never immutable
    let res = call_raw(
        &app,
        Request::get("/_app/x.js").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.headers()["cache-control"], "no-cache");
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert_eq!(body_of(res).await, "// plain");
    let res = call_raw(
        &app,
        Request::get("/robots.txt").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["cache-control"], "no-cache");
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    assert_eq!(body_of(res).await, "User-agent: *");
    // a missing file is a 404, never the HTML page
    for p in [
        "/_app/immutable/missing.js",
        "/_app/version.json",
        "/favicon.ico",
    ] {
        let res = call_raw(&app, Request::get(p).body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), 404, "{p}");
        assert!(
            !res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html"),
            "{p}"
        );
    }
    // API paths keep their JSON 404 (unknown routes are not gated; see plan A's malformed_input test) and the browser
    // endpoints never fall back
    let res = call_raw(
        &app,
        Request::get("/api/v1/nope").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.status(), 404);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    assert_eq!(
        call_raw(
            &app,
            Request::get("/auth/callback").body(Body::empty()).unwrap()
        )
        .await
        .status(),
        400
    );
    for p in ["/auth/login/", "/auth/status/", "/auth/logout/", "/health/"] {
        let res = call_raw(&app, Request::get(p).body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), 404, "{p}");
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("application/json"),
            "{p}"
        );
    }
    // traversal through the directory override is refused
    let res = call_raw(
        &app,
        Request::get("/../../etc/passwd")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_ne!(res.status(), 200);
    let res = call_raw(
        &app,
        Request::get("/_app/../../../etc/passwd")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(
        res.status() == 404 || res.status() == 400,
        "{}",
        res.status()
    );
    // no build at all → 503 with a plain message
    let empty = tempfile::tempdir().unwrap();
    let app = h.app_with_assets(empty.path());
    let res = call_raw(&app, Request::get("/board").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), 503);
    assert_eq!(
        body_of(res).await,
        "client not built: run `mise run web` (see deploy/README.md)"
    );
}
