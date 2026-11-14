use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_protocol::SearchHit;
use serde::Deserialize;

/// The query of `GET /api/v1/search`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SearchQuery {
    /// Text to find in titles, briefs, descriptions and external references, case-insensitive and literal; or a key
    /// (`12` and `#12` under the current prefix, `abc-12` in any case).
    pub(crate) q: String,
}

/// `reeve search <text>`: initiatives, epics and stories that contain the text (at most 50), in tiers. Shared by
/// `GET /api/v1/search` and the MCP tool `search_items`. `q` is what the route takes: its query.
pub(crate) async fn search(
    state: &AppState,
    _who: &Acting<'_>,
    q: &SearchQuery,
) -> Result<Vec<SearchHit>, AppError> {
    Ok(state.handlers.search(&q.q).await?)
}
