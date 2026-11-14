use crate::{
    actions::StoryQuery,
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

/// A story's owner must be an allowed user (allowed by the floor or a database entry): an owner nobody can sign
/// in as would hold work no one can pick up.
fn allowed_owner(
    policy: &reeve_identity::AccessPolicy,
    owner: Option<String>,
) -> Result<Option<String>, AppError> {
    match owner {
        Some(o) if !policy.is_allowed(&o) => Err(reeve_planning::HandlerError::Validation(
            format!("{} is not an allowed user", normalise_email(&o)),
        )
        .into()),
        other => Ok(other),
    }
}

#[utoipa::path(get, path = "/stories", params(StoryQuery), responses((status = 200, body = Response<Vec<StoryWithReadiness>>)))]
async fn list(
    State(s): State<AppState>,
    c: Caller,
    ApiQuery(q): ApiQuery<StoryQuery>,
) -> Result<Json<Response<Vec<StoryWithReadiness>>>, AppError> {
    Ok(ok(crate::actions::list_stories(&s, &c.acting(), q).await?))
}
#[utoipa::path(post, path = "/stories", request_body = CreateStory, responses((status = 200, body = Response<StoryDto>)))]
async fn create(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<CreateStory>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let st = s
        .handlers
        .create_story_attaching(
            &c.ctx(),
            &b.idempotency_key,
            &b.title,
            b.kind,
            super::resolve_opt(&s.handlers, b.epic.as_ref(), ItemType::Epic).await?,
            b.description.as_deref().unwrap_or(""),
            b.criteria,
            b.labels,
            b.external_ref,
            allowed_owner(c.policy(), b.owner)?,
            &b.attachments,
        )
        .await?;
    Ok(ok(s.handlers.story_dto(st.key(), c.ctx().now).await?))
}
#[utoipa::path(get, path = "/stories/{key}", params(("key" = String, Path)), responses((status = 200, body = Response<StoryDto>)))]
async fn get(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    Ok(ok(s.handlers.story_dto(&key, c.ctx().now).await?))
}
#[utoipa::path(patch, path = "/stories/{key}", params(("key" = String, Path)), request_body = EditItem, responses((status = 200, body = Response<StoryDto>)))]
async fn edit(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<EditItem>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let criteria = b.criteria.map(|v| {
        v.into_iter()
            .map(|c| reeve_planning::Criterion {
                text: c.text,
                done: c.done,
            })
            .collect()
    });
    let st = s
        .handlers
        .edit_story_attaching(
            &c.ctx(),
            &key,
            b.version,
            b.title,
            b.description,
            b.kind,
            super::resolve_edit(&s.handlers, b.epic, ItemType::Epic).await?,
            criteria,
            match b.owner {
                Some(Some(o)) => Some(allowed_owner(c.policy(), Some(o))?),
                other => other,
            },
            b.labels,
            b.rank,
            b.external_ref,
            &b.attachments,
        )
        .await?;
    Ok(ok(s.handlers.story_dto(st.key(), c.ctx().now).await?))
}
#[utoipa::path(post, path = "/stories/{key}/move", params(("key" = String, Path)), request_body = MoveStory, responses((status = 200, body = Response<StoryDto>), (status = 422, body = Response<StoryDto>)))]
async fn mv(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<MoveStory>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let st = s
        .handlers
        .move_story(
            &c.ctx(),
            &key,
            b.version,
            b.to,
            b.resolution,
            b.override_reason,
        )
        .await?;
    Ok(ok(s.handlers.story_dto(st.key(), c.ctx().now).await?))
}
/// Places a todo story before another todo story, or at the end of todo when `before` is absent: one write, one
/// `reranked` entry, and only this story's version changes.
#[utoipa::path(post, path = "/stories/{key}/rank", params(("key" = String, Path)), request_body = RankStory, responses((status = 200, body = Response<StoryDto>)))]
async fn rank(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<RankStory>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let st = s
        .handlers
        .rank_story(
            &c.ctx(),
            &key,
            b.version,
            super::resolve_opt(&s.handlers, b.before.as_ref(), ItemType::Story)
                .await?
                .as_ref(),
        )
        .await?;
    Ok(ok(s.handlers.story_dto(st.key(), c.ctx().now).await?))
}
#[utoipa::path(get, path = "/stories/{key}/ready", params(("key" = String, Path)), responses((status = 200, body = Response<Readiness>)))]
async fn ready(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<Readiness>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    Ok(ok(s.handlers.readiness(&key).await?))
}
#[utoipa::path(get, path = "/stories/{key}/context", params(("key" = String, Path)), responses((status = 200, body = Response<StoryContextDto>)))]
async fn context(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<StoryContextDto>>, AppError> {
    Ok(ok(
        crate::actions::story_context(&s, &c.acting(), &key).await?
    ))
}
#[utoipa::path(post, path = "/stories/{key}/blocks", params(("key" = String, Path)), request_body = BlockEdge, responses((status = 200, body = Response<Empty>)))]
async fn block(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<BlockEdge>,
) -> Result<Json<EmptyResponse>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let blocker = s
        .handlers
        .resolve_ref(b.blocker.as_str(), ItemType::Story)
        .await?;
    s.handlers.add_block(&c.ctx(), &key, &blocker).await?;
    Ok(ok(Empty {}))
}

/// Removes "`blocker` blocks `key`". Both segments resolve like every key input; removing an edge that does not exist
/// answers ok and changes nothing.
#[utoipa::path(delete, path = "/stories/{key}/blocks/{blocker}", params(("key" = String, Path), ("blocker" = String, Path)), responses((status = 200, body = Response<Empty>)))]
async fn unblock(
    State(s): State<AppState>,
    c: Caller,
    ApiPath((key, blocker)): ApiPath<(String, String)>,
) -> Result<Json<EmptyResponse>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let blocker = s.handlers.resolve_ref(&blocker, ItemType::Story).await?;
    s.handlers.remove_block(&c.ctx(), &key, &blocker).await?;
    Ok(ok(Empty {}))
}

