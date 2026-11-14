//! The browser sign-in flow: `/auth/login` redirects to the provider, `/auth/callback` turns a verified identity into
//! an interactive token held in an `HttpOnly` cookie, `/auth/logout` revokes it. Failures render a small HTML page;
//! the API's JSON envelope is untouched. No secret, state, nonce or provider error text reaches a page or an `info` log.
use super::{has_client_header, session_cookie_name, session_secret};
use crate::state::{AppState, PendingLogin};
use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};
use rand::{Rng, RngCore};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Binds a pending sign-in to the browser that started it; holds `hex(sha256(state))`, never the state. This is the
/// name on an http instance (local development), scoped to `Path=/auth`.
pub const LOGIN_COOKIE: &str = "reeve_login";
/// The login cookie's name on an https instance. The prefix requires `Path=/`, so the `/auth` scoping is given up;
/// the cookie is `HttpOnly`, lives ten minutes and holds only a hash.
pub const HOST_LOGIN_COOKIE: &str = "__Host-reeve_login";
/// How long a pending sign-in lives, on the server and in the login cookie.
const LOGIN_TTL_MINUTES: i64 = 10;
/// At most this many live pending sign-ins are held. When the map is full a new sign-in is refused with a 503 rather
/// than evicting a live one (evicting would let an anonymous flood cancel every real sign-in in progress). The reverse
/// proxy rate-limits `/auth/login` (deploy/README.md); a sealed login cookie that removes the map is the follow-up if
/// this ever bites.
pub const MAX_PENDING: usize = 1000;
/// A `next` longer than this is dropped before any processing.
const MAX_NEXT: usize = 2048;

/// The login cookie name for this instance: `__Host-` prefixed exactly when the public URL is https.
pub fn login_cookie_name(https: bool) -> &'static str {
    if https {
        HOST_LOGIN_COOKIE
    } else {
        LOGIN_COOKIE
    }
}

/// Mounted at `/auth` in `app()`, outside the OpenAPI router (none of these is an API operation).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/status", get(status))
        .route("/login", get(login))
        .route("/callback", get(callback))
        .route("/logout", post(logout))
}

/// A minimal HTML page for the browser routes' failures.
pub struct Page {
    status: StatusCode,
    title: &'static str,
    text: &'static str,
}

impl Page {
    fn new(status: StatusCode, title: &'static str, text: &'static str) -> Self {
        Page {
            status,
            title,
            text,
        }
    }
    fn bad_request(text: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "Sign-in failed", text)
    }
    fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Something went wrong",
            "the server could not complete this request; try again",
        )
    }
}

impl IntoResponse for Page {
    fn into_response(self) -> Response {
        // Title and text are compile-time constants from this module, so nothing here needs escaping.
        let body = format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{t}</title></head>\
             <body><h1>{t}</h1><p>{x}.</p><p><a href=\"/auth/signin\">Back to sign-in</a></p></body></html>",
            t = self.title,
            x = self.text
        );
        (self.status, Html(body)).into_response()
    }
}

/// A failure page together with the cookie changes made before it (a cleared login cookie, say).
fn fail(jar: CookieJar, page: Page) -> Response {
    (jar, page).into_response()
}

/// `{"sign_in": bool}`: whether this instance has browser sign-in. No auth, no cookie, no side effect.
async fn status(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "sign_in": s.oidc.is_some() }))
}

fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// A PKCE verifier: 64 characters from the unreserved set (RFC 7636 allows 43..=128).
fn pkce_verifier() -> String {
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

fn state_hash(state: &str) -> String {
    hex::encode(Sha256::digest(state.as_bytes()))
}

/// Compares two strings without an early exit on the first differing byte.
pub(crate) fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// `next` must be a same-origin path. Policy:
/// - longer than 2 048 bytes → dropped, before anything else;
/// - percent-decode exactly once more (the `Query` extractor already decoded once), only to *detect* encoded
///   slashes, backslashes and control characters; on that decoded text require: starts with "/", second character
///   is not "/" or a backslash, no backslash and no ASCII control character anywhere (browsers turn `\` into `/` and
///   drop tab and newline; a CR or LF would also break the `Location` header), no "@" in the path;
/// - both the decoded text and the `next` as received are joined onto the public URL, and each joined URL must keep
///   the public URL's origin and yield a path that starts with exactly one "/" and holds no backslash or control
///   character. This is checked on the *output*, after WHATWG dot-segment removal: `/..//evil.example` collapses to
///   `//evil.example`, a protocol-relative URL to another host.
///
/// The redirect is the path and query of `next` as received, joined (so `/a/../board` becomes `/board` and a
/// legitimate `%26` in the query survives). Anything that fails becomes "/".
fn local_path(next: Option<&str>, public_url: &url::Url) -> String {
    fn bad_text(s: &str) -> bool {
        s.contains('\\') || s.chars().any(|c| c.is_control())
    }
    fn joined_path(candidate: &str, public_url: &url::Url) -> Option<String> {
        let joined = public_url.join(candidate).ok()?;
        if joined.origin() != public_url.origin() {
            return None;
        }
        let out = match joined.query() {
            Some(q) => format!("{}?{q}", joined.path()),
            None => joined.path().to_string(),
        };
        if !out.starts_with('/') || out.starts_with("//") || bad_text(&out) {
            return None;
        }
        Some(out)
    }
    fn check(next: &str, public_url: &url::Url) -> Option<String> {
        if next.len() > MAX_NEXT {
            return None;
        }
        let decoded = urlencoding::decode(next).ok()?;
        let mut chars = decoded.chars();
        if chars.next() != Some('/') || matches!(chars.next(), Some('/' | '\\')) {
            return None;
        }
        if bad_text(&decoded) || bad_text(next) {
            return None;
        }
        let path_part = decoded.split(['?', '#']).next().unwrap_or("");
        if path_part.contains('@') {
            return None;
        }
        joined_path(&decoded, public_url)?;
        joined_path(next, public_url)
    }
    next.and_then(|n| check(n, public_url))
        .unwrap_or_else(|| "/".into())
}

fn login_cookie(value: String, https: bool) -> Cookie<'static> {
    Cookie::build((login_cookie_name(https), value))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path(if https { "/" } else { "/auth" })
        .secure(https)
        .max_age(time::Duration::minutes(LOGIN_TTL_MINUTES))
        .build()
}

/// A removal cookie (empty, `Max-Age=0`, expired) with the attributes of the one it clears. Added rather than passed
/// to `CookieJar::remove`, which emits nothing when the request did not carry the cookie.
pub(crate) fn removal(mut c: Cookie<'static>) -> Cookie<'static> {
    c.make_removal();
    c
}

fn clear_login_cookie(jar: CookieJar, https: bool) -> CookieJar {
    jar.add(removal(login_cookie(String::new(), https)))
}

fn clear_session_cookie(jar: CookieJar, https: bool) -> CookieJar {
    jar.add(removal(session_cookie(String::new(), https, 1)))
}

fn session_cookie(secret: String, https: bool, days: i64) -> Cookie<'static> {
    Cookie::build((session_cookie_name(https), secret))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(https)
        .max_age(time::Duration::days(days))
        .build()
}

#[derive(Deserialize)]
struct LoginQuery {
    next: Option<String>,
}

async fn login(
    State(s): State<AppState>,
    jar: CookieJar,
    q: Result<Query<LoginQuery>, QueryRejection>,
) -> Result<(CookieJar, Redirect), Page> {
    let Some(oidc) = s.oidc.clone() else {
        return Err(Page::new(
            StatusCode::NOT_FOUND,
            "Sign-in is not configured",
            "sign-in is not configured on this instance; ask the operator for a CLI token",
        ));
    };
    let public_url = s.config.public_url();
    let next = q.ok().and_then(|Query(q)| q.next);
    let next = local_path(next.as_deref(), &public_url);
    let (state, nonce, verifier) = (random_hex(32), random_hex(32), pkce_verifier());
    let url = oidc.begin(&state, &nonce, &verifier).await.map_err(|e| {
        tracing::error!(error = %e, "oidc begin failed");
        Page::internal()
    })?;
    let now = Utc::now();
    {
        let mut logins = s.logins.lock().await;
        logins.retain(|_, p| !expired(p, now));
        if logins.len() >= MAX_PENDING {
            tracing::warn!("sign-in refused: the pending sign-in map is full");
            return Err(Page::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Sign-in is busy",
                "too many sign-ins in progress, try again in a minute",
            ));
        }
        logins.insert(
            state.clone(),
            PendingLogin {
                nonce,
                pkce_verifier: verifier,
                next,
                created_at: now,
            },
        );
    }
    let https = s.config.https();
    Ok((
        jar.add(login_cookie(state_hash(&state), https)),
        Redirect::to(&url),
    ))
}

