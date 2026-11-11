use super::dto::urgent_fields;
use super::import::{Claim, with_ref};
use super::*;
use crate::domain::{items::*, progress::progress, urgency::Urgency};
use reeve_protocol::{InitiativeDto, ItemType, Key};
use sqlx::{Postgres, Transaction};

/// The one place an `InitiativeDto` is filled: its progress and its urgency.
pub(super) fn initiative_dto_of(
    i: &Initiative,
    counts: &StoryCounts,
    urg: &Urgency,
) -> InitiativeDto {
    let mut dto = InitiativeDto::from(i);
    dto.progress = progress(counts.completed, counts.dropped, counts.total);
    (dto.urgent_mark, dto.urgent_sources) = urgent_fields(urg, i.key());
    dto
}

impl Handlers {
    /// Idempotent under overlap: the insert is `ON CONFLICT (idempotency_key) DO NOTHING`; when it inserts nothing the
    /// transaction (including the key the sequence handed out) rolls back and the existing item is returned without a second activity entry.
    pub async fn create_initiative(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        brief: &str,
        horizon: &str,
        target_month: Option<String>,
        owner: Option<String>,
        external_ref: Option<String>,
    ) -> Result<Initiative, HandlerError> {
        self.create_initiative_attaching(
            c,
            idem,
            title,
            brief,
            horizon,
            target_month,
            owner,
            external_ref,
            &[],
        )
        .await
    }

