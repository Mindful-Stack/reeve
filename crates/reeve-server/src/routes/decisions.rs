use crate::{
    actions::{DecisionQuery, decision_dto, decision_scope, resolve_on},
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_planning::QueueEntry;
use reeve_protocol::*;
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

type Dec = Json<Response<RequiredDecisionDto>>;

fn entry_dto(s: &AppState, e: &QueueEntry) -> DecisionQueueEntryDto {
    let mut dto = DecisionQueueEntryDto::from(e);
    crate::actions::browse(s, &mut dto.decision);
    dto
}

#[utoipa::path(get, path = "/decisions", params(DecisionQuery), responses((status = 200, body = Response<Vec<RequiredDecisionDto>>)))]
async fn list(
    State(s): State<AppState>,
    c: Caller,
    ApiQuery(q): ApiQuery<DecisionQuery>,
) -> Result<Json<Response<Vec<RequiredDecisionDto>>>, AppError> {
    Ok(ok(
        crate::actions::list_decisions(&s, &c.acting(), &q).await?
    ))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DecisionQueueQuery {
    /// Only decisions on this item (closed ones included). A reference that matches nothing yields an empty queue.
    pub on: Option<KeyRef>,
    /// Every decision under this initiative (closed ones included). Not with `on`.
    pub initiative: Option<KeyRef>,
    /// Unscoped: also list closed decisions.
    #[serde(default)]
    pub include_closed: bool,
}

#[utoipa::path(get, path = "/decisions/queue", params(DecisionQueueQuery), responses((status = 200, body = Response<DecisionQueueDto>)))]
async fn queue(
    State(s): State<AppState>,
    c: Caller,
    ApiQuery(q): ApiQuery<DecisionQueueQuery>,
) -> Result<Json<Response<DecisionQueueDto>>, AppError> {
    let Some(scope) = decision_scope(
        &s.handlers,
        q.on.as_ref(),
        q.initiative.as_ref(),
        q.include_closed,
    )
    .await?
    else {
        return Ok(ok(DecisionQueueDto::default()));
    };
    let (entries, summary) = s.handlers.decision_queue(&scope, c.ctx().now).await?;
    Ok(ok(DecisionQueueDto {
        entries: entries.iter().map(|e| entry_dto(&s, e)).collect(),
        summary: summary.into(),
    }))
}

/// One decision as the queue shows it: lane, window, next step and the work it holds or affects, from one snapshot.
/// 404 for a missing id.
#[utoipa::path(get, path = "/decisions/{id}/context", params(("id" = i64, Path)), responses((status = 200, body = Response<DecisionQueueEntryDto>), (status = 404, body = Response<Empty>)))]
async fn context(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<DecisionQueueEntryDto>>, AppError> {
    let e = s.handlers.decision_context(id, c.ctx().now).await?;
    Ok(ok(entry_dto(&s, &e)))
}

#[utoipa::path(post, path = "/decisions", request_body = RequireDecision, responses((status = 200, body = Response<RequiredDecisionDto>)))]
async fn create(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<RequireDecision>,
) -> Result<Dec, AppError> {
    let on = resolve_on(&s.handlers, &b.on).await?;
    let d = s
        .handlers
        .require_decision(
            &c.ctx(),
            &b.idempotency_key,
            &on,
            &b.title,
            &b.decider,
            b.due,
            b.reversibility,
        )
        .await?;
    Ok(ok(decision_dto(&s, &d)))
}
#[utoipa::path(get, path = "/decisions/{id}", params(("id" = i64, Path)), responses((status = 200, body = Response<RequiredDecisionDto>)))]
async fn get(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Dec, AppError> {
    let d = s.handlers.get_decision(id).await?;
    Ok(ok(decision_dto(&s, &d)))
}
#[utoipa::path(patch, path = "/decisions/{id}", params(("id" = i64, Path)), request_body = EditDecision, responses((status = 200, body = Response<RequiredDecisionDto>)))]
async fn edit(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<EditDecision>,
) -> Result<Dec, AppError> {
    let d = s
        .handlers
        .edit_decision(
            &c.ctx(),
            id,
            b.version,
            b.title,
            b.decider,
            b.due,
            b.reversibility,
        )
        .await?;
    Ok(ok(decision_dto(&s, &d)))
}
#[utoipa::path(post, path = "/decisions/{id}/link", params(("id" = i64, Path)), request_body = LinkRecord, responses((status = 200, body = Response<RequiredDecisionDto>)))]
async fn link(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<LinkRecord>,
) -> Result<Dec, AppError> {
    let d = s
        .handlers
        .link_record(&c.ctx(), id, &b.repo, &b.path, &b.record_id, b.pr_url)
        .await?;
    Ok(ok(decision_dto(&s, &d)))
}
#[utoipa::path(post, path = "/decisions/{id}/check", params(("id" = i64, Path)), request_body = RecordCheckResult, responses((status = 200, body = Response<RequiredDecisionDto>), (status = 403, body = Response<Empty>, description = "an accepted result from a caller who is not an admin"), (status = 409, body = Response<Empty>)))]
async fn check(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<RecordCheckResult>,
) -> Result<Dec, AppError> {
    // The request's current role (kb/ADR-0015 R5), handed to planning as a value (kb/ADR-0010 R2). Any token kind of
    // an admin may store a result: storing is not an override (kb/ADR-0027 R2), and the scheduled run is automation.
    let is_admin = c.0.role == Role::Admin;
    let d = s
        .handlers
        .record_check(&c.ctx(), id, &b.record_id, b.version, b.check, is_admin)
        .await?;
    Ok(ok(decision_dto(&s, &d)))
}
#[utoipa::path(post, path = "/decisions/{id}/close", params(("id" = i64, Path)), request_body = CloseDecision, responses((status = 200, body = Response<RequiredDecisionDto>), (status = 400, body = Response<Empty>, description = "no reason, already closed, or decided on a one-way decision (kb/ADR-0013 R3)")))]
async fn close(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<CloseDecision>,
) -> Result<Dec, AppError> {
    let d = s
        .handlers
        .close_decision(&c.ctx(), id, b.kind, &b.reason)
        .await?;
    Ok(ok(decision_dto(&s, &d)))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(queue))
        .routes(routes!(get, edit))
        .routes(routes!(context))
        .routes(routes!(link))
        .routes(routes!(check))
        .routes(routes!(close))
}
