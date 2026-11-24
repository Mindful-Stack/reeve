use super::*;
use crate::domain::web_link::WebLink;

/// A stored web link: its id, the item it is on, and the link. Ids rise with insertion, so ordering by id is
/// ordering by when a web link was added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWebLink {
    pub id: i64,
    pub item: Key,
    pub link: WebLink,
}

/// Web links (kb/ADR-0031 R5): rows of their own, each on one item. No method reads or writes an item row, a
/// version or a readiness fact, and nothing here reads the clock. `handler/moves.rs` may not reach this port (the
/// isolation lint bans `ports.web_links` there).
#[async_trait]
pub trait WebLinkRepo: Send + Sync {
    /// Stores the link on the item and answers its id; None when the item already has a web link with this URL,
    /// whatever its title (nothing written).
    async fn add_web_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: &Key,
        link: &WebLink,
    ) -> Result<Option<i64>, RepoError>;
    /// Deletes one web link of the item by id and answers what it was; None when the item has none with that id.
    async fn remove_web_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: &Key,
        id: i64,
    ) -> Result<Option<WebLink>, RepoError>;
    /// Every web link of `items`, in one statement, oldest first. Joins no other table.
    async fn web_links_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        items: &[Key],
    ) -> Result<Vec<StoredWebLink>, RepoError>;
    /// Every web link there is, in one statement and in no promised order: the export reads this on its
    /// transaction and orders the rows itself. Joins no other table and takes no item list.
    async fn all_web_links_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<StoredWebLink>, RepoError>;
}