    /// `create_initiative`, claiming `attachments` for the new initiative in its transaction (kb/ADR-0022 R2).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_initiative_attaching(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        brief: &str,
        horizon: &str,
        target_month: Option<String>,
        owner: Option<String>,
        external_ref: Option<String>,
        attachments: &[Uuid],
    ) -> Result<Initiative, HandlerError> {
        self.create_initiative_reporting(
            c,
            idem,
            title,
            brief,
            horizon,
            target_month,
            owner,
            external_ref,
            attachments,
        )
        .await
        .map(|(x, _)| x)
    }

    /// `create_initiative`, and whether this call inserted it (false when its idempotency key already had one).
    pub(crate) async fn create_initiative_reporting(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        brief: &str,
        horizon: &str,
        target_month: Option<String>,
        owner: Option<String>,
        external_ref: Option<String>,
        attachments: &[Uuid],
    ) -> Result<(Initiative, bool), HandlerError> {
        let attachments = attachment_ids(attachments)?;
        if let Some(k) = self
            .ports
            .keys
            .find_by_idempotency(ItemType::Initiative, idem)
            .await?
        {
            return Ok((self.ports.initiatives.get_initiative(&k).await?, false));
        }
        let mut tx = self.pool.begin().await?;
        let r = match self
            .claim_ref(&mut tx, ItemType::Initiative, idem, external_ref)
            .await?
        {
            Claim::Free(r) => r,
            Claim::Retried(k) => {
                tx.rollback().await?;
                return Ok((self.ports.initiatives.get_initiative(&k).await?, false));
            }
        };
        let key = self
            .ports
            .keys
            .next_key(&mut tx, &self.settings.key_prefix)
            .await?;
        let i = Initiative::new(
            key,
            title,
            brief,
            horizon,
            &self.settings.horizon_buckets,
            target_month,
            owner.map(|e| reeve_protocol::normalise_email(&e)),
        )?
        .with_external_ref(r);
        if !self
            .ports
            .initiatives
            .insert_initiative(&mut tx, &i, idem, c.now)
            .await?
        {
            tx.rollback().await?;
            let k = self
                .ports
                .keys
                .find_by_idempotency(ItemType::Initiative, idem)
                .await?
                .ok_or(HandlerError::NotFound)?;
            return Ok((self.ports.initiatives.get_initiative(&k).await?, false));
        }
        self.claim_attachments(&mut tx, c, &attachments, ItemType::Initiative, i.key())
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "initiative",
                    &i.key().to_string(),
                    "created",
                    with_ref(serde_json::json!({"title": i.title()}), i.external_ref()),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok((i, true))
    }

    pub async fn edit_initiative(
        &self,
        c: &Ctx,
        key: &Key,
        version: i64,
        title: Option<String>,
        brief: Option<String>,
        horizon: Option<String>,
        target_month: Option<Option<String>>,
        owner: Option<Option<String>>,
        external_ref: Option<String>,
    ) -> Result<Initiative, HandlerError> {
        self.edit_initiative_attaching(
            c,
            key,
            version,
            title,
            brief,
            horizon,
            target_month,
            owner,
            external_ref,
            &[],
        )
        .await
    }

    /// `edit_initiative`, claiming `attachments` for the initiative in its transaction (kb/ADR-0022 R2).
    #[allow(clippy::too_many_arguments)]
    pub async fn edit_initiative_attaching(
        &self,
        c: &Ctx,
        key: &Key,
        version: i64,
        title: Option<String>,
        brief: Option<String>,
        horizon: Option<String>,
        target_month: Option<Option<String>>,
        owner: Option<Option<String>>,
        external_ref: Option<String>,
        attachments: &[Uuid],
    ) -> Result<Initiative, HandlerError> {
        let attachments = attachment_ids(attachments)?;
        let cur = self.ports.initiatives.get_initiative(key).await?;
        let new_horizon = horizon.as_deref().unwrap_or(cur.horizon());
        check_horizon_change(cur.horizon(), new_horizon, &self.settings.horizon_buckets)?;
        // Validated above, so the constructor sees the chosen horizon as its only bucket.
        let chosen = [new_horizon.to_string()];
        let next = Initiative::new(
            cur.key().clone(),
            title.as_deref().unwrap_or(cur.title()),
            brief.as_deref().unwrap_or(cur.brief()),
            new_horizon,
            &chosen,
            target_month.unwrap_or(cur.target_month().map(String::from)),
            owner
                .map(|o| o.map(|e| reeve_protocol::normalise_email(&e)))
                .unwrap_or(cur.owner().map(String::from)),
        )?;
        let mut tx = self.pool.begin().await?;
        let (r, set) = self
            .set_ref(&mut tx, key, cur.external_ref(), external_ref)
            .await?;
        let next = Initiative::from_persisted(
            next.key().clone(),
            next.title().into(),
            next.brief().into(),
            next.horizon().into(),
            next.target_month().map(String::from),
            next.owner().map(String::from),
            cur.discovery().cloned(),
            r,
            cur.version(),
        );
        self.ports
            .initiatives
            .update_initiative(&mut tx, &next, version, c.now)
            .await?;
        self.claim_attachments(&mut tx, c, &attachments, ItemType::Initiative, key)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "initiative",
                    &key.to_string(),
                    "edited",
                    with_ref(serde_json::json!({}), set.as_deref()),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.ports.initiatives.get_initiative(key).await?)
    }

    /// One initiative with its progress and urgency, all from one `REPEATABLE READ READ ONLY` snapshot.
    pub async fn initiative_dto(
        &self,
        key: &Key,
        now: DateTime<Utc>,
    ) -> Result<InitiativeDto, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let urg = self.urgency_in(&mut tx, now).await?;
            self.initiative_dto_in(&mut tx, key, &urg).await
        }
        .await;
        end_read(tx, out).await
    }

    /// `initiative_dto` on the caller's transaction: the initiative and its story counts from its snapshot, with the
    /// urgency read on that same snapshot.
    pub(crate) async fn initiative_dto_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
        urg: &Urgency,
    ) -> Result<InitiativeDto, HandlerError> {
        let i = self.ports.initiatives.get_initiative_in(tx, key).await?;
        let counts = self
            .ports
            .reads
            .story_counts_under_initiative_in(tx, key)
            .await?;
        Ok(initiative_dto_of(&i, &counts, urg))
    }

    /// Every initiative with its progress and urgency, from one `REPEATABLE READ READ ONLY` snapshot in a fixed number
    /// of statements.
    pub async fn list_initiatives(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<InitiativeDto>, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let urg = self.urgency_in(&mut tx, now).await?;
            let initiatives = self.ports.initiatives.list_initiatives_in(&mut tx).await?;
            let counts = self
                .ports
                .reads
                .story_counts_by_initiative_in(&mut tx)
                .await?;
            Ok(initiatives
                .iter()
                .map(|i| {
                    let c = counts.get(i.key()).copied().unwrap_or_default();
                    initiative_dto_of(i, &c, &urg)
                })
                .collect())
        }
        .await;
        end_read(tx, out).await
    }

    /// Advisory findings on an initiative or an epic (`domain::findings`), from one read-only snapshot: the item, the
    /// required decisions on it and above it, and the stories under it. Never consulted by a move.
    pub async fn findings(
        &self,
        item: ItemType,
        key: &Key,
    ) -> Result<Vec<reeve_protocol::Finding>, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let r = self.findings_in(&mut tx, item, key).await;
        let rb = tx.rollback().await;
        let v = r?;
        rb?;
        Ok(v)
    }

    async fn findings_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        item: ItemType,
        key: &Key,
    ) -> Result<Vec<reeve_protocol::Finding>, HandlerError> {
        use crate::domain::findings::{FindingsInput, compute_findings};
        let (brief, discovery_complete, above, stories) = match item {
            ItemType::Initiative => {
                let i = self.ports.initiatives.get_initiative_in(tx, key).await?;
                let counts = self
                    .ports
                    .reads
                    .story_counts_under_initiative_in(tx, key)
                    .await?;
                (
                    i.brief().to_string(),
                    i.discovery().is_some(),
                    None,
                    counts.total,
                )
            }
            ItemType::Epic => {
                let e = self.ports.epics.get_epic_in(tx, key).await?;
                let filter = crate::ports::StoryFilter {
                    epic: Some(key.clone()),
                    ..Default::default()
                };
                let stories = self
                    .ports
                    .stories
                    .list_stories(tx, &filter, crate::ports::DoneWindow::All)
                    .await?
                    .len() as u32;
                (
                    e.brief().to_string(),
                    e.discovery().is_some(),
                    e.initiative().cloned(),
                    stories,
                )
            }
            ItemType::Story => return Err(HandlerError::NotFound),
        };
        let own = self.ports.decisions.list_decisions_on_in(tx, key).await?;
        let ancestor_has_decision = match &above {
            Some(ik) => !self
                .ports
                .decisions
                .list_decisions_on_in(tx, ik)
                .await?
                .is_empty(),
            None => false,
        };
        Ok(compute_findings(&FindingsInput {
            brief: &brief,
            discovery_complete,
            own_decisions: &own,
            ancestor_has_decision,
            stories,
        }))
    }

    /// Discovery mark on an initiative or an epic; clearing it is logged like setting it.
    pub async fn mark_discovery(
        &self,
        c: &Ctx,
        item: ItemType,
        key: &Key,
        version: i64,
        complete: bool,
    ) -> Result<(), HandlerError> {
        // Reads and validation happen before the transaction so no connection is held while the pool is used.
        enum Cur {
            I(Initiative),
            E(Epic),
        }
        let (cur, entity) = match item {
            ItemType::Initiative => (
                Cur::I(self.ports.initiatives.get_initiative(key).await?),
                "initiative",
            ),
            ItemType::Epic => (Cur::E(self.ports.epics.get_epic(key).await?), "epic"),
            ItemType::Story => {
                return Err(HandlerError::Validation(
                    "stories have no discovery mark".into(),
                ));
            }
        };
        let (already, current) = match &cur {
            Cur::I(i) => (i.discovery().is_some(), i.version()),
            Cur::E(e) => (e.discovery().is_some(), e.version()),
        };
        // A stale version is a conflict like every other edit, even when the mark would not change.
        if version != current {
            return Err(HandlerError::Conflict { current });
        }
        if complete == already {
            return Ok(()); // the first mark stands; reopening an unmarked item is a no-op
        }
        let mark = if complete {
            Some(DiscoveryMark {
                at: c.now,
                by: c.actor.clone(),
            })
        } else {
            None
        };
        let mut tx = self.pool.begin().await?;
        match cur {
            Cur::I(i) => {
                let n = Initiative::from_persisted(
                    i.key().clone(),
                    i.title().into(),
                    i.brief().into(),
                    i.horizon().into(),
                    i.target_month().map(String::from),
                    i.owner().map(String::from),
                    mark,
                    i.external_ref().map(String::from),
                    i.version(),
                );
                self.ports
                    .initiatives
                    .update_initiative(&mut tx, &n, version, c.now)
                    .await?;
            }
            Cur::E(e) => {
                let n = Epic::from_persisted(
                    e.key().clone(),
                    e.title().into(),
                    e.initiative().cloned(),
                    e.brief().into(),
                    mark,
                    e.external_ref().map(String::from),
                    e.version(),
                );
                self.ports
                    .epics
                    .update_epic(&mut tx, &n, version, c.now)
                    .await?;
            }
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    entity,
                    &key.to_string(),
                    if complete {
                        "discovery_marked"
                    } else {
                        "discovery_reopened"
                    },
                    serde_json::json!({"item_type": entity, "complete": complete}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
