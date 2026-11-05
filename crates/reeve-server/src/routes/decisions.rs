use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_planning::{DecisionScope, HandlerError, Handlers};
use reeve_protocol::*;
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DecisionQuery {
    /// A story, epic or initiative key or external reference. A reference that matches nothing yields an empty list, not 404.
    pub on: Option<KeyRef>,
    #[serde(default)]
    pub open_with_record: bool,
    /// Without `on`, also list closed decisions (a list scoped by `on` always includes them).
    #[serde(default)]
    pub include_closed: bool,
    /// Every decision under this initiative: on it, its epics and their stories, closed ones included. Not
    /// with `on`.
    pub initiative: Option<KeyRef>,
}

/// A decision target is untyped: `Handlers::resolve_any` checks native keys of every item type before any external
/// reference.
async fn resolve_on(h: &Handlers, raw: &KeyRef) -> Result<Key, HandlerError> {
    h.resolve_any(raw.as_str()).await.map(|(_, k)| k)
}

type Dec = Json<Response<RequiredDecisionDto>>;

/// The scope `on`/`initiative`/`include_closed` names. `None`: a reference that matches nothing, answered as an
/// empty list, not 404. Both given is a validation error. The list and the queue resolve through this one function,
/// so an unknown key behaves the same for both.
async fn scope(
    h: &Handlers,
    on: Option<&KeyRef>,
    initiative: Option<&KeyRef>,
    include_closed: bool,
) -> Result<Option<DecisionScope>, AppError> {
    if let Some(r) = initiative {
        if on.is_some() {
            return Err(HandlerError::Validation("give on or initiative, not both".into()).into());
        }
        return match h.resolve_ref(r.as_str(), ItemType::Initiative).await {
            Ok(k) => Ok(Some(DecisionScope::Initiative(k))),
            Err(HandlerError::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        };
    }
    match on {
        Some(r) => match resolve_on(h, r).await {
            Ok(k) => Ok(Some(DecisionScope::On(k))),
            Err(HandlerError::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        },
        None => Ok(Some(DecisionScope::All { include_closed })),
    }
}

#[utoipa::path(get, path = "/decisions", params(DecisionQuery), responses((status = 200, body = Response<Vec<RequiredDecisionDto>>)))]
async fn list(
    State(s): State<AppState>,
    _c: Caller,
    ApiQuery(q): ApiQuery<DecisionQuery>,
) -> Result<Json<Response<Vec<RequiredDecisionDto>>>, AppError> {
    let rows = match scope(
        &s.handlers,
        q.on.as_ref(),
        q.initiative.as_ref(),
        q.include_closed,
    )
    .await?
    {
        None => Vec::new(),
        Some(DecisionScope::Initiative(k)) => {
            match s.handlers.decisions_under_initiative(&k).await {
                Ok(rows) => rows,
                Err(HandlerError::NotFound) => Vec::new(),
                Err(e) => return Err(e.into()),
            }
        }
        Some(DecisionScope::On(k)) => {
            s.handlers
                .list_decisions(Some(&k), q.open_with_record, q.include_closed)
                .await?
        }
        Some(DecisionScope::All { include_closed }) => {
            s.handlers
                .list_decisions(None, q.open_with_record, include_closed)
                .await?
        }
    };
    Ok(ok(rows.iter().map(RequiredDecisionDto::from).collect()))
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
    let Some(scope) = scope(
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
        entries: entries.iter().map(DecisionQueueEntryDto::from).collect(),
        summary: summary.into(),
    }))
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
    Ok(ok(RequiredDecisionDto::from(&d)))
}
#[utoipa::path(get, path = "/decisions/{id}", params(("id" = i64, Path)), responses((status = 200, body = Response<RequiredDecisionDto>)))]
async fn get(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Dec, AppError> {
    Ok(ok(RequiredDecisionDto::from(
        &s.handlers.get_decision(id).await?,
    )))
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
    Ok(ok(RequiredDecisionDto::from(&d)))
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
    Ok(ok(RequiredDecisionDto::from(&d)))
}
#[utoipa::path(post, path = "/decisions/{id}/check", params(("id" = i64, Path)), request_body = RecordCheckResult, responses((status = 200, body = Response<RequiredDecisionDto>), (status = 403, body = Response<Empty>, description = "an accepted result from a caller who is not an admin"), (status = 409, body = Response<Empty>)))]
async fn check(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<RecordCheckResult>,
) -> Result<Dec, AppError> {
    // The request's current role (kb/ADR-0008 R2), handed to planning as a value (kb/ADR-0010 R2). Any token kind of
    // an admin may store a result: storing is not an override (kb/ADR-0003 R2), and the scheduled run is automation.
    let is_admin = c.0.role == Role::Admin;
    let d = s
        .handlers
        .record_check(&c.ctx(), id, &b.record_id, b.version, b.check, is_admin)
        .await?;
    Ok(ok(RequiredDecisionDto::from(&d)))
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
    Ok(ok(RequiredDecisionDto::from(&d)))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(queue))
        .routes(routes!(get, edit))
        .routes(routes!(link))
        .routes(routes!(check))
        .routes(routes!(close))
}
