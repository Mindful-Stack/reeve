use super::dto::{epic_promise_field, urgent_fields};
use super::import::{Claim, with_ref};
use super::*;
use crate::domain::promise::{Clock, Promise};
use crate::domain::{items::*, urgency::Urgency};
use reeve_protocol::{EpicDto, ItemType, Key};

/// The move an epic edit wrote: the initiative the epic left and the one it joined (`None`: none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpicMove {
    pub from: Option<Key>,
    pub to: Option<Key>,
}

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
        self.create_epic_attaching(c, idem, title, initiative, brief, external_ref, &[])
            .await
    }

    /// `create_epic`, claiming `attachments` for the new epic in its transaction (kb/ADR-0022 R2).
    pub async fn create_epic_attaching(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        initiative: Option<Key>,
        brief: &str,
        external_ref: Option<String>,
        attachments: &[Uuid],
    ) -> Result<Epic, HandlerError> {
        self.create_epic_reporting(c, idem, title, initiative, brief, external_ref, attachments)
            .await
            .map(|(x, _)| x)
    }

    /// `create_epic`, and whether this call inserted it (false when its idempotency key already had one). Public for
    /// the server's create action, whose MCP receipt says whether this call created the item.
    pub async fn create_epic_reporting(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        initiative: Option<Key>,
        brief: &str,
        external_ref: Option<String>,
        attachments: &[Uuid],
    ) -> Result<(Epic, bool), HandlerError> {
        let attachments = attachment_ids(attachments)?;
        if let Some(k) = self
            .ports
            .keys
            .find_by_idempotency(ItemType::Epic, idem)
            .await?
        {
            return Ok((self.ports.epics.get_epic(&k).await?, false));
        }
        if let Some(ik) = &initiative {
            self.ports.initiatives.get_initiative(ik).await?;
        }
        let mut tx = self.pool.begin().await?;
        let r = match self
            .claim_ref(&mut tx, ItemType::Epic, idem, external_ref)
            .await?
        {
            Claim::Free(r) => r,
            Claim::Retried(k) => {
                tx.rollback().await?;
                return Ok((self.ports.epics.get_epic(&k).await?, false));
            }
        };
        let key = self
            .ports
            .keys
            .next_key(&mut tx, &self.settings.key_prefix)
            .await?;
        let e = Epic::new(key, title, initiative, brief)?.with_external_ref(r);
        if !self
            .ports
            .epics
            .insert_epic(&mut tx, &e, idem, c.now)
            .await?
        {
            tx.rollback().await?;
            let k = self
                .ports
                .keys
                .find_by_idempotency(ItemType::Epic, idem)
                .await?
                .ok_or(HandlerError::NotFound)?;
            return Ok((self.ports.epics.get_epic(&k).await?, false));
        }
        self.claim_attachments(&mut tx, c, &attachments, ItemType::Epic, e.key())
            .await?;
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
        self.edit_epic_attaching(c, key, version, title, brief, initiative, external_ref, &[])
            .await
    }

    /// `edit_epic`, claiming `attachments` for the epic in its transaction (kb/ADR-0022 R2).
    #[allow(clippy::too_many_arguments)]
    pub async fn edit_epic_attaching(
        &self,
        c: &Ctx,
        key: &Key,
        version: i64,
        title: Option<String>,
        brief: Option<String>,
        initiative: Option<Option<Key>>,
        external_ref: Option<String>,
        attachments: &[Uuid],
    ) -> Result<Epic, HandlerError> {
        self.edit_epic_reporting(
            c,
            key,
            version,
            title,
            brief,
            initiative,
            external_ref,
            attachments,
        )
        .await
        .map(|(x, _)| x)
    }

    /// `edit_epic_attaching`, and the move this edit wrote (`None`: it did not change the initiative). Answers the
    /// epic as this write left it, not a later read of it. The `edited` entry names every field the edit changed,
    /// and the initiative the epic left and the one it joined when the edit moved it: the entry and the answer take
    /// the move from one value. Public for the server's edit action, whose MCP receipt says where the epic moved.
    #[allow(clippy::too_many_arguments)]
    pub async fn edit_epic_reporting(
        &self,
        c: &Ctx,
        key: &Key,
        version: i64,
        title: Option<String>,
        brief: Option<String>,
        initiative: Option<Option<Key>>,
        external_ref: Option<String>,
        attachments: &[Uuid],
    ) -> Result<(Epic, Option<EpicMove>), HandlerError> {
        let attachments = attachment_ids(attachments)?;
        let cur = self.ports.epics.get_epic(key).await?;
        // The version the caller read must be the row read here: the row written below is rebuilt from this read (the
        // discovery mark included), and the conditional update only proves the row is at the caller's version when it
        // is written, which a concurrent write can make true of a caller ahead of this read.
        if cur.version() != version {
            return Err(HandlerError::Conflict {
                current: cur.version(),
            });
        }
        let ini = initiative.unwrap_or(cur.initiative().cloned());
        if let Some(ik) = &ini {
            self.ports.initiatives.get_initiative(ik).await?;
        }
        let next = Epic::new(
            cur.key().clone(),
            title.as_deref().unwrap_or(cur.title()),
            ini,
            brief.as_deref().unwrap_or(cur.brief()),
        )?;
        // The entry names every field the edit changed, in the shape `edit_decision` gives its entry. Only a move
        // carries its values (the keys it left and joined, null for none): it changes whose discovery mark and
        // required decisions this epic's stories wait on, and a bare `edited` would not tell it from a retitle. A
        // title or a brief is named, never copied: the entry keeps no earlier text. An edit that changes nothing
        // names no field.
        let moved = (next.initiative() != cur.initiative()).then(|| EpicMove {
            from: cur.initiative().cloned(),
            to: next.initiative().cloned(),
        });
        let fields: Vec<&str> = [
            ("title", next.title() != cur.title()),
            ("brief", next.brief() != cur.brief()),
            ("initiative", moved.is_some()),
        ]
        .into_iter()
        .filter_map(|(field, changed)| changed.then_some(field))
        .collect();
        let mut payload = serde_json::json!({});
        if !fields.is_empty() {
            payload["fields"] = serde_json::json!(fields);
        }
        if let Some(m) = &moved {
            payload["from"] =
                serde_json::json!({"initiative": m.from.as_ref().map(ToString::to_string)});
            payload["to"] =
                serde_json::json!({"initiative": m.to.as_ref().map(ToString::to_string)});
        }
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
        self.ports
            .epics
            .update_epic(&mut tx, &next, version, c.now)
            .await?;
        self.claim_attachments(&mut tx, c, &attachments, ItemType::Epic, key)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "epic",
                    &key.to_string(),
                    "edited",
                    with_ref(payload, set.as_deref()),
                ),
            )
            .await?;
        tx.commit().await?;
        // The row as this write left it: the update wrote `next` at `version` (the version read, checked above) and
        // raised it by one.
        let written = Epic::from_persisted(
            next.key().clone(),
            next.title().into(),
            next.initiative().cloned(),
            next.brief().into(),
            next.discovery().cloned(),
            next.external_ref().map(String::from),
            version + 1,
        );
        Ok((written, moved))
    }

    /// One epic with its urgency, from one `REPEATABLE READ READ ONLY` snapshot. Its item links are read on the same
    /// snapshot (kb/ADR-0031); a list read never carries them.
    pub async fn epic_dto(&self, key: &Key, now: DateTime<Utc>) -> Result<EpicDto, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let urg = self.urgency_in(&mut tx, now).await?;
            let clock = self.promise_clock_in(&mut tx, now).await?;
            let e = self.ports.epics.get_epic_in(&mut tx, key).await?;
            let mut promises = self
                .ports
                .reads
                .promise_facts_in(&mut tx, &[], std::slice::from_ref(key))
                .await?;
            let mut dto = epic_dto_of(&e, &urg, promises.epics.remove(key).as_ref(), &clock);
            dto.links = Some(self.item_link_dtos_in(&mut tx, key).await?);
            dto.web_links = Some(self.web_link_dtos_in(&mut tx, key).await?);
            Ok(dto)
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
            let clock = self.promise_clock_in(&mut tx, now).await?;
            let epics = self.ports.epics.list_epics_in(&mut tx, initiative).await?;
            let keys: Vec<Key> = epics.iter().map(|e| e.key().clone()).collect();
            let promises = self
                .ports
                .reads
                .promise_facts_in(&mut tx, &[], &keys)
                .await?;
            Ok(epics
                .iter()
                .map(|e| epic_dto_of(e, &urg, promises.epics.get(e.key()), &clock))
                .collect())
        }
        .await;
        end_read(tx, out).await
    }
}

/// The one place an `EpicDto` is filled: the epic, its urgency and its own promise (read by the caller on the same
/// snapshot), classified against `clock`.
pub(super) fn epic_dto_of(
    e: &Epic,
    urg: &Urgency,
    promise: Option<&Promise>,
    clock: &Clock,
) -> EpicDto {
    let mut dto = EpicDto::from(e);
    (dto.urgent_mark, dto.urgent_sources) = urgent_fields(urg, e.key());
    dto.promise = epic_promise_field(promise, clock);
    dto
}
