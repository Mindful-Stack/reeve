//! `GET /api/v1/export`: the whole instance as JSON. Only an admin with an interactive token may
//! export; the document is built by `crate::export::export_document` (one snapshot, no secrets).
use crate::{
    auth::Caller,
    error::{AppError, ok},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

#[utoipa::path(get, path = "/export", responses((status = 200, body = Response<ExportDto>), (status = 403, description = "not an admin with an interactive token")))]
async fn export(
    State(s): State<AppState>,
    c: Caller,
) -> Result<Json<Response<ExportDto>>, AppError> {
    if c.0.role != Role::Admin || c.0.token_kind != TokenKind::Interactive {
        return Err(AppError::Forbidden);
    }
    Ok(ok(
        crate::export::export_document(&s.export_sources(), c.1).await?
    ))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(export))
}
