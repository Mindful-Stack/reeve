use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::ApiJson,
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

fn pairs(b: &WarRoomItems) -> Vec<(ItemType, Key)> {
    b.items.iter().map(|r| (r.item, r.key.clone())).collect()
}

#[utoipa::path(get, path = "/war-room", responses((status = 200, body = Response<Vec<WarRoomEntryDto>>)))]
async fn war_room_list(
    State(s): State<AppState>,
    c: Caller,
) -> Result<Json<Response<Vec<WarRoomEntryDto>>>, AppError> {
    let now = c.ctx().now;
    Ok(ok(s
        .handlers
        .war_room(now)
        .await?
        .iter()
        .map(Into::into)
        .collect()))
}
#[utoipa::path(post, path = "/war-room/clear", request_body = WarRoomItems, responses((status = 200, body = Response<Vec<WarRoomEntryDto>>)))]
async fn war_room_clear(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<WarRoomItems>,
) -> Result<Json<Response<Vec<WarRoomEntryDto>>>, AppError> {
    let ctx = c.ctx();
    s.handlers.war_room_clear(&ctx, &pairs(&b)).await?;
    Ok(ok(s
        .handlers
        .war_room(ctx.now)
        .await?
        .iter()
        .map(Into::into)
        .collect()))
}
#[utoipa::path(post, path = "/war-room/renew", request_body = WarRoomItems, responses((status = 200, body = Response<Vec<WarRoomEntryDto>>)))]
async fn war_room_renew(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<WarRoomItems>,
) -> Result<Json<Response<Vec<WarRoomEntryDto>>>, AppError> {
    let ctx = c.ctx();
    s.handlers.war_room_renew(&ctx, &pairs(&b)).await?;
    Ok(ok(s
        .handlers
        .war_room(ctx.now)
        .await?
        .iter()
        .map(Into::into)
        .collect()))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(war_room_list))
        .routes(routes!(war_room_clear))
        .routes(routes!(war_room_renew))
}
