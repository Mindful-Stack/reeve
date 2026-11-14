//! `GET /api/v1/search?q=`: items by text.
use crate::{
    actions::SearchQuery,
    auth::Caller,
    error::{AppError, ok},
    extract::ApiQuery,
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

/// Initiatives, epics and stories that contain the text (at most 50), in tiers: the item whose key the query names,
/// an exact external reference, title matches, the rest; newest first within a tier. Both exact tiers are `exact`.
#[utoipa::path(get, path = "/search", params(SearchQuery), responses((status = 200, body = Response<Vec<SearchHit>>)))]
async fn search(
    State(s): State<AppState>,
    c: Caller,
    ApiQuery(q): ApiQuery<SearchQuery>,
) -> Result<Json<Response<Vec<SearchHit>>>, AppError> {
    Ok(ok(crate::actions::search(&s, &c.acting(), &q).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(search))
}
