#![allow(clippy::too_many_arguments)] // handlers mirror the request shapes; argument structs would only move the count
mod attachments;
mod brief_templates;
mod comments;
mod decisions;
mod dto;
mod epics;
mod export;
mod import;
mod initiatives;
mod item_links;
mod moves;
mod plans;
mod promise;
mod prs;
mod search;
mod similar;
mod sprints;
mod stories;
mod urgent;
mod web_links;
use crate::ports::*;
use attachments::attachment_ids;
pub use attachments::{MAX_CLAIM, MAX_UNCLAIMED_BYTES};
use chrono::{DateTime, Utc};
pub use decisions::DecisionScope;
pub use dto::{
    attachment_dto, epic_promise_field, goal_dto, sprint_dto, sprint_planning_dto,
    sprint_promise_dto, sprint_view_dto, story_promise_field,
};
pub use epics::EpicMove;
use reeve_protocol::{ItemType, Key, KeyRef, Reason, Via};
pub use similar::SIMILAR_MIN;
pub use sprints::{SprintPlanning, SprintView, Sprints};
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
pub use urgent::{WarRoomEntry, WarRoomInitiative};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Ctx {
    pub actor: String,
    pub via: Via,
    pub now: DateTime<Utc>,
    /// The client a write was made through, when it was not one of Reeve's own (kb/ADR-0029 R9): a provenance
    /// string beside `via`, written into the activity entry's payload as `client`. Planning knows nothing else
    /// about where a call came from.
    pub client: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub key_prefix: String,
    pub horizon_buckets: Vec<String>,
    /// Names of the configured repositories (`[[repos]]`); a record reference must name one of them.
    pub repos: Vec<String>,
    /// `[board] done_window_days`: how far back `DoneFilter::Recent` reaches.
    pub done_window_days: u32,
    /// `[sprints] length_days`: a new sprint's default length.
    pub sprint_length_days: u32,
    /// `[sprints] upkeep_line`: the percentage the sprint view shows as the team's reference line (not a budget).
    pub upkeep_line: u8,
}

/// What `Handlers::resolve` may return: one item type, or any.
#[derive(Clone, Copy)]
enum Scope {
    One(ItemType),
    Any,
}

/// What a plan's create answers (`create_initiative_guarded` and its epic and story forms): the item its
/// idempotency key has at this level, and whether this call inserted it; or the item of another level that already
/// has the key, in which case this call wrote nothing.
enum Guarded<T> {
    Item { item: T, created: bool },
    HeldElsewhere { level: ItemType, key: Key },
}

#[derive(Clone)]
pub struct Handlers {
    pub ports: PlanningPorts,
    pub activity: Arc<dyn ActivitySink>,
    pub pool: PgPool,
    pub settings: Settings,
}

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error("not found")]
    NotFound,
    #[error("version conflict: current is {current}")]
    Conflict { current: i64 },
    #[error("validation: {0}")]
    Validation(String),
    #[error("refused")]
    Refused(Vec<Reason>),
    /// The caller's role does not allow this. Planning is told the role by the route as a value; it never reads identity.
    #[error("forbidden")]
    Forbidden,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}
impl From<RepoError> for HandlerError {
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::NotFound => Self::NotFound,
            RepoError::Conflict { current } => Self::Conflict { current },
            RepoError::Db(e) => Self::Db(e),
        }
    }
}
impl From<crate::domain::items::ItemError> for HandlerError {
    fn from(e: crate::domain::items::ItemError) -> Self {
        Self::Validation(e.to_string())
    }
}
impl From<crate::domain::item_link::ItemLinkError> for HandlerError {
    fn from(e: crate::domain::item_link::ItemLinkError) -> Self {
        Self::Validation(e.to_string())
    }
}
impl From<crate::domain::web_link::WebLinkError> for HandlerError {
    fn from(e: crate::domain::web_link::WebLinkError) -> Self {
        Self::Validation(e.to_string())
    }
}
impl From<crate::domain::brief_template::BriefTemplateError> for HandlerError {
    fn from(e: crate::domain::brief_template::BriefTemplateError) -> Self {
        Self::Validation(e.to_string())
    }
}
impl From<crate::domain::decision::DecisionError> for HandlerError {
    fn from(e: crate::domain::decision::DecisionError) -> Self {
        use crate::domain::decision::DecisionError;
        match e {
            DecisionError::StaleReference { current } => HandlerError::Conflict { current },
            other => HandlerError::Validation(other.to_string()),
        }
    }
}