fn expired(p: &PendingLogin, now: DateTime<Utc>) -> bool {
    now - p.created_at > chrono::Duration::minutes(LOGIN_TTL_MINUTES)
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Whether the browser's login cookie, under the name this instance issues, binds this `state`.
fn bound(jar: &CookieJar, state: &str, https: bool) -> bool {
    jar.get(login_cookie_name(https))
        .is_some_and(|c| same(c.value(), &state_hash(state)))
}

async fn callback(
    State(s): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    q: Result<Query<CallbackQuery>, QueryRejection>,
) -> Response {
    let https = s.config.https();
    let Ok(Query(q)) = q else {
        return fail(jar, Page::bad_request("the sign-in response was malformed"));
    };
    let Some(oidc) = s.oidc.clone() else {
        return fail(
            jar,
            Page::new(
                StatusCode::NOT_FOUND,
                "Sign-in is not configured",
                "sign-in is not configured on this instance",
            ),
        );
    };
    // The provider reported an error (typically the user cancelled): consume the pending login if this browser
    // owns it, never echo the provider's text.
    if q.error.is_some() {
        if let Some(state) = q.state.as_deref()
            && bound(&jar, state, https)
        {
            s.logins.lock().await.remove(state);
            return fail(
                clear_login_cookie(jar, https),
                Page::bad_request("sign-in was cancelled"),
            );
        }
        return fail(jar, Page::bad_request("sign-in was cancelled"));
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return fail(
            jar,
            Page::bad_request("the sign-in response was incomplete"),
        );
    };
    // Login CSRF: the pending entry is touched only by the browser that started it.
    if !bound(&jar, &state, https) {
        return fail(
            jar,
            Page::bad_request("this sign-in was started in another browser"),
        );
    }
    let now = Utc::now();
    let pending = s.logins.lock().await.remove(&state);
    let jar = clear_login_cookie(jar, https);
    let Some(pending) = pending.filter(|p| !expired(p, now)) else {
        return fail(jar, Page::bad_request("sign-in expired, start again"));
    };
    let who = match oidc
        .finish(&code, &pending.pkce_verifier, &pending.nonce)
        .await
    {
        Ok(who) => who,
        Err(e) => {
            tracing::info!("sign-in failed at the provider");
            tracing::debug!(error = %e, "oidc finish failed");
            return fail(jar, Page::bad_request("sign-in failed"));
        }
    };
    let policy = match s.access.load(&s.identity).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "sign-in could not read the access policy");
            return fail(jar, Page::internal());
        }
    };
    // Re-login: a live session this browser already holds for the same person is revoked before the new one is
    // issued, so one browser holds one live session and a re-login never orphans a 30-day token.
    if let Some(old) = session_secret(&headers, https)
        && let Ok(p) = reeve_identity::authenticate_api(&*s.tokens, &policy, &old, now).await
        && p.email == reeve_identity::normalise_email(&who.email)
        && let Err(e) = revoke_session(&s, &old).await
    {
        tracing::error!(error = %e, "sign-in could not revoke the previous session");
        return fail(jar, Page::internal());
    }
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let token = match reeve_identity::sign_in(
        &s.pool,
        &*s.tokens,
        &*log,
        &policy,
        who,
        s.config.session_days,
        now,
    )
    .await
    {
        Ok(t) => t,
        Err(reeve_identity::AuthError::NotAllowed | reeve_identity::AuthError::Disabled) => {
            return (jar, Redirect::to("/auth/signin?reason=not_allowed")).into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "sign-in could not issue a session");
            return fail(jar, Page::internal());
        }
    };
    (
        jar.add(session_cookie(token.secret, https, s.config.session_days)),
        Redirect::to(&pending.next),
    )
        .into_response()
}

/// Signs out without the `Caller` extractor, so a dead session can still sign out. Revokes the cookie's token when
/// it is live, then always clears the cookie and sends the browser to the sign-in page. A cookie write, so it needs
/// the client header like any other (CSRF).
async fn logout(State(s): State<AppState>, jar: CookieJar, headers: HeaderMap) -> Response {
    if !has_client_header(&headers) {
        return fail(
            jar,
            Page::new(
                StatusCode::FORBIDDEN,
                "Sign-out refused",
                "sign out from the Reeve page itself",
            ),
        );
    }
    let https = s.config.https();
    let jar = clear_session_cookie(jar, https);
    if let Some(secret) = session_secret(&headers, https)
        && let Err(e) = revoke_session(&s, &secret).await
    {
        tracing::error!(error = %e, "sign-out could not revoke the session");
        return fail(jar, Page::internal());
    }
    (jar, Redirect::to("/auth/signin")).into_response()
}

