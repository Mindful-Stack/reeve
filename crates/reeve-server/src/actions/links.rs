//! Item links and web links of one item (kb/ADR-0031): what `reeve link ls | add | rm` do. The MCP tools
//! `add_item_link`, `remove_item_link`, `add_web_link` and `remove_web_link` call these same actions (kb/ADR-0029
//! R10, by the observation for GRA-901 on that record).
use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_protocol::{
    AddItemLink, AddWebLink, ItemLinkDto, ItemLinkType, ItemType, Key, LinkListingDto, WebLinkDto,
};

/// What writing an item link answers: the list its route sends, and beside it what a tool's receipt needs.
#[allow(dead_code)] // read by the MCP tools, added next
pub(crate) struct ItemLinkWritten {
    /// The item named first, as resolved: its level and its key.
    pub(crate) from: (ItemType, Key),
    /// The other item, as resolved.
    pub(crate) to: (ItemType, Key),
    /// Whether this call added or removed the link. False: it was already there, or was not there.
    pub(crate) changed: bool,
    /// The first item's item links after the write.
    pub(crate) links: Vec<ItemLinkDto>,
}

/// What writing a web link answers: the list its route sends, and beside it what a tool's receipt needs.
#[allow(dead_code)] // read by the MCP tools, added next
pub(crate) struct WebLinkWritten {
    /// The item, as resolved.
    pub(crate) item: Key,
    /// The web link's id: the one added, the one that was already there, or the one a removal named.
    pub(crate) id: i64,
    /// Whether this call added or removed it.
    pub(crate) changed: bool,
    /// The item's web links after the write.
    pub(crate) web_links: Vec<WebLinkDto>,
}

/// `reeve link ls <KEY>`: the item links and the web links of one item of any level. Behind `GET /api/v1/items/{key}/links`. `key`
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
/// `POST /api/v1/items/{key}/links`. Answers the item's item links after the write (`Handlers::item_links_of`: item links only, not the listing); adding a link that exists changes
/// nothing. Answers the list and what a receipt needs.
pub(crate) async fn add_item_link(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: AddItemLink,
) -> Result<ItemLinkWritten, AppError> {
    let from = state.handlers.resolve_any(key).await?;
    let to = state.handlers.resolve_any(b.to.as_str()).await?;
    let changed = state
        .handlers
        .add_item_link(&who.ctx, &from.1, b.link_type, &to.1)
        .await?;
    let links = state.handlers.item_links_of(&from.1).await?;
    Ok(ItemLinkWritten {
        from,
        to,
        changed,
        links,
    })
}

/// `reeve link rm <KEY> <name> <OTHER>`: removes exactly that link, named in its forward form (for `relates_to`,
/// either order). Behind `DELETE /api/v1/items/{key}/links/{type}/{to}`; the three arguments are its path
/// segments. Removing a link that does not exist changes nothing. Answers the list and what a receipt needs.
pub(crate) async fn remove_item_link(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    link_type: &str,
    to: &str,
) -> Result<ItemLinkWritten, AppError> {
    let link_type = ItemLinkType::from_wire(link_type).ok_or_else(|| {
        // The names come from the protocol's table, in its order and its wire spelling: a name added there is
        // listed here without an edit.
        AppError::Validation(format!(
            "unknown item link name {:?}: one of {}",
            crate::text::echo(link_type),
            ItemLinkType::ALL.map(ItemLinkType::wire).join(", ")
        ))
    })?;
    let from = state.handlers.resolve_any(key).await?;
    let to = state.handlers.resolve_any(to).await?;
    let changed = state
        .handlers
        .remove_item_link(&who.ctx, &from.1, link_type, &to.1)
        .await?;
    let links = state.handlers.item_links_of(&from.1).await?;
    Ok(ItemLinkWritten {
        from,
        to,
        changed,
        links,
    })
}

/// `reeve link add <KEY> --url <URL> --title <TITLE>`: attaches a titled URL to the item. Behind
/// `POST /api/v1/items/{key}/web-links`. Answers the item's web links after the write (`Handlers::web_links_of`:
/// web links only, not the listing); the same URL with the same title changes nothing, and with another title it
/// is refused. Answers the list and what a receipt needs.
pub(crate) async fn add_web_link(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: AddWebLink,
) -> Result<WebLinkWritten, AppError> {
    let (_, item) = state.handlers.resolve_any(key).await?;
    let (id, changed) = state
        .handlers
        .add_web_link_reporting(&who.ctx, &item, &b.title, &b.url)
        .await?;
    let web_links = state.handlers.web_links_of(&item).await?;
    Ok(WebLinkWritten {
        item,
        id,
        changed,
        web_links,
    })
}

/// `reeve link rm <KEY> --url <URL>` (the CLI finds the id by the URL): removes one web link of the item by id.
/// Behind `DELETE /api/v1/items/{key}/web-links/{id}`. Removing one that does not exist changes nothing. Answers
/// the list and what a receipt needs.
pub(crate) async fn remove_web_link(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    id: i64,
) -> Result<WebLinkWritten, AppError> {
    let (_, item) = state.handlers.resolve_any(key).await?;
    let changed = state.handlers.remove_web_link(&who.ctx, &item, id).await?;
    let web_links = state.handlers.web_links_of(&item).await?;
    Ok(WebLinkWritten {
        item,
        id,
        changed,
        web_links,
    })
}
