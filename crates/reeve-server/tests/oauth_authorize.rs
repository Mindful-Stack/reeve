//! `GET /oauth/authorize` (GRA-657; kb/ADR-0029 R3, R4, R11): what it holds, where it sends the browser, and
//! that it never redirects to an address that was not validated. Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
mod common;
use axum::{body::Body, http::Request};
use common::{oauth::*, *};

/// The parameters of one authorize request, in the order sent.
type Sent = Vec<(&'static str, String)>;

const UNLISTED: &str = "https://attacker.example/callback";
const LISTED_BY_INSTANCE_ONLY: &str = "https://other.example/callback";

fn location(res: &axum::http::Response<Body>) -> String {
    res.headers()["location"].to_str().unwrap().to_string()
}

/// A raw query, for what `form_urlencoded` would never write.
fn raw(query: &str) -> Request<Body> {
    Request::get(format!("/oauth/authorize?{query}"))
        .body(Body::empty())
        .unwrap()
}

/// Catches a request id in the cookie (it must be the id's SHA-256, so the cookie alone opens nothing), a cookie
/// a script can read or that outlives the request, an answer a cache may keep, and an authorize that does not
/// fetch the client's document.
#[tokio::test]
async fn a_valid_request_is_held_and_the_browser_goes_to_the_page() {
    let f = flow().await;
    let res = call_raw(&f.h.app, authorize_request(&authorize_params(&f))).await;
    assert_eq!(res.status(), 303);
    assert!(is_no_store(res.headers()));
    let to = location(&res);
    let id = to.strip_prefix("/connect?request=").expect(&to);
    assert_eq!(id.len(), 48, "{id}");
    let cookies = cookies_of(&res);
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    let cookie = &cookies[0];
    assert!(
        cookie.starts_with(&format!(
            "reeve_connect={};",
            reeve_identity::hash_secret(id)
        )),
        "{cookie}"
    );
    for part in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=600"] {
        assert!(cookie.contains(part), "{cookie} lacks {part}");
    }
    assert!(
        !cookie.contains(id),
        "the cookie holds the hash, never the id"
    );
    assert_eq!(f.host.hits("/client.json"), 1);
    let view =
        f.h.state
            .authorization_requests
            .read(id, 1, f.h.state.clock.now())
            .unwrap();
    assert_eq!(
        (
            view.client_host.as_str(),
            view.client_name.as_str(),
            view.redirect_uri.as_str(),
            view.requested_at
        ),
        ("127.0.0.1", "Example App", CALLBACK, f.h.started)
    );
}

/// kb/ADR-0029 R3 and RFC 6749 section 4.1.2.1: until the client and its redirect address are validated, the
/// browser is never sent to the address the request names. Catches an open redirect on each early refusal, a
/// cookie set for a refused request, a refusal that names the wrong problem, and a parser that lets a malformed
/// query through.
#[tokio::test]
async fn a_client_or_address_that_is_not_validated_is_never_redirected_to() {
    let f = flow_with(&[CALLBACK, LISTED_BY_INSTANCE_ONLY]).await;
    f.host.answer(
        "/gone.json",
        reeve_identity::FakeAnswer::Status {
            code: 404,
            location: None,
        },
    );
    let ok = authorize_params(&f);
    let good = query_of(&ok);
    let cases: Vec<(&str, Request<Body>, &str)> = vec![
        (
            "no client_id",
            authorize_request(&with(ok.clone(), "client_id", None)),
            "invalid_request",
        ),
        (
            "no redirect_uri",
            authorize_request(&with(ok.clone(), "redirect_uri", None)),
            "invalid_request",
        ),
        (
            "an empty redirect_uri",
            authorize_request(&with(ok.clone(), "redirect_uri", Some(""))),
            "invalid_request",
        ),
        (
            "client_id twice",
            authorize_request(&twice(ok.clone(), "client_id", &f.client_id)),
            "invalid_request",
        ),
        (
            "redirect_uri twice",
            authorize_request(&twice(ok.clone(), "redirect_uri", UNLISTED)),
            "invalid_request",
        ),
        (
            "an address the instance does not list",
            authorize_request(&with(ok.clone(), "redirect_uri", Some(UNLISTED))),
            "app_not_enabled",
        ),
        (
            "an address the instance lists and the client does not",
            authorize_request(&with(
                ok.clone(),
                "redirect_uri",
                Some(LISTED_BY_INSTANCE_ONLY),
            )),
            "invalid_request",
        ),
        (
            "a client on a host the instance does not list",
            authorize_request(&with(
                ok.clone(),
                "client_id",
                Some("https://attacker.example/client.json"),
            )),
            "app_not_enabled",
        ),
        (
            "a client id that breaks the id rules (a query) on the listed host",
            authorize_request(&with(
                ok.clone(),
                "client_id",
                Some(&format!("{}?x=1", f.client_id)),
            )),
            "invalid_request",
        ),
        (
            "a client whose document is gone",
            authorize_request(&with(
                ok.clone(),
                "client_id",
                Some(&f.host.client_id("/gone.json")),
            )),
            "invalid_request",
        ),
        // The query itself is malformed: nothing in it is trusted, the valid client included.
        (
            "a percent escape that is not two hex digits",
            raw(&format!("{good}&state=%zz")),
            "invalid_request",
        ),
        (
            "a percent escape cut short",
            raw(&format!("{good}&x=%2")),
            "invalid_request",
        ),
        (
            "a state that is not UTF-8 once decoded",
            raw(&format!("{good}&state=%ff%fe")),
            "invalid_request",
        ),
        (
            "a name that is not UTF-8 once decoded",
            raw(&format!("{good}&%ff=1")),
            "invalid_request",
        ),
    ];
    for (what, req, problem) in cases {
        let res = call_raw(&f.h.app, req).await;
        assert_eq!(res.status(), 303, "{what}");
        assert_eq!(
            location(&res),
            format!("/connect?problem={problem}"),
            "{what}"
        );
        assert!(cookies_of(&res).is_empty(), "{what}");
        assert!(is_no_store(res.headers()), "{what}");
    }
    // The controls: the same harness accepts the valid request, also with escapes the parser must decode.
    for query in [good.clone(), format!("{good}&x=%41%2b+b")] {
        let res = call_raw(&f.h.app, raw(&query)).await;
        assert!(
            location(&res).starts_with("/connect?request="),
            "{}",
            location(&res)
        );
    }
}

