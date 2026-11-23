//! CLI distribution (kb/ADR-0030 R3): `/cli/{name}` without sign-in for the bundle's names and the two scripts, 404
//! for anything else or while distribution is off, credentials ignored, the `/cli/` headers on every answer.
mod common;
use axum::{body::Body, http::Request};
use common::*;
use http_body_util::BodyExt;
use reeve_server::{assets, cli_dist};

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

async fn bytes_of(res: axum::response::Response) -> Vec<u8> {
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

/// The headers every `/cli/` answer carries, and the cookie it never sets.
fn assert_cli_headers(res: &axum::response::Response, what: &str) {
    assert_eq!(res.headers()["cache-control"], "no-cache", "{what}");
    assert_eq!(res.headers()["x-content-type-options"], "nosniff", "{what}");
    assert_eq!(res.headers()["referrer-policy"], "no-referrer", "{what}");
    assert!(cookies_of(res).is_empty(), "{what} set a cookie");
}

/// Distinguishes a route that serves the wrong file for a name (the contents differ), drops a header, or serves the
/// scripts unrendered.
#[tokio::test]
async fn the_bundle_and_scripts_are_served_to_anyone() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    let sums = cli_bundle(dir.path());
    let app = h.app_with_cli(dir.path());
    for name in CLI_BINARIES {
        let res = call_raw(&app, get(&format!("/cli/{name}"))).await;
        assert_eq!(res.status(), 200, "{name}");
        assert_eq!(
            res.headers()["content-type"],
            "application/octet-stream",
            "{name}"
        );
        let want = cli_binary(name);
        assert_eq!(
            res.headers()["content-length"],
            want.len().to_string().as_str(),
            "{name}"
        );
        assert_cli_headers(&res, name);
        assert_eq!(bytes_of(res).await, want.into_bytes(), "{name}");
    }
    for (name, want) in [
        ("VERSION", format!("{CLI_VERSION}\n")),
        ("SHA256SUMS", sums),
    ] {
        let res = call_raw(&app, get(&format!("/cli/{name}"))).await;
        assert_eq!(res.status(), 200, "{name}");
        assert_eq!(
            res.headers()["content-type"],
            "text/plain; charset=utf-8",
            "{name}"
        );
        assert_cli_headers(&res, name);
        assert_eq!(
            String::from_utf8(bytes_of(res).await).unwrap(),
            want,
            "{name}"
        );
    }
    for (name, line) in [
        ("install.sh", "REEVE_URL='http://127.0.0.1:7474'"),
        ("install.ps1", "$ReeveUrl = 'http://127.0.0.1:7474'"),
    ] {
        let res = call_raw(&app, get(&format!("/cli/{name}"))).await;
        assert_eq!(res.status(), 200, "{name}");
        assert_eq!(
            res.headers()["content-type"],
            "text/plain; charset=utf-8",
            "{name}"
        );
        assert_cli_headers(&res, name);
        let body = String::from_utf8(bytes_of(res).await).unwrap();
        assert!(
            body.contains(line),
            "{name} not rendered with the harness's public_url"
        );
        assert!(
            !body.contains("@REEVE_URL@"),
            "{name} still holds the placeholder"
        );
    }
    // HEAD: headers, no body (axum's `get` answers HEAD by dropping the body).
    let res = call_raw(
        &app,
        Request::head("/cli/reeve-linux-x86_64")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    assert_cli_headers(&res, "HEAD");
    assert_eq!(res.headers()["content-type"], "application/octet-stream");
    // The length GET's body has, though HEAD sends none.
    assert_eq!(
        res.headers()["content-length"],
        cli_binary("reeve-linux-x86_64").len().to_string().as_str()
    );
    assert!(bytes_of(res).await.is_empty(), "HEAD sent a body");
}

/// Anything but those names is a hardened 404 and never the client page: unknown names, traversal, deeper paths, a
/// name that is not UTF-8 (axum's `Path<String>` would answer 400), and every name while distribution is off.
/// Distinguishes a missing never-page rule in `assets::serve`, a lookup that joins the request into a path, a handler
/// that takes `Path<String>` directly, and headers added only on the success path.
#[cfg(debug_assertions)]
#[tokio::test]
async fn anything_else_under_cli_is_a_hardened_404() {
    let h = harness().await;
    let cli_dir = tempfile::tempdir().unwrap();
    cli_bundle(cli_dir.path());
    std::fs::write(
        cli_dir.path().join("extra"),
        "not one of the bundle's files",
    )
    .unwrap();
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><title>Reeve</title>",
    )
    .unwrap();
    let mut state = h.state.clone();
    state.assets = assets::Assets::Dir(web.path().to_path_buf());
    let off = reeve_server::app(state.clone());
    let url = h.state.config.configured_public_url().unwrap();
    state.cli_dist = Some(std::sync::Arc::new(
        cli_dist::CliDist::load(cli_dir.path(), Some(&url)).unwrap(),
    ));
    let on = reeve_server::app(state);
    let paths = [
        "/cli",
        "/cli/",
        "/cli/extra",
        "/cli/reeve",
        "/cli/VERSION.bak",
        "/cli/install.SH",
        "/cli/%2E%2E%2FVERSION",
        "/cli/..%2F..%2Fetc%2Fpasswd",
        "/cli/a/b",
        "/cli/reeve-linux-x86_64/x",
        "/cli/%FF",
        "/cli/%C3%28",
    ];
    for (what, app) in [("on", &on), ("off", &off)] {
        for p in paths {
            let res = call_raw(app, get(p)).await;
            assert_eq!(res.status(), 404, "{what}: {p}");
            assert_cli_headers(&res, &format!("{what}: {p}"));
            assert!(
                !bytes_of(res).await.starts_with(b"<!doctype"),
                "{what}: {p} fell back to the client page"
            );
        }
    }
    for name in ["VERSION", "install.sh", "reeve-linux-x86_64"] {
        let res = call_raw(&off, get(&format!("/cli/{name}"))).await;
        assert_eq!(res.status(), 404, "off: {name}");
        assert!(!res.headers().contains_key("x-reeve-cli"), "off: {name}");
    }
    let res = call_raw(&off, get("/health")).await;
    assert_eq!(res.status(), 200);
    assert!(
        !res.headers().contains_key("x-reeve-cli"),
        "off: /health offers a build"
    );
}

