//! `/items/{key}/links`: the item links of one item of any level (kb/ADR-0031). Handler names are unique across
//! the API on purpose, so their operation ids need no path prefix (`routes::unique_operation_ids`).
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use utoipa_axum::{router::OpenApiRouter, routes};

/// The item's item links, each read from this item's side. `key` is a key or an external reference of an
/// initiative, an epic or a story.
#[utoipa::path(get, path = "/items/{key}/links", params(("key" = String, Path)), responses((status = 200, body = Response<LinkListingDto>)))]
async fn list_item_links(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<LinkListingDto>>, AppError> {
    Ok(ok(
        crate::actions::list_item_links(&s, &c.acting(), &key).await?
    ))
}

/// Says that the item in the path follows, duplicates or relates to `to`. Informational: it holds no work back
/// (a story is held back with `POST /stories/{key}/blocks`). Adding a link that exists answers ok and changes
/// nothing. Answers the item's links.
#[utoipa::path(post, path = "/items/{key}/links", params(("key" = String, Path)), request_body = AddItemLink, responses((status = 200, body = Response<Vec<ItemLinkDto>>)))]
async fn add_item_link(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(key): ApiPath<String>,
    ApiJson(b): ApiJson<AddItemLink>,
) -> Result<Json<Response<Vec<ItemLinkDto>>>, AppError> {
    Ok(ok(
        crate::actions::add_item_link(&s, &c.acting(), &key, b).await?
    ))
}

/// Removes "`key` <type> `to`", the forward form: a link an item shows as "is followed by X" is removed by naming
/// X first. For `relates_to` either order names the same link. Removing a link that does not exist answers ok and
/// changes nothing. Answers the item's links.
#[utoipa::path(delete, path = "/items/{key}/links/{type}/{to}", params(("key" = String, Path), ("type" = ItemLinkType, Path), ("to" = String, Path)), responses((status = 200, body = Response<Vec<ItemLinkDto>>)))]
async fn remove_item_link(
    State(s): State<AppState>,
    c: Caller,
    ApiPath((key, link_type, to)): ApiPath<(String, String, String)>,
) -> Result<Json<Response<Vec<ItemLinkDto>>>, AppError> {
    Ok(ok(crate::actions::remove_item_link(
        &s,
        &c.acting(),
        &key,
        &link_type,
        &to,
    )
    .await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_item_links, add_item_link))
        .routes(routes!(remove_item_link))
}