/// Once the client is known, every refusal goes back to its validated address with `state` and `iss` (RFC 9207
/// section 2) and the standard's error code. Catches a missing `iss` or `state` on an error, a code on an error,
/// a wrong error code, a repeated parameter read as its first value, and a refused request that is held or gets
/// a cookie.
#[tokio::test]
async fn after_the_client_is_known_every_refusal_goes_back_with_state_and_iss() {
    let f = flow().await;
    let ok = authorize_params(&f);
    let cases: Vec<(&str, Sent, &str)> = vec![
        (
            "response_type missing",
            with(ok.clone(), "response_type", None),
            "invalid_request",
        ),
        (
            "response_type token",
            with(ok.clone(), "response_type", Some("token")),
            "unsupported_response_type",
        ),
        (
            "no code_challenge",
            with(ok.clone(), "code_challenge", None),
            "invalid_request",
        ),
        (
            "a challenge of 42 characters",
            with(ok.clone(), "code_challenge", Some(&"a".repeat(42))),
            "invalid_request",
        ),
        (
            "a challenge with a +",
            with(
                ok.clone(),
                "code_challenge",
                Some(&format!("{}+", "a".repeat(43))),
            ),
            "invalid_request",
        ),
        (
            "no code_challenge_method",
            with(ok.clone(), "code_challenge_method", None),
            "invalid_request",
        ),
        (
            "the plain method",
            with(ok.clone(), "code_challenge_method", Some("plain")),
            "invalid_request",
        ),
        (
            "no resource",
            with(ok.clone(), "resource", None),
            "invalid_target",
        ),
        (
            "another resource",
            with(ok.clone(), "resource", Some(ISSUER)),
            "invalid_target",
        ),
        (
            "the resource with a slash",
            with(ok.clone(), "resource", Some(&format!("{RESOURCE}/"))),
            "invalid_target",
        ),
        (
            "the resource twice, both right",
            twice(ok.clone(), "resource", RESOURCE),
            "invalid_target",
        ),
        (
            "an unknown scope",
            with(ok.clone(), "scope", Some("read admin")),
            "invalid_scope",
        ),
        (
            "scope twice",
            twice(ok.clone(), "scope", "read"),
            "invalid_request",
        ),
        (
            "response_type twice",
            twice(ok.clone(), "response_type", "code"),
            "invalid_request",
        ),
        (
            "the challenge twice",
            twice(ok.clone(), "code_challenge", CHALLENGE),
            "invalid_request",
        ),
        // The resource is judged before every other repeated parameter, whatever the order they were sent in.
        (
            "response_type and the resource twice",
            twice(
                twice(ok.clone(), "response_type", "code"),
                "resource",
                RESOURCE,
            ),
            "invalid_target",
        ),
    ];
    for (what, params, error) in cases {
        let res = call_raw(&f.h.app, authorize_request(&params)).await;
        assert_eq!(res.status(), 303, "{what}");
        assert!(is_no_store(res.headers()), "{what}");
        let to = url::Url::parse(&location(&res)).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(without_query(&to), CALLBACK, "{what}");
        assert_eq!(param(&to, "error").as_deref(), Some(error), "{what}");
        assert_eq!(param(&to, "state").as_deref(), Some("s-1"), "{what}");
        assert_eq!(param(&to, "iss").as_deref(), Some(ISSUER), "{what}");
        assert_eq!(param(&to, "code"), None, "{what}");
        assert!(cookies_of(&res).is_empty(), "{what}");
    }
    // `state` sent twice cannot be echoed, and one over 2048 bytes is not held: the error goes back with `iss`.
    for (what, params, echoed) in [
        ("state twice", twice(ok.clone(), "state", "s-2"), None),
        (
            "a state of 2049 bytes",
            with(ok.clone(), "state", Some(&"s".repeat(2049))),
            Some("s".repeat(2049)),
        ),
    ] {
        let res = call_raw(&f.h.app, authorize_request(&params)).await;
        let to = url::Url::parse(&location(&res)).unwrap();
        assert_eq!(
            (
                param(&to, "error").as_deref(),
                param(&to, "state"),
                param(&to, "iss").as_deref()
            ),
            (Some("invalid_request"), echoed, Some(ISSUER)),
            "{what}"
        );
        assert!(cookies_of(&res).is_empty(), "{what}");
    }
    // The controls: what is optional or ignored is accepted.
    for (what, params) in [
        (
            "offline_access is accepted and ignored",
            with(ok.clone(), "scope", Some("read write offline_access")),
        ),
        ("no scope", with(ok.clone(), "scope", None)),
        ("no state", with(ok.clone(), "state", None)),
        (
            "a state of exactly 2048 bytes",
            with(ok.clone(), "state", Some(&"s".repeat(2048))),
        ),
        (
            "an unknown parameter, twice",
            twice(
                with(ok.clone(), "prompt", Some("consent")),
                "prompt",
                "login",
            ),
        ),
    ] {
        let res = call_raw(&f.h.app, authorize_request(&params)).await;
        assert!(
            location(&res).starts_with("/connect?request="),
            "{what}: {}",
            location(&res)
        );
    }
}

