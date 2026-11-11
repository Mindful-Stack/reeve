use super::*;
use crate::domain::attachment::{Attachment, AttachmentOwner, ImageType};
use bytes::Bytes;
use uuid::Uuid;

const COLS: &str = "id, initiative_key, epic_key, story_key, uploaded_by, uploaded_at, filename, content_type, byte_size, sha256";
const UNCLAIMED: &str = "initiative_key IS NULL AND epic_key IS NULL AND story_key IS NULL";

/// The owner column of an item type: a static name, never formatted from input.
fn owner_column(item: ItemType) -> &'static str {
    match item {
        ItemType::Initiative => "initiative_key",
        ItemType::Epic => "epic_key",
        ItemType::Story => "story_key",
    }
}

fn attachment_from(r: &sqlx::postgres::PgRow) -> Result<Attachment, sqlx::Error> {
    let owner = [
        (ItemType::Initiative, "initiative_key"),
        (ItemType::Epic, "epic_key"),
        (ItemType::Story, "story_key"),
    ]
    .into_iter()
    .find_map(|(t, c)| r.get::<Option<String>, _>(c).map(|k| (t, k)));
    let owner = match owner {
        Some((t, k)) => AttachmentOwner::Item(t, key(&k)),
        None => AttachmentOwner::Unclaimed,
    };
    let ct: String = r.get("content_type");
    let sha: Vec<u8> = r.get("sha256");
    Ok(Attachment::from_persisted(
        r.get("id"),
        owner,
        r.get("uploaded_by"),
        r.get("uploaded_at"),
        r.get("filename"),
        ImageType::from_mime(&ct).ok_or_else(|| bad("content type", &ct))?,
        r.get("byte_size"),
        sha.try_into().map_err(|_| bad("sha256", "length"))?,
    ))
}

