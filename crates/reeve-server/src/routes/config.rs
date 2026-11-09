//! `GET /api/v1/config`: the instance settings the client must not hardcode (kb/ADR-0016 R2).
use crate::{
    auth::Caller,
    error::{AppError, ok},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::{InstanceConfigDto, Response};
use utoipa_axum::{router::OpenApiRouter, routes};

/// Gated like every route, so `sign_in_configured` serves signed-in pages (the token page's hints); the sign-in page
/// learns whether sign-in exists from `/auth/login` answering 303 or 404.
#[utoipa::path(get, path = "/config", responses((status = 200, body = Response<InstanceConfigDto>)))]
async fn instance_config(
    State(s): State<AppState>,
    _c: Caller,
) -> Result<Json<Response<InstanceConfigDto>>, AppError> {
    Ok(ok(InstanceConfigDto {
        key_prefix: s.config.key_prefix.clone(),
        horizon_buckets: s.config.horizon_buckets.clone(),
        repos: s.config.repos.iter().map(|r| r.name.clone()).collect(),
        sign_in_configured: s.oidc.is_some(),
        import_enabled: s.config.import.enabled,
        done_window_days: s.config.board.done_window_days,
    }))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(instance_config))
}
