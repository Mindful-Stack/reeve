//! Item links (kb/ADR-0031) in the handlers: adding, removing and reading them. Informational: nothing here is
//! read by a move or by readiness (R3; the isolation lint bans these names in `handler/moves.rs`), and no item's
//! version, rank or progress changes. Any caller who may edit an item may add or remove its item links, whatever
//! its token kind (R7).
use super::*;
use crate::domain::item_link::{ItemLink, ItemLinkError};
use reeve_protocol::{ItemLinkDto, ItemLinkType, LinkListingDto};
use sqlx::{Postgres, Transaction};

/// An item link ready to write: the link as stored, and the level of each of its two items. The one value the
/// `_in` writers take, and the one place where a level is paired with a key: `new` takes each end as
/// `(level, key)` in the order the caller named them, builds the stored link (`ItemLink::new`, which puts the
/// lower key of a `relates_to` first) and keeps each level with its own key through that swap. So no caller
/// reorders anything, and an activity entry cannot be recorded under the other item's level.
///
/// It reads nothing. The caller says that both items exist at those levels: the public handlers look each key up
/// (`Handlers::item_type_of`), and a handler that created the items in its own transaction (plan apply, GRA-879)
/// passes the levels it created them at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedItemLink {
    link: ItemLink,
    from_level: ItemType,
    to_level: ItemType,
}

impl ResolvedItemLink {
    /// "`from` follows, duplicates or relates to `to`", each end with its level, in the order the caller names
    /// them. A link from an item to itself is refused (kb/ADR-0031 R1). `pub(super)`: for the handlers in this
    /// module tree, which is what plan apply (`handler/plans.rs`) needs.
    pub(super) fn new(
        from: (ItemType, Key),
        link_type: ItemLinkType,
        to: (ItemType, Key),
    ) -> Result<Self, ItemLinkError> {
        let link = ItemLink::new(from.1.clone(), link_type, to.1.clone())?;
        // The ends differ (no self link), so the stored `from` is exactly one of the two keys named.
        let (from_level, to_level) = if *link.from_key() == from.1 {
            (from.0, to.0)
        } else {
            (to.0, from.0)
        };
        Ok(Self {
            link,
            from_level,
            to_level,
        })
    }
    /// The link as stored.
    pub(super) fn link(&self) -> &ItemLink {
        &self.link
    }
    /// The two items as stored, `from` first, each with its own level.
    pub(super) fn ends(&self) -> [(ItemType, &Key); 2] {
        [
            (self.from_level, self.link.from_key()),
            (self.to_level, self.link.to_key()),
        ]
    }
}

impl Handlers {
    /// The link between two keys, ready to write. Validation first (a self link), before any read; then both
    /// items must exist (NotFound), which also gives each its level. It reads through the pool, so only the
    /// public forms call it: a caller of an `_in` form builds its own `ResolvedItemLink`.
    ///
    /// The lookup stays here although the server's action has already resolved both references
    /// (`resolve_any` answers the level too): taking the level from the caller would let a public caller pair a
    /// key with a level nobody checked, and the cost is one primary-key lookup per end.
    async fn resolved_item_link(
        &self,
        from: &Key,
        link_type: ItemLinkType,
        to: &Key,
    ) -> Result<ResolvedItemLink, HandlerError> {
        ItemLink::new(from.clone(), link_type, to.clone())?;
        let from = (self.item_type_of(from).await?, from.clone());
        let to = (self.item_type_of(to).await?, to.clone());
        Ok(ResolvedItemLink::new(from, link_type, to)?)
    }

    /// One entry on each item the link names, with the same payload: the ends as stored. Recorded last, after
    /// the row is written, on the same transaction.
    async fn record_item_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        link: &ResolvedItemLink,
        action: &'static str,
    ) -> Result<(), HandlerError> {
        let ends = link.ends();
        let link = link.link();
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
                        item_name(level),
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
        let link = self.resolved_item_link(from, link_type, to).await?;
        let mut tx = self.pool.begin().await?;
        let added = self.add_item_link_in(&mut tx, c, &link).await?;
        tx.commit().await?;
        Ok(added)
    }

    /// Inside the caller's transaction: writes the link and, when this call added it, records `item_link_added` on
    /// each of its two items, last, each under its own level and key. `link` is the only thing the caller brings: a
    /// `ResolvedItemLink`, built by `ResolvedItemLink::new` from the two ends as named. Nothing here reads through
    /// the pool, so it also serves a caller whose
    /// items are not committed yet, and it neither commits nor rolls back: a link that already exists writes and
    /// logs nothing and answers false, and the transaction stays the caller's. Plan apply (GRA-879) composes it
    /// with the plan's other writes.
    pub(super) async fn add_item_link_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        link: &ResolvedItemLink,
    ) -> Result<bool, HandlerError> {
        if !self.ports.item_links.add_item_link(tx, link.link()).await? {
            return Ok(false);
        }
        self.record_item_link(tx, c, link, "item_link_added")
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
        let link = self.resolved_item_link(from, link_type, to).await?;
        let mut tx = self.pool.begin().await?;
        let removed = self.remove_item_link_in(&mut tx, c, &link).await?;
        tx.commit().await?;
        Ok(removed)
    }

    /// Inside the caller's transaction: removes the link and, when it was there, records `item_link_removed` on
    /// each of its two items, last. The same contract as `add_item_link_in`: the caller brings a `ResolvedItemLink`,
    /// nothing is read through the pool, and the transaction is neither committed nor rolled back
    /// here.
    pub(super) async fn remove_item_link_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        link: &ResolvedItemLink,
    ) -> Result<bool, HandlerError> {
        if !self
            .ports
            .item_links
            .remove_item_link(tx, link.link())
            .await?
        {
            return Ok(false);
        }
        self.record_item_link(tx, c, link, "item_link_removed")
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

#[cfg(test)]
mod tests {
    use super::*;
    use ItemLinkType::{Duplicates, Follows, RelatesTo};

    fn k(s: &str) -> Key {
        s.parse().unwrap()
    }

    /// Each level stays with its own key, whichever end was named first: `relates_to` is stored with the lower
    /// key first, so naming the higher key first swaps the ends, and the levels swap with them.
    #[test]
    fn a_resolved_item_link_keeps_each_level_with_its_key_through_the_swap() {
        let (epic, story) = ((ItemType::Epic, k("ABC-2")), (ItemType::Story, k("ABC-9")));
        for (from, to) in [(&epic, &story), (&story, &epic)] {
            let link = ResolvedItemLink::new(from.clone(), RelatesTo, to.clone()).unwrap();
            assert_eq!(
                link.ends(),
                [
                    (ItemType::Epic, &k("ABC-2")),
                    (ItemType::Story, &k("ABC-9"))
                ],
                "named {} first",
                from.1
            );
            assert_eq!(
                (link.link().from_key(), link.link().to_key()),
                (&k("ABC-2"), &k("ABC-9"))
            );
        }
        // A directed name keeps the ends as named, each with its level.
        for t in [Follows, Duplicates] {
            let link = ResolvedItemLink::new(story.clone(), t, epic.clone()).unwrap();
            assert_eq!(
                link.ends(),
                [
                    (ItemType::Story, &k("ABC-9")),
                    (ItemType::Epic, &k("ABC-2"))
                ],
                "{t:?}"
            );
        }
        assert_eq!(
            ResolvedItemLink::new(epic.clone(), Follows, (ItemType::Epic, k("ABC-2"))),
            Err(ItemLinkError::SelfLink)
        );
    }
}
