//! `POST /api/v1/import/*`: the Jira import. Always routed and documented, so the OpenAPI document is
//! the same on every instance, but each handler answers 404 unless `[import] enabled = true` (checked before the
//! caller's role, so a switched-off instance reveals nothing), and only an admin with an interactive token may import. The body is parsed
//! only after that gate, so a malformed body on a switched-off instance is still 404.
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::ApiJson,
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

fn gate(s: &AppState, c: &Caller) -> Result<(), AppError> {
    if !s.config.import.enabled {
        return Err(AppError::NotFound);
    }
    if c.0.role != Role::Admin || c.0.token_kind != TokenKind::Interactive {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

#[utoipa::path(post, path = "/import/initiatives", operation_id = "import_initiatives", request_body = ImportInitiative, responses((status = 200, body = Response<Imported>), (status = 403, description = "not an admin with an interactive token")))]
async fn initiatives(
    State(s): State<AppState>,
    c: Caller,
    b: Result<ApiJson<ImportInitiative>, AppError>,
) -> Result<Json<Response<Imported>>, AppError> {
    gate(&s, &c)?;
    let ApiJson(b) = b?;
    Ok(ok(s.handlers.import_initiative(&c.ctx(), b).await?))
}

#[utoipa::path(post, path = "/import/epics", operation_id = "import_epics", request_body = ImportEpic, responses((status = 200, body = Response<Imported>), (status = 403, description = "not an admin with an interactive token")))]
async fn epics(
    State(s): State<AppState>,
    c: Caller,
    b: Result<ApiJson<ImportEpic>, AppError>,
) -> Result<Json<Response<Imported>>, AppError> {
    gate(&s, &c)?;
    let ApiJson(b) = b?;
    Ok(ok(s.handlers.import_epic(&c.ctx(), b).await?))
}

#[utoipa::path(post, path = "/import/stories", operation_id = "import_stories", request_body = ImportStory, responses((status = 200, body = Response<Imported>), (status = 403, description = "not an admin with an interactive token")))]
async fn stories(
    State(s): State<AppState>,
    c: Caller,
    b: Result<ApiJson<ImportStory>, AppError>,
) -> Result<Json<Response<Imported>>, AppError> {
    gate(&s, &c)?;
    let ApiJson(b) = b?;
    Ok(ok(s.handlers.import_story(&c.ctx(), b).await?))
}

#[utoipa::path(post, path = "/import/comments", operation_id = "import_comments", request_body = ImportComment, responses((status = 200, body = Response<Imported>), (status = 403, description = "not an admin with an interactive token")))]
async fn comments(
    State(s): State<AppState>,
    c: Caller,
    b: Result<ApiJson<ImportComment>, AppError>,
) -> Result<Json<Response<Imported>>, AppError> {
    gate(&s, &c)?;
    let ApiJson(b) = b?;
    Ok(ok(s.handlers.import_comment(&c.ctx(), b).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(initiatives))
        .routes(routes!(epics))
        .routes(routes!(stories))
        .routes(routes!(comments))
}
