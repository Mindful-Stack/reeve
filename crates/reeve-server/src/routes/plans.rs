//! `POST /api/v1/plans`: a plan of items, required decisions and blockers in one request (`reeve apply`).
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::ApiJson,
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

/// Validated whole before anything is written; applying the same plan id again creates nothing.
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
