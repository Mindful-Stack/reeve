use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_protocol::{ItemType, StoryContextDto};

/// `reeve story context <KEY>`: everything an agent needs before starting a story. Shared by
/// `GET /api/v1/stories/{key}/context` and the MCP tool `get_story`. `key` is what the route takes: its path
/// parameter, a story's key or external reference.
pub async fn story_context(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
) -> Result<StoryContextDto, AppError> {
    let key = state.handlers.resolve_ref(key, ItemType::Story).await?;
    let mut context = state.handlers.story_context(&key, who.ctx.now).await?;
    for d in &mut context.required_decisions {
        super::browse(state, d);
    }
    Ok(context)
}
