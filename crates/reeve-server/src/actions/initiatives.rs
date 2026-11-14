use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_protocol::{InitiativeDto, ItemType};

/// `reeve initiative ls`: every initiative with its progress. Shared by `GET /api/v1/initiatives` and the MCP tool
/// `list_initiatives`. The route takes no input.
pub(crate) async fn list_initiatives(
    state: &AppState,
    who: &Acting<'_>,
) -> Result<Vec<InitiativeDto>, AppError> {
    Ok(state.handlers.list_initiatives(who.ctx.now).await?)
}

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
