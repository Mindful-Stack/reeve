//! `POST /oauth/token` (kb/ADR-0029 R5, R11): exchanges an authorization code, or a refresh token, for an MCP
//! token and a new refresh token. A form of at most 16 KiB in; JSON out with `Cache-Control: no-store` and
//! `Pragma: no-cache` (RFC 6749 section 5.1); every refusal after the switch is an OAuth error object (section
//! 5.2), never the API's envelope. Clients are public: nothing authenticates them. Nothing a caller sent is ever
//! logged: every reason is a fixed phrase.
use super::oauth::{self, Params, no_store};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use reeve_identity::{ExchangeRequest, GrantError, Issuing, RefreshRequest, TokenPair};
use serde_json::json;

/// The largest body read. A raw `Request` gets no default limit, so this is the only one.
const MAX_BODY: usize = 16 * 1024;
/// Every parameter this route reads, for either grant. Each may be sent once.
const RECOGNISED: [&str; 9] = [
    "grant_type",
    "client_id",
    "client_secret",
    "code",
    "code_verifier",
    "redirect_uri",
    "resource",
    "scope",
    "refresh_token",
];

/// A refusal: the status, RFC 6749's error code, a fixed reason for the operator's log, and a header the status
/// calls for.
struct Refusal {
    status: StatusCode,
    error: &'static str,
    reason: &'static str,
    header: Option<(header::HeaderName, HeaderValue)>,
}

fn bad(error: &'static str, reason: &'static str) -> Refusal {
    Refusal {
        status: StatusCode::BAD_REQUEST,
        error,
        reason,
        header: None,
    }
}

fn internal(e: impl std::fmt::Debug) -> Refusal {
    tracing::error!(error = ?e, "token endpoint: internal error");
    Refusal {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        error: "server_error",
        reason: "internal error",
        header: None,
    }
}

/// The scheme of an `Authorization` header, when it is a plain token; `Basic` otherwise (the one scheme RFC 6749
/// defines for client authentication). It is echoed in a response header, so only letters, digits and `-` pass.
fn scheme_of(value: &HeaderValue) -> String {
    value
        .to_str()
        .ok()
        .and_then(|v| v.split(' ').next())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 32
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .unwrap_or("Basic")
        .to_string()
}

/// `ANY /oauth/token`: the switch, then POST only, then the body, then the grant.
pub(crate) async fn token(State(s): State<AppState>, req: Request) -> Response {
    if !s.config.mcp.enabled {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let (parts, body) = req.into_parts();
    let outcome = if parts.method != Method::POST {
        Err(Refusal {
            status: StatusCode::METHOD_NOT_ALLOWED,
            error: "invalid_request",
            reason: "not a POST",
            header: Some((header::ALLOW, HeaderValue::from_static("POST"))),
        })
    } else {
        match axum::body::to_bytes(body, MAX_BODY).await {
            Ok(bytes) => grant(&s, &parts.headers, &bytes).await,
            Err(_) => Err(Refusal {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                error: "invalid_request",
                reason: "the body is larger than 16 KiB or could not be read",
                header: None,
            }),
        }
    };
    let res = match outcome {
        Ok(pair) => Json(json!({
            "access_token": pair.access_token,
            "token_type": "Bearer",
            "expires_in": pair.expires_in,
            "refresh_token": pair.refresh_token,
            "scope": pair.scope,
        }))
        .into_response(),
        Err(refusal) => {
            if refusal.status != StatusCode::INTERNAL_SERVER_ERROR {
                tracing::warn!(
                    error = refusal.error,
                    reason = refusal.reason,
                    "token request refused"
                );
            }
            let mut res = (refusal.status, Json(json!({"error": refusal.error}))).into_response();
            if let Some((name, value)) = refusal.header {
                res.headers_mut().insert(name, value);
            }
            res
        }
    };
    no_store(res)
}

async fn grant(s: &AppState, headers: &HeaderMap, body: &[u8]) -> Result<TokenPair, Refusal> {
    // RFC 6749 section 5.2: a request that tries to authenticate with a header is answered 401, naming the
    // scheme it used.
    if let Some(sent) = headers.get(header::AUTHORIZATION) {
        let challenge = HeaderValue::from_str(&format!("{} realm=\"reeve\"", scheme_of(sent)))
            .unwrap_or(HeaderValue::from_static("Basic realm=\"reeve\""));
        return Err(Refusal {
            status: StatusCode::UNAUTHORIZED,
            error: "invalid_client",
            reason: "an Authorization header: this server accepts public clients only",
            header: Some((header::WWW_AUTHENTICATE, challenge)),
        });
    }
    let is_form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        });
    let params = std::str::from_utf8(body)
        .ok()
        .filter(|_| is_form)
        .and_then(|text| Params::parse(text).ok())
        .ok_or_else(|| bad("invalid_request", "the body is not a well-formed form"))?;
    // Every parameter this route knows, before any value is read. A repeated resource is asked on its own first,
    // as at authorize, so the answer does not depend on which repeated parameter `repeated` names first.
    if params.repeated(&["resource"]).is_some() {
        return Err(bad("invalid_target", "resource is sent more than once"));
    }
    if params.repeated(&RECOGNISED).is_some() {
        return Err(bad("invalid_request", "a parameter is sent more than once"));
    }
    if params.get("client_secret").is_some() {
        return Err(bad(
            "invalid_client",
            "a client_secret: this server accepts public clients only",
        ));
    }
    let need = |name: &'static str| {
        params
            .get(name)
            .ok_or_else(|| bad("invalid_request", "a required parameter is missing"))
    };
    let with = Issuing {
        pool: &s.pool,
        store: &*s.tokens,
        access: &*s.access,
        floor: &s.identity,
    };
    let now = s.clock.now();
    let outcome = match need("grant_type")? {
        "authorization_code" => {
            let request = ExchangeRequest {
                code: need("code")?,
                // Required here: the code was issued to a client, and a public client names itself (OAuth 2.1
                // section 4.1.3).
                client_id: need("client_id")?,
                redirect_uri: params.get("redirect_uri"),
                resource: params.get("resource"),
                verifier: need("code_verifier")?,
            };
            reeve_identity::exchange_code(with, request, &s.authorization_requests, now).await
        }
        "refresh_token" => {
            let request = RefreshRequest {
                refresh_token: need("refresh_token")?,
                client_id: params.get("client_id"),
                scope: params.get("scope"),
                // RFC 8707: checked when sent.
                resource_matches: params
                    .get("resource")
                    .is_none_or(|sent| sent == oauth::resource(&s.config)),
            };
            let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
            reeve_identity::refresh_tokens(with, &*log, request, now).await
        }
        _ => {
            return Err(bad(
                "unsupported_grant_type",
                "a grant type this server does not have",
            ));
        }
    };
    outcome.map_err(|e| match e {
        GrantError::InvalidGrant(reason) => bad("invalid_grant", reason),
        GrantError::InvalidTarget => bad("invalid_target", "the resource is not this endpoint"),
        GrantError::InvalidScope => bad("invalid_scope", "more than was granted"),
        GrantError::Db(e) => internal(e),
    })
}
