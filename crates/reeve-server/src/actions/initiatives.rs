use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_protocol::{InitiativeDto, ItemType};

/// `reeve initiative show <KEY>`: one initiative. Shared by `GET /api/v1/initiatives/{key}` and the MCP tool
/// `get_initiative`. `key` is what the route takes: its path parameter, an initiative's key or external reference.
pub(crate) async fn get_initiative(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
) -> Result<InitiativeDto, AppError> {
    let key = state
        .handlers
        .resolve_ref(key, ItemType::Initiative)
        .await?;
    Ok(state.handlers.initiative_dto(&key, who.ctx.now).await?)
}
