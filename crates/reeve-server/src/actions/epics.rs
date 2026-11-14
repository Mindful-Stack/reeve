use super::{Acting, FilterRef, resolve_filter};
use crate::{error::AppError, state::AppState};
use reeve_protocol::{EpicDto, ItemType, KeyRef};
use serde::Deserialize;

/// The query of `GET /api/v1/epics`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EpicQuery {
    /// An initiative key or external reference. A reference that matches nothing yields an empty list, not 404.
    pub(crate) initiative: Option<KeyRef>,
}

/// `reeve epic ls`: every epic, of one initiative when given. Shared by `GET /api/v1/epics` and the MCP tool
/// `list_epics`. `q` is what the route takes: its query.
pub(crate) async fn list_epics(
    state: &AppState,
    who: &Acting<'_>,
    q: &EpicQuery,
) -> Result<Vec<EpicDto>, AppError> {
    let initiative =
        match resolve_filter(&state.handlers, q.initiative.as_ref(), ItemType::Initiative).await? {
            FilterRef::Nothing => return Ok(Vec::new()),
            FilterRef::Is(k) => Some(k),
            FilterRef::Unset => None,
        };
    Ok(state
        .handlers
        .epic_dtos(initiative.as_ref(), who.ctx.now)
        .await?)
}

/// `reeve epic show <KEY>`: one epic. Shared by `GET /api/v1/epics/{key}` and the MCP tool `get_epic`. `key` is
/// what the route takes: its path parameter, an epic's key or external reference.
pub(crate) async fn get_epic(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
) -> Result<EpicDto, AppError> {
    let key = state.handlers.resolve_ref(key, ItemType::Epic).await?;
    Ok(state.handlers.epic_dto(&key, who.ctx.now).await?)
}
