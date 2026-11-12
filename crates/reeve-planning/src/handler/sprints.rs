//! Sprints and goals (kb/ADR-0014). The handlers lock, load, call `domain::sprint` and record activity; every rule lives
//! in the domain. Lock order everywhere: the sprint row first (`sprint_for_update`), then goal rows, so a close and a goal
//! write or link queue on the sprint row rather than deadlock, and each rechecks the sprint's state under that lock.
//! Activity is each transaction's final work: entries are collected and recorded in order immediately before commit.
use super::*;
use crate::domain::promise::{Clock, SprintPromise, StoryFacts, sprint_range, story_promise};
use crate::domain::sprint::{self, Goal, GoalLink, Op, Sprint};
use chrono::NaiveDate;
use reeve_protocol::{Confidence, SprintState, SprintWarning, Verdict};

/// What the sprint page and the board strip read in one call, from one snapshot. `active_summary` is the active
/// sprint's numbers over `[opened_at, now)`; None without an active sprint.
#[derive(Debug, Clone)]
pub struct Sprints {
    pub active: Option<Sprint>,
    pub draft: Option<Sprint>,
    /// Newest first.
    pub closed: Vec<Sprint>,
    /// Every sprint's goals, by sprint then id.
    pub goals: Vec<Goal>,
    pub links: Vec<GoalLink>,
    pub active_summary: Option<sprint::Summary>,
}

/// One sprint as `GET /sprints/{id}` shows it, from one snapshot.
#[derive(Debug, Clone)]
pub struct SprintView {
    pub sprint: Sprint,
    pub goals: Vec<Goal>,
    pub links: Vec<GoalLink>,
    /// Its numbers; None for a draft, which has no window.
    pub summary: Option<sprint::Summary>,
    /// Its promises (kb/ADR-0025 R8), by date then key; empty for a draft.
    pub promises: Vec<SprintPromise>,
    /// The planned end its promises start after (`sprint_range`); None for the first sprint and for a draft.
    pub promises_after: Option<NaiveDate>,
}

fn allowed(s: &Sprint, op: Op) -> Result<(), HandlerError> {
    sprint::allow(s.state, op).map_err(|r| HandlerError::Refused(vec![r]))
}

/// A unique violation on the named index (kb/ADR-0014 R1's last guard).
fn violates(e: &RepoError, index: &str) -> bool {
    matches!(e, RepoError::Db(sqlx::Error::Database(d))
        if d.code().as_deref() == Some("23505") && d.constraint() == Some(index))
}

/// Trimmed, or None when blank.
fn note(n: Option<String>) -> Option<String> {
    n.map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
}

