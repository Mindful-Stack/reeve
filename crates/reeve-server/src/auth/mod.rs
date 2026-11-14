//! Who is calling: the bearer header (CLI, automation) or the browser's session cookie, resolved to the same
//! `Principal` on every request (kb/ADR-0015 R5). The browser sign-in flow lives in `routes`; the public OAuth
//! surface of the MCP endpoint (kb/ADR-0029) lives in `oauth`, `authorize` and `token`.
pub(crate) mod authorize;
pub(crate) mod oauth;
pub mod routes;
pub(crate) mod token;

use crate::{error::AppError, state::AppState};
use axum::{
    extract::FromRequestParts,
    http::{Method, request::Parts},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use reeve_identity::{AuthError, McpPrincipal, Principal, authenticate_api, authenticate_mcp};

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

/// The secret of a request's bearer header: `Ok(None)` without an `Authorization` header, `Err` with the reason when
/// the header is not a bearer token. The one reading of that header, for `Caller` and `McpCaller` alike.
pub(crate) fn bearer_secret(headers: &axum::http::HeaderMap) -> Result<Option<&str>, &'static str> {
    let Some(header) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Ok(None);
    };
    let header = header
        .to_str()
        .map_err(|_| "authorization header is not text")?;
    match header.split_once(' ') {
        Some((scheme, secret)) if scheme.eq_ignore_ascii_case("bearer") => Ok(Some(secret.trim())),
        _ => Err("authorization header is not a bearer token"),
    }
}

/// The authenticated caller, the instant the request was authenticated (the clock is read once per request, here),
/// and the secret it presented, kept so a long-lived response can re-authenticate on every tick (kb/ADR-0015 R5).
pub struct Caller(
    pub Principal,
    pub DateTime<Utc>,
    String,
    reeve_identity::AccessPolicy,
);

impl FromRequestParts<AppState> for Caller {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        // Every refusal names its reason at `warn` (never the secret); `authenticate_api` logs why a secret was refused.
        let refuse = |reason: &'static str, err: AppError| {
            tracing::warn!(method = %parts.method, path = %request_path(parts), reason, "request refused");
            err
        };
        let (secret, via) = match bearer_secret(&parts.headers) {
            // 1. A bearer header decides alone; a cookie never rescues a bad header.
            Ok(Some(secret)) => (secret.to_string(), "bearer"),
            Err(reason) => return Err(refuse(reason, AppError::Unauthenticated)),
            // 2. The session cookie, with the CSRF header rule for anything but a read.
            Ok(None) => {
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
        let now = state.clock.now();
        // Loaded on every request, never cached (kb/ADR-0015 R5); a load failure is internal, never a fallback to the floor.
        let policy = state.access.load(&state.identity).await?;
        match authenticate_api(&*state.tokens, &policy, &secret, now).await {
            Ok(p) => Ok(Caller(p, now, secret, policy)),
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
    /// The policy this request was authenticated with: handlers reuse it rather than reading the tables again.
    pub fn policy(&self) -> &reeve_identity::AccessPolicy {
        &self.3
    }
    /// The bearer or session-cookie secret this request authenticated with.
    pub fn secret(&self) -> &str {
        &self.2
    }

    /// What an action needs to know about this caller.
    pub(crate) fn acting(&self) -> crate::actions::Acting<'_> {
        let via = match self.0.token_kind {
            reeve_protocol::TokenKind::Interactive => reeve_protocol::Via::Interactive,
            reeve_protocol::TokenKind::Automation => reeve_protocol::Via::Automation,
        };
        crate::actions::Acting::new(&self.0.email, via, self.1, &self.3, None)
    }

    pub fn ctx(&self) -> reeve_planning::Ctx {
        self.acting().ctx
    }
}

/// The caller of the MCP endpoint (kb/ADR-0029 R6): an MCP token in the bearer header and nothing else. It never
/// reads cookies. Not an extractor: the endpoint calls `from_headers` itself, after its switch and origin check.
pub(crate) struct McpCaller {
    pub(crate) principal: McpPrincipal,
    /// The instant the request was authenticated; the clock is read once per request, here.
    pub(crate) now: DateTime<Utc>,
    policy: reeve_identity::AccessPolicy,
}

/// A client as an activity entry names it (kb/ADR-0029 R9): the host of the connection's client id, which is the
/// address of the client's metadata document. A client id without a host (the developer command's
/// `urn:reeve:developer-command`) is recorded as it is, so an entry always names something.
fn client_label(client_id: &str) -> String {
    url::Url::parse(client_id)
        .ok()
        .and_then(|address| address.host_str().map(str::to_string))
        .unwrap_or_else(|| client_id.to_string())
}

impl McpCaller {
    /// `Unauthenticated` for every refusal; `Db` when the access policy or the token could not be read, which is an
    /// internal error and never a fallback to the floor. Each refusal is logged once at `warn`, never with the
    /// secret: a missing or malformed bearer header here, with what the request said about its protocol (before
    /// sign-in exists, this line is how it is learnt which revision a client speaks); a refused token by
    /// `authenticate_mcp`, with its reason.
    pub(crate) async fn from_headers(
        state: &AppState,
        headers: &axum::http::HeaderMap,
    ) -> Result<Self, AuthError> {
        let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        let refuse = |reason: &'static str| {
            tracing::warn!(
                path = "/mcp",
                reason,
                protocol_version = ?text("mcp-protocol-version"),
                mcp_method = ?text("mcp-method"),
                "request refused"
            );
            AuthError::Unauthenticated
        };
        let secret = bearer_secret(headers)
            .map_err(refuse)?
            .ok_or_else(|| refuse("no bearer header"))?;
        let now = state.clock.now();
        // Loaded on every request, never cached (kb/ADR-0015 R5, kb/ADR-0029 R8).
        let policy = state.access.load(&state.identity).await?;
        let principal = authenticate_mcp(&*state.tokens, &policy, secret, now).await?;
        Ok(Self {
            principal,
            now,
            policy,
        })
    }

    /// What an action needs to know about this caller. An MCP token is always interactive (kb/ADR-0029 R5), and
    /// its writes name the connection's client (R9).
    pub(crate) fn acting(&self) -> crate::actions::Acting<'_> {
        crate::actions::Acting::new(
            &self.principal.email,
            reeve_protocol::Via::Interactive,
            self.now,
            &self.policy,
            Some(client_label(&self.principal.client_id)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::client_label;

    #[test]
    fn a_client_is_named_by_the_host_of_its_id_or_by_the_id_when_it_has_no_host() {
        assert_eq!(
            client_label("https://claude.ai/oauth/client-metadata.json"),
            "claude.ai"
        );
        assert_eq!(
            client_label("http://127.0.0.1:4455/client.json"),
            "127.0.0.1"
        );
        assert_eq!(
            client_label("urn:reeve:developer-command"),
            "urn:reeve:developer-command"
        );
        assert_eq!(client_label("not a url"), "not a url");
    }
}