async fn revoke_session(s: &AppState, secret: &str) -> Result<(), reeve_identity::AuthError> {
    let Some(row) = s
        .tokens
        .find_by_hash(&reeve_identity::hash_secret(secret))
        .await?
    else {
        return Ok(());
    };
    if row.revoked_at.is_some() {
        return Ok(());
    }
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    reeve_identity::revoke_token(
        &s.pool,
        &*s.tokens,
        &*log,
        &row.owner,
        reeve_identity::Via::Interactive,
        row.id,
        Utc::now(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public() -> url::Url {
        url::Url::parse("https://reeve.example.com").unwrap()
    }

    #[test]
    fn local_path_keeps_same_origin_paths_and_drops_everything_else() {
        let u = public();
        for (next, want) in [
            (Some("/board"), "/board"),
            (Some("/board?epic=TST-2"), "/board?epic=TST-2"),
            (Some("/a/../b"), "/b"),
            (Some("/a/../board"), "/board"),
            // decoded exactly once: the `Query` extractor's decode is the only one the redirect sees
            (Some("/stories?q=a%26b"), "/stories?q=a%26b"),
            (None, "/"),
            (Some(""), "/"),
            (Some("board"), "/"),
            (Some("https://evil.example"), "/"),
            (Some("//evil.example"), "/"),
            (Some("/\\evil.example"), "/"),
            (Some("/a\\b"), "/"),
            (Some("/\t/evil.example"), "/"),
            (Some("/ok\r\nSet-Cookie: x=y"), "/"),
            (Some("/%2F%2Fevil.example"), "/"),
            (Some("/%5Cevil.example"), "/"),
            (Some("/%0d%0aX: y"), "/"),
            (Some("/%ff"), "/"),
            (Some("javascript:alert(1)"), "/"),
            // dot segments that collapse to a protocol-relative `//host` after the join
            (Some("/..//evil.example"), "/"),
            (Some("/.//evil.example"), "/"),
            (Some("/a/..//evil.example"), "/"),
            (Some("/%2e%2e//evil.example"), "/"),
            (Some("/..%2F%2Fevil.example"), "/"),
            (Some("/./%2Fevil.example"), "/"),
            (Some("/@evil.example"), "/"),
        ] {
            assert_eq!(local_path(next, &u), want, "{next:?}");
        }
        let long = format!("/{}", "a".repeat(MAX_NEXT));
        assert_eq!(
            local_path(Some(&long), &u),
            "/",
            "a next over the cap is dropped"
        );
        let at_cap = format!("/{}", "a".repeat(MAX_NEXT - 1));
        assert_eq!(local_path(Some(&at_cap), &u), at_cap);
    }

    #[test]
    fn cookies_are_secure_and_host_prefixed_exactly_on_https() {
        let c = session_cookie("s".into(), true, 30).to_string();
        assert!(c.starts_with("__Host-reeve_session=s;"), "{c}");
        for part in [
            "HttpOnly",
            "SameSite=Lax",
            "Path=/",
            "Secure",
            "Max-Age=2592000",
        ] {
            assert!(c.contains(part), "{c} lacks {part}");
        }
        assert!(!c.contains("Domain"), "{c}");
        let c = session_cookie("s".into(), false, 1).to_string();
        assert!(
            c.starts_with("reeve_session=s;") && !c.contains("Secure"),
            "{c}"
        );
        let c = login_cookie("h".into(), true).to_string();
        assert!(c.starts_with("__Host-reeve_login=h;"), "{c}");
        assert!(
            c.contains("Secure")
                && c.contains("Path=/;")
                && !c.contains("Path=/auth")
                && c.contains("Max-Age=600"),
            "{c}"
        );
        assert!(!c.contains("Domain"), "{c}");
        let c = login_cookie("h".into(), false).to_string();
        assert!(
            c.starts_with("reeve_login=h;") && c.contains("Path=/auth") && !c.contains("Secure"),
            "{c}"
        );
    }

    #[test]
    fn the_removal_cookie_keeps_the_attributes_a_host_prefixed_cookie_needs() {
        let jar = clear_session_cookie(CookieJar::new(), true);
        let res = (jar, "x").into_response();
        let sc = res.headers()[axum::http::header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(sc.starts_with("__Host-reeve_session=;"), "{sc}");
        assert!(
            sc.contains("Max-Age=0") && sc.contains("Secure") && sc.contains("Path=/"),
            "{sc}"
        );
    }

    #[test]
    fn secrets_are_long_and_the_verifier_is_in_range() {
        assert_eq!(random_hex(32).len(), 64);
        let v = pkce_verifier();
        assert_eq!(v.len(), 64);
        assert!(v.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }
}