impl Handlers {
    pub(crate) fn rec(
        &self,
        c: &Ctx,
        entity_type: &'static str,
        key: &str,
        action: &'static str,
        payload: serde_json::Value,
    ) -> ActivityRecord {
        // Every payload planning writes is a JSON object, and none has a field of this name.
        let mut payload = payload;
        debug_assert!(
            payload.is_object(),
            "an activity payload is an object: {payload}"
        );
        if let (Some(client), Some(fields)) = (&c.client, payload.as_object_mut()) {
            fields.insert("client".into(), serde_json::Value::String(client.clone()));
        }
        ActivityRecord {
            at: c.now,
            actor: c.actor.clone(),
            via: c.via,
            entity_type,
            entity_key: key.to_string(),
            action,
            payload,
        }
    }

    /// The one key-resolution path.
    async fn resolve(
        &self,
        raw: &str,
        scope: Scope,
        external_refs: bool,
    ) -> Result<(ItemType, Key), HandlerError> {
        if let Ok(k) = raw.parse::<Key>() {
            let current = k.prefix() == self.settings.key_prefix;
            match scope {
                Scope::One(t) if current => return Ok((t, k)),
                Scope::One(t) => {
                    if self.ports.keys.key_exists(t, &k).await? {
                        return Ok((t, k));
                    }
                }
                Scope::Any => {
                    if let Some(t) = self.ports.keys.item_type_of_key(&k).await? {
                        return Ok((t, k));
                    }
                    // No external reference can start with the current prefix.
                    if current {
                        return Err(HandlerError::NotFound);
                    }
                }
            }
        }
        if !external_refs || raw.parse::<KeyRef>().is_err() {
            return Err(HandlerError::NotFound);
        }
        let one;
        let types: &[ItemType] = match scope {
            Scope::One(t) => {
                one = [t];
                &one
            }
            Scope::Any => &[ItemType::Story, ItemType::Epic, ItemType::Initiative],
        };
        for &t in types {
            if let Some(k) = self.ports.keys.find_by_external_ref(t, raw).await? {
                return Ok((t, k));
            }
        }
        Err(HandlerError::NotFound)
    }

    /// Every key input goes through here. Native keys come first: a key of the current prefix is returned as is (the
    /// handler that uses it reports a missing item as NotFound, and no external reference can start with the current
    /// prefix), and a key of any other prefix is returned when an item of this type has it (items issued before a
    /// `key_prefix` change keep their keys). Only then is anything in `KeyRef` grammar looked up as the `external_ref`
    /// of that item type; everything else is NotFound. So an external reference never shadows a real key.
    pub async fn resolve_ref(&self, raw: &str, item: ItemType) -> Result<Key, HandlerError> {
        self.resolve(raw, Scope::One(item), true)
            .await
            .map(|(_, k)| k)
    }

    /// What a create of an initiative or an epic does before it writes anything, in this order: its checks that come
    /// before the idempotency lookup (the attachment ids: deduplicated, at most `MAX_CLAIM`), then the lookup of
    /// `idem` in `item`'s own table. Answers the ids to claim and the key a create under `idem` already made, if one
    /// did. The one place that sequence is written: both create handlers and `made_under` call it (the epic's
    /// through `before_epic_create`, which adds its own check after it), so a replay is refused for the same reasons
    /// whichever of them answers it.
    pub(super) async fn before_create(
        &self,
        item: ItemType,
        idem: &str,
        attachments: &[Uuid],
    ) -> Result<(Vec<Uuid>, Option<Key>), HandlerError> {
        let attachments = attachment_ids(attachments)?;
        let made = self.ports.keys.find_by_idempotency(item, idem).await?;
        Ok((attachments, made))
    }

