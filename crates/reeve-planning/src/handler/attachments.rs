//! Attachments (kb/ADR-0022, kb/ADR-0023): uploading, reading, listing and deleting them, and claiming them for an item
//! inside the write that saves the text showing them.
use super::*;
use crate::domain::attachment::{
    Attachment, AttachmentOwner, ImageType, MAX_BYTES, UploadError, chunks, strip_metadata,
};
use bytes::Bytes;

/// The most attachments one write may claim.
pub const MAX_CLAIM: usize = 50;
/// How long an upload may stay unclaimed before the sweep takes it (kb/ADR-0022 R4).
const UNCLAIMED_DAYS: i64 = 7;
/// The longest upload idempotency key.
const MAX_KEY: usize = 200;

/// A write's attachment ids, deduplicated and sorted (so two writes claiming the same set lock rows in one order),
/// and at most `MAX_CLAIM` of them.
pub(super) fn attachment_ids(ids: &[Uuid]) -> Result<Vec<Uuid>, HandlerError> {
    let mut v = ids.to_vec();
    v.sort();
    v.dedup();
    if v.len() > MAX_CLAIM {
        return Err(HandlerError::Validation(format!(
            "at most {MAX_CLAIM} images can be saved at once"
        )));
    }
    Ok(v)
}

fn upload_error(e: UploadError) -> HandlerError {
    HandlerError::Validation(e.to_string())
}

impl Handlers {
    /// Stores an upload, unclaimed: size, then type by its bytes, then metadata removed, then the chunks, after the
    /// sweep, in one transaction. A repeated idempotency key returns the first upload (to its uploader only) and
    /// stores nothing.
    pub async fn upload_attachment(
        &self,
        c: &Ctx,
        filename: &str,
        idem: Option<&str>,
        body: Bytes,
    ) -> Result<Attachment, HandlerError> {
        // The sweep runs for every upload attempt, accepted or not, in a transaction of its own (kb/ADR-0022 R4).
        let mut sweep = self.pool.begin().await?;
        self.ports
            .attachments
            .sweep_unclaimed_attachments(&mut sweep, c.now - chrono::Duration::days(UNCLAIMED_DAYS))
            .await?;
        sweep.commit().await?;
        if body.len() > MAX_BYTES {
            return Err(upload_error(UploadError::TooLarge));
        }
        // An empty key is no key; a long one would not fit the index.
        let idem = idem.filter(|k| !k.is_empty());
        if idem.is_some_and(|k| k.chars().count() > MAX_KEY) {
            return Err(HandlerError::Validation(format!(
                "idempotency_key must be at most {MAX_KEY} characters"
            )));
        }
        if let Some(k) = idem
            && let Some(a) = self
                .ports
                .attachments
                .find_attachment_by_idempotency(k)
                .await?
        {
            return self.visible(a, &c.actor);
        }
        let t = ImageType::sniff(&body).ok_or_else(|| upload_error(UploadError::Unsupported))?;
        let stored = strip_metadata(t, body).map_err(upload_error)?;
        let a = Attachment::new(Uuid::new_v4(), &c.actor, c.now, filename, t, &stored);
        let mut tx = self.pool.begin().await?;
        if !self
            .ports
            .attachments
            .insert_attachment(&mut tx, &a, idem)
            .await?
        {
            // A concurrent request with the same key stored first.
            tx.rollback().await?;
            let first = self
                .ports
                .attachments
                .find_attachment_by_idempotency(idem.unwrap_or_default())
                .await?
                .ok_or(HandlerError::NotFound)?;
            return self.visible(first, &c.actor);
        }
        for (n, part) in chunks(&stored).enumerate() {
            self.ports
                .attachments
                .insert_chunk(&mut tx, a.id(), n as i32, part)
                .await?;
        }
        tx.commit().await?;
        Ok(a)
    }

