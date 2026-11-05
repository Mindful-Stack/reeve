//! `GET /api/v1/search?q=`: items by text.
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::ApiQuery,
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// Text to find in titles, briefs, descriptions and external references; case-insensitive and literal.
    pub q: String,
}

/// Initiatives, epics and stories that contain the text, newest first (at most 50).
#[utoipa::path(get, path = "/search", params(SearchQuery), responses((status = 200, body = Response<Vec<SearchHit>>)))]
async fn search(
    State(s): State<AppState>,
    _c: Caller,
    ApiQuery(q): ApiQuery<SearchQuery>,
) -> Result<Json<Response<Vec<SearchHit>>>, AppError> {
    Ok(ok(s.handlers.search(&q.q).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(search))
}
