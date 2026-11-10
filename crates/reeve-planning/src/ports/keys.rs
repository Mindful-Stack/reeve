use super::*;

/// The key sequence and the lookups that span item types: by key, idempotency key or external reference (kb/ADR-0002 R5: one sequence).
#[async_trait]
pub trait KeyRepo: Send + Sync {
    async fn next_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        prefix: &str,
    ) -> Result<Key, RepoError>;
    async fn find_by_idempotency(
        &self,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<Key>, RepoError>;
    /// `find_by_idempotency` on the caller's transaction: a check made while it holds a lock takes no second connection.
    async fn find_by_idempotency_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<Key>, RepoError>;
    /// Whether an item of this type has this key (whatever its prefix: items issued before a `key_prefix` change keep
    /// their keys).
    async fn key_exists(&self, item: ItemType, key: &Key) -> Result<bool, RepoError>;
    /// The type of the item that has this key, across every type, in one statement (story, epic, initiative; the key
    /// sequence is shared, so at most one has it).
    async fn item_type_of_key(&self, key: &Key) -> Result<Option<ItemType>, RepoError>;
    /// The item type of each key that names an item (story first, then epic, then initiative, as `item_type_of_key`),
    /// on the caller's transaction in one statement, so a handler holding row locks takes no second connection.
    async fn item_types_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        keys: &[Key],
    ) -> Result<HashMap<Key, ItemType>, RepoError>;
    /// The key of the item whose `external_ref` (a Jira key such as `JPD-77`; UNIQUE per table) is this one.
    async fn find_by_external_ref(
        &self,
        item: ItemType,
        external_ref: &str,
    ) -> Result<Option<Key>, RepoError>;
    /// `find_by_external_ref` inside the caller's transaction (the import reads it under `lock_external_ref`).
    async fn find_by_external_ref_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        external_ref: &str,
    ) -> Result<Option<Key>, RepoError>;
    /// Serialises every import of one external reference, whatever the item type, until the transaction ends
    /// (`pg_advisory_xact_lock`). Taken first in the import transaction, before the key sequence and the activity lock.
    async fn lock_external_ref(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        external_ref: &str,
    ) -> Result<(), RepoError>;
}
