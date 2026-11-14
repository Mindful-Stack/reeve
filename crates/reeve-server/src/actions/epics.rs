use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_protocol::{EpicDto, ItemType};

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
