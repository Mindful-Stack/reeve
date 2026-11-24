//! Web links (kb/ADR-0031 R5) in the handlers: adding, removing and reading them. Informational: nothing here is
//! read by a move or by readiness (R3; the isolation lint bans these names in `handler/moves.rs`), and no item's
//! version changes. Any caller who may edit an item may add or remove its web links, whatever its token kind (R7).
use super::*;
use crate::domain::web_link::WebLink;
use crate::ports::StoredWebLink;
use reeve_protocol::WebLinkDto;
use sqlx::{Postgres, Transaction};

/// Whether `link` would be a new web link on an item that has the web links `stored`: true when none of them has
/// its URL (compared as stored), false when one has that URL under the same title (a repeat, which changes
/// nothing), and a validation error when one has it under another title (a web link is not edited in place: remove
/// it first). The one place that compares and words the refusal: `add_web_link_in` calls it when its insert found
/// the URL taken, and a check that must refuse before anything is written (plan apply's, GRA-879) calls it on the
/// item's stored web links.
pub(super) fn web_link_is_new(
    stored: &[StoredWebLink],
    link: &WebLink,
) -> Result<bool, HandlerError> {
    match stored.iter().find(|w| w.link.url() == link.url()) {
        None => Ok(true),
        Some(w) if w.link.title() == link.title() => Ok(false),
        Some(w) => Err(HandlerError::Validation(format!(
            "this item already has a web link to that URL, titled {:?}: remove it first, then add it with the new title",
            w.link.title()
        ))),
    }
}

impl Handlers {
    /// Attaches a titled URL to an item of any level. A bad title or URL is a validation error; the item must
    /// exist (NotFound otherwise). Answers whether this call added it: the same URL with the same title is already
    /// there and changes nothing; the same URL with another title is a validation error that says to remove it
    /// first (a web link is not edited in place). Logged as `web_link_added` on the item, in the same transaction;
    /// no version moves.
    pub async fn add_web_link(
        &self,
        c: &Ctx,
        item: &Key,
        title: &str,
        url: &str,
    ) -> Result<bool, HandlerError> {
        let link = WebLink::new(title, url)?;
        let level = self.item_type_of(item).await?;
        let mut tx = self.pool.begin().await?;
        let added = self.add_web_link_in(&mut tx, c, level, item, &link).await?;
        tx.commit().await?;
        Ok(added)
    }

    /// Inside the caller's transaction: attaches `link` to `item` and, when this call added it, records
    /// `web_link_added` on the item, last. `item` is the item's key and `level` its level; the caller has resolved
    /// both and validated the link (`WebLink::new`). Nothing here reads through the pool, so it also serves a
    /// caller whose item is not committed yet, and it neither commits nor rolls back. The same URL with the same
    /// title answers false and writes nothing; with another title it is `web_link_is_new`'s validation error, and
    /// what the caller does with its transaction then is the caller's. Plan apply (GRA-879) composes it with the
    /// plan's other writes.
    pub(super) async fn add_web_link_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        level: ItemType,
        item: &Key,
        link: &WebLink,
    ) -> Result<bool, HandlerError> {
        let Some(id) = self.ports.web_links.add_web_link(tx, item, link).await? else {
            // The item already has this URL: the same title is a repeat, another title is not an edit.
            let stored = self
                .ports
                .web_links
                .web_links_of_in(tx, std::slice::from_ref(item))
                .await?;
            return if web_link_is_new(&stored, link)? {
                // It was removed between the insert and the read: nothing was added, so say so.
                Err(HandlerError::Conflict { current: -1 })
            } else {
                Ok(false)
            };
        };
        self.activity
            .record(
                tx,
                self.rec(
                    c,
                    item_name(level),
                    &item.to_string(),
                    "web_link_added",
                    serde_json::json!({"id": id, "title": link.title(), "url": link.url()}),
                ),
            )
            .await?;
        Ok(true)
    }

    /// Removes one web link of an item by its id. The item must exist; an id the item does not have is a no-op
    /// that logs nothing. Logged as `web_link_removed` on the item with the title and URL it had.
    pub async fn remove_web_link(
        &self,
        c: &Ctx,
        item: &Key,
        id: i64,
    ) -> Result<bool, HandlerError> {
        let level = self.item_type_of(item).await?;
        let mut tx = self.pool.begin().await?;
        let removed = self.remove_web_link_in(&mut tx, c, level, item, id).await?;
        tx.commit().await?;
        Ok(removed)
    }

    /// Inside the caller's transaction: removes the web link `id` of `item` and, when the item had it, records
    /// `web_link_removed` with the title and URL it had, last. The same contract as `add_web_link_in`: the caller
    /// brings the item's key and level, nothing is read through the pool, and the transaction is neither committed
    /// nor rolled back here.
    pub(super) async fn remove_web_link_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        level: ItemType,
        item: &Key,
        id: i64,
    ) -> Result<bool, HandlerError> {
        let Some(link) = self.ports.web_links.remove_web_link(tx, item, id).await? else {
            return Ok(false);
        };
        self.activity
            .record(
                tx,
                self.rec(
                    c,
                    item_name(level),
                    &item.to_string(),
                    "web_link_removed",
                    serde_json::json!({"id": id, "title": link.title(), "url": link.url()}),
                ),
            )
            .await?;
        Ok(true)
    }

    /// One item's web links, oldest first, and nothing else, from one read-only snapshot. NotFound when no item has
    /// the key. The answer of a write that changed a web link (the POST and the DELETE): it loads no item link.
    pub async fn web_links_of(&self, key: &Key) -> Result<Vec<WebLinkDto>, HandlerError> {
        self.item_type_of(key).await?;
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = self.web_link_dtos_in(&mut tx, key).await;
        end_read(tx, out).await
    }

    /// The web links of one item on the caller's transaction, oldest first. One statement.
    pub(crate) async fn web_link_dtos_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Vec<WebLinkDto>, HandlerError> {
        Ok(self
            .ports
            .web_links
            .web_links_of_in(tx, std::slice::from_ref(key))
            .await?
            .iter()
            .map(super::dto::web_link_dto)
            .collect())
    }
}
