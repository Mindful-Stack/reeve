//! Sprints and goals (kb/ADR-0014). Every route takes any caller: planning decides from `Ctx.via` which writes need an
//! interactive token (kb/ADR-0014 R5), so automation is refused `interactive_only` there, not here.
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_planning::{Handlers, goal_dto, sprint_dto, sprint_view_dto};
use reeve_protocol::*;
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

type GoalAnswer = Json<Response<GoalDto>>;

/// The goal as it is now, with its links. Write routes answer through this: a fresh read-only snapshot taken after
/// their write committed, not the write's own transaction, so a concurrent write that commits in between can make the
/// answer show a later version than this write produced.
async fn goal_now(h: &Handlers, id: i64) -> Result<GoalDto, AppError> {
    let (g, links) = h.goal(id).await?;
    Ok(goal_dto(&g, &links))
}

/// The sprint as it is now, with its goals and warnings. Like `goal_now`, a fresh snapshot after the write committed,
/// so a concurrent write can make the answer show a later version.
async fn sprint_now(h: &Handlers, id: i64) -> Result<SprintDto, AppError> {
    let (s, goals, links) = h.sprint_with_goals(id).await?;
    Ok(sprint_dto(&s, &goals, &links))
}

/// The active sprint with its numbers, the draft, and the closed sprints newest first, from one snapshot.
#[utoipa::path(get, path = "/sprints", responses((status = 200, body = Response<SprintsDto>)))]
async fn list(
    State(s): State<AppState>,
    c: Caller,
) -> Result<Json<Response<SprintsDto>>, AppError> {
    let now = c.ctx().now;
    let x = s.handlers.sprints(now).await?;
    let line = s.handlers.settings.upkeep_line;
    Ok(ok(SprintsDto {
        today: Some(now.date_naive()),
        active: x.active.as_ref().map(|a| {
            sprint_view_dto(
                a,
                &x.goals,
                &x.links,
                x.active_summary.as_ref(),
                &x.promises,
                line,
            )
        }),
        draft: x.draft.as_ref().map(|d| sprint_dto(d, &x.goals, &x.links)),
        closed: x
            .closed
            .iter()
            .map(|c| sprint_dto(c, &x.goals, &x.links))
            .collect(),
    }))
}

