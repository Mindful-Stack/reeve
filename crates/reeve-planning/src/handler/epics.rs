use super::import::{Claim, with_ref};
use super::urgent::urgent_fields;
use super::*;
use crate::domain::{items::*, urgency::Urgency};
use reeve_protocol::{EpicDto, ItemType, Key};

impl Handlers {
    pub async fn create_epic(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        initiative: Option<Key>,
        brief: &str,
        external_ref: Option<String>,
    ) -> Result<Epic, HandlerError> {
        self.create_epic_reporting(c, idem, title, initiative, brief, external_ref)
            .await
            .map(|(x, _)| x)
    }

    /// `create_epic`, and whether this call inserted it (false when its idempotency key already had one).
    pub(crate) async fn create_epic_reporting(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        initiative: Option<Key>,
        brief: &str,
        external_ref: Option<String>,
    ) -> Result<(Epic, bool), HandlerError> {
        if let Some(k) = self.repo.find_by_idempotency(ItemType::Epic, idem).await? {
            return Ok((self.repo.get_epic(&k).await?, false));
        }
        if let Some(ik) = &initiative {
            self.repo.get_initiative(ik).await?;
        }
        let mut tx = self.pool.begin().await?;
        let r = match self
            .claim_ref(&mut tx, ItemType::Epic, idem, external_ref)
            .await?
        {
            Claim::Free(r) => r,
            Claim::Retried(k) => {
                tx.rollback().await?;
                return Ok((self.repo.get_epic(&k).await?, false));
            }
        };
        let key = self
            .repo
            .next_key(&mut tx, &self.settings.key_prefix)
            .await?;
        let e = Epic::new(key, title, initiative, brief)?.with_external_ref(r);
        if !self.repo.insert_epic(&mut tx, &e, idem, c.now).await? {
            tx.rollback().await?;
            let k = self
                .repo
                .find_by_idempotency(ItemType::Epic, idem)
                .await?
                .ok_or(HandlerError::NotFound)?;
            return Ok((self.repo.get_epic(&k).await?, false));
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "epic",
                    &e.key().to_string(),
                    "created",
                    with_ref(serde_json::json!({"title": e.title()}), e.external_ref()),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok((e, true))
    }

    pub async fn edit_epic(
        &self,
        c: &Ctx,
        key: &Key,
        version: i64,
        title: Option<String>,
        brief: Option<String>,
        initiative: Option<Option<Key>>,
        external_ref: Option<String>,
    ) -> Result<Epic, HandlerError> {
        let cur = self.repo.get_epic(key).await?;
        let ini = initiative.unwrap_or(cur.initiative().cloned());
        if let Some(ik) = &ini {
            self.repo.get_initiative(ik).await?;
        }
        let next = Epic::new(
            cur.key().clone(),
            title.as_deref().unwrap_or(cur.title()),
            ini,
            brief.as_deref().unwrap_or(cur.brief()),
        )?;
        let mut tx = self.pool.begin().await?;
        let (r, set) = self
            .set_ref(&mut tx, key, cur.external_ref(), external_ref)
            .await?;
        let next = Epic::from_persisted(
            next.key().clone(),
            next.title().into(),
            next.initiative().cloned(),
            next.brief().into(),
            cur.discovery().cloned(),
            r,
            cur.version(),
        );
        self.repo
            .update_epic(&mut tx, &next, version, c.now)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "epic",
                    &key.to_string(),
                    "edited",
                    with_ref(serde_json::json!({}), set.as_deref()),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.repo.get_epic(key).await?)
    }

    /// One epic with its urgency, from one `REPEATABLE READ READ ONLY` snapshot.
    pub async fn epic_dto(&self, key: &Key, now: DateTime<Utc>) -> Result<EpicDto, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let urg = self.urgency_in(&mut tx, now).await?;
            let e = self.repo.get_epic_in(&mut tx, key).await?;
            Ok(epic_dto_of(&e, &urg))
        }
        .await;
        end_read(tx, out).await
    }

    /// Every epic (of one initiative when given) with its urgency, from one `REPEATABLE READ READ ONLY` snapshot in a
    /// fixed number of statements.
    pub async fn epic_dtos(
        &self,
        initiative: Option<&Key>,
        now: DateTime<Utc>,
    ) -> Result<Vec<EpicDto>, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let urg = self.urgency_in(&mut tx, now).await?;
            let epics = self.repo.list_epics_in(&mut tx, initiative).await?;
            Ok(epics.iter().map(|e| epic_dto_of(e, &urg)).collect())
        }
        .await;
        end_read(tx, out).await
    }
}

/// The one place an `EpicDto` is filled: the epic and its urgency.
pub(super) fn epic_dto_of(e: &Epic, urg: &Urgency) -> EpicDto {
    let mut dto = EpicDto::from(e);
    (dto.urgent_mark, dto.urgent_sources) = urgent_fields(urg, e.key());
    dto
}
