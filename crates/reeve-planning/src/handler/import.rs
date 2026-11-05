//! The Jira import. Grandfathering is create-with-state, never an override: every item is inserted in
//! the state it had, the gate is never consulted (no `move_story`), and the only activity entry is `imported`. The
//! import is create-only and re-runnable: the external reference is the idempotency key, and a reference that already
//! exists answers `created: false` and writes nothing. A reference names one item across all three types: one already
//! imported under another type is refused (`Validation`, naming that item), and concurrent imports of one reference are
//! serialised by a transaction-scoped advisory lock on it, so two types can never both claim it.
use super::*;
use crate::domain::items::*;
use crate::ports::RepoError;
use reeve_protocol::{
    DeliveryState, ImportComment, ImportEpic, ImportInitiative, ImportStory, Imported, ItemType,
};
use sqlx::{Postgres, Transaction};

/// A non-empty reference of at most 200 characters, trimmed.
fn check_ref(r: &str) -> Result<String, HandlerError> {
    let r = r.trim();
    if r.is_empty() || r.chars().count() > 200 {
        return Err(HandlerError::Validation(
            "external_ref must be 1..=200 characters".into(),
        ));
    }
    Ok(r.to_string())
}

/// An item's reference: `check_ref`, then the `KeyRef` grammar (`JPD-77`), and never in this instance's own key space
/// (`ABC-3` on a `ABC` instance would be shadowed by the real key, which `resolve_ref` checks first).
pub(super) fn check_item_ref(r: &str, prefix: &str) -> Result<String, HandlerError> {
    let r = check_ref(r)?;
    if r.parse::<reeve_protocol::KeyRef>().is_err() {
        return Err(HandlerError::Validation(format!(
            "external_ref {r:?} is not a reference like JPD-77"
        )));
    }
    if r.starts_with(&format!("{prefix}-")) {
        return Err(HandlerError::Validation(format!(
            "external_ref {r:?} is in this instance's own key space ({prefix}-)"
        )));
    }
    Ok(r)
}

/// Trimmed; blank is refused, as `comment_story` refuses a blank body.
fn non_blank(what: &str, s: &str) -> Result<String, HandlerError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(HandlerError::Validation(format!("empty comment {what}")));
    }
    Ok(s.to_string())
}

/// A `created` entry's payload, naming the item's origin when it records one.
pub(super) fn with_ref(mut payload: serde_json::Value, r: Option<&str>) -> serde_json::Value {
    if let Some(r) = r {
        payload["external_ref"] = r.into();
    }
    payload
}

/// What `claim_ref` found: the (validated) reference is free to record, or a retry of this create already holds it.
pub(super) enum Claim {
    Free(Option<String>),
    Retried(Key),
}

fn existing(key: Key) -> Imported {
    Imported {
        key: Some(key),
        id: None,
        created: false,
    }
}

