//! The public OAuth surface of the MCP endpoint (kb/ADR-0029 R2, R11): the routes that answer without a caller.
//! They are always mounted, outside the API router, so they are not in the OpenAPI document and never answer in
//! the API's envelope. The `[mcp]` switch is inside each handler: off, each answers 404. `[mcp] enabled` loads only
//! together with `[oidc]` (`ServerConfig::validate`), so an instance without sign-in answers 404 here as well.
//!
//! This file holds the two metadata documents. The authorization server's document names `/oauth/authorize`
//! (GRA-657) and `/oauth/token` (GRA-658); those routes are added to `router()` by those stories.
use crate::{config::ServerConfig, state::AppState};
use axum::{
    Json, Router,
    extract::State,
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};

/// RFC 9728 section 3.1: the well-known part goes between the host and the resource's path (`/mcp`).
pub(crate) const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
/// RFC 8414 section 3: the issuer has no path, so nothing follows the well-known part.
pub(crate) const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";
pub(crate) const AUTHORIZE_PATH: &str = "/oauth/authorize";
pub(crate) const TOKEN_PATH: &str = "/oauth/token";
/// The two scopes on the wire (kb/ADR-0029 R7). `offline_access` is accepted at authorize and never advertised.
pub(crate) const SCOPES: [&str; 2] = ["read", "write"];

/// The resource an MCP token is issued for: the endpoint's address, `<issuer>/mcp`. Clients send it as the
/// `resource` parameter and compare it with the document as a string.
pub(crate) fn resource(config: &ServerConfig) -> String {
    format!("{}/mcp", config.issuer())
}

/// The address of the protected resource's document: what the 401 from `/mcp` names (RFC 9728 section 5.1).
pub(crate) fn resource_metadata_url(config: &ServerConfig) -> String {
    format!("{}{PROTECTED_RESOURCE_PATH}", config.issuer())
}

fn protected_resource(config: &ServerConfig) -> Value {
    json!({
        "resource": resource(config),
        "authorization_servers": [config.issuer()],
        "scopes_supported": SCOPES,
        "bearer_methods_supported": ["header"],
    })
}

fn authorization_server(config: &ServerConfig) -> Value {
    let issuer = config.issuer();
    json!({
        "issuer": issuer.as_str(),
        "authorization_endpoint": format!("{issuer}{AUTHORIZE_PATH}"),
        "token_endpoint": format!("{issuer}{TOKEN_PATH}"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
        "scopes_supported": SCOPES,
    })
}

/// One metadata document. The switch comes first, so a switched-off instance says nothing about itself whatever
/// the method; then a read answers and anything else is 405 in plain text.
fn document(s: &AppState, method: &Method, build: fn(&ServerConfig) -> Value) -> Response {
    if !s.config.mcp.enabled {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    if *method != Method::GET && *method != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
            "this address answers GET only",
        )
            .into_response();
    }
    Json(build(&s.config)).into_response()
}

async fn protected_resource_metadata(State(s): State<AppState>, method: Method) -> Response {
    document(&s, &method, protected_resource)
}

async fn authorization_server_metadata(State(s): State<AppState>, method: Method) -> Response {
    document(&s, &method, authorization_server)
}

/// Merged into the application outside `api_router()`. `any` keeps every method away from the router's own 405,
/// which answers in the API's envelope.
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(PROTECTED_RESOURCE_PATH, any(protected_resource_metadata))
        .route(
            AUTHORIZATION_SERVER_PATH,
            any(authorization_server_metadata),
        )
}