/// One sprint with its numbers, derived on read over its window (a draft has none: zero counts). A closed sprint's
/// counts at close are in its `closed` entry: `GET /activity?entity_type=sprint&entity_key=<id>&action=closed`.
#[utoipa::path(get, path = "/sprints/{id}", params(("id" = i64, Path)), responses((status = 200, body = Response<SprintViewDto>)))]
async fn get(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<SprintViewDto>>, AppError> {
    let v = s.handlers.sprint_view(id, c.ctx().now).await?;
    Ok(ok(sprint_view_dto(
        &v.sprint,
        &v.goals,
        &v.links,
        v.summary.as_ref(),
        &v.promises,
        s.handlers.settings.upkeep_line,
    )))
}

/// Opens the draft (interactive only) and creates the next draft; the answer carries the goal-count warnings.
#[utoipa::path(post, path = "/sprints/{id}/open", params(("id" = i64, Path)), request_body = OpenSprint, responses((status = 200, body = Response<SprintDto>)))]
async fn open(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<OpenSprint>,
) -> Result<Json<Response<SprintDto>>, AppError> {
    s.handlers
        .open_sprint(&c.ctx(), id, b.version, b.ends_on)
        .await?;
    Ok(ok(sprint_now(&s.handlers, id).await?))
}

/// Closes the active sprint with a verdict per goal (interactive only); refused `verdict_missing` naming the goals
/// without one.
#[utoipa::path(post, path = "/sprints/{id}/close", params(("id" = i64, Path)), request_body = CloseSprint, responses((status = 200, body = Response<SprintDto>)))]
async fn close(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<CloseSprint>,
) -> Result<Json<Response<SprintDto>>, AppError> {
    let verdicts = b
        .verdicts
        .into_iter()
        .map(|v| (v.goal_id, v.verdict, v.note))
        .collect();
    s.handlers
        .close_sprint(&c.ctx(), id, b.version, verdicts)
        .await?;
    Ok(ok(sprint_now(&s.handlers, id).await?))
}

/// Adds a goal to the draft, or to the active sprint with `active` (interactive only).
#[utoipa::path(post, path = "/goals", request_body = CreateGoal, responses((status = 200, body = Response<CreatedGoalDto>)))]
async fn create_goal(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<CreateGoal>,
) -> Result<Json<Response<CreatedGoalDto>>, AppError> {
    let (g, warnings) = s
        .handlers
        .add_goal(
            &c.ctx(),
            &b.idempotency_key,
            &b.outcome,
            &b.check,
            b.owner,
            b.notes.as_deref().unwrap_or(""),
            b.active,
        )
        .await?;
    Ok(ok(CreatedGoalDto {
        goal: goal_now(&s.handlers, g.id).await?,
        warnings,
    }))
}

#[utoipa::path(patch, path = "/goals/{id}", params(("id" = i64, Path)), request_body = EditGoal, responses((status = 200, body = Response<GoalDto>)))]
async fn edit_goal(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<EditGoal>,
) -> Result<GoalAnswer, AppError> {
    s.handlers
        .edit_goal(
            &c.ctx(),
            id,
            b.version,
            b.outcome,
            b.check,
            b.owner,
            b.notes,
        )
        .await?;
    Ok(ok(goal_now(&s.handlers, id).await?))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GoalVersion {
    pub version: i64,
}

/// Removes a draft's goal and its links (interactive only).
#[utoipa::path(delete, path = "/goals/{id}", params(("id" = i64, Path), GoalVersion), responses((status = 200, body = Response<Empty>)))]
async fn remove_goal(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiQuery(q): ApiQuery<GoalVersion>,
) -> Result<Json<EmptyResponse>, AppError> {
    s.handlers.remove_goal(&c.ctx(), id, q.version).await?;
    Ok(ok(Empty {}))
}

/// Records a person's confidence (interactive only); the latest wins and it clears any suggestion.
#[utoipa::path(post, path = "/goals/{id}/confidence", params(("id" = i64, Path)), request_body = RecordConfidence, responses((status = 200, body = Response<GoalDto>)))]
async fn confidence(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<RecordConfidence>,
) -> Result<GoalAnswer, AppError> {
    let g = s
        .handlers
        .record_confidence(&c.ctx(), id, b.confidence, b.note)
        .await?;
    Ok(ok(goal_now(&s.handlers, g.id).await?))
}

/// Suggests a confidence for a person to record or discard; any token kind.
#[utoipa::path(post, path = "/goals/{id}/suggestion", params(("id" = i64, Path)), request_body = SuggestConfidence, responses((status = 200, body = Response<GoalDto>)))]
async fn suggestion(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<SuggestConfidence>,
) -> Result<GoalAnswer, AppError> {
    let g = s
        .handlers
        .suggest_confidence(&c.ctx(), id, b.confidence, b.note)
        .await?;
    Ok(ok(goal_now(&s.handlers, g.id).await?))
}

/// Links items of any level to a goal; any token kind. Each reference resolves like any untyped item reference
/// (`Handlers::resolve_any`); one that names no item is 404 and links nothing.
#[utoipa::path(post, path = "/goals/{id}/links", params(("id" = i64, Path)), request_body = LinkGoal, responses((status = 200, body = Response<GoalDto>)))]
async fn link(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<LinkGoal>,
) -> Result<GoalAnswer, AppError> {
    let mut keys = Vec::with_capacity(b.items.len());
    for r in &b.items {
        keys.push(s.handlers.resolve_any(r.as_str()).await?.1);
    }
    s.handlers.link_goal(&c.ctx(), id, &keys).await?;
    Ok(ok(goal_now(&s.handlers, id).await?))
}

/// Unlinks an item from a goal; any token kind. No such link answers ok and changes nothing.
#[utoipa::path(delete, path = "/goals/{id}/links/{key}", params(("id" = i64, Path), ("key" = String, Path)), responses((status = 200, body = Response<GoalDto>)))]
async fn unlink(
    State(s): State<AppState>,
    c: Caller,
    ApiPath((id, key)): ApiPath<(i64, String)>,
) -> Result<GoalAnswer, AppError> {
    let (_, key) = s.handlers.resolve_any(&key).await?;
    s.handlers.unlink_goal(&c.ctx(), id, &key).await?;
    Ok(ok(goal_now(&s.handlers, id).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(get))
        .routes(routes!(open))
        .routes(routes!(close))
        .routes(routes!(create_goal))
        .routes(routes!(edit_goal, remove_goal))
        .routes(routes!(confidence))
        .routes(routes!(suggestion))
        .routes(routes!(link))
        .routes(routes!(unlink))
}