    /// The key of the item of type `item` (an initiative or an epic) that a create under the stored idempotency key
    /// `idem` made, if one did, judged as the create handler judges a replay: what it checks before its lookup
    /// (`before_create`) refuses here too, with the same message, so a request the handler's replay refuses is
    /// refused whoever answers it. Writes nothing. Public for the server's create actions, which answer a retry
    /// whose named brief template is gone since the first attempt with that item rather than a refusal.
    pub async fn made_under(
        &self,
        item: ItemType,
        idem: &str,
        attachments: &[Uuid],
    ) -> Result<Option<Key>, HandlerError> {
        self.before_create(item, idem, attachments)
            .await
            .map(|(_, made)| made)
    }

    /// The item of another level than `item` that has the idempotency key `idem`, read under the key's
    /// transaction-scoped lock, taken first. The lock is what makes the look and the insert after it one step for
    /// two creates under one key at different levels: the second waits here, then sees the first one's item. Shaped
    /// as `ref_holder` (`handler/import.rs`), which guards an external reference the same way.
    async fn idempotency_holder_elsewhere(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<(ItemType, Key)>, HandlerError> {
        self.ports.keys.lock_idempotency_key(tx, idem).await?;
        for other in [ItemType::Initiative, ItemType::Epic, ItemType::Story] {
            if other == item {
                continue;
            }
            if let Some(k) = self
                .ports
                .keys
                .find_by_idempotency_in(tx, other, idem)
                .await?
            {
                return Ok(Some((other, k)));
            }
        }
        Ok(None)
    }

    /// The type of the item that has `key` as its native key, checked across every type; NotFound when none has it.
    pub(crate) async fn item_type_of(&self, key: &Key) -> Result<ItemType, HandlerError> {
        self.resolve(&key.to_string(), Scope::Any, false)
            .await
            .map(|(t, _)| t)
    }

    /// Resolve an untyped item reference (a decision target): the single "any item" resolution path. Native keys of
    /// every type come first, so an item issued under an earlier prefix (`OLD-1`, an epic) wins over a story whose
    /// `external_ref` is `OLD-1`. Only when no item of any type has the key is the input (in `KeyRef` grammar) looked up
    /// as an external reference: a story, then an epic, then an initiative (a Jira key names one Jira issue, so at most
    /// one table holds it).
    pub async fn resolve_any(&self, raw: &str) -> Result<(ItemType, Key), HandlerError> {
        self.resolve(raw, Scope::Any, true).await
    }
}

/// The item type's name, as activity entries and messages spell it.
fn item_name(item: ItemType) -> &'static str {
    match item {
        ItemType::Initiative => "initiative",
        ItemType::Epic => "epic",
        ItemType::Story => "story",
    }
}

/// kb/ADR-0014 R5, kb/ADR-0017 R4: only an interactive user may take these actions. Automation and the system are both
/// refused.
pub(crate) fn interactive(c: &Ctx) -> Result<(), HandlerError> {
    if c.via == Via::Interactive {
        Ok(())
    } else {
        Err(HandlerError::Refused(vec![Reason::InteractiveOnly]))
    }
}

/// Ends a read-only snapshot. The read's own error wins over a rollback error, so a failed read is reported as itself.
pub(crate) async fn end_read<T>(
    tx: Transaction<'_, Postgres>,
    r: Result<T, HandlerError>,
) -> Result<T, HandlerError> {
    let rb = tx.rollback().await;
    let v = r?;
    rb?;
    Ok(v)
}
