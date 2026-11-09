//! Urgent marks (kb/ADR-0017): marking, clearing, the war room, and the urgency every DTO assembler reads.
use super::*;
use crate::domain::items::{Epic, Initiative, Story};
use crate::domain::urgency::{MarkState, Urgency, UrgentMark, urgency};
use std::collections::HashMap;

const MAX_REASON: usize = 200;
const MAX_BULK: usize = 100;

/// One row of the war room strip.
#[derive(Debug, Clone, PartialEq)]
pub struct WarRoomEntry {
    pub item: ItemType,
    pub key: Key,
    pub title: String,
    /// The marked item's initiative (itself for an initiative): key, title, horizon.
    pub initiative: Option<(Key, String, String)>,
    pub state: MarkState,
}

fn clean_reason(r: Option<String>) -> Result<Option<String>, HandlerError> {
    let r = r.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if r.as_ref().is_some_and(|s| s.chars().count() > MAX_REASON) {
        return Err(HandlerError::Validation(format!(
            "reason must be at most {MAX_REASON} characters"
        )));
    }
    Ok(r)
}

impl Handlers {
    /// The urgency of every item, from the caller's snapshot: two statements when any mark exists, one otherwise.
    pub(crate) async fn urgency_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        now: DateTime<Utc>,
    ) -> Result<Urgency, HandlerError> {
        let marks = self.repo.urgent_marks_in(tx).await?;
        let mut epics = Vec::new();
        let mut initiatives = Vec::new();
        for m in &marks {
            match m.item {
                ItemType::Initiative => initiatives.push(m.key.clone()),
                ItemType::Epic => epics.push(m.key.clone()),
                ItemType::Story => epics.extend(m.parent.clone()),
            }
        }
        let lineage = self
            .repo
            .urgency_lineage_in(tx, &epics, &initiatives)
            .await?;
        Ok(urgency(
            &marks,
            &lineage,
            now,
            self.settings.sprint_length_days,
        ))
    }

    /// Marks (or renews) one item; the reason is optional. Interactive only (kb/ADR-0017 R4).
    pub async fn mark_urgent(
        &self,
        c: &Ctx,
        item: ItemType,
        key: &Key,
        reason: Option<String>,
    ) -> Result<(), HandlerError> {
        interactive(c)?;
        let reason = clean_reason(reason)?;
        let mut tx = self.pool.begin().await?;
        self.mark_in(&mut tx, c, item, key, reason).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn mark_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        item: ItemType,
        key: &Key,
        reason: Option<String>,
    ) -> Result<(), HandlerError> {
        let mark = UrgentMark {
            at: c.now,
            by: c.actor.clone(),
            reason: reason.clone(),
        };
        if !self.repo.set_urgent_mark(tx, item, key, &mark).await? {
            return Err(HandlerError::NotFound);
        }
        let name = item_name(item);
        self.activity
            .record(
                tx,
                self.rec(
                    c,
                    name,
                    &key.to_string(),
                    "urgent_marked",
                    serde_json::json!({"item_type": name, "reason": reason}),
                ),
            )
            .await?;
        Ok(())
    }

    /// Clears one item's mark; no entry when it carried none. NotFound when no item of that type has the key.
    pub async fn clear_urgent(
        &self,
        c: &Ctx,
        item: ItemType,
        key: &Key,
    ) -> Result<(), HandlerError> {
        interactive(c)?;
        if !self.repo.key_exists(item, key).await? {
            return Err(HandlerError::NotFound);
        }
        let mut tx = self.pool.begin().await?;
        self.clear_in(&mut tx, c, item, key).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn clear_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        item: ItemType,
        key: &Key,
    ) -> Result<(), HandlerError> {
        if self.repo.clear_urgent_mark(tx, item, key).await? {
            let name = item_name(item);
            self.activity
                .record(
                    tx,
                    self.rec(
                        c,
                        name,
                        &key.to_string(),
                        "urgent_cleared",
                        serde_json::json!({"item_type": name}),
                    ),
                )
                .await?;
        }
        Ok(())
    }

    /// Every stored mark, active ones first, each group oldest first, from one read-only snapshot.
    pub async fn war_room(&self, now: DateTime<Utc>) -> Result<Vec<WarRoomEntry>, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = self.war_room_in(&mut tx, now).await;
        end_read(tx, out).await
    }

    async fn war_room_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        now: DateTime<Utc>,
    ) -> Result<Vec<WarRoomEntry>, HandlerError> {
        let urg = self.urgency_in(tx, now).await?;
        if urg.marks().next().is_none() {
            return Ok(Vec::new());
        }
        // Whole-table reads of initiatives and epics: an instance holds tens of them, and this keeps the strip to a
        // fixed number of statements whatever the number of marks.
        let initiatives: HashMap<Key, Initiative> = self
            .repo
            .initiatives_in(tx)
            .await?
            .into_iter()
            .map(|i| (i.key().clone(), i))
            .collect();
        let epics: HashMap<Key, Epic> = self
            .repo
            .epics_in(tx)
            .await?
            .into_iter()
            .map(|e| (e.key().clone(), e))
            .collect();
        let story_keys: Vec<Key> = urg
            .marks()
            .filter(|(_, m)| m.item == ItemType::Story)
            .map(|(k, _)| k.clone())
            .collect();
        let stories: HashMap<Key, Story> = self
            .repo
            .stories_by_keys(tx, &story_keys)
            .await?
            .into_iter()
            .map(|s| (s.key().clone(), s))
            .collect();
        let initiative = |k: Option<&Key>| {
            k.and_then(|k| initiatives.get(k)).map(|i| {
                (
                    i.key().clone(),
                    i.title().to_string(),
                    i.horizon().to_string(),
                )
            })
        };
        let mut out: Vec<WarRoomEntry> = urg
            .marks()
            .filter_map(|(k, m)| {
                let (title, init) = match m.item {
                    ItemType::Initiative => {
                        let i = initiatives.get(k)?;
                        (i.title().to_string(), initiative(Some(k)))
                    }
                    ItemType::Epic => {
                        let e = epics.get(k)?;
                        (e.title().to_string(), initiative(e.initiative()))
                    }
                    ItemType::Story => {
                        let s = stories.get(k)?;
                        let i = s
                            .epic()
                            .and_then(|e| epics.get(e))
                            .and_then(|e| e.initiative());
                        (s.title().to_string(), initiative(i))
                    }
                };
                Some(WarRoomEntry {
                    item: m.item,
                    key: k.clone(),
                    title,
                    initiative: init,
                    state: m.clone(),
                })
            })
            .collect();
        out.sort_by_key(|e| (!e.state.active, e.state.mark.at, e.key.number()));
        Ok(out)
    }

    /// Clears exactly the listed marks in one transaction (the rows the caller saw). An item of the wrong type or an
    /// unknown key fails the whole request.
    pub async fn war_room_clear(
        &self,
        c: &Ctx,
        items: &[(ItemType, Key)],
    ) -> Result<(), HandlerError> {
        interactive(c)?;
        let mut tx = self.bulk_begin(items).await?;
        for (t, k) in items {
            self.clear_in(&mut tx, c, *t, k).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Re-marks exactly the listed items, each keeping its reason; an item without a mark is skipped.
    pub async fn war_room_renew(
        &self,
        c: &Ctx,
        items: &[(ItemType, Key)],
    ) -> Result<(), HandlerError> {
        interactive(c)?;
        let mut tx = self.bulk_begin(items).await?;
        let reasons: HashMap<Key, Option<String>> = self
            .repo
            .urgent_marks_in(&mut tx)
            .await?
            .into_iter()
            .map(|m| (m.key, m.mark.reason))
            .collect();
        for (t, k) in items {
            if let Some(reason) = reasons.get(k) {
                self.mark_in(&mut tx, c, *t, k, reason.clone()).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    async fn bulk_begin(
        &self,
        items: &[(ItemType, Key)],
    ) -> Result<Transaction<'static, Postgres>, HandlerError> {
        if items.len() > MAX_BULK {
            return Err(HandlerError::Validation(format!(
                "at most {MAX_BULK} items"
            )));
        }
        let mut tx = self.pool.begin().await?;
        let keys: Vec<Key> = items.iter().map(|(_, k)| k.clone()).collect();
        let types = self.repo.item_types_in(&mut tx, &keys).await?;
        if let Some((t, k)) = items.iter().find(|(t, k)| types.get(k) != Some(t)) {
            return Err(HandlerError::Validation(format!(
                "{k} is not a {}",
                item_name(*t)
            )));
        }
        Ok(tx)
    }
}
