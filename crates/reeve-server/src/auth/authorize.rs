//! `GET /oauth/authorize` (kb/ADR-0029 R3, R4, R11): validates a client's request to connect, holds it in the
//! server's state, binds it to this browser with a cookie and sends the browser to the approval page, `/connect`.
//! Only the request's id passes through the browser, so nothing the client sent can be altered on the way.
//!
//! Until the client and its redirect address are both validated nothing is sent to the client (RFC 6749 section
//! 4.1.2.1): the browser goes to the page with `?problem=`. After that a refusal goes back to the validated
//! address with `error`, `state` and `iss`. No answer of this route may be stored by a cache, the 404 of a
//! switched-off instance and the 405 included: `authorize` marks whatever `answer` returns.
use super::{
    oauth::{self, Params, answer_address, no_store},
    routes::same,
};
use crate::state::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use reeve_identity::{CodeChallenge, hash_secret, resolve_client};

/// Binds an authorization request to the browser that started it; holds the SHA-256 of the request's id, never
/// the id. This is the name on an http instance (local development).
pub const CONNECT_COOKIE: &str = "reeve_connect";
/// The name on an https instance: browsers accept the prefix only with `Secure`, `Path=/` and no `Domain`.
pub const HOST_CONNECT_COOKIE: &str = "__Host-reeve_connect";
/// As long as a request lives.
const CONNECT_MINUTES: i64 = 10;
/// The longest `state` held for a client. kb/ADR-0029 R11 caps the number of requests; this caps what each holds.
const MAX_STATE: usize = 2048;
/// Every parameter this route reads. Each may be sent once.
const RECOGNISED: [&str; 8] = [
    "response_type",
    "client_id",
    "redirect_uri",
    "code_challenge",
    "code_challenge_method",
    "resource",
    "scope",
    "state",
];

fn connect_cookie_name(https: bool) -> &'static str {
    if https {
        HOST_CONNECT_COOKIE
    } else {
        CONNECT_COOKIE
    }
}

/// `Path=/` in both modes: the API under `/api/v1` must receive it. One name, so the latest request in a browser
/// wins: a second connect in another tab orphans the first, which then shows as expired.
pub(crate) fn connect_cookie(value: String, https: bool) -> Cookie<'static> {
    Cookie::build((connect_cookie_name(https), value))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(https)
        .max_age(time::Duration::minutes(CONNECT_MINUTES))
        .build()
}

/// Whether the request's cookies, under the name this instance issues, bind the authorization request `id`.
#[cfg_attr(not(test), allow(dead_code))] // its callers are the API routes of the next commit, which removes this line
pub(crate) fn connect_bound(headers: &HeaderMap, id: &str, https: bool) -> bool {
    CookieJar::from_headers(headers)
        .get(connect_cookie_name(https))
        .is_some_and(|c| same(c.value(), &hash_secret(id)))
}

/// The page, saying why nothing can be approved. Never the client's address.
fn problem(code: &'static str) -> Response {
    Redirect::to(&format!("/connect?problem={code}")).into_response()
}

/// The checks that come after the client is known, in this order. `Err` is the OAuth error code.
///
/// | Check | Refused with |
/// |---|---|
/// | `resource` sent more than once, whatever else is repeated | `invalid_target` |
/// | any other recognised parameter sent more than once (`state` included) | `invalid_request` |
/// | `response_type` missing | `invalid_request` |
/// | `response_type` not `code` | `unsupported_response_type` |
/// | `code_challenge` missing or not 43 to 128 unreserved characters; the method not `S256` | `invalid_request` |
/// | `resource` missing or not `<issuer>/mcp` | `invalid_target` |
/// | `scope` naming anything but `read`, `write`, `offline_access` | `invalid_scope` |
/// | `state` longer than 2048 bytes | `invalid_request` |
fn judge(params: &Params, resource: &str) -> Result<CodeChallenge, &'static str> {
    // RFC 8707: a resource that cannot be read as one address is an invalid target. Asked on its own, so that it
    // does not depend on which repeated parameter `repeated` names first.
    if params.repeated(&["resource"]).is_some() {
        return Err("invalid_target");
    }
    if params.repeated(&RECOGNISED).is_some() {
        return Err("invalid_request");
    }
    match params.get("response_type") {
        Some("code") => {}
        None => return Err("invalid_request"),
        Some(_) => return Err("unsupported_response_type"),
    }
    // RFC 7636 section 4.4.1: a missing challenge and an unsupported transformation are both invalid_request.
    let challenge = params
        .get("code_challenge")
        .and_then(CodeChallenge::parse)
        .ok_or("invalid_request")?;
    if params.get("code_challenge_method") != Some("S256") {
        return Err("invalid_request");
    }
    // RFC 8707 section 2; kb/ADR-0029 R4 requires it.
    if params.get("resource") != Some(resource) {
        return Err("invalid_target");
    }
    // `offline_access` is accepted and ignored: a refresh token is always issued.
    if let Some(scope) = params.get("scope")
        && !scope
            .split(' ')
            .all(|name| oauth::SCOPES.contains(&name) || name == "offline_access")
    {
        return Err("invalid_scope");
    }
    if params
        .get("state")
        .is_some_and(|state| state.len() > MAX_STATE)
    {
        return Err("invalid_request");
    }
    Ok(challenge)
}

