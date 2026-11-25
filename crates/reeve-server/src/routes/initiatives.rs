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
    c: Caller,
) -> Result<Json<Response<Vec<InitiativeDto>>>, AppError> {
    Ok(ok(crate::actions::list_initiatives(&s, &c.acting()).await?))
}
#[utoipa::path(post, path = "/initiatives", request_body = CreateInitiative, responses((status = 200, body = Response<InitiativeDto>)))]
async fn create(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<CreateInitiative>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    Ok(ok(crate::actions::create_initiative(&s, &c.acting(), b)
        .await?
        .0))
}
#[utoipa::path(get, path = "/initiatives/{key}", params(("key" = String, Path)), responses((status = 200, body = Response<InitiativeDto>)))]
async fn get(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    Ok(ok(
        crate::actions::get_initiative(&s, &c.acting(), &key).await?
    ))
}
#[utoipa::path(patch, path = "/initiatives/{key}", params(("key" = String, Path)), request_body = EditItem, responses((status = 200, body = Response<InitiativeDto>)))]
async fn edit(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<EditItem>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    // The initiative as written is the MCP receipt's; the route answers the read after the commit.
    let edited = crate::actions::edit_initiative(&s, &c.acting(), &key, b).await?;
    Ok(ok(edited.read))
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
    Ok(ok(s.handlers.initiative_dto(&key, c.ctx().now).await?))
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

#[utoipa::path(post, path = "/initiatives/{key}/urgent", params(("key" = String, Path)), request_body = MarkUrgent, responses((status = 200, body = Response<InitiativeDto>)))]
async fn mark_urgent(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<MarkUrgent>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Initiative).await?;
    let ctx = c.ctx();
    s.handlers
        .mark_urgent(&ctx, ItemType::Initiative, &key, b.reason)
        .await?;
    Ok(ok(s.handlers.initiative_dto(&key, ctx.now).await?))
}
#[utoipa::path(delete, path = "/initiatives/{key}/urgent", params(("key" = String, Path)), responses((status = 200, body = Response<InitiativeDto>)))]
async fn clear_urgent(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<InitiativeDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Initiative).await?;
    let ctx = c.ctx();
    s.handlers
        .clear_urgent(&ctx, ItemType::Initiative, &key)
        .await?;
    Ok(ok(s.handlers.initiative_dto(&key, ctx.now).await?))
}
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(mark_urgent, clear_urgent))
        .routes(routes!(list, create))
        .routes(routes!(get, edit))
        .routes(routes!(discovery))
        .routes(routes!(findings))
}
