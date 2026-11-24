//! Item links of one item (kb/ADR-0031): what `reeve link ls | add | rm` do. No MCP tool calls these yet
//! (kb/ADR-0029 R10: a tool is added only by an observation on that record); when one is, it calls these same
//! actions.
use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_protocol::{AddItemLink, ItemLinkDto, LinkListingDto, LinkType};

/// `reeve link ls <KEY>`: the item links of one item of any level. Behind `GET /api/v1/items/{key}/links`. `key`
/// is what the route takes: its path parameter, a key or an external reference.
pub(crate) async fn list_item_links(
    state: &AppState,
    _who: &Acting<'_>,
    key: &str,
) -> Result<LinkListingDto, AppError> {
    let (_, key) = state.handlers.resolve_any(key).await?;
    Ok(state.handlers.link_listing(&key).await?)
}

/// `reeve link add <KEY> <name> <OTHER>`: the item in the path follows, duplicates or relates to `b.to`. Behind
/// `POST /api/v1/items/{key}/links`. Answers the item's links after the write; adding a link that exists changes
/// nothing.
pub(crate) async fn add_item_link(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: AddItemLink,
) -> Result<Vec<ItemLinkDto>, AppError> {
    let (_, from) = state.handlers.resolve_any(key).await?;
    let (_, to) = state.handlers.resolve_any(b.to.as_str()).await?;
    state
        .handlers
        .add_item_link(&who.ctx, &from, b.link_type, &to)
        .await?;
    Ok(state.handlers.link_listing(&from).await?.links)
}

/// `reeve link rm <KEY> <name> <OTHER>`: removes exactly that link, named in its forward form (for `relates_to`,
/// either order). Behind `DELETE /api/v1/items/{key}/links/{type}/{to}`; the three arguments are its path
/// segments. Removing a link that does not exist changes nothing.
pub(crate) async fn remove_item_link(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    link_type: &str,
    to: &str,
) -> Result<Vec<ItemLinkDto>, AppError> {
    let link_type = LinkType::from_wire(link_type).ok_or_else(|| {
        // The names come from the protocol's table, in its order and its wire spelling: a name added there is
        // listed here without an edit.
        AppError::Validation(format!(
            "unknown item link name {:?}: one of {}",
            crate::text::echo(link_type),
            LinkType::ALL.map(LinkType::wire).join(", ")
        ))
    })?;
    let (_, from) = state.handlers.resolve_any(key).await?;
    let (_, to) = state.handlers.resolve_any(to).await?;
    state
        .handlers
        .remove_item_link(&who.ctx, &from, link_type, &to)
        .await?;
    Ok(state.handlers.link_listing(&from).await?.links)
}