/// `ANY /oauth/authorize`: the switch, then GET only, then the request. Every answer is marked `no-store` here.
pub(crate) async fn authorize(
    State(s): State<AppState>,
    method: Method,
    uri: Uri,
    jar: CookieJar,
) -> Response {
    no_store(answer(s, method, uri, jar).await)
}

async fn answer(s: AppState, method: Method, uri: Uri, jar: CookieJar) -> Response {
    if !s.config.mcp.enabled {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    if method != Method::GET {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET")],
            "this address answers GET only",
        )
            .into_response();
    }
    // 1. The query, then the client and its redirect address. Until both are validated, nothing is sent to the
    // client.
    let Ok(params) = Params::parse(uri.query().unwrap_or("")) else {
        tracing::warn!(reason = "the query is malformed", "authorize refused");
        return problem("invalid_request");
    };
    let (None, Some(client_id), Some(redirect_uri)) = (
        params.repeated(&["client_id", "redirect_uri"]),
        params.get("client_id"),
        params.get("redirect_uri"),
    ) else {
        tracing::warn!(
            reason = "client_id or redirect_uri missing or repeated",
            "authorize refused"
        );
        return problem("invalid_request");
    };
    let client = match resolve_client(
        &*s.client_fetcher,
        &s.config.client_policy(),
        client_id,
        redirect_uri,
    )
    .await
    {
        Ok(client) => client,
        // Already logged by `resolve_client`.
        Err(refusal) => {
            return problem(if refusal.not_enabled() {
                "app_not_enabled"
            } else {
                "invalid_request"
            });
        }
    };
    // 2. From here a refusal goes back to the validated address, with `state` and `iss`. A `state` sent twice
    // cannot be echoed.
    let issuer = s.config.issuer();
    let state = match params.repeated(&["state"]) {
        Some(_) => None,
        None => params.get("state"),
    };
    let back = |error: &'static str| {
        tracing::warn!(
            client_host = client.client_host(),
            reason = error,
            "authorize refused"
        );
        Redirect::to(&answer_address(
            client.redirect_uri(),
            &[("error", error)],
            state,
            &issuer,
        ))
        .into_response()
    };
    let resource = oauth::resource(&s.config);
    let challenge = match judge(&params, &resource) {
        Ok(challenge) => challenge,
        Err(error) => return back(error),
    };
    // 3. Hold it, bind this browser to it, and go to the page.
    let opened = s.authorization_requests.open(
        &client,
        challenge,
        &resource,
        state.map(str::to_string),
        s.clock.now(),
    );
    match opened {
        Ok(id) => (
            jar.add(connect_cookie(hash_secret(&id), s.config.https())),
            Redirect::to(&format!("/connect?request={id}")),
        )
            .into_response(),
        Err(reeve_identity::RequestsFull) => back("temporarily_unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a connect cookie without the attributes the `__Host-` prefix needs, one scoped away from the API,
    /// and an https instance that accepts the unprefixed name (which a sibling subdomain or a plain-http answer
    /// could plant). The router test on an https instance is in `tests/connection_requests.rs`.
    #[test]
    fn the_connect_cookie_is_host_prefixed_exactly_on_https_and_read_only_under_that_name() {
        let c = connect_cookie("h".into(), true).to_string();
        assert!(c.starts_with("__Host-reeve_connect=h;"), "{c}");
        for part in [
            "HttpOnly",
            "SameSite=Lax",
            "Path=/",
            "Secure",
            "Max-Age=600",
        ] {
            assert!(c.contains(part), "{c} lacks {part}");
        }
        assert!(!c.contains("Domain"), "{c}");
        let c = connect_cookie("h".into(), false).to_string();
        assert!(
            c.starts_with("reeve_connect=h;") && !c.contains("Secure"),
            "{c}"
        );

        let id = "the-request-id";
        let headers = |cookie: String| {
            let mut h = HeaderMap::new();
            h.insert(header::COOKIE, cookie.parse().unwrap());
            h
        };
        let hash = hash_secret(id);
        assert!(connect_bound(
            &headers(format!("reeve_connect={hash}")),
            id,
            false
        ));
        assert!(connect_bound(
            &headers(format!("__Host-reeve_connect={hash}")),
            id,
            true
        ));
        assert!(!connect_bound(
            &headers(format!("reeve_connect={hash}")),
            id,
            true
        ));
        assert!(!connect_bound(
            &headers(format!("__Host-reeve_connect={hash}")),
            id,
            false
        ));
        assert!(
            !connect_bound(&headers(format!("reeve_connect={id}")), id, false),
            "the id itself is not the proof"
        );
        assert!(!connect_bound(
            &headers(format!("reeve_connect={hash}")),
            "another-id",
            false
        ));
        assert!(!connect_bound(&HeaderMap::new(), id, false));
    }
}
