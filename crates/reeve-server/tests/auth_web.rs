mod common;
use axum::{body::Body, http::Request};
use common::*;

/// GET /auth/login?next=..., returning the state the provider carries back and the `reeve_login=...` cookie pair.
async fn begin_login(app: &axum::Router, next: &str) -> (String, String) {
    begin_login_named(app, next, "reeve_login").await
}

/// As `begin_login`, for the login cookie issued under `name` (`__Host-reeve_login` on https).
async fn begin_login_named(app: &axum::Router, next: &str, name: &str) -> (String, String) {
    let res = call_raw(
        app,
        Request::get(format!("/auth/login?next={}", urlencoding::encode(next)))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 303, "login for next {next:?}");
    let loc = res.headers()["location"].to_str().unwrap().to_string();
    let state = loc.rsplit("state=").next().unwrap().to_string();
    let cookie = cookies_of(&res)
        .into_iter()
        .find(|c| c.starts_with(&format!("{name}=")))
        .expect("login cookie");
    (state, cookie.split(';').next().unwrap().to_string())
}

fn callback(state: &str, login_cookie: &str) -> Request<Body> {
    Request::get(format!("/auth/callback?code=static&state={state}"))
        .header("cookie", login_cookie)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn login_callback_sets_a_session_cookie_that_authenticates_and_logout_revokes_it() {
    let h = harness().await;
    // GET /auth/login?next=/board → 303 to the provider; the static provider's URL carries the state back to our callback
    let res = call_raw(
        &h.app,
        Request::get("/auth/login?next=/board")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 303);
    let loc = res.headers()["location"].to_str().unwrap().to_string();
    assert!(
        loc.starts_with("http://127.0.0.1:7474/auth/callback?code=static&state="),
        "{loc}"
    );
    let state = loc.rsplit("state=").next().unwrap().to_string();
    // the login response binds the pending state to this browser with a short-lived cookie
    let login_cookie = cookies_of(&res)
        .into_iter()
        .find(|c| c.starts_with("reeve_login="))
        .expect("login cookie");
    assert!(
        login_cookie.contains("HttpOnly")
            && login_cookie.contains("Path=/auth")
            && login_cookie.contains("Max-Age=600"),
        "{login_cookie}"
    );
    assert!(
        !login_cookie.contains(&state),
        "the cookie carries a hash of the state, not the state"
    );
    let login_cookie = login_cookie.split(';').next().unwrap().to_string();
    // a callback with a valid state but no login cookie is 400 and sets no session (login CSRF)
    let res = call_raw(
        &h.app,
        Request::get(format!("/auth/callback?code=static&state={state}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 400);
    assert!(
        cookies_of(&res)
            .iter()
            .all(|c| !c.starts_with("reeve_session="))
    );
    // with the login cookie → 303 to next, Set-Cookie reeve_session; HttpOnly, SameSite=Lax, Path=/, not Secure on http; login cookie cleared
    let res = call_raw(&h.app, callback(&state, &login_cookie)).await;
    assert_eq!(res.status(), 303);
    assert_eq!(res.headers()["location"], "/board");
    let cookies = cookies_of(&res);
    let sc = cookies
        .iter()
        .find(|c| c.starts_with("reeve_session="))
        .unwrap()
        .clone();
    assert!(sc.starts_with("reeve_session=reeve_"), "{sc}");
    assert!(
        sc.contains("HttpOnly")
            && sc.contains("SameSite=Lax")
            && sc.contains("Path=/")
            && !sc.contains("Secure"),
        "{sc}"
    );
    assert!(
        sc.contains(&format!("Max-Age={}", 30 * 24 * 60 * 60)),
        "{sc}"
    );
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("reeve_login=") && c.contains("Max-Age=0")),
        "login cookie must be cleared"
    );
    let cookie = sc.split(';').next().unwrap().to_string();
    // the same state cannot be replayed, even with the login cookie
    let res = call_raw(&h.app, callback(&state, &login_cookie)).await;
    assert_eq!(res.status(), 400);
    // cookie authenticates GET /api/v1/me as dan, interactive
    let res = call_raw(
        &h.app,
        Request::get("/api/v1/me")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    let me = body_json(res).await;
    assert_eq!(me["value"]["email"], "dan@example.com");
    assert_eq!(me["value"]["token_kind"], "interactive");
    // a cookie-authenticated POST without the client header is 403 (CSRF); with it, it proceeds
    let post = |hdr: bool| {
        let mut r = Request::post("/api/v1/initiatives")
            .header("cookie", &cookie)
            .header("content-type", "application/json");
        if hdr {
            r = r.header("x-reeve-client", "web");
        }
        r.body(Body::from(
            r#"{"idempotency_key":"i1","title":"X","horizon":"Now"}"#,
        ))
        .unwrap()
    };
    assert_eq!(call_raw(&h.app, post(false)).await.status(), 403);
    assert_eq!(call_raw(&h.app, post(true)).await.status(), 200);
    // bearer requests never need the header
    let (st, _) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/epics",
        Some(serde_json::json!({"idempotency_key":"e1","title":"E"})),
    )
    .await;
    assert_eq!(st, 200);
    // logout without the client header is refused and leaves the session live
    let res = call_raw(
        &h.app,
        Request::post("/auth/logout")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 403);
    let res = call_raw(
        &h.app,
        Request::get("/api/v1/me")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    // logout revokes the session token and clears the cookie
    let res = call_raw(
        &h.app,
        Request::post("/auth/logout")
            .header("cookie", &cookie)
            .header("x-reeve-client", "web")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 303);
    assert_eq!(res.headers()["location"], "/auth/signin");
    assert!(
        cookies_of(&res)
            .iter()
            .any(|c| c.starts_with("reeve_session=") && c.contains("Max-Age=0"))
    );
    let res = call_raw(
        &h.app,
        Request::get("/api/v1/me")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 401);
    // logout with a dead or missing cookie still clears the cookie and redirects (no 401 page on sign-out)
    for with_cookie in [true, false] {
        let mut r = Request::post("/auth/logout").header("x-reeve-client", "web");
        if with_cookie {
            r = r.header("cookie", &cookie);
        }
        let res = call_raw(&h.app, r.body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), 303);
        assert!(
            cookies_of(&res)
                .iter()
                .any(|c| c.starts_with("reeve_session=") && c.contains("Max-Age=0"))
        );
    }
}

#[tokio::test]
async fn callback_refuses_an_email_outside_the_allowlist_and_unknown_state() {
    let h = harness_with(|cfg| {
        cfg.replace(
            "static_email = \"dan@example.com\"",
            "static_email = \"x@other.org\"",
        )
    })
    .await;
    let (state, login_cookie) = begin_login(&h.app, "/").await;
    let res = call_raw(&h.app, callback(&state, &login_cookie)).await;
    // not allowed → redirected to the client's sign-in page with a reason, no session cookie
    assert_eq!(res.status(), 303);
    assert_eq!(res.headers()["location"], "/auth/signin?reason=not_allowed");
    assert!(
        cookies_of(&res)
            .iter()
            .all(|c| !c.starts_with("reeve_session="))
    );
    let res = call_raw(&h.app, callback("nope", &login_cookie)).await;
    assert_eq!(res.status(), 400);
    // the provider reporting an error (user cancelled) is a 400 page, not a JSON rejection; it consumes the pending login
    let (state2, login_cookie2) = begin_login(&h.app, "/").await;
    let res = call_raw(
        &h.app,
        Request::get(format!("/auth/callback?error=access_denied&state={state2}"))
            .header("cookie", &login_cookie2)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 400);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(
        cookies_of(&res)
            .iter()
            .any(|c| c.starts_with("reeve_login=") && c.contains("Max-Age=0"))
    );
    let res = call_raw(&h.app, callback(&state2, &login_cookie2)).await;
    assert_eq!(
        res.status(),
        400,
        "a cancelled login cannot be completed afterwards"
    );
    // server-side expiry: backdate the pending entry through the harness state
    let (state3, login_cookie3) = begin_login(&h.app, "/").await;
    h.state
        .logins
        .lock()
        .await
        .get_mut(&state3)
        .unwrap()
        .created_at -= chrono::Duration::minutes(11);
    let res = call_raw(&h.app, callback(&state3, &login_cookie3)).await;
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn a_login_cookie_from_another_sign_in_does_not_consume_the_pending_login() {
    let h = harness().await;
    let (victim_state, victim_cookie) = begin_login(&h.app, "/").await;
    let (_, attacker_cookie) = begin_login(&h.app, "/").await;
    let res = call_raw(&h.app, callback(&victim_state, &attacker_cookie)).await;
    assert_eq!(res.status(), 400);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    // the victim's own browser can still complete it
    let res = call_raw(&h.app, callback(&victim_state, &victim_cookie)).await;
    assert_eq!(res.status(), 303);
}

#[tokio::test]
async fn next_must_be_a_local_path() {
    let h = harness().await;
    for bad in [
        "https://evil.example",
        "//evil.example/x",
        "javascript:alert(1)",
        "/\\evil.example",
        "/\t/evil.example",
        "/ok\r\nSet-Cookie: x=y",
        "/%2F%2Fevil.example",
        // dot segments collapse after the join into a protocol-relative `//evil.example`
        "/..//evil.example",
        "/.//evil.example",
        "/%2e%2e//evil.example",
        "/..%2F%2Fevil.example",
        "/./%2Fevil.example",
        "/@evil.example",
        &format!("/{}", "a".repeat(9 * 1024)),
    ] {
        let (state, login_cookie) = begin_login(&h.app, bad).await;
        let res = call_raw(&h.app, callback(&state, &login_cookie)).await;
        assert_eq!(
            res.status(),
            303,
            "next {bad:?} must not break the redirect"
        );
        assert_eq!(
            res.headers()["location"],
            "/",
            "next {bad:?} must be dropped"
        );
    }
    for (next, want) in [
        ("/board?epic=TST-2", "/board?epic=TST-2"),
        ("/a/../board", "/board"),
    ] {
        let (state, login_cookie) = begin_login(&h.app, next).await;
        let res = call_raw(&h.app, callback(&state, &login_cookie)).await;
        assert_eq!(res.headers()["location"], want, "next {next:?}");
    }
}

#[tokio::test]
async fn status_reports_whether_sign_in_is_configured_and_login_is_a_page_when_it_is_not() {
    let h = harness().await;
    let res = call_raw(
        &h.app,
        Request::get("/auth/status").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    assert!(cookies_of(&res).is_empty());
    assert_eq!(body_json(res).await["sign_in"], true);

    let h = harness_with(|cfg| {
        cfg.replace(
            "[oidc]\nprovider = \"static\"\nstatic_email = \"dan@example.com\"\n",
            "",
        )
    })
    .await;
    assert!(h.state.oidc.is_none());
    let res = call_raw(
        &h.app,
        Request::get("/auth/status").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(body_json(res).await["sign_in"], false);
    let res = call_raw(
        &h.app,
        Request::get("/auth/login").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.status(), 404);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(cookies_of(&res).is_empty());
}

#[tokio::test]
async fn the_cookie_path_checks_the_token_on_every_request_and_accepts_only_the_issued_name() {
    let h = harness().await;
    // any interactive token works as the cookie value under the name this (http) instance issues
    let me = |cookie: String| {
        Request::get("/api/v1/me")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap()
    };
    let res = call_raw(&h.app, me(format!("reeve_session={}", h.victim_token))).await;
    assert_eq!(res.status(), 200);
    assert_eq!(body_json(res).await["value"]["email"], h.victim_email);
    // the other spelling is not this instance's cookie: the request has no session
    let res = call_raw(
        &h.app,
        me(format!("__Host-reeve_session={}", h.victim_token)),
    )
    .await;
    assert_eq!(
        res.status(),
        401,
        "an http instance ignores __Host-reeve_session"
    );
    // a garbage cookie is 401, and a cookie write with the header but a dead token is 401, not 403
    let res = call_raw(&h.app, me("reeve_session=reeve_nope".into())).await;
    assert_eq!(res.status(), 401);
    let res = call_raw(
        &h.app,
        Request::post("/api/v1/initiatives")
            .header("cookie", "reeve_session=reeve_nope")
            .header("x-reeve-client", "web")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"idempotency_key":"i9","title":"X","horizon":"Now"}"#,
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 401);
    // the login cookie, too, is read only under the issued name
    let (state, login_cookie) = begin_login(&h.app, "/").await;
    let renamed = login_cookie.replacen("reeve_login=", "__Host-reeve_login=", 1);
    let res = call_raw(&h.app, callback(&state, &renamed)).await;
    assert_eq!(
        res.status(),
        400,
        "an http instance ignores __Host-reeve_login"
    );
    let res = call_raw(&h.app, callback(&state, &login_cookie)).await;
    assert_eq!(res.status(), 303);
    // disabling the user stops the cookie on the next request (kb/ADR-0008 R2)
    let (st, _) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/me", None).await;
    assert_eq!(st, 200);
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = h.state.activity.clone();
    reeve_identity::disable_user(
        &h.state.pool,
        &*h.state.tokens,
        &*log,
        "test",
        reeve_identity::Via::System,
        &h.victim_email,
        chrono::Utc::now(),
    )
    .await
    .unwrap();
    let res = call_raw(
        &h.app,
        Request::get("/api/v1/me")
            .header("cookie", format!("reeve_session={}", h.victim_token))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn an_https_instance_issues_and_accepts_only_the_host_prefixed_cookies() {
    let h = https_harness().await;
    assert!(h.state.config.https());
    let res = call_raw(
        &h.app,
        Request::get("/auth/login?next=/board")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 303);
    let cookies = cookies_of(&res);
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    let lc = &cookies[0];
    assert!(lc.starts_with("__Host-reeve_login="), "{lc}");
    assert!(
        lc.contains("HttpOnly")
            && lc.contains("Secure")
            && lc.contains("Path=/;")
            && !lc.contains("Path=/auth")
            && !lc.contains("Domain")
            && lc.contains("Max-Age=600"),
        "{lc}"
    );
    let loc = res.headers()["location"].to_str().unwrap().to_string();
    let state = loc.rsplit("state=").next().unwrap().to_string();
    let login_cookie = lc.split(';').next().unwrap().to_string();
    // a planted unprefixed login cookie with the right hash does not bind the state on https (login CSRF by injection)
    let planted = login_cookie.replacen("__Host-reeve_login=", "reeve_login=", 1);
    let res = call_raw(&h.app, callback(&state, &planted)).await;
    assert_eq!(res.status(), 400);
    assert!(
        cookies_of(&res)
            .iter()
            .all(|c| !c.contains("reeve_session="))
    );
    // the issued name completes it: the session cookie is __Host-reeve_session, Secure, Path=/
    let res = call_raw(&h.app, callback(&state, &login_cookie)).await;
    assert_eq!(res.status(), 303);
    assert_eq!(res.headers()["location"], "/board");
    let cookies = cookies_of(&res);
    let sc = cookies
        .iter()
        .find(|c| c.starts_with("__Host-reeve_session="))
        .expect("session cookie")
        .clone();
    assert!(sc.starts_with("__Host-reeve_session=reeve_"), "{sc}");
    assert!(
        sc.contains("HttpOnly")
            && sc.contains("SameSite=Lax")
            && sc.contains("Secure")
            && sc.contains("Path=/")
            && !sc.contains("Domain"),
        "{sc}"
    );
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("__Host-reeve_login=") && c.contains("Max-Age=0")),
        "{cookies:?}"
    );
    assert!(
        cookies.iter().all(|c| c.starts_with("__Host-")),
        "{cookies:?}"
    );
    let secret = sc
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("__Host-reeve_session=")
        .to_string();
    let me = |cookie: String| {
        Request::get("/api/v1/me")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        call_raw(&h.app, me(format!("__Host-reeve_session={secret}")))
            .await
            .status(),
        200
    );
    // a planted plain `reeve_session` (a sibling subdomain, a plain-http answer) is no session at all
    assert_eq!(
        call_raw(&h.app, me(format!("reeve_session={}", h.victim_token)))
            .await
            .status(),
        401
    );
    // logout clears the issued name only, and a planted unprefixed cookie is neither read nor revoked
    let res = call_raw(
        &h.app,
        Request::post("/auth/logout")
            .header(
                "cookie",
                format!(
                    "__Host-reeve_session={secret}; reeve_session={}",
                    h.victim_token
                ),
            )
            .header("x-reeve-client", "web")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 303);
    let cookies = cookies_of(&res);
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    assert!(
        cookies[0].starts_with("__Host-reeve_session=")
            && cookies[0].contains("Max-Age=0")
            && cookies[0].contains("Secure")
            && cookies[0].contains("Path=/"),
        "{cookies:?}"
    );
    assert_eq!(
        call_raw(&h.app, me(format!("__Host-reeve_session={secret}")))
            .await
            .status(),
        401
    );
    let (st, _) = call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None).await;
    assert_eq!(st, 200, "logout revokes only the issued cookie's token");
}

#[tokio::test]
async fn a_full_pending_login_map_refuses_new_sign_ins_instead_of_evicting_live_ones() {
    let h = harness().await;
    let (live_state, live_cookie) = begin_login(&h.app, "/").await;
    {
        let mut logins = h.state.logins.lock().await;
        let now = chrono::Utc::now();
        let mut i = 0;
        while logins.len() < reeve_server::MAX_PENDING {
            logins.insert(
                format!("flood-{i}"),
                reeve_server::PendingLogin {
                    nonce: "n".into(),
                    pkce_verifier: "v".into(),
                    next: "/".into(),
                    created_at: now,
                },
            );
            i += 1;
        }
    }
    let res = call_raw(
        &h.app,
        Request::get("/auth/login").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.status(), 503);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(
        cookies_of(&res).is_empty(),
        "a refused sign-in sets no cookie"
    );
    assert_eq!(h.state.logins.lock().await.len(), reeve_server::MAX_PENDING);
    // the live sign-in that was already in progress is not evicted and still completes
    let res = call_raw(&h.app, callback(&live_state, &live_cookie)).await;
    assert_eq!(res.status(), 303);
    // an expired entry frees its slot on the next sign-in
    h.state
        .logins
        .lock()
        .await
        .get_mut("flood-0")
        .unwrap()
        .created_at -= chrono::Duration::minutes(11);
    let res = call_raw(
        &h.app,
        Request::get("/auth/login").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.status(), 303);
}

#[tokio::test]
async fn signing_in_again_revokes_the_session_the_browser_already_holds_for_the_same_person() {
    let h = harness().await;
    let (state, login_cookie) = begin_login(&h.app, "/").await;
    let res = call_raw(&h.app, callback(&state, &login_cookie)).await;
    let first = cookies_of(&res)
        .into_iter()
        .find(|c| c.starts_with("reeve_session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let me = |cookie: &str| {
        Request::get("/api/v1/me")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(call_raw(&h.app, me(&first)).await.status(), 200);
    // the second sign-in arrives with the first session's cookie alongside the login cookie
    let (state, login_cookie) = begin_login(&h.app, "/").await;
    let res = call_raw(
        &h.app,
        callback(&state, &format!("{login_cookie}; {first}")),
    )
    .await;
    assert_eq!(res.status(), 303);
    let second = cookies_of(&res)
        .into_iter()
        .find(|c| c.starts_with("reeve_session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    assert_ne!(first, second);
    assert_eq!(
        call_raw(&h.app, me(&first)).await.status(),
        401,
        "the previous session is revoked"
    );
    assert_eq!(call_raw(&h.app, me(&second)).await.status(), 200);
    // a session of a different person in the same browser is left alone
    let bob = format!("reeve_session={}", h.victim_token);
    let (state, login_cookie) = begin_login(&h.app, "/").await;
    let res = call_raw(&h.app, callback(&state, &format!("{login_cookie}; {bob}"))).await;
    assert_eq!(res.status(), 303);
    assert_eq!(call_raw(&h.app, me(&bob)).await.status(), 200);
}

/// The real Google adapter behind `/auth/login` and `/auth/callback`, pointed at a fake issuer. Every other test in
/// this file signs in through the static provider.
#[cfg(feature = "static-oidc")]
mod through_the_google_adapter {
    use super::*;
    use reeve_identity::{FakeIssuer, Grant, Signer};
    use std::collections::HashMap;

    const CLIENT_ID: &str = "reeve-server-test";
    const CLIENT_SECRET: &str = "reeve-server-secret";

    /// The harness's router with `state.oidc` replaced by the Google adapter discovering `fake` and redirecting to
    /// the harness's callback (the formula `build_oidc` uses), with the production refresh windows.
    async fn app_with_google(h: &Harness, fake: &FakeIssuer) -> axum::Router {
        let redirect = format!(
            "{}/auth/callback",
            h.state.config.public_url().as_str().trim_end_matches('/')
        );
        let oidc = reeve_identity::make_google_oidc_for_tests(
            reeve_identity::GoogleOidcConfig {
                client_id: CLIENT_ID.into(),
                client_secret: CLIENT_SECRET.into(),
                redirect_url: redirect,
            },
            reeve_identity::GoogleOidcTestOptions {
                issuer: fake.issuer_url(),
                min_refresh_gap: std::time::Duration::from_secs(60),
                max_metadata_age: std::time::Duration::from_secs(24 * 60 * 60),
                clock: reeve_identity::TestClock::new(),
            },
        )
        .await
        .unwrap();
        let mut state = h.state.clone();
        state.oidc = Some(oidc);
        reeve_server::app(state)
    }

    /// `/auth/login?next=/board` → the fake's authorization endpoint; `code` registered for `email` with what the
    /// adapter sent; `/auth/callback` with the login cookie. Returns the callback's response.
    async fn sign_in_as(
        app: &axum::Router,
        fake: &FakeIssuer,
        code: &str,
        email: &str,
        verified: bool,
    ) -> axum::http::Response<Body> {
        let res = call_raw(
            app,
            Request::get("/auth/login?next=/board")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), 303);
        let loc = res.headers()["location"].to_str().unwrap().to_string();
        assert!(
            loc.starts_with(&format!("{}/authorize?", fake.issuer_url())),
            "{loc}"
        );
        let q: HashMap<String, String> = url::Url::parse(&loc)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:7474/auth/callback");
        assert_eq!(q["client_id"], CLIENT_ID);
        let login_cookie = cookies_of(&res)
            .into_iter()
            .find(|c| c.starts_with("reeve_login="))
            .expect("login cookie");
        let login_cookie = login_cookie.split(';').next().unwrap().to_string();
        let mut claims = fake.claims(&q["nonce"], email);
        claims.email_verified = Some(verified);
        fake.register(
            code,
            Grant {
                challenge: q["code_challenge"].clone(),
                challenge_method: q["code_challenge_method"].clone(),
                redirect_uri: q["redirect_uri"].clone(),
                claims,
                signer: Signer::Published(FakeIssuer::KEY_A),
            },
        );
        call_raw(
            app,
            Request::get(format!(
                "/auth/callback?code={code}&state={}",
                urlencoding::encode(&q["state"])
            ))
            .header("cookie", login_cookie)
            .body(Body::empty())
            .unwrap(),
        )
        .await
    }

    #[tokio::test]
    async fn the_callback_signs_people_in_through_the_real_adapter_and_refuses_unverified_and_unlisted_emails()
     {
        let h = harness().await;
        let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
        let app = app_with_google(&h, &fake).await;
        for (code, email, role) in [
            ("code-admin", "dan@example.com", "admin"),
            ("code-member", "ada@example.com", "member"),
        ] {
            let hits = fake.token_hits();
            let res = sign_in_as(&app, &fake, code, email, true).await;
            assert_eq!(fake.token_hits(), hits + 1, "{email}");
            assert_eq!(res.status(), 303, "{email}");
            assert_eq!(res.headers()["location"], "/board", "{email}");
            let session = cookies_of(&res)
                .into_iter()
                .find(|c| c.starts_with("reeve_session="))
                .expect("session cookie");
            let res = call_raw(
                &app,
                Request::get("/api/v1/me")
                    .header("cookie", session.split(';').next().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(res.status(), 200, "{email}");
            let me = body_json(res).await;
            assert_eq!(me["value"]["email"], email);
            assert_eq!(me["value"]["role"], role);
            assert_eq!(me["value"]["token_kind"], "interactive");
        }
        for (code, email, verified) in [
            ("code-unverified", "carol@example.com", false),
            ("code-unlisted", "eve@elsewhere.test", true),
        ] {
            let hits = fake.token_hits();
            let res = sign_in_as(&app, &fake, code, email, verified).await;
            assert_eq!(
                fake.token_hits(),
                hits + 1,
                "{email}: the adapter exchanged the code; sign_in refused"
            );
            assert_eq!(res.status(), 303, "{email}");
            assert_eq!(
                res.headers()["location"],
                "/auth/signin?reason=not_allowed",
                "{email}"
            );
            assert!(
                cookies_of(&res)
                    .iter()
                    .all(|c| !c.starts_with("reeve_session=")),
                "{email}"
            );
        }
    }
}