impl Handlers {
    /// The draft, creating it inside `tx` when there is none (the first goal of an instance, or after a manual delete).
    async fn draft_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        now: DateTime<Utc>,
    ) -> Result<Sprint, HandlerError> {
        if let Some(d) = self
            .ports
            .sprints
            .sprint_in_state(tx, SprintState::Draft)
            .await?
        {
            return Ok(d);
        }
        let active = self
            .ports
            .sprints
            .sprint_in_state(tx, SprintState::Active)
            .await?;
        let (s, e) = sprint::draft_dates(
            active.map(|a| a.ends_on),
            now,
            self.settings.sprint_length_days,
        );
        match self.ports.sprints.insert_draft(tx, s, e, now).await? {
            Some(d) => Ok(d),
            // A concurrent first goal created it (sprints_one_draft_idx); ON CONFLICT DO NOTHING left ours out, and
            // this statement sees the committed one.
            None => self
                .ports
                .sprints
                .sprint_in_state(tx, SprintState::Draft)
                .await?
                .ok_or(HandlerError::NotFound),
        }
    }

    /// Locks a goal's sprint, then the goal, after checking the sprint's state allows `op`.
    async fn goal_locked(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        op: Op,
    ) -> Result<(Sprint, Goal), HandlerError> {
        // A goal never changes sprint, so the unlocked read names the right row to lock.
        let sprint_id = self.ports.sprints.get_goal_in(tx, id).await?.sprint_id;
        let s = self.ports.sprints.sprint_for_update(tx, sprint_id).await?;
        allowed(&s, op)?;
        let g = self.ports.sprints.goal_for_update(tx, id).await?;
        Ok((s, g))
    }

    async fn record_all(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        entries: Vec<ActivityRecord>,
    ) -> Result<(), HandlerError> {
        for e in entries {
            self.activity.record(tx, e).await?;
        }
        Ok(())
    }

    /// Adds a goal to the draft (or to the active sprint with `to_active`). Interactive only (kb/ADR-0014 R5); the owner
    /// defaults to the caller. Returns the goal and the target sprint's warnings after the add.
    pub async fn add_goal(
        &self,
        c: &Ctx,
        idem: &str,
        outcome: &str,
        check: &str,
        owner: Option<String>,
        notes: &str,
        to_active: bool,
    ) -> Result<(Goal, Vec<SprintWarning>), HandlerError> {
        interactive(c)?;
        if outcome.trim().is_empty() || check.trim().is_empty() {
            return Err(HandlerError::Validation(
                "outcome and check are required".into(),
            ));
        }
        let owner = reeve_protocol::normalise_email(owner.as_deref().unwrap_or(&c.actor));
        if owner.is_empty() {
            return Err(HandlerError::Validation("owner must not be blank".into()));
        }
        let mut tx = self.pool.begin().await?;
        let target = if to_active {
            self.ports
                .sprints
                .sprint_in_state(&mut tx, SprintState::Active)
                .await?
                .ok_or(HandlerError::Refused(vec![Reason::SprintNotActive]))?
        } else {
            self.draft_in(&mut tx, c.now).await?
        };
        // Recheck under the lock: a concurrent open may have made the draft active, or a close the active one closed.
        let target = self
            .ports
            .sprints
            .sprint_for_update(&mut tx, target.id)
            .await?;
        allowed(&target, Op::AddGoal)?;
        match (to_active, target.state) {
            (false, SprintState::Draft) | (true, SprintState::Active) => {}
            (false, _) => return Err(HandlerError::Refused(vec![Reason::SprintNotDraft])),
            (true, _) => return Err(HandlerError::Refused(vec![Reason::SprintNotActive])),
        }
        let (g, inserted) = self
            .ports
            .sprints
            .insert_goal(
                &mut tx,
                target.id,
                outcome.trim(),
                check.trim(),
                &owner,
                notes.trim(),
                idem,
                c.now,
            )
            .await?;
        let count = self
            .ports
            .sprints
            .goals_of_in(&mut tx, &[g.sprint_id])
            .await?
            .len();
        if inserted {
            self.activity
                .record(
                    &mut tx,
                    self.rec(
                        c,
                        "goal",
                        &g.id.to_string(),
                        "created",
                        serde_json::json!({"sprint_id": g.sprint_id, "outcome": g.outcome, "check": g.check, "owner": g.owner}),
                    ),
                )
                .await?;
        }
        tx.commit().await?;
        Ok((g, sprint::warnings(count)))
    }

    /// Opens the draft: it becomes active from `c.now`, and the next draft is created in the same transaction.
    /// Interactive only (kb/ADR-0014 R5).
    pub async fn open_sprint(
        &self,
        c: &Ctx,
        id: i64,
        version: i64,
        ends_on: Option<NaiveDate>,
    ) -> Result<Sprint, HandlerError> {
        interactive(c)?;
        let mut tx = self.pool.begin().await?;
        let mut s = self.ports.sprints.sprint_for_update(&mut tx, id).await?;
        if s.version != version {
            return Err(HandlerError::Conflict { current: s.version });
        }
        allowed(&s, Op::Open)?;
        if self
            .ports
            .sprints
            .sprint_in_state(&mut tx, SprintState::Active)
            .await?
            .is_some()
        {
            return Err(HandlerError::Refused(vec![Reason::SprintActive]));
        }
        let (starts, ends) = sprint::open_dates(c.now, ends_on, self.settings.sprint_length_days)
            .map_err(|e| HandlerError::Validation(e.to_string()))?;
        s.state = SprintState::Active;
        s.opened_at = Some(c.now);
        s.starts_on = starts;
        s.ends_on = ends;
        if let Err(e) = self.ports.sprints.save_sprint(&mut tx, &s).await {
            // Defensive: two opens of the one draft already meet at the version check above (FOR UPDATE, then the
            // version), so this fires only if an active row appeared some other way; the index is R1's last guard.
            return Err(if violates(&e, "sprints_one_active_idx") {
                HandlerError::Refused(vec![Reason::SprintActive])
            } else {
                e.into()
            });
        }
        let (ns, ne) = sprint::draft_dates(Some(ends), c.now, self.settings.sprint_length_days);
        // The draft being opened was the one draft (sprints_one_draft_idx) and is active now, so this insert cannot
        // meet another; None would mean a second draft appeared, which only a second open could make.
        let next = self
            .ports
            .sprints
            .insert_draft(&mut tx, ns, ne, c.now)
            .await?
            .ok_or(HandlerError::Refused(vec![Reason::SprintNotDraft]))?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "sprint",
                    &id.to_string(),
                    "opened",
                    serde_json::json!({"starts_on": starts, "ends_on": ends, "next_draft": next.id}),
                ),
            )
            .await?;
        tx.commit().await?;
        self.sprint(id).await
    }

    /// Closes the active sprint with a verdict per goal; refused with every goal still lacking one. Interactive only
    /// (kb/ADR-0014 R5). The sprint row is locked first, so no goal write or link can commit into it after this
    /// transaction reads its goals.
    pub async fn close_sprint(
        &self,
        c: &Ctx,
        id: i64,
        version: i64,
        verdicts: Vec<(i64, Verdict, Option<String>)>,
    ) -> Result<Sprint, HandlerError> {
        interactive(c)?;
        let mut tx = self.pool.begin().await?;
        let mut s = self.ports.sprints.sprint_for_update(&mut tx, id).await?;
        allowed(&s, Op::Close)?;
        if s.version != version {
            return Err(HandlerError::Conflict { current: s.version });
        }
        let mut goals = self.ports.sprints.goals_of_in(&mut tx, &[id]).await?;
        let mut judged = Vec::new();
        for (gid, v, n) in verdicts {
            let g = goals.iter_mut().find(|g| g.id == gid).ok_or_else(|| {
                HandlerError::Validation(format!("goal {gid} is not in sprint {id}"))
            })?;
            g.verdict = Some(v);
            g.verdict_note = note(n);
            if !judged.contains(&gid) {
                judged.push(gid);
            }
        }
        let missing: Vec<i64> = goals
            .iter()
            .filter(|g| g.verdict.is_none())
            .map(|g| g.id)
            .collect();
        if !missing.is_empty() {
            return Err(HandlerError::Refused(vec![Reason::VerdictMissing {
                goals: missing,
            }]));
        }
        let judged: Vec<&Goal> = goals.iter().filter(|g| judged.contains(&g.id)).collect();
        for g in &judged {
            self.ports.sprints.save_goal(&mut tx, g).await?;
        }
        s.state = SprintState::Closed;
        s.closed_at = Some(c.now);
        self.ports.sprints.save_sprint(&mut tx, &s).await?;
        // The counts at close (kb/ADR-0014 R6), over `[opened_at, closed_at)`, in this transaction and before any entry.
        // They live only in the `closed` entry; planning never reads them back (views derive their numbers on read).
        let summary = self
            .sprint_view_in(&mut tx, &s, c.now)
            .await?
            .expect("a closed sprint has a window");
        let counts = reeve_protocol::SprintCountsDto::from(&summary.counts);
        let closed_payload = serde_json::json!({
            "counts": serde_json::to_value(counts).expect("counts serialise")
        });
        let mut entries: Vec<ActivityRecord> = judged
            .iter()
            .map(|g| {
                self.rec(
                    c,
                    "goal",
                    &g.id.to_string(),
                    "judged",
                    serde_json::json!({"verdict": g.verdict, "note": g.verdict_note}),
                )
            })
            .collect();
        entries.push(self.rec(c, "sprint", &id.to_string(), "closed", closed_payload));
        self.record_all(&mut tx, entries).await?;
        tx.commit().await?;
        self.sprint(id).await
    }

    /// Edits a goal's fields (each optional; outcome, check and owner must not be blank). Interactive only
    /// (kb/ADR-0014 R5). Logs each changed field's old and new value; an edit that changes nothing writes nothing.
    pub async fn edit_goal(
        &self,
        c: &Ctx,
        id: i64,
        version: i64,
        outcome: Option<String>,
        check: Option<String>,
        owner: Option<String>,
        notes: Option<String>,
    ) -> Result<Goal, HandlerError> {
        interactive(c)?;
        let outcome = outcome.map(|t| t.trim().to_string());
        let check = check.map(|t| t.trim().to_string());
        let owner = owner.map(|o| reeve_protocol::normalise_email(&o));
        let notes = notes.map(|t| t.trim().to_string());
        if [&outcome, &check, &owner]
            .iter()
            .any(|f| f.as_deref() == Some(""))
        {
            return Err(HandlerError::Validation(
                "outcome, check and owner must not be blank".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let (_, mut g) = self.goal_locked(&mut tx, id, Op::EditGoal).await?;
        if g.version != version {
            return Err(HandlerError::Conflict { current: g.version });
        }
        let mut fields = Vec::new();
        let mut from = serde_json::Map::new();
        let mut to = serde_json::Map::new();
        for (name, new, field) in [
            ("outcome", outcome, &mut g.outcome),
            ("check", check, &mut g.check),
            ("owner", owner, &mut g.owner),
            ("notes", notes, &mut g.notes),
        ] {
            if let Some(v) = new
                && v != *field
            {
                from.insert(name.into(), serde_json::json!(field));
                to.insert(name.into(), serde_json::json!(v));
                *field = v;
                fields.push(name);
            }
        }
        if fields.is_empty() {
            tx.rollback().await?;
            return Ok(g);
        }
        self.ports.sprints.save_goal(&mut tx, &g).await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "goal",
                    &id.to_string(),
                    "edited",
                    serde_json::json!({"fields": fields, "from": from, "to": to}),
                ),
            )
            .await?;
        tx.commit().await?;
        g.version += 1;
        Ok(g)
    }

    /// Removes a draft's goal and its links. Interactive only (kb/ADR-0014 R5). Each linked item's history shows the
    /// link ending (`goal_unlinked` under the item), then the goal's `removed`.
    pub async fn remove_goal(&self, c: &Ctx, id: i64, version: i64) -> Result<(), HandlerError> {
        interactive(c)?;
        let mut tx = self.pool.begin().await?;
        let (_, g) = self.goal_locked(&mut tx, id, Op::RemoveGoal).await?;
        if g.version != version {
            return Err(HandlerError::Conflict { current: g.version });
        }
        let links = self.ports.sprints.links_of_in(&mut tx, &[id]).await?;
        // On this transaction: it holds the row locks, so it must not wait for a second connection.
        let keys: Vec<Key> = links.iter().map(|l| l.item_key.clone()).collect();
        let types = self.ports.keys.item_types_in(&mut tx, &keys).await?;
        let mut entries = Vec::with_capacity(links.len() + 1);
        for l in &links {
            let t = *types.get(&l.item_key).ok_or(HandlerError::NotFound)?;
            entries.push(self.rec(
                c,
                item_name(t),
                &l.item_key.to_string(),
                "goal_unlinked",
                serde_json::json!({"goal_id": id}),
            ));
        }
        self.ports.sprints.delete_goal(&mut tx, id).await?;
        entries.push(self.rec(
            c,
            "goal",
            &id.to_string(),
            "removed",
            serde_json::json!({"sprint_id": g.sprint_id, "outcome": g.outcome}),
        ));
        self.record_all(&mut tx, entries).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records a person's confidence in a goal of the active sprint; the latest recording wins (no version), and it
    /// clears any suggestion. Interactive only (kb/ADR-0014 R5).
    pub async fn record_confidence(
        &self,
        c: &Ctx,
        id: i64,
        confidence: Confidence,
        note_text: Option<String>,
    ) -> Result<Goal, HandlerError> {
        interactive(c)?;
        let mut tx = self.pool.begin().await?;
        let (_, mut g) = self.goal_locked(&mut tx, id, Op::Confidence).await?;
        let suggested = g.suggested_confidence.map(|s| {
            serde_json::json!({"confidence": s, "note": g.suggested_note, "by": g.suggested_by, "at": g.suggested_at})
        });
        g.confidence = Some(confidence);
        g.confidence_note = note(note_text);
        g.confidence_at = Some(c.now);
        g.suggested_confidence = None;
        g.suggested_note = None;
        g.suggested_at = None;
        g.suggested_by = None;
        self.ports.sprints.save_goal(&mut tx, &g).await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "goal",
                    &id.to_string(),
                    "confidence_recorded",
                    serde_json::json!({"confidence": confidence, "note": g.confidence_note, "suggested": suggested}),
                ),
            )
            .await?;
        tx.commit().await?;
        g.version += 1;
        Ok(g)
    }

    /// Drafts a confidence for a person to record or discard; any token kind (kb/ADR-0014 R5). Never touches the
    /// recorded confidence; the latest suggestion wins (no version).
    pub async fn suggest_confidence(
        &self,
        c: &Ctx,
        id: i64,
        confidence: Confidence,
        note_text: Option<String>,
    ) -> Result<Goal, HandlerError> {
        let mut tx = self.pool.begin().await?;
        let (_, mut g) = self.goal_locked(&mut tx, id, Op::Suggest).await?;
        g.suggested_confidence = Some(confidence);
        g.suggested_note = note(note_text);
        g.suggested_at = Some(c.now);
        g.suggested_by = Some(reeve_protocol::normalise_email(&c.actor));
        self.ports.sprints.save_goal(&mut tx, &g).await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "goal",
                    &id.to_string(),
                    "confidence_suggested",
                    serde_json::json!({"confidence": confidence, "note": g.suggested_note}),
                ),
            )
            .await?;
        tx.commit().await?;
        g.version += 1;
        Ok(g)
    }

    /// Links work to a goal; any token kind (kb/ADR-0014 R5). Each key must name an item (NotFound otherwise); a link
    /// that already exists writes nothing. Logged under the item, so its history shows it.
    pub async fn link_goal(
        &self,
        c: &Ctx,
        goal_id: i64,
        items: &[Key],
    ) -> Result<(), HandlerError> {
        let mut typed = Vec::with_capacity(items.len());
        for k in items {
            typed.push((self.item_type_of(k).await?, k));
        }
        let mut tx = self.pool.begin().await?;
        // Sprint, then goal: a goal removed while this waited on the sprint answers NotFound, not a foreign-key error.
        self.goal_locked(&mut tx, goal_id, Op::Link).await?;
        let by = reeve_protocol::normalise_email(&c.actor);
        let mut entries = Vec::new();
        for (t, k) in typed {
            if self
                .ports
                .sprints
                .insert_goal_link(&mut tx, goal_id, k, &by, c.now)
                .await?
            {
                entries.push(self.rec(
                    c,
                    item_name(t),
                    &k.to_string(),
                    "goal_linked",
                    serde_json::json!({"goal_id": goal_id}),
                ));
            }
        }
        // Every insert first, then the entries, immediately before commit.
        self.record_all(&mut tx, entries).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Unlinks an item from a goal; any token kind (kb/ADR-0014 R5). No link, no write. Logged under the item.
    pub async fn unlink_goal(&self, c: &Ctx, goal_id: i64, item: &Key) -> Result<(), HandlerError> {
        let t = self.item_type_of(item).await?;
        let mut tx = self.pool.begin().await?;
        let g = self.ports.sprints.get_goal_in(&mut tx, goal_id).await?;
        let s = self
            .ports
            .sprints
            .sprint_for_update(&mut tx, g.sprint_id)
            .await?;
        allowed(&s, Op::Unlink)?;
        if !self
            .ports
            .sprints
            .delete_goal_link(&mut tx, goal_id, item)
            .await?
        {
            tx.rollback().await?;
            return Ok(());
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    item_name(t),
                    &item.to_string(),
                    "goal_unlinked",
                    serde_json::json!({"goal_id": goal_id}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every sprint with its goals and links, and the active sprint's numbers up to `now`, from one read-only snapshot.
    /// No promises: the active sprint's list is read only by `sprint_view` until a client renders it here (GRA-610), so
    /// this read, made on every board load, does not pay for it.
    pub async fn sprints(&self, now: DateTime<Utc>) -> Result<Sprints, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = self.sprints_in(&mut tx, now).await;
        end_read(tx, out).await
    }

    async fn sprints_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        now: DateTime<Utc>,
    ) -> Result<Sprints, HandlerError> {
        let all = self.ports.sprints.sprints_in(tx).await?;
        let ids: Vec<i64> = all.iter().map(|s| s.id).collect();
        let goals = self.ports.sprints.goals_of_in(tx, &ids).await?;
        let goal_ids: Vec<i64> = goals.iter().map(|g| g.id).collect();
        let links = self.ports.sprints.links_of_in(tx, &goal_ids).await?;
        let mut out = Sprints {
            active: None,
            draft: None,
            closed: vec![],
            goals,
            links,
            active_summary: None,
        };
        for s in all {
            match s.state {
                SprintState::Active => out.active = Some(s),
                SprintState::Draft => out.draft = Some(s),
                SprintState::Closed => out.closed.push(s),
            }
        }
        if let Some(a) = &out.active {
            out.active_summary = self.summary_in(tx, a, &out.goals, &out.links, now).await?;
        }
        Ok(out)
    }

    /// One sprint with its goals and links, its numbers and its promises. The numbers are over `[opened_at, closed_at)`
    /// for a closed sprint, over `[opened_at, now)` for the active one, and None for a draft, which has no window. The
    /// promises (kb/ADR-0025 R8) follow the planned ends, classified against `now`. Any sprint, closed ones included, is
    /// derived on read (kb/ADR-0014 R6).
    pub async fn sprint_view(
        &self,
        id: i64,
        now: DateTime<Utc>,
    ) -> Result<SprintView, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let (s, goals, links) = self.sprint_with_goals_in(&mut tx, id).await?;
            let summary = self.summary_in(&mut tx, &s, &goals, &links, now).await?;
            let all = self.ports.sprints.sprints_in(&mut tx).await?;
            let (promises_after, promises) =
                self.sprint_promises_in(&mut tx, &s, &all, now).await?;
            Ok(SprintView {
                sprint: s,
                goals,
                links,
                summary,
                promises,
                promises_after,
            })
        }
        .await;
        end_read(tx, out).await
    }

    /// kb/ADR-0025 R8: the sprint's promises, derived on read and never stored. The effective dates in the sprint's
    /// promise range (`sprint_range`: after the previous non-draft sprint's planned end, up to this sprint's own), whatever
    /// the sprint's start or close: a sprint that closes late does not keep the promises due after
    /// its planned end. The reads give candidates by the story's or its epic's date; `story_promise` decides inheritance
    /// and the effective date, so a story that does not inherit its epic's date is judged on its own. A row without a
    /// state (done without `done_at`) is left out. By date, then key. A draft lists none: the list is filled for active
    /// and closed sprints. `all` is every sprint, read on the caller's snapshot; the clock is built from it rather than
    /// read again. Returns the previous planned end beside the rows (None for the first sprint and for a draft).
    async fn sprint_promises_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Sprint,
        all: &[Sprint],
        now: DateTime<Utc>,
    ) -> Result<(Option<NaiveDate>, Vec<SprintPromise>), HandlerError> {
        let Some((prev, upto)) = sprint_range(s, all) else {
            return Ok((None, vec![]));
        };
        let clock = Clock::at(now, all);
        let rows = self.ports.reads.promised_between_in(tx, prev, upto).await?;
        // The reads order by key; the stable sort by date keeps that order within a date.
        let mut out: Vec<SprintPromise> = rows
            .into_iter()
            .filter_map(|r| {
                let facts = StoryFacts {
                    delivery: r.delivery,
                    resolution: r.resolution,
                    done_at: r.row.done_at,
                    own: r.row.own.as_ref(),
                    epic: r.row.epic.as_ref().map(|(k, p)| (k, p)),
                };
                let sp = story_promise(&facts, &clock)?;
                SprintPromise::listed(r.key, r.title, sp, prev, upto)
            })
            .collect();
        out.sort_by_key(|p| p.on());
        Ok((prev, out))
    }

    /// One sprint with its goals and links, from one read-only snapshot.
    pub async fn sprint_with_goals(
        &self,
        id: i64,
    ) -> Result<(Sprint, Vec<Goal>, Vec<GoalLink>), HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = self.sprint_with_goals_in(&mut tx, id).await;
        end_read(tx, out).await
    }

    async fn sprint_with_goals_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<(Sprint, Vec<Goal>, Vec<GoalLink>), HandlerError> {
        let s = self.ports.sprints.get_sprint_in(tx, id).await?;
        let goals = self.ports.sprints.goals_of_in(tx, &[id]).await?;
        let ids: Vec<i64> = goals.iter().map(|g| g.id).collect();
        let links = self.ports.sprints.links_of_in(tx, &ids).await?;
        Ok((s, goals, links))
    }

    /// One goal with its links, from one read-only snapshot.
    pub async fn goal(&self, id: i64) -> Result<(Goal, Vec<GoalLink>), HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let g = self.ports.sprints.get_goal_in(&mut tx, id).await?;
            let links = self.ports.sprints.links_of_in(&mut tx, &[id]).await?;
            Ok((g, links))
        }
        .await;
        end_read(tx, out).await
    }

    /// The numbers of `s` on the caller's transaction: its goals and links (two statements), then the summary. Used by
    /// the close, so the `closed` entry's counts are what `sprint_view` computes right after it.
    pub(super) async fn sprint_view_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Sprint,
        now: DateTime<Utc>,
    ) -> Result<Option<sprint::Summary>, HandlerError> {
        let goals = self.ports.sprints.goals_of_in(tx, &[s.id]).await?;
        let ids: Vec<i64> = goals.iter().map(|g| g.id).collect();
        let links = self.ports.sprints.links_of_in(tx, &ids).await?;
        self.summary_in(tx, s, &goals, &links, now).await
    }

    /// The numbers of `s` over its window, in a fixed number of statements whatever the number of goals and stories:
    /// the covered and completed stories, then the readiness of the unfinished ones (for "blocked by N decisions"),
    /// then `sprint::summarise`. `goals` and `links` may hold other sprints' too; only `s`'s count.
    async fn summary_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Sprint,
        goals: &[Goal],
        links: &[GoalLink],
        now: DateTime<Utc>,
    ) -> Result<Option<sprint::Summary>, HandlerError> {
        let (from, to) = match (s.state, s.opened_at, s.closed_at) {
            (SprintState::Active, Some(from), _) => (from, now.max(from)),
            (SprintState::Closed, Some(from), Some(to)) => (from, to),
            _ => return Ok(None),
        };
        let goal_ids: Vec<i64> = goals
            .iter()
            .filter(|g| g.sprint_id == s.id)
            .map(|g| g.id)
            .collect();
        let links: Vec<GoalLink> = links
            .iter()
            .filter(|l| goal_ids.contains(&l.goal_id))
            .cloned()
            .collect();
        let linked: Vec<Key> = links.iter().map(|l| l.item_key.clone()).collect();
        let stories = self
            .ports
            .reads
            .sprint_story_times(tx, &linked, from, to)
            .await?;
        let unfinished: Vec<Key> = stories
            .iter()
            .filter(|st| st.delivery != reeve_protocol::DeliveryState::Done)
            .map(|st| st.key.clone())
            .collect();
        let mut inputs = self.ports.reads.readiness_inputs(tx, &unfinished).await?;
        let mut blocked = std::collections::HashMap::new();
        for (k, i) in inputs.iter_mut() {
            self.mark_configured(i);
            let ids: Vec<i64> = crate::domain::readiness::compute_readiness(i)
                .reasons
                .iter()
                .filter_map(Reason::decision_id)
                .collect();
            if !ids.is_empty() {
                blocked.insert(k.clone(), ids);
            }
        }
        Ok(Some(sprint::summarise(
            &goal_ids, &links, &stories, &blocked, from, to,
        )))
    }

    /// One sprint as it is now.
    pub async fn sprint(&self, id: i64) -> Result<Sprint, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = self
            .ports
            .sprints
            .get_sprint_in(&mut tx, id)
            .await
            .map_err(Into::into);
        end_read(tx, out).await
    }
}
