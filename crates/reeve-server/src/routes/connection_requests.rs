//! `/connection-requests`: what the approval page (`/connect` in the web client) reads and answers
//! (kb/ADR-0029 R4, R11). The request itself is held by `reeve_identity::AuthorizationRequests`; these routes add
//! the browser's proof, the connect cookie. Any interactive API token of the person may answer (as a session
//! cookie, with the client header, or as a bearer header): the proofs are the token's kind, the connect cookie,
//! and the approval value bound to the token that read the request. A request that is unknown, expired, answered
//! or another browser's is one and the same 404. Each handler checks the `[mcp]` switch before the caller and
//! parses its body after both, and no answer it produces, a failure included, may be stored by a cache: each
//! handler is one line that marks whatever its `*_request` function returns.
use crate::{
    auth::{
        Caller,
        authorize::{connect_bound, connect_cookie},
        oauth::{answer_address, no_store},
        routes::removal,
    },
    error::{AppError, ok},
    extract::{ApiJson, ApiPath},
    state::AppState,
};
use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response as HttpResponse},
};
use axum_extra::extract::cookie::CookieJar;
use reeve_protocol::*;
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

/// An answered request: the connect cookie cleared, and where the page sends the browser.
type Answered = (CookieJar, Json<Response<ConnectionAnswerDto>>);

/// The switch, then the caller, then this browser's claim to the request `id`.
fn gate(
    s: &AppState,
    c: Result<Caller, AppError>,
    headers: &HeaderMap,
    id: &str,
) -> Result<Caller, AppError> {
    if !s.config.mcp.enabled {
        return Err(AppError::NotFound);
    }
    let c = c?;
    if !connect_bound(headers, id, s.config.https()) {
        return Err(AppError::NotFound);
    }
    Ok(c)
}

fn host_of(address: &str) -> String {
    url::Url::parse(address)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// The answer to an answered request: the connect cookie is cleared, and the page is told where to go.
fn answered(s: &AppState, jar: CookieJar, redirect_to: String) -> Answered {
    (
        jar.add(removal(connect_cookie(String::new(), s.config.https()))),
        ok(ConnectionAnswerDto { redirect_to }),
    )
}

#[utoipa::path(get, path = "/connection-requests/{id}", params(("id" = String, Path)),
    responses((status = 200, body = Response<ConnectionRequestDto>)))]
async fn read(
    State(s): State<AppState>,
    c: Result<Caller, AppError>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<String>,
) -> HttpResponse {
    no_store(read_request(&s, c, &headers, &id).into_response())
}

fn read_request(
    s: &AppState,
    c: Result<Caller, AppError>,
    headers: &HeaderMap,
    id: &str,
) -> Result<Json<Response<ConnectionRequestDto>>, AppError> {
    let c = gate(s, c, headers, id)?;
    let view = s
        .authorization_requests
        .read(id, c.0.token_id, c.1)
        .ok_or(AppError::NotFound)?;
    Ok(ok(ConnectionRequestDto {
        client_host: view.client_host,
        client_name: view.client_name,
        redirect_host: host_of(&view.redirect_uri),
        email: c.0.email.clone(),
        requested_at: view.requested_at,
        approval: view.approval,
    }))
}

#[utoipa::path(post, path = "/connection-requests/{id}/approve", params(("id" = String, Path)),
    request_body = ApproveConnection,
    responses((status = 200, body = Response<ConnectionAnswerDto>), (status = 403, description = "an interactive token only")))]
async fn approve(
    State(s): State<AppState>,
    c: Result<Caller, AppError>,
    jar: CookieJar,
    headers: HeaderMap,
    ApiPath(id): ApiPath<String>,
    b: Result<ApiJson<ApproveConnection>, AppError>,
) -> HttpResponse {
    no_store(
        approve_request(&s, c, jar, &headers, &id, b)
            .await
            .into_response(),
    )
}

async fn approve_request(
    s: &AppState,
    c: Result<Caller, AppError>,
    jar: CookieJar,
    headers: &HeaderMap,
    id: &str,
    b: Result<ApiJson<ApproveConnection>, AppError>,
) -> Result<Answered, AppError> {
    let c = gate(s, c, headers, id)?;
    let ApiJson(b) = b?;
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let (back, code) = reeve_identity::approve_request(
        &s.pool,
        &s.authorization_requests,
        &*log,
        &c.0,
        id,
        &b.approval,
        b.scope,
        c.1,
    )
    .await?
    .ok_or(AppError::NotFound)?;
    let redirect_to = answer_address(
        &back.redirect_uri,
        &[("code", &code)],
        back.state.as_deref(),
        &s.config.issuer(),
    );
    Ok(answered(s, jar, redirect_to))
}

#[utoipa::path(post, path = "/connection-requests/{id}/refuse", params(("id" = String, Path)),
    request_body = RefuseConnection,
    responses((status = 200, body = Response<ConnectionAnswerDto>), (status = 403, description = "an interactive token only")))]
async fn refuse(
    State(s): State<AppState>,
    c: Result<Caller, AppError>,
    jar: CookieJar,
    headers: HeaderMap,
    ApiPath(id): ApiPath<String>,
    b: Result<ApiJson<RefuseConnection>, AppError>,
) -> HttpResponse {
    no_store(refuse_request(&s, c, jar, &headers, &id, b).into_response())
}

fn refuse_request(
    s: &AppState,
    c: Result<Caller, AppError>,
    jar: CookieJar,
    headers: &HeaderMap,
    id: &str,
    b: Result<ApiJson<RefuseConnection>, AppError>,
) -> Result<Answered, AppError> {
    let c = gate(s, c, headers, id)?;
    let ApiJson(b) = b?;
    let back =
        reeve_identity::refuse_request(&s.authorization_requests, &c.0, id, &b.approval, c.1)?
            .ok_or(AppError::NotFound)?;
    let redirect_to = answer_address(
        &back.redirect_uri,
        &[("error", "access_denied")],
        back.state.as_deref(),
        &s.config.issuer(),
    );
    Ok(answered(s, jar, redirect_to))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(read))
        .routes(routes!(approve))
        .routes(routes!(refuse))
}
