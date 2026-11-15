//! `POST /oauth/register` (kb/ADR-0029 R3, R11; RFC 7591): an app that publishes no metadata document registers
//! itself and is given a client id. Without a caller. JSON of at most 16 KiB in; JSON out, never the API's
//! envelope; every answer says `Cache-Control: no-store`. Two switches come first, before anything is read:
//! `[mcp] enabled` and `[mcp] dynamic_registration`. What a request must say, what is stored and the log lines
//! are identity's (`reeve_identity::register_client`). No secret is issued: clients are public.
use super::oauth::no_store;
use crate::state::AppState;
use axum::{
    Json,
    extract::{Request, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use reeve_identity::RegistrationRefusal;
use serde_json::json;

/// The largest body read. A raw `Request` gets no default limit, so this is the only one.
const MAX_BODY: usize = 16 * 1024;
/// What a full store tells a caller to wait, in seconds. Room appears when a registration expires.
const RETRY_AFTER_SECONDS: &str = "3600";

/// RFC 7591 section 3.2.2: a JSON object with `error`.
fn refused(status: StatusCode, error: &'static str) -> Response {
    (status, Json(json!({"error": error}))).into_response()
}

/// `ANY /oauth/register`. Every answer is marked `no-store` here, the 404 and the 405 included.
pub(crate) async fn register(State(s): State<AppState>, req: Request) -> Response {
    no_store(answer(&s, req).await)
}

async fn answer(s: &AppState, req: Request) -> Response {
    // Before the method, the headers and the body: a switched-off instance says nothing and reads nothing.
    if !s.config.mcp.enabled || !s.config.mcp.dynamic_registration {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let (parts, body) = req.into_parts();
    if parts.method != Method::POST {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "POST")],
            "this address answers POST only",
        )
            .into_response();
    }
    // RFC 7591 section 3.1: "with a content type of "application/json"".
    let is_json = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        });
    if !is_json {
        tracing::warn!(
            reason = "the content type is not application/json",
            "mcp client registration refused"
        );
        return refused(StatusCode::BAD_REQUEST, "invalid_client_metadata");
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY).await else {
        tracing::warn!(
            reason = "the body is larger than 16 KiB or could not be read",
            "mcp client registration refused"
        );
        return refused(StatusCode::PAYLOAD_TOO_LARGE, "invalid_client_metadata");
    };
    let registered = reeve_identity::register_client(
        &s.pool,
        &*s.registered_clients,
        &s.config.client_policy(),
        &bytes,
        s.clock.now(),
    )
    .await;
    match registered {
        // RFC 7591 section 3.2.1: the id and every registered value, those this server set included, whatever
        // the app asked for. No `client_secret`, so no `client_secret_expires_at`.
        Ok(client) => (
            StatusCode::CREATED,
            Json(json!({
                "client_id": client.client_id,
                "client_id_issued_at": client.created_at.timestamp(),
                "client_name": client.client_name,
                "redirect_uris": client.redirect_uris,
                "token_endpoint_auth_method": "none",
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
            })),
        )
            .into_response(),
        // Each already logged by `register_client`.
        Err(RegistrationRefusal::InvalidRedirectUri { .. }) => {
            refused(StatusCode::BAD_REQUEST, "invalid_redirect_uri")
        }
        Err(RegistrationRefusal::InvalidMetadata(_)) => {
            refused(StatusCode::BAD_REQUEST, "invalid_client_metadata")
        }
        // The route has no address to send a browser back to, so the store's refusal is an answer of its own:
        // RFC 6749's name for it (RFC 7591 defines none).
        Err(RegistrationRefusal::Full) => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, RETRY_AFTER_SECONDS)],
            Json(json!({"error": "temporarily_unavailable"})),
        )
            .into_response(),
        Err(RegistrationRefusal::Db(_)) => {
            refused(StatusCode::INTERNAL_SERVER_ERROR, "server_error")
        }
    }
}
