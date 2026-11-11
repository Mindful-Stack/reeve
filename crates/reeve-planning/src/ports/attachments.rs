use super::*;
use crate::domain::attachment::Attachment;
use bytes::Bytes;
use uuid::Uuid;

/// What a claim did (kb/ADR-0022 R2).
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimOutcome {
    /// The attachments this claim newly owned, with their filenames; ids already owned by the same item, and ids that
    /// no longer exist, are left out.
    Claimed(Vec<(Uuid, String)>),
    /// An id belongs to another person's upload or to another item; nothing is claimed.
    Refused,
}

/// The attachment rows and their chunks.
#[async_trait]
pub trait AttachmentRepo: Send + Sync {
    /// False when the idempotency key already names an attachment; nothing is written then.
    async fn insert_attachment(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        a: &Attachment,
        idempotency_key: Option<&str>,
    ) -> Result<bool, RepoError>;
    async fn insert_chunk(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: Uuid,
        n: i32,
        bytes: &[u8],
    ) -> Result<(), RepoError>;
    async fn find_attachment_by_idempotency(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<Attachment>, RepoError>;
    /// NotFound when there is no such attachment.
    async fn get_attachment(&self, id: Uuid) -> Result<Attachment, RepoError>;
    /// One chunk, by its own query on the pool (kb/ADR-0022 R3); None past the last one.
    async fn attachment_chunk(&self, id: Uuid, n: i32) -> Result<Option<Bytes>, RepoError>;
    /// An item's attachments, oldest first.
    async fn list_attachments_for(
        &self,
        item: ItemType,
        key: &Key,
    ) -> Result<Vec<Attachment>, RepoError>;
    /// Gives the caller's unclaimed uploads among `ids` to the item, in one conditional statement; the claimed ones come
    /// back in the order of `ids`.
    async fn claim_attachments(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ids: &[Uuid],
        actor: &str,
        item: ItemType,
        key: &Key,
    ) -> Result<ClaimOutcome, RepoError>;
    /// The row and, by cascade, its chunks; the row as it was deleted, or None when there was none.
    async fn delete_attachment(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> Result<Option<Attachment>, RepoError>;
    /// Deletes up to `limit` unclaimed attachments uploaded before `older_than`, skipping rows another transaction has
    /// locked (a claim, or a concurrent sweep); the caller computes the cutoff. How many it deleted.
    async fn sweep_unclaimed_attachments(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        older_than: DateTime<Utc>,
        limit: i64,
    ) -> Result<u64, RepoError>;
    /// The bytes of `uploader`'s unclaimed attachments.
    async fn unclaimed_bytes(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        uploader: &str,
    ) -> Result<i64, RepoError>;
}
