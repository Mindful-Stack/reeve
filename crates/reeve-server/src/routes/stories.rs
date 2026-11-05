use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_planning::{GoalFilter, StoryFilter};
use reeve_protocol::*;
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

/// A story's owner must be an allowed user (`[identity]` allowed domains and emails): an owner nobody can sign
/// in as would hold work no one can pick up.
fn allowed_owner(s: &AppState, owner: Option<String>) -> Result<Option<String>, AppError> {
    match owner {
        Some(o) if !s.identity.is_allowed(&o) => Err(reeve_planning::HandlerError::Validation(
            format!("{} is not an allowed user", normalise_email(&o)),
        )
        .into()),
        other => Ok(other),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StoryQuery {
    /// An epic key or external reference. A reference that matches nothing yields an empty list, not 404.
    pub epic: Option<KeyRef>,
    pub owner: Option<String>,
    pub delivery: Option<DeliveryState>,
    #[serde(default)]
    pub untriaged: bool,
    pub label: Option<String>,
    #[serde(default)]
    pub ready: bool,
    /// `recent`: done stories only when they entered done within `[board] done_window_days`; default `all`.
    #[serde(default)]
    pub done: DoneFilter,
    /// Stories serving the active sprint's goals: a goal id, `any` (serving some goal) or `none` (upkeep). Combines
    /// with every other filter.
    pub goal: Option<String>,
}

/// `goal=<id>|any|none`.
fn goal_filter(raw: Option<&str>) -> Result<Option<GoalFilter>, AppError> {
    Ok(match raw {
        None => None,
        Some("any") => Some(GoalFilter::Any),
        Some("none") => Some(GoalFilter::None),
        Some(id) => Some(GoalFilter::Id(id.parse().map_err(|_| {
            reeve_planning::HandlerError::Validation("goal must be any, none or a goal id".into())
        })?)),
    })
}

#[utoipa::path(get, path = "/stories", params(StoryQuery), responses((status = 200, body = Response<Vec<StoryWithReadiness>>)))]
async fn list(
    State(s): State<AppState>,
    c: Caller,
    ApiQuery(q): ApiQuery<StoryQuery>,
) -> Result<Json<Response<Vec<StoryWithReadiness>>>, AppError> {
    let goal = goal_filter(q.goal.as_deref())?;
    let epic = match super::resolve_filter(&s.handlers, q.epic.as_ref(), ItemType::Epic).await? {
        super::FilterRef::Nothing => return Ok(ok(Vec::new())),
        super::FilterRef::Is(k) => Some(k),
        super::FilterRef::Unset => None,
    };
    let f = StoryFilter {
        epic,
        owner: q.owner,
        delivery: q.delivery,
        untriaged: q.untriaged,
        label: q.label,
        goal,
    };
    Ok(ok(s
        .handlers
        .list_stories(&f, q.ready, q.done, c.ctx().now)
        .await?))
}
#[utoipa::path(post, path = "/stories", request_body = CreateStory, responses((status = 200, body = Response<StoryDto>)))]
async fn create(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<CreateStory>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let st = s
        .handlers
        .create_story(
            &c.ctx(),
            &b.idempotency_key,
            &b.title,
            b.kind,
            super::resolve_opt(&s.handlers, b.epic.as_ref(), ItemType::Epic).await?,
            b.description.as_deref().unwrap_or(""),
            b.criteria,
            b.labels,
            b.external_ref,
            allowed_owner(&s, b.owner)?,
        )
        .await?;
    Ok(ok(s.handlers.story_dto(&st).await?))
}
#[utoipa::path(get, path = "/stories/{key}", params(("key" = String, Path)), responses((status = 200, body = Response<StoryDto>)))]
async fn get(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<StoryDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    let st = s.handlers.get_story(&key).await?;
    Ok(ok(s.handlers.story_dto(&st).await?))
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
        .edit_story(
            &c.ctx(),
            &key,
            b.version,
            b.title,
            b.description,
            b.kind,
            super::resolve_edit(&s.handlers, b.epic, ItemType::Epic).await?,
            criteria,
            match b.owner {
                Some(Some(o)) => Some(allowed_owner(&s, Some(o))?),
                other => other,
            },
            b.labels,
            b.rank,
            b.external_ref,
        )
        .await?;
    Ok(ok(s.handlers.story_dto(&st).await?))
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
    Ok(ok(s.handlers.story_dto(&st).await?))
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
    Ok(ok(s.handlers.story_dto(&st).await?))
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
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<StoryContextDto>>, AppError> {
    let key = s.handlers.resolve_ref(&key, ItemType::Story).await?;
    Ok(ok(s.handlers.story_context(&key).await?))
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

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get, edit))
        .routes(routes!(mv))
        .routes(routes!(rank))
        .routes(routes!(ready))
        .routes(routes!(context))
        .routes(routes!(block))
        .routes(routes!(unblock))
}
