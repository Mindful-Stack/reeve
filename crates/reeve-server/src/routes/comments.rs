use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

/// Any item's comments by its key alone: the key says what it is.
#[utoipa::path(get, path = "/items/{key}/comments", params(("key" = String, Path)), responses((status = 200, body = Response<Vec<CommentDto>>)))]
async fn item_list(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<Vec<CommentDto>>>, AppError> {
    let (t, key) = s.handlers.resolve_any(&key).await?;
    Ok(ok(s.handlers.comments_for_item(t, &key).await?))
}
#[utoipa::path(post, path = "/items/{key}/comments", params(("key" = String, Path)), request_body = AddComment, responses((status = 200, body = Response<Vec<CommentDto>>)))]
async fn item_add(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<AddComment>,
) -> Result<Json<Response<Vec<CommentDto>>>, AppError> {
    let ((t, key), _) = crate::actions::add_item_comment(&s, &c.acting(), &key, b).await?;
    Ok(ok(s.handlers.comments_for_item(t, &key).await?))
}
#[utoipa::path(get, path = "/stories/{key}/comments", params(("key" = String, Path)), responses((status = 200, body = Response<Vec<CommentDto>>)))]
async fn story_list(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<Vec<CommentDto>>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    Ok(ok(s.handlers.comments_for_story(&key).await?))
}
#[utoipa::path(post, path = "/stories/{key}/comments", params(("key" = String, Path)), request_body = AddComment, responses((status = 200, body = Response<Vec<CommentDto>>)))]
async fn story_add(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<AddComment>,
) -> Result<Json<Response<Vec<CommentDto>>>, AppError> {
    // The key's space is the caller's to decide, never the body's (`Acting::write_key`).
    let idem = b
        .idempotency_key
        .as_deref()
        .map(|key| c.acting().write_key(key))
        .transpose()?;
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    s.handlers
        .comment_story_attaching(&c.ctx(), &key, &b.body, idem.as_deref(), &b.attachments)
        .await?;
    Ok(ok(s.handlers.comments_for_story(&key).await?))
}
#[utoipa::path(get, path = "/decisions/{id}/comments", params(("id" = i64, Path)), responses((status = 200, body = Response<Vec<CommentDto>>)))]
async fn decision_list(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<Vec<CommentDto>>>, AppError> {
    Ok(ok(s.handlers.comments_for_decision(id).await?))
}
#[utoipa::path(post, path = "/decisions/{id}/comments", params(("id" = i64, Path)), request_body = AddComment, responses((status = 200, body = Response<Vec<CommentDto>>)))]
async fn decision_add(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<AddComment>,
) -> Result<Json<Response<Vec<CommentDto>>>, AppError> {
    crate::actions::add_decision_comment(&s, &c.acting(), id, b).await?;
    Ok(ok(s.handlers.comments_for_decision(id).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(story_list, story_add))
        .routes(routes!(decision_list, decision_add))
        .routes(routes!(item_list, item_add))
}
