//! `POST /api/v1/plans`: a plan of items, required decisions, blockers, item links and web links in one request
//! (`reeve apply`).
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::ApiJson,
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

/// A plan of items, required decisions, blockers, item links and web links. Validated whole before anything is
/// written. Items and required decisions are keyed by plan id and name; blockers, item links and web links are
/// idempotent by content; so applying the same plan id again creates nothing. Each write commits on its own: a
/// failed apply is finished by applying the same id again once the cause is corrected.
#[utoipa::path(post, path = "/plans", request_body = ApplyPlan, responses((status = 200, body = Response<PlanApplied>)))]
async fn apply(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<ApplyPlan>,
) -> Result<Json<Response<PlanApplied>>, AppError> {
    Ok(ok(s.handlers.apply_plan(&c.ctx(), b).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(apply))
}
