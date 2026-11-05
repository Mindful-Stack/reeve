use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

#[utoipa::path(get, path = "/initiatives", responses((status = 200, body = Response<Vec<InitiativeDto>>)))]
async fn list(
    State(s): State<AppState>,
    _c: Caller,
) -> Result<Json<Response<Vec<InitiativeDto>>>, AppError> {
    Ok(ok(s.handlers.list_initiatives().await?))
}
#[utoipa::path(post, path = "/initiatives", request_body = CreateInitiative, responses((status = 200, body = Response<InitiativeDto>)))]
async fn create(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<CreateInitiative>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    let i = s
        .handlers
        .create_initiative(
            &c.ctx(),
            &b.idempotency_key,
            &b.title,
            b.brief.as_deref().unwrap_or(""),
            &b.horizon,
            b.target_month,
            b.owner,
            b.external_ref,
        )
        .await?;
    Ok(ok(s.handlers.initiative_dto(i.key()).await?))
}
#[utoipa::path(get, path = "/initiatives/{key}", params(("key" = String, Path)), responses((status = 200, body = Response<InitiativeDto>)))]
async fn get(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Initiative).await?;
    Ok(ok(s.handlers.initiative_dto(&key).await?))
}
#[utoipa::path(patch, path = "/initiatives/{key}", params(("key" = String, Path)), request_body = EditItem, responses((status = 200, body = Response<InitiativeDto>)))]
async fn edit(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<EditItem>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Initiative).await?;
    s.handlers
        .edit_initiative(
            &c.ctx(),
            &key,
            b.version,
            b.title,
            b.brief,
            b.horizon,
            b.target_month,
            b.owner,
            b.external_ref,
        )
        .await?;
    Ok(ok(s.handlers.initiative_dto(&key).await?))
}
#[utoipa::path(post, path = "/initiatives/{key}/discovery", params(("key" = String, Path)), request_body = DiscoveryMark, responses((status = 200, body = Response<InitiativeDto>)))]
async fn discovery(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<DiscoveryMark>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Initiative).await?;
    s.handlers
        .mark_discovery(&c.ctx(), ItemType::Initiative, &key, b.version, b.complete)
        .await?;
    Ok(ok(s.handlers.initiative_dto(&key).await?))
}

/// Advisory findings: computed on read, never stored, never consulted by a move.
#[utoipa::path(get, path = "/initiatives/{key}/findings", params(("key" = String, Path)), responses((status = 200, body = Response<Vec<Finding>>)))]
async fn findings(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<Vec<Finding>>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Initiative).await?;
    Ok(ok(s.handlers.findings(ItemType::Initiative, &key).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get, edit))
        .routes(routes!(discovery))
        .routes(routes!(findings))
}