#[async_trait]
impl AttachmentRepo for PgPlanningRepo {
    async fn insert_attachment(
        &self,
        tx: Tx<'_, '_>,
        a: &Attachment,
        idempotency_key: Option<&str>,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query(
            "INSERT INTO attachments (id, uploaded_by, uploaded_at, filename, content_type, byte_size, sha256, idempotency_key) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING",
        )
        .bind(a.id())
        .bind(a.uploaded_by())
        .bind(a.uploaded_at())
        .bind(a.filename())
        .bind(a.image_type().mime())
        .bind(a.byte_size())
        .bind(a.sha256().to_vec())
        .bind(idempotency_key)
        .execute(&mut **tx)
        .await?;
        Ok(r.rows_affected() == 1)
    }
    async fn insert_chunk(
        &self,
        tx: Tx<'_, '_>,
        id: Uuid,
        n: i32,
        bytes: &[u8],
    ) -> Result<(), RepoError> {
        sqlx::query("INSERT INTO attachment_chunks (attachment_id, n, bytes) VALUES ($1,$2,$3)")
            .bind(id)
            .bind(n)
            .bind(bytes)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn find_attachment_by_idempotency(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<Attachment>, RepoError> {
        let r = sqlx::query(&format!(
            "SELECT {COLS} FROM attachments WHERE idempotency_key = $1"
        ))
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(r.as_ref().map(attachment_from).transpose()?)
    }
    async fn get_attachment(&self, id: Uuid) -> Result<Attachment, RepoError> {
        let r = sqlx::query(&format!("SELECT {COLS} FROM attachments WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(RepoError::NotFound)?;
        Ok(attachment_from(&r)?)
    }
    async fn attachment_chunk(&self, id: Uuid, n: i32) -> Result<Option<Bytes>, RepoError> {
        let b: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT bytes FROM attachment_chunks WHERE attachment_id = $1 AND n = $2",
        )
        .bind(id)
        .bind(n)
        .fetch_optional(&self.pool)
        .await?;
        Ok(b.map(Bytes::from))
    }
    async fn list_attachments_for(
        &self,
        item: ItemType,
        key: &Key,
    ) -> Result<Vec<Attachment>, RepoError> {
        let col = owner_column(item);
        let rows = sqlx::query(&format!(
            "SELECT {COLS} FROM attachments WHERE {col} = $1 ORDER BY uploaded_at, id"
        ))
        .bind(key.to_string())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(attachment_from).collect::<Result<_, _>>()?)
    }
    async fn claim_attachments(
        &self,
        tx: Tx<'_, '_>,
        ids: &[Uuid],
        actor: &str,
        item: ItemType,
        key: &Key,
    ) -> Result<ClaimOutcome, RepoError> {
        let col = owner_column(item);
        // One statement takes the row locks: a concurrent claim of the same id waits, then re-checks the WHERE against
        // the committed row and finds it owned (PostgreSQL, "Read Committed Isolation Level").
        let claimed: Vec<(Uuid, String)> = sqlx::query_as(&format!(
            "UPDATE attachments SET {col} = $1 WHERE id = ANY($2) AND uploaded_by = $3 AND {UNCLAIMED} RETURNING id, filename"
        ))
        .bind(key.to_string())
        .bind(ids)
        .bind(actor)
        .fetch_all(&mut **tx)
        .await?;
        let mut claimed = claimed;
        claimed.sort_by_key(|(c, _)| ids.iter().position(|i| i == c));
        let rest: Vec<Uuid> = ids
            .iter()
            .filter(|id| !claimed.iter().any(|(c, _)| c == *id))
            .copied()
            .collect();
        if !rest.is_empty() {
            // Left: ids already owned by this item (a no-op), ids that no longer exist (swept or deleted: ignored), and
            // anything else, which refuses the claim.
            let elsewhere: bool = sqlx::query_scalar(&format!(
                "SELECT EXISTS (SELECT 1 FROM attachments WHERE id = ANY($1) AND {col} IS DISTINCT FROM $2)"
            ))
            .bind(&rest)
            .bind(key.to_string())
            .fetch_one(&mut **tx)
            .await?;
            if elsewhere {
                return Ok(ClaimOutcome::Refused);
            }
        }
        Ok(ClaimOutcome::Claimed(claimed))
    }
    async fn delete_attachment(
        &self,
        tx: Tx<'_, '_>,
        id: Uuid,
    ) -> Result<Option<Attachment>, RepoError> {
        // The row as it was deleted: whether it was claimed is decided from this, inside the transaction, so a claim
        // that commits between a caller's read and this delete cannot lose its `attachment_removed` entry.
        let r = sqlx::query(&format!(
            "DELETE FROM attachments WHERE id = $1 RETURNING {COLS}"
        ))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(r.as_ref().map(attachment_from).transpose()?)
    }
    async fn sweep_unclaimed_attachments(
        &self,
        tx: Tx<'_, '_>,
        older_than: DateTime<Utc>,
        limit: i64,
    ) -> Result<u64, RepoError> {
        // A bounded batch, locked in id order and skipping locked rows: a sweep never waits on a claim or on another
        // sweep, and never deadlocks with one (PostgreSQL, "The Locking Clause", SKIP LOCKED).
        let r = sqlx::query(&format!(
            "DELETE FROM attachments WHERE id IN (SELECT id FROM attachments WHERE {UNCLAIMED} AND uploaded_at < $1 \
             ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED)"
        ))
        .bind(older_than)
        .bind(limit)
        .execute(&mut **tx)
        .await?;
        Ok(r.rows_affected())
    }
    async fn unclaimed_bytes(&self, tx: Tx<'_, '_>, uploader: &str) -> Result<i64, RepoError> {
        Ok(sqlx::query_scalar(&format!(
            "SELECT COALESCE(SUM(byte_size), 0)::bigint FROM attachments WHERE uploaded_by = $1 AND {UNCLAIMED}"
        ))
        .bind(uploader)
        .fetch_one(&mut **tx)
        .await?)
    }
}

/// Every claimed attachment, on the export's snapshot, ordered by upload time then id (`PlanningReads::attachments_in`).
pub(super) async fn claimed_in(tx: Tx<'_, '_>) -> Result<Vec<Attachment>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM attachments WHERE NOT ({UNCLAIMED}) ORDER BY uploaded_at, id"
    ))
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.iter().map(attachment_from).collect::<Result<_, _>>()?)
}
