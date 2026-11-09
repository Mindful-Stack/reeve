use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EpicQuery {
    /// An initiative key or external reference. A reference that matches nothing yields an empty list, not 404.
    pub initiative: Option<KeyRef>,
}

#[utoipa::path(get, path = "/epics", params(EpicQuery), responses((status = 200, body = Response<Vec<EpicDto>>)))]
async fn list(
    State(s): State<AppState>,
    _c: Caller,
    ApiQuery(q): ApiQuery<EpicQuery>,
) -> Result<Json<Response<Vec<EpicDto>>>, AppError> {
    let initiative =
        match super::resolve_filter(&s.handlers, q.initiative.as_ref(), ItemType::Initiative)
            .await?
        {
            super::FilterRef::Nothing => return Ok(ok(Vec::new())),
            super::FilterRef::Is(k) => Some(k),
            super::FilterRef::Unset => None,
        };
    let epics = s.handlers.list_epics(initiative.as_ref()).await?;
    Ok(ok(epics.iter().map(EpicDto::from).collect()))
}
#[utoipa::path(post, path = "/epics", request_body = CreateEpic, responses((status = 200, body = Response<EpicDto>)))]
async fn create(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<CreateEpic>,
) -> Result<Json<Response<EpicDto>>, AppError> {
    let e = s
        .handlers
        .create_epic(
            &c.ctx(),
            &b.idempotency_key,
            &b.title,
            super::resolve_opt(&s.handlers, b.initiative.as_ref(), ItemType::Initiative).await?,
            b.brief.as_deref().unwrap_or(""),
            b.external_ref,
        )
        .await?;
    Ok(ok(EpicDto::from(&e)))
}
#[utoipa::path(get, path = "/epics/{key}", params(("key" = String, Path)), responses((status = 200, body = Response<EpicDto>)))]
async fn get(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<EpicDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Epic).await?;
    Ok(ok(EpicDto::from(&s.handlers.get_epic(&key).await?)))
}
#[utoipa::path(patch, path = "/epics/{key}", params(("key" = String, Path)), request_body = EditItem, responses((status = 200, body = Response<EpicDto>)))]
async fn edit(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<EditItem>,
) -> Result<Json<Response<EpicDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Epic).await?;
    let e = s
        .handlers
        .edit_epic(
            &c.ctx(),
            &key,
            b.version,
            b.title,
            b.brief,
            super::resolve_edit(&s.handlers, b.initiative, ItemType::Initiative).await?,
            b.external_ref,
        )
        .await?;
    Ok(ok(EpicDto::from(&e)))
}
#[utoipa::path(post, path = "/epics/{key}/discovery", params(("key" = String, Path)), request_body = DiscoveryMark, responses((status = 200, body = Response<EpicDto>)))]
async fn discovery(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<DiscoveryMark>,
) -> Result<Json<Response<EpicDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Epic).await?;
    s.handlers
        .mark_discovery(&c.ctx(), ItemType::Epic, &key, b.version, b.complete)
        .await?;
    Ok(ok(EpicDto::from(&s.handlers.get_epic(&key).await?)))
}

/// Advisory findings: computed on read, never stored, never consulted by a move.
#[utoipa::path(get, path = "/epics/{key}/findings", params(("key" = String, Path)), responses((status = 200, body = Response<Vec<Finding>>)))]
async fn findings(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<Vec<Finding>>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Epic).await?;
    Ok(ok(s.handlers.findings(ItemType::Epic, &key).await?))
}

#[utoipa::path(post, path = "/epics/{key}/urgent", params(("key" = String, Path)), request_body = MarkUrgent, responses((status = 200, body = Response<EpicDto>)))]
async fn mark_urgent(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<MarkUrgent>,
) -> Result<Json<Response<EpicDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Epic).await?;
    let ctx = c.ctx();
    s.handlers
        .mark_urgent(&ctx, ItemType::Epic, &key, b.reason)
        .await?;
    Ok(ok(EpicDto::from(&s.handlers.get_epic(&key).await?)))
}
#[utoipa::path(delete, path = "/epics/{key}/urgent", params(("key" = String, Path)), responses((status = 200, body = Response<EpicDto>)))]
async fn clear_urgent(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<EpicDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Epic).await?;
    let ctx = c.ctx();
    s.handlers.clear_urgent(&ctx, ItemType::Epic, &key).await?;
    Ok(ok(EpicDto::from(&s.handlers.get_epic(&key).await?)))
}
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(mark_urgent, clear_urgent))
        .routes(routes!(list, create))
        .routes(routes!(get, edit))
        .routes(routes!(discovery))
        .routes(routes!(findings))
}
