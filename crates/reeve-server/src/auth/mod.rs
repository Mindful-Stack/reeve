//! Who is calling: the bearer header (CLI, automation) or the browser's session cookie, resolved to the same
//! `Principal` on every request (kb/ADR-0015 R5). The browser sign-in flow lives in `routes`.
pub mod routes;

use crate::{error::AppError, state::AppState};
use axum::{
    extract::FromRequestParts,
    http::{Method, request::Parts},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use reeve_identity::{AuthError, Principal, authenticate};

/// The session cookie's name on an http instance (local development).
pub const SESSION_COOKIE: &str = "reeve_session";
/// The session cookie's name on an https instance: the prefix makes browsers refuse it without `Secure`, `Path=/`
/// and a `Domain`, so a sibling subdomain cannot plant or shadow it.
pub const HOST_SESSION_COOKIE: &str = "__Host-reeve_session";
/// Cookie-authenticated requests other than GET and HEAD must carry `X-Reeve-Client: web` (CSRF): a browser cannot
/// add a custom header cross-origin without a CORS preflight, and the server answers none.
pub const CLIENT_HEADER: &str = "x-reeve-client";

/// The session cookie name for this instance: `__Host-` prefixed exactly when the public URL is https.
pub fn session_cookie_name(https: bool) -> &'static str {
    if https {
        HOST_SESSION_COOKIE
    } else {
        SESSION_COOKIE
    }
}

/// The session secret a request's cookies carry, read only under the name this instance issues. A request that
/// carries the other spelling is treated as having no session: accepting the unprefixed name on an https instance
/// would let a sibling subdomain or a plain-http answer plant a session the `__Host-` prefix exists to refuse.
pub fn session_secret(headers: &axum::http::HeaderMap, https: bool) -> Option<String> {
    CookieJar::from_headers(headers)
        .get(session_cookie_name(https))
        .map(|c| c.value().to_string())
        .filter(|v| !v.is_empty())
}

/// Whether a request carries the web client's CSRF header.
pub fn has_client_header(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(CLIENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "web")
}

/// The authenticated caller, the instant the request was authenticated (the clock is read once per request, here),
/// and the secret it presented, kept so a long-lived response can re-authenticate on every tick (kb/ADR-0015 R5).
pub struct Caller(pub Principal, pub DateTime<Utc>, String);

impl FromRequestParts<AppState> for Caller {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        // Every refusal names its reason at `warn` (never the secret); `authenticate` logs why a secret was refused.
        let refuse = |reason: &'static str, err: AppError| {
            tracing::warn!(method = %parts.method, path = %request_path(parts), reason, "request refused");
            err
        };
        let (secret, via) = match parts.headers.get(axum::http::header::AUTHORIZATION) {
            // 1. A bearer header decides alone; a cookie never rescues a bad header.
            Some(header) => {
                let header = header.to_str().map_err(|_| {
                    refuse(
                        "authorization header is not text",
                        AppError::Unauthenticated,
                    )
                })?;
                match header.split_once(' ') {
                    Some((scheme, secret)) if scheme.eq_ignore_ascii_case("bearer") => {
                        (secret.trim().to_string(), "bearer")
                    }
                    _ => {
                        return Err(refuse(
                            "authorization header is not a bearer token",
                            AppError::Unauthenticated,
                        ));
                    }
                }
            }
            // 2. The session cookie, with the CSRF header rule for anything but a read.
            None => {
                let secret = session_secret(&parts.headers, state.config.https())
                    .ok_or_else(|| refuse("no credential", AppError::Unauthenticated))?;
                if parts.method != Method::GET
                    && parts.method != Method::HEAD
                    && !has_client_header(&parts.headers)
                {
                    return Err(refuse(
                        "csrf: cookie write without the client header",
                        AppError::Forbidden,
                    ));
                }
                (secret, "cookie")
            }
        };
        // Either way the token's kind, role, expiry and the owner's disabled flag are checked now, on this request.
        let now = Utc::now();
        match authenticate(&*state.tokens, &state.identity, &secret, now).await {
            Ok(p) => Ok(Caller(p, now, secret)),
            Err(AuthError::Db(e)) => Err(AppError::from(e)),
            Err(e) => {
                tracing::warn!(method = %parts.method, path = %request_path(parts), via, error = %e, "request refused: the credential did not authenticate");
                Err(AppError::Unauthenticated)
            }
        }
    }
}

/// The path as the client sent it: nested routers see only the part below their prefix (`/me` for `/api/v1/me`).
fn request_path(parts: &Parts) -> &str {
    parts
        .extensions
        .get::<axum::extract::OriginalUri>()
        .map_or(parts.uri.path(), |u| u.0.path())
}

impl Caller {
    /// The bearer or session-cookie secret this request authenticated with.
    pub fn secret(&self) -> &str {
        &self.2
    }

    pub fn ctx(&self) -> reeve_planning::Ctx {
        reeve_planning::Ctx {
            actor: self.0.email.clone(),
            via: match self.0.token_kind {
                reeve_protocol::TokenKind::Interactive => reeve_protocol::Via::Interactive,
                reeve_protocol::TokenKind::Automation => reeve_protocol::Via::Automation,
            },
            now: self.1,
        }
    }
}
