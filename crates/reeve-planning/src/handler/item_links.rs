//! Item links (kb/ADR-0031) in the handlers: adding, removing and reading them. Informational: nothing here is
//! read by a move or by readiness (R3; the isolation lint bans these names in `handler/moves.rs`), and no item's
//! version, rank or progress changes. Any caller who may edit an item may add or remove its item links, whatever
//! its token kind (R7).
use super::*;
use crate::domain::item_link::ItemLink;
use reeve_protocol::{ItemLinkDto, ItemLinkType, LinkListingDto};
use sqlx::{Postgres, Transaction};

impl Handlers {
    /// The link as stored, and its two ends with their levels, `from` first. Validation first (a self link),
    /// then both items must exist. It reads through the pool, so only the public forms call it: a caller of an
    /// `_in` form brings its own ends.
    async fn item_link_ends(
        &self,
        from: &Key,
        link_type: ItemLinkType,
        to: &Key,
    ) -> Result<(ItemLink, [(ItemType, Key); 2]), HandlerError> {
        let link = ItemLink::new(from.clone(), link_type, to.clone())?;
        let ends = [
            (
                self.item_type_of(link.from_key()).await?,
                link.from_key().clone(),
            ),
            (
                self.item_type_of(link.to_key()).await?,
                link.to_key().clone(),
            ),
        ];
        Ok((link, ends))
    }

    /// One entry on each item the link names, with the same payload: the ends as stored. Recorded last, after
    /// the row is written, on the same transaction.
    async fn record_item_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        link: &ItemLink,
        ends: &[(ItemType, Key); 2],
        action: &'static str,
    ) -> Result<(), HandlerError> {
        let payload = serde_json::json!({
            "type": link.link_type().wire(),
            "from": link.from_key(),
            "to": link.to_key(),
        });
        for (level, key) in ends {
            self.activity
                .record(
                    tx,
                    self.rec(
                        c,
                        item_name(*level),
                        &key.to_string(),
                        action,
                        payload.clone(),
                    ),
                )
                .await?;
        }
        Ok(())
    }

    /// Adds "`from` follows, duplicates or relates to `to`". Both items must exist, at any level (NotFound
    /// otherwise); a link from an item to itself is a validation error. Answers whether this call added it: a link
    /// that already exists changes nothing and logs nothing. The row and one `item_link_added` entry on each of
    /// the two items commit together; no version moves.
    pub async fn add_item_link(
        &self,
        c: &Ctx,
        from: &Key,
        link_type: ItemLinkType,
        to: &Key,
    ) -> Result<bool, HandlerError> {
        let (link, ends) = self.item_link_ends(from, link_type, to).await?;
        let mut tx = self.pool.begin().await?;
        let added = self.add_item_link_in(&mut tx, c, &link, &ends).await?;
        tx.commit().await?;
        Ok(added)
    }

    /// Inside the caller's transaction: writes the link and, when this call added it, records `item_link_added` on
    /// each of its two items, last. `link` is the link as stored and `ends` its two items with their levels, `from`
    /// first; the caller has resolved both. Nothing here reads through the pool, so it also serves a caller whose
    /// items are not committed yet, and it neither commits nor rolls back: a link that already exists writes and
    /// logs nothing and answers false, and the transaction stays the caller's. Plan apply (GRA-879) composes it
    /// with the plan's other writes.
    pub(super) async fn add_item_link_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        link: &ItemLink,
        ends: &[(ItemType, Key); 2],
    ) -> Result<bool, HandlerError> {
        if !self.ports.item_links.add_item_link(tx, link).await? {
            return Ok(false);
        }
        self.record_item_link(tx, c, link, ends, "item_link_added")
            .await?;
        Ok(true)
    }

    /// Removes exactly "`from` <name> `to`" (for `relates_to`, either order). Both items must exist; a link that
    /// does not exist is a no-op that logs nothing. Logged as `item_link_removed` on each of the two items.
    pub async fn remove_item_link(
        &self,
        c: &Ctx,
        from: &Key,
        link_type: ItemLinkType,
        to: &Key,
    ) -> Result<bool, HandlerError> {
        let (link, ends) = self.item_link_ends(from, link_type, to).await?;
        let mut tx = self.pool.begin().await?;
        let removed = self.remove_item_link_in(&mut tx, c, &link, &ends).await?;
        tx.commit().await?;
        Ok(removed)
    }

    /// Inside the caller's transaction: removes the link and, when it was there, records `item_link_removed` on
    /// each of its two items, last. The same contract as `add_item_link_in`: the caller brings the link and its
    /// resolved ends, nothing is read through the pool, and the transaction is neither committed nor rolled back
    /// here.
    pub(super) async fn remove_item_link_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        link: &ItemLink,
        ends: &[(ItemType, Key); 2],
    ) -> Result<bool, HandlerError> {
        if !self.ports.item_links.remove_item_link(tx, link).await? {
            return Ok(false);
        }
        self.record_item_link(tx, c, link, ends, "item_link_removed")
            .await?;
        Ok(true)
    }

    /// One item's item links as it reads them, from one read-only snapshot. NotFound when no item has the key.
    pub async fn link_listing(&self, key: &Key) -> Result<LinkListingDto, HandlerError> {
        self.item_type_of(key).await?;
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            Ok(LinkListingDto {
                links: self.item_link_dtos_in(&mut tx, key).await?,
            })
        }
        .await;
        end_read(tx, out).await
    }

    /// The item links of one item on the caller's transaction: each with its side, its wording and the level and
    /// title of the item at the other end, ordered by name (`follows`, `duplicates`, `relates_to`), `out` before
    /// `in`, then the other item's key number (`Key::number`), then its key text. Two statements, none for the
    /// second when there are no links.
    pub(crate) async fn item_link_dtos_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Vec<ItemLinkDto>, HandlerError> {
        let links = self
            .ports
            .item_links
            .item_links_of_in(tx, std::slice::from_ref(key))
            .await?;
        let others: Vec<Key> = links
            .iter()
            .filter_map(|l| l.seen_from(key))
            .map(|(_, other)| other.clone())
            .collect();
        let heads = self.ports.reads.item_heads_in(tx, &others).await?;
        let mut out: Vec<ItemLinkDto> = links
            .iter()
            .filter_map(|l| {
                let (direction, other) = l.seen_from(key)?;
                // Nothing deletes an item, so the head is there; a row whose other end is gone is left out
                // rather than shown without a title.
                let head = heads.get(other)?;
                Some(super::dto::item_link_dto(
                    l.link_type(),
                    direction,
                    other,
                    head,
                ))
            })
            .collect();
        // By name, `out` before `in`, then the other item's key number; the key text breaks the tie between two
        // prefixes (`A-2` and `LONG-2` can both exist: each prefix has its own counter), so the order is total.
        out.sort_by_cached_key(|l| {
            (
                l.link_type,
                l.direction,
                l.item.key.number(),
                l.item.key.to_string(),
            )
        });
        Ok(out)
    }
}