    /// kb/ADR-0023 R6: anyone sees a claimed attachment, only its uploader an unclaimed one; anyone else is told it does
    /// not exist.
    fn visible(&self, a: Attachment, viewer: &str) -> Result<Attachment, HandlerError> {
        if a.visible_to(viewer) {
            Ok(a)
        } else {
            Err(HandlerError::NotFound)
        }
    }

    /// An attachment's metadata, if `viewer` may see it.
    pub async fn attachment_meta(
        &self,
        id: Uuid,
        viewer: &str,
    ) -> Result<Attachment, HandlerError> {
        let a = self.ports.attachments.get_attachment(id).await?;
        self.visible(a, viewer)
    }

    /// One stored chunk, read by its own query (kb/ADR-0022 R3). Callers check `attachment_meta` first.
    pub async fn attachment_chunk(&self, id: Uuid, n: i32) -> Result<Option<Bytes>, HandlerError> {
        Ok(self.ports.attachments.attachment_chunk(id, n).await?)
    }

    /// An item's attachments, oldest first; NotFound for an unknown item.
    pub async fn list_attachments(
        &self,
        item: ItemType,
        key: &Key,
    ) -> Result<Vec<Attachment>, HandlerError> {
        match item {
            ItemType::Initiative => {
                self.ports.initiatives.get_initiative(key).await?;
            }
            ItemType::Epic => {
                self.ports.epics.get_epic(key).await?;
            }
            ItemType::Story => {
                self.ports.stories.get_story(key).await?;
            }
        }
        Ok(self
            .ports
            .attachments
            .list_attachments_for(item, key)
            .await?)
    }

    /// Hard-deletes an attachment the caller may see (kb/ADR-0023 R7). Whether it was claimed, and so whether an
    /// `attachment_removed` entry is written on its owner, is read from the deleted row itself.
    pub async fn delete_attachment(&self, c: &Ctx, id: Uuid) -> Result<(), HandlerError> {
        self.attachment_meta(id, &c.actor).await?;
        let mut tx = self.pool.begin().await?;
        let Some(gone) = self
            .ports
            .attachments
            .delete_attachment(&mut tx, id)
            .await?
        else {
            tx.rollback().await?;
            return Err(HandlerError::NotFound);
        };
        match gone.owner() {
            AttachmentOwner::Item(item, key) => {
                self.activity
                    .record(
                        &mut tx,
                        self.rec(
                            c,
                            item_name(*item),
                            &key.to_string(),
                            "attachment_removed",
                            serde_json::json!({"attachment_id": id, "filename": gone.filename()}),
                        ),
                    )
                    .await?;
            }
            // Claimed by nobody: only its uploader could see it, and it is not logged (kb/ADR-0023 R7).
            AttachmentOwner::Unclaimed if gone.uploaded_by() == c.actor => {}
            AttachmentOwner::Unclaimed => {
                tx.rollback().await?;
                return Err(HandlerError::NotFound);
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Inside the caller's write, after its row is written and before its own entry: claims `ids` for the item, or
    /// refuses the whole write (the caller's transaction is dropped, so it rolls back). One `attachment_added` entry
    /// per newly claimed attachment, in the order asked.
    pub(super) async fn claim_attachments(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        ids: &[Uuid],
        item: ItemType,
        key: &Key,
    ) -> Result<(), HandlerError> {
        if ids.is_empty() {
            return Ok(());
        }
        match self
            .ports
            .attachments
            .claim_attachments(tx, ids, &c.actor, item, key)
            .await?
        {
            ClaimOutcome::Refused => Err(HandlerError::Validation(
                "an image in this text is already attached to another item; remove it and add it again".into(),
            )),
            ClaimOutcome::Claimed(rows) => {
                for (id, filename) in rows {
                    self.activity
                        .record(
                            tx,
                            self.rec(
                                c,
                                item_name(item),
                                &key.to_string(),
                                "attachment_added",
                                serde_json::json!({"attachment_id": id, "filename": filename}),
                            ),
                        )
                        .await?;
                }
                Ok(())
            }
        }
    }
}
