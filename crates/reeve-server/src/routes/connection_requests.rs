//! `/connection-requests`: what the approval page (`/connect` in the web client) reads and answers
//! (kb/ADR-0029 R4, R11). The request itself is held by `reeve_identity::AuthorizationRequests`; these routes add
//! the browser's proof, the connect cookie. Any interactive API token of the person may read and answer (as a session
//! cookie, with the client header, or as a bearer header): the proofs are the token's kind, the connect cookie,
//! and the approval value bound to the token that read the request. A request that is unknown, expired, answered
//! or another browser's is one and the same 404. The routes' own middleware, `guard`, runs before any extractor:
//! it answers 404 when `[mcp]` is off, and marks every answer, an extractor's refusal and the router's 405
//! included, as one no cache may store.
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
    extract::{Request, State},
    http::HeaderMap,
    middleware::Next,
    response::{IntoResponse, Response as HttpResponse},
};
use axum_extra::extract::cookie::CookieJar;
use reeve_protocol::*;
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, router::UtoipaMethodRouter, routes};

/// The switch, then whatever the route answers, marked `no-store`. Before any extractor, so a switched-off
/// instance neither authenticates the caller nor reads the path or the body.
async fn guard(mcp_enabled: bool, req: Request, next: Next) -> HttpResponse {
    if !mcp_enabled {
        return no_store(AppError::NotFound.into_response());
    }
    no_store(next.run(req).await)
}

/// This browser's claim to the request `id`.
fn bound(s: &AppState, headers: &HeaderMap, id: &str) -> Result<(), AppError> {
    if connect_bound(headers, id, s.config.https()) {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

fn host_of(address: &str) -> String {
    url::Url::parse(address)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// The answer to an answered request: the connect cookie is cleared, and the page is told where to go.
fn answered(s: &AppState, jar: CookieJar, redirect_to: String) -> impl IntoResponse + use<> {
    (
        jar.add(removal(connect_cookie(String::new(), s.config.https()))),
        ok(ConnectionAnswerDto { redirect_to }),
    )
}

#[utoipa::path(get, path = "/connection-requests/{id}", params(("id" = String, Path)),
    responses((status = 200, body = Response<ConnectionRequestDto>), (status = 403, body = Response<Empty>, description = "an interactive token only")))]
async fn read(
    State(s): State<AppState>,
    c: Caller,
    headers: HeaderMap,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<Response<ConnectionRequestDto>>, AppError> {
    bound(&s, &headers, &id)?;
    let view = reeve_identity::read_request(&s.authorization_requests, &c.0, &id, c.1)?
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
    responses((status = 200, body = Response<ConnectionAnswerDto>), (status = 403, body = Response<Empty>, description = "an interactive token only")))]
async fn approve(
    State(s): State<AppState>,
    c: Caller,
    jar: CookieJar,
    headers: HeaderMap,
    ApiPath(id): ApiPath<String>,
    ApiJson(b): ApiJson<ApproveConnection>,
) -> Result<impl IntoResponse, AppError> {
    bound(&s, &headers, &id)?;
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let (back, code) = reeve_identity::approve_request(
        &s.pool,
        &s.authorization_requests,
        &*log,
        &c.0,
        &id,
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
    Ok(answered(&s, jar, redirect_to))
}

#[utoipa::path(post, path = "/connection-requests/{id}/refuse", params(("id" = String, Path)),
    request_body = RefuseConnection,
    responses((status = 200, body = Response<ConnectionAnswerDto>), (status = 403, body = Response<Empty>, description = "an interactive token only")))]
async fn refuse(
    State(s): State<AppState>,
    c: Caller,
    jar: CookieJar,
    headers: HeaderMap,
    ApiPath(id): ApiPath<String>,
    ApiJson(b): ApiJson<RefuseConnection>,
) -> Result<impl IntoResponse, AppError> {
    bound(&s, &headers, &id)?;
    let back =
        reeve_identity::refuse_request(&s.authorization_requests, &c.0, &id, &b.approval, c.1)?
            .ok_or(AppError::NotFound)?;
    let redirect_to = answer_address(
        &back.redirect_uri,
        &[("error", "access_denied")],
        back.state.as_deref(),
        &s.config.issuer(),
    );
    Ok(answered(&s, jar, redirect_to))
}

/// A route whose 405 is the API's envelope, set here rather than by `app`'s `method_not_allowed_fallback`: that
/// one replaces only a default fallback, which would then bypass `guard`.
fn with_405(
    (schemas, paths, method_router): UtoipaMethodRouter<AppState>,
) -> UtoipaMethodRouter<AppState> {
    (
        schemas,
        paths,
        method_router.fallback(|| async { AppError::MethodNotAllowed }),
    )
}

/// `mcp_enabled` is `[mcp] enabled`, read once when the app is built (the configuration does not change while
/// the server runs).
pub fn router(mcp_enabled: bool) -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(with_405(routes!(read)))
        .routes(with_405(routes!(approve)))
        .routes(with_405(routes!(refuse)))
        .layer(axum::middleware::from_fn(move |req, next| {
            guard(mcp_enabled, req, next)
        }))
}