/// The other failures carry the headers too: a method other than GET or HEAD (the route's own 405), a bundle file
/// that vanished after startup (500). Also pins that `build_state` reads no bundle by itself.
#[tokio::test]
async fn failures_under_cli_carry_the_headers_too() {
    let h = harness().await;
    assert!(
        h.state.cli_dist.is_none(),
        "build_state must leave distribution off; only serve turns it on"
    );
    let dir = tempfile::tempdir().unwrap();
    cli_bundle(dir.path());
    let app = h.app_with_cli(dir.path());
    std::fs::remove_file(dir.path().join("reeve-macos-x86_64")).unwrap();
    for (what, req, status) in [
        (
            "POST",
            Request::post("/cli/VERSION").body(Body::empty()).unwrap(),
            405,
        ),
        ("a vanished binary", get("/cli/reeve-macos-x86_64"), 500),
    ] {
        let res = call_raw(&app, req).await;
        assert_eq!(res.status(), status, "{what}");
        assert_cli_headers(&res, what);
    }
}

/// A token or cookie on a `/cli/` request is ignored: a bad bearer would be a 401 behind `Caller`, a session cookie
/// could otherwise be refreshed. Everything else keeps its sign-in (kb/ADR-0015 R5).
#[tokio::test]
async fn a_token_or_cookie_is_ignored_and_none_is_set() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    cli_bundle(dir.path());
    let app = h.app_with_cli(dir.path());
    let admin = format!("Bearer {}", h.admin_token);
    for (k, v) in [
        ("authorization", "Bearer not-a-token"),
        ("authorization", admin.as_str()),
        ("cookie", "reeve_session=garbage"),
        ("cookie", "__Host-reeve_session=garbage"),
    ] {
        let res = call_raw(
            &app,
            Request::get("/cli/VERSION")
                .header(k, v)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), 200, "{k}: {v}");
        assert!(cookies_of(&res).is_empty(), "{k}: {v} set a cookie");
        assert_eq!(
            String::from_utf8(bytes_of(res).await).unwrap(),
            format!("{CLI_VERSION}\n")
        );
    }
    assert_eq!(
        call_raw(&app, get("/api/v1/me")).await.status(),
        401,
        "the API must still require a caller"
    );
}

/// Every response carries the build the server offers while distribution is on: API answers, a 401, an unknown API
/// path, a 405, `/cli/` itself, the static fallback, an MCP route, an OAuth address, and the event stream's head.
/// Distinguishes a header added per route (the fallbacks would lack it) and a layer that buffers bodies (the event
/// stream never ends: the timeout turns a hang into a failure).
#[tokio::test]
async fn every_response_carries_the_build_while_distribution_is_on() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    cli_bundle(dir.path());
    let app = h.app_with_cli(dir.path());
    let bearer = format!("Bearer {}", h.admin_token);
    let authed = |p: &str| {
        Request::get(p)
            .header("authorization", bearer.as_str())
            .body(Body::empty())
            .unwrap()
    };
    let cases: Vec<(&str, Request<Body>, Option<u16>)> = vec![
        ("health", get("/health"), Some(200)),
        ("openapi", get("/api/openapi.json"), Some(200)),
        ("me", authed("/api/v1/me"), Some(200)),
        ("me without a caller", get("/api/v1/me"), Some(401)),
        (
            "an unknown API path",
            get("/api/v1/no-such-route"),
            Some(404),
        ),
        (
            "a wrong method",
            Request::post("/health").body(Body::empty()).unwrap(),
            Some(405),
        ),
        ("a CLI file", get("/cli/VERSION"), Some(200)),
        ("an unknown CLI name", get("/cli/nope"), Some(404)),
        ("auth status", get("/auth/status"), None),
        // The status depends on whether the client and MCP are built and configured; the header is what matters.
        ("the static fallback", get("/board"), None),
        ("the MCP endpoint", get("/mcp"), None),
        (
            "an OAuth address",
            get("/.well-known/oauth-authorization-server"),
            None,
        ),
    ];
    for (what, req, status) in cases {
        let res = call_raw(&app, req).await;
        if let Some(s) = status {
            assert_eq!(res.status(), s, "{what}");
        }
        assert_eq!(
            res.headers()
                .get("x-reeve-cli")
                .map(|v| v.to_str().unwrap()),
            Some(CLI_VERSION),
            "{what}"
        );
    }
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        call_raw(&app, authed("/api/v1/events")),
    )
    .await
    .expect("the event stream's head must arrive without its body");
    assert_eq!(res.status(), 200);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    assert_eq!(res.headers()["x-reeve-cli"], CLI_VERSION);
}