/// Sets, changes or replaces the promise (kb/ADR-0025 R2): a date or a sprint shortcut, not both; neither changes only
/// the recipient. Interactive only; takes no version.
#[utoipa::path(post, path = "/stories/{key}/promise", params(("key" = String, Path)), request_body = SetPromise, responses((status = 200, body = Response<StoryDto>)))]
async fn set_promise(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<SetPromise>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let ctx = c.ctx();
    s.handlers
        .set_promise(&ctx, ItemType::Story, &key, b)
        .await?;
    Ok(ok(s.handlers.story_dto(&key, ctx.now).await?))
}
/// Clears the promise (R2); clearing none changes nothing. Interactive only. `cleared` says whether this request
/// cleared the story's own promise; the story is read after the write.
#[utoipa::path(delete, path = "/stories/{key}/promise", params(("key" = String, Path)), responses((status = 200, body = Response<StoryPromiseCleared>)))]
async fn clear_promise(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<StoryPromiseCleared>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let ctx = c.ctx();
    let cleared = s
        .handlers
        .clear_promise(&ctx, ItemType::Story, &key)
        .await?;
    Ok(ok(StoryPromiseCleared {
        cleared,
        story: s.handlers.story_dto(&key, ctx.now).await?,
    }))
}
#[utoipa::path(post, path = "/stories/{key}/urgent", params(("key" = String, Path)), request_body = MarkUrgent, responses((status = 200, body = Response<StoryDto>)))]
async fn mark_urgent(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<MarkUrgent>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let ctx = c.ctx();
    s.handlers
        .mark_urgent(&ctx, ItemType::Story, &key, b.reason)
        .await?;
    Ok(ok(s.handlers.story_dto(&key, ctx.now).await?))
}
#[utoipa::path(delete, path = "/stories/{key}/urgent", params(("key" = String, Path)), responses((status = 200, body = Response<StoryDto>)))]
async fn clear_urgent(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let ctx = c.ctx();
    s.handlers.clear_urgent(&ctx, ItemType::Story, &key).await?;
    Ok(ok(s.handlers.story_dto(&key, ctx.now).await?))
}
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SimilarQuery {
    /// The title being written, 8 to 200 characters.
    pub q: String,
    /// 1 to 10; default 5.
    pub limit: Option<i64>,
}

/// Open stories, and stories done within `[board] done_window_days`, whose title is similar to `q` (pg_trgm), most
/// similar first. A duplicate check: advice only.
#[utoipa::path(get, path = "/stories/similar", params(SimilarQuery), responses((status = 200, body = Response<Vec<SimilarStory>>)))]
async fn similar(
    State(s): State<AppState>,
    c: Caller,
    ApiQuery(q): ApiQuery<SimilarQuery>,
) -> Result<Json<Response<Vec<SimilarStory>>>, AppError> {
    Ok(ok(s
        .handlers
        .similar_stories(&q.q, q.limit, c.ctx().now)
        .await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(mark_urgent, clear_urgent))
        .routes(routes!(set_promise, clear_promise))
        .routes(routes!(list, create))
        .routes(routes!(similar))
        .routes(routes!(get, edit))
        .routes(routes!(mv))
        .routes(routes!(rank))
        .routes(routes!(ready))
        .routes(routes!(context))
        .routes(routes!(block))
        .routes(routes!(unblock))
}
