use super::*;
use crate::domain::item_link::ItemLink;

/// Item links (kb/ADR-0031): rows of their own, owned by neither item. No method reads or writes an item row, a
/// version or a readiness fact, and nothing here reads the clock. `handler/moves.rs` may not reach this port (the
/// isolation lint bans `ports.item_links` there).
#[async_trait]
pub trait ItemLinkRepo: Send + Sync {
    /// Stores the link; false when it was already there (nothing written).
    async fn add_item_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        link: &ItemLink,
    ) -> Result<bool, RepoError>;
    /// Deletes the link; false when there was none (nothing deleted).
    async fn remove_item_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        link: &ItemLink,
    ) -> Result<bool, RepoError>;
    /// Every item link with one of `items` at either end, in one statement, ordered by from key, to key and name.
    /// Joins no other table: the title and level of the item at the other end come from
    /// `PlanningReads::item_heads_in`.
    async fn item_links_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        items: &[Key],
    ) -> Result<Vec<ItemLink>, RepoError>;
    /// Every item link there is, in one statement and in no promised order: the export reads this on its
    /// transaction and orders the rows itself. Joins no other table and takes no item list.
    async fn all_item_links_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<ItemLink>, RepoError>;
}