/// True when an insert failed on a unique index (a concurrent import of the same reference won on `external_ref`).
fn unique_violation(e: &RepoError) -> bool {
    matches!(e, RepoError::Db(e) if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505"))
}

impl Handlers {
    pub async fn import_initiative(
        &self,
        c: &Ctx,
        i: ImportInitiative,
    ) -> Result<Imported, HandlerError> {
        let r = check_item_ref(&i.external_ref, &self.settings.key_prefix)?;
        let owner = i.owner.map(|e| reeve_protocol::normalise_email(&e));
        // Validated before anything is looked up, so a malformed row is refused even on a re-run.
        let probe = Initiative::new(
            Key::new(&self.settings.key_prefix, 1).expect("prefix validated by config"),
            &i.title,
            i.brief.as_deref().unwrap_or(""),
            &i.horizon,
            &self.settings.horizon_buckets,
            i.target_month.clone(),
            owner.clone(),
        )?;
        let mut tx = self.pool.begin().await?;
        if let Some(k) = self.imported_as(&mut tx, ItemType::Initiative, &r).await? {
            tx.rollback().await?;
            return Ok(existing(k));
        }
        let key = self
            .repo
            .next_key(&mut tx, &self.settings.key_prefix)
            .await?;
        let item = Initiative::from_persisted(
            key.clone(),
            probe.title().into(),
            probe.brief().into(),
            probe.horizon().into(),
            probe.target_month().map(String::from),
            probe.owner().map(String::from),
            self.mark(c, i.discovery_complete),
            Some(r.clone()),
            1,
        );
        let inserted = self.repo.insert_initiative(&mut tx, &item, &r, c.now).await;
        self.finish(c, tx, inserted, ItemType::Initiative, "initiative", key, &r)
            .await
    }

    pub async fn import_epic(&self, c: &Ctx, e: ImportEpic) -> Result<Imported, HandlerError> {
        let r = check_item_ref(&e.external_ref, &self.settings.key_prefix)?;
        let probe = Epic::new(
            Key::new(&self.settings.key_prefix, 1).expect("prefix validated by config"),
            &e.title,
            e.initiative.clone(),
            e.brief.as_deref().unwrap_or(""),
        )?;
        if let Some(ik) = &e.initiative {
            self.repo.get_initiative(ik).await?;
        }
        let mut tx = self.pool.begin().await?;
        if let Some(k) = self.imported_as(&mut tx, ItemType::Epic, &r).await? {
            tx.rollback().await?;
            return Ok(existing(k));
        }
        let key = self
            .repo
            .next_key(&mut tx, &self.settings.key_prefix)
            .await?;
        let item = Epic::from_persisted(
            key.clone(),
            probe.title().into(),
            probe.initiative().cloned(),
            probe.brief().into(),
            self.mark(c, e.discovery_complete),
            Some(r.clone()),
            1,
        );
        let inserted = self.repo.insert_epic(&mut tx, &item, &r, c.now).await;
        self.finish(c, tx, inserted, ItemType::Epic, "epic", key, &r)
            .await
    }

    pub async fn import_story(&self, c: &Ctx, s: ImportStory) -> Result<Imported, HandlerError> {
        let r = check_item_ref(&s.external_ref, &self.settings.key_prefix)?;
        if (s.delivery == DeliveryState::Done) != s.resolution.is_some() {
            return Err(HandlerError::Validation(
                "a resolution is given exactly when the story is done".into(),
            ));
        }
        let probe = Story::new(
            Key::new(&self.settings.key_prefix, 1).expect("prefix validated by config"),
            &s.title,
            s.kind,
            s.epic.clone(),
            s.description.as_deref().unwrap_or(""),
            s.criteria.iter().map(|x| x.text.clone()).collect(),
            s.owner.map(|e| reeve_protocol::normalise_email(&e)),
            s.labels,
        )?;
        if let Some(ek) = &s.epic {
            self.repo.get_epic(ek).await?;
        }
        let mut tx = self.pool.begin().await?;
        if let Some(k) = self.imported_as(&mut tx, ItemType::Story, &r).await? {
            tx.rollback().await?;
            return Ok(existing(k));
        }
        let key = self
            .repo
            .next_key(&mut tx, &self.settings.key_prefix)
            .await?;
        let criteria = probe
            .criteria()
            .iter()
            .zip(&s.criteria)
            .map(|(x, given)| Criterion {
                text: x.text.clone(),
                done: given.done,
            })
            .collect();
        let item = Story::from_persisted(
            key.clone(),
            probe.title().into(),
            probe.kind(),
            probe.epic().cloned(),
            probe.description().into(),
            criteria,
            probe.owner().map(String::from),
            probe.labels().to_vec(),
            probe.rank(),
            s.delivery,
            s.resolution,
            Some(r.clone()),
            1,
        );
        let inserted = self.repo.insert_story(&mut tx, &item, &r, c.now).await;
        self.finish(c, tx, inserted, ItemType::Story, "story", key, &r)
            .await
    }

    /// The comment keeps the Jira author and date; the activity entry is dated by the import (`c.now`).
    ///
    /// Unlike the items, the comment's reference is NOT passed as the idempotency key: a caller may use any string as
    /// a comment idempotency key (`jira-comment:900` included), and a reference matching one must never answer
    /// `created: false` with that caller's comment. The `external_ref` unique index alone makes the import
    /// re-runnable; a concurrent import of the same reference waits on it and finds the winner by reference.
    pub async fn import_comment(
        &self,
        c: &Ctx,
        m: ImportComment,
    ) -> Result<Imported, HandlerError> {
        let r = check_ref(&m.external_ref)?;
        let author = non_blank("author", &m.author)?;
        let body = non_blank("body", &m.body)?;
        self.repo.get_story(&m.story).await?;
        let mut tx = self.pool.begin().await?;
        let (id, inserted) = self
            .repo
            .insert_comment(
                &mut tx,
                Some(&m.story),
                None,
                &author,
                &body,
                Some(&r),
                None,
                m.at,
            )
            .await?;
        if !inserted {
            tx.rollback().await?;
            return Ok(Imported {
                key: None,
                id: Some(id),
                created: false,
            });
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "story",
                    &m.story.to_string(),
                    "imported",
                    serde_json::json!({"external_ref": r, "comment_id": id}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(Imported {
            key: None,
            id: Some(id),
            created: true,
        })
    }

    /// The key of the item already imported under this reference as `item` (a re-run: `created: false`). A reference
    /// is one Jira issue, so it names at most one item across all three types: the same reference already imported
    /// under another type is refused, naming that item, and never creates a second item with it.
    ///
    /// Read under the reference's transaction-scoped lock (`ref_holder`): two imports of one reference under different
    /// types (no unique index spans the three tables) are serialised, and the second sees the first's committed item.
    /// A same-type race with a writer that holds no lock is still caught by the unique index (`finish`).
    async fn imported_as(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        r: &str,
    ) -> Result<Option<Key>, HandlerError> {
        match self.ref_holder(tx, r).await? {
            None => Ok(None),
            Some((other, k)) if other == item => Ok(Some(k)),
            Some((other, k)) => Err(HandlerError::Validation(format!(
                "{r} is already imported as {} {k}",
                item_name(other)
            ))),
        }
    }

    /// For a create (not an import) that records its origin: the reference is validated like an imported one and must
    /// name no item yet, of any type, so a later import of it finds this item. A create overlapping its own retry waits
    /// here on the reference's lock and then finds the reference held by the item its idempotency key already made:
    /// that answers `Claim::Retried` with that item, not a refusal.
    pub(super) async fn claim_ref(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        idem: &str,
        r: Option<String>,
    ) -> Result<Claim, HandlerError> {
        let Some(r) = r else {
            return Ok(Claim::Free(None));
        };
        let r = check_item_ref(&r, &self.settings.key_prefix)?;
        if let Some((other, k)) = self.ref_holder(tx, &r).await? {
            if other == item
                && self
                    .repo
                    .find_by_idempotency_in(tx, item, idem)
                    .await?
                    .as_ref()
                    == Some(&k)
            {
                return Ok(Claim::Retried(k));
            }
            return Err(HandlerError::Validation(format!(
                "{r} is already the external reference of {} {k}",
                item_name(other)
            )));
        }
        Ok(Claim::Free(Some(r)))
    }

    /// For an edit of `key`, whose reference is `cur`: the reference it keeps, and the one it newly records (for the
    /// `edited` entry). An item without one claims `given` as a create does; the reference it already has again is no
    /// change; another one is refused, because the importer finds the item by it and a changed reference would
    /// let a later import create a second item.
    pub(super) async fn set_ref(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
        cur: Option<&str>,
        given: Option<String>,
    ) -> Result<(Option<String>, Option<String>), HandlerError> {
        let keep = cur.map(String::from);
        let Some(given) = given else {
            return Ok((keep, None));
        };
        let r = check_item_ref(&given, &self.settings.key_prefix)?;
        match cur {
            Some(c) if c == r => Ok((keep, None)),
            Some(c) => Err(HandlerError::Validation(format!(
                "{key} already has external reference {c}; it cannot be changed"
            ))),
            None => {
                if let Some((other, k)) = self.ref_holder(tx, &r).await? {
                    return Err(HandlerError::Validation(format!(
                        "{r} is already the external reference of {} {k}",
                        item_name(other)
                    )));
                }
                Ok((Some(r.clone()), Some(r)))
            }
        }
    }

    /// The item holding this reference, of any type, read under the reference's transaction-scoped lock, taken first.
    /// Every writer of a reference (import or create) takes this lock before the key sequence and the activity lock,
    /// so the lock order is the same on every path.
    async fn ref_holder(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        r: &str,
    ) -> Result<Option<(ItemType, Key)>, HandlerError> {
        self.repo.lock_external_ref(tx, r).await?;
        for item in [ItemType::Initiative, ItemType::Epic, ItemType::Story] {
            if let Some(k) = self.repo.find_by_external_ref_in(tx, item, r).await? {
                return Ok(Some((item, k)));
            }
        }
        Ok(None)
    }

    fn mark(&self, c: &Ctx, complete: bool) -> Option<DiscoveryMark> {
        complete.then(|| DiscoveryMark {
            at: c.now,
            by: c.actor.clone(),
        })
    }

    /// Records the one `imported` entry and commits; an insert that lost a race to the same reference (or to the same
    /// idempotency key) rolls back, including the key the sequence handed out, and answers with the winner.
    async fn finish(
        &self,
        c: &Ctx,
        mut tx: Transaction<'_, Postgres>,
        inserted: Result<bool, RepoError>,
        item: ItemType,
        entity: &'static str,
        key: Key,
        r: &str,
    ) -> Result<Imported, HandlerError> {
        let won = match inserted {
            Ok(won) => won,
            Err(e) if unique_violation(&e) => false,
            Err(e) => return Err(e.into()),
        };
        if !won {
            tx.rollback().await?;
            return match self.repo.find_by_external_ref(item, r).await? {
                Some(k) => Ok(existing(k)),
                None => Err(HandlerError::Validation(format!(
                    "{r:?} is already the idempotency key of a {entity} that was not imported"
                ))),
            };
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    entity,
                    &key.to_string(),
                    "imported",
                    serde_json::json!({"external_ref": r}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(Imported {
            key: Some(key),
            id: None,
            created: true,
        })
    }
}