/// kb/ADR-0029 R11: the server caps pending requests. Catches a store that grows past its bound, and one that
/// stays full after its requests have expired. The clock is fixed: "eleven minutes later" is an `advance`.
#[tokio::test]
async fn a_store_full_of_requests_people_are_looking_at_answers_temporarily_unavailable() {
    let f = flow().await;
    let s = &f.h.state;
    let client = reeve_identity::resolve_client(
        &*s.client_fetcher,
        &s.config.client_policy(),
        &f.client_id,
        CALLBACK,
    )
    .await
    .unwrap();
    let now = s.clock.now();
    for _ in 0..reeve_identity::MAX_AUTHORIZATION_REQUESTS {
        let challenge = reeve_identity::CodeChallenge::parse(CHALLENGE).unwrap();
        let id = s
            .authorization_requests
            .open(&client, challenge, RESOURCE, None, now)
            .unwrap();
        s.authorization_requests.read(&id, 1, now).unwrap();
    }
    let res = call_raw(&f.h.app, authorize_request(&authorize_params(&f))).await;
    let to = url::Url::parse(&location(&res)).unwrap();
    assert_eq!(without_query(&to), CALLBACK);
    assert_eq!(
        param(&to, "error").as_deref(),
        Some("temporarily_unavailable")
    );
    assert_eq!(param(&to, "iss").as_deref(), Some(ISSUER));
    s.clock.advance(chrono::Duration::minutes(10));
    let res = call_raw(&f.h.app, authorize_request(&authorize_params(&f))).await;
    assert!(
        location(&res).starts_with("/connect?request="),
        "{}",
        location(&res)
    );
}

/// Authorize answers GET only, in plain text, and that answer is not stored either. (Off, it is a 404 for every
/// method: `tests/mcp_discovery.rs`.)
#[tokio::test]
async fn authorize_answers_get_only() {
    let f = flow().await;
    let post = Request::post("/oauth/authorize")
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = send(&f.h.app, post).await;
    assert_eq!(status, 405);
    assert_eq!(headers["allow"], "GET");
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    assert!(is_no_store(&headers));
}

/// Every answer of authorize says `no-store`, the 404 of a switched-off instance included. Catches an answer
/// that leaves the handler unmarked.
#[tokio::test]
async fn off_authorize_answers_a_404_that_is_not_stored() {
    let h = harness().await;
    let (status, headers, _) = send(&h.app, raw("client_id=x")).await;
    assert_eq!(status, 404);
    assert!(is_no_store(&headers));
}

/// kb/ADR-0029 R4: the approval page "MUST NOT be framed". The page is the web client's, so its headers are the
/// ones every page gets; this pins them for `/connect`. Catches the policy losing `frame-ancestors`.
#[cfg(debug_assertions)]
#[tokio::test]
async fn the_approval_page_is_served_with_a_policy_that_forbids_framing() {
    let f = flow().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><title>Reeve</title>",
    )
    .unwrap();
    let app = f.h.app_with_assets(dir.path());
    let page = Request::get("/connect?request=abc")
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = send(&app, page).await;
    assert_eq!(status, 200);
    let policy = headers["content-security-policy"].to_str().unwrap();
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
}
