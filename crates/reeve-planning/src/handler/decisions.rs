use super::*;
use crate::domain::decision::*;
use crate::domain::queue::{self, QueueEntry, QueueSummary};
use crate::domain::readiness::satisfies;
use reeve_protocol::{CheckResult, ClosureKind, DeliveryState, Key, Reversibility, TokenKind, Via};
use std::collections::HashSet;

impl Handlers {
    pub async fn require_decision(
        &self,
        c: &Ctx,
        idem: &str,
        on: &Key,
        title: &str,
        decider: &str,
        due: Option<chrono::NaiveDate>,
        reversibility: Option<Reversibility>,
    ) -> Result<RequiredDecision, HandlerError> {
        self.require_decision_reporting(c, idem, on, title, decider, due, reversibility)
            .await
            .map(|(d, _)| d)
    }

    /// `require_decision`, and whether this call inserted it (false when its idempotency key already had one).
    pub(crate) async fn require_decision_reporting(
        &self,
        c: &Ctx,
        idem: &str,
        on: &Key,
        title: &str,
        decider: &str,
        due: Option<chrono::NaiveDate>,
        reversibility: Option<Reversibility>,
    ) -> Result<(RequiredDecision, bool), HandlerError> {
        if title.trim().is_empty() || decider.trim().is_empty() {
            return Err(HandlerError::Validation(
                "title and decider are required".into(),
            ));
        }
        let item = self.item_type_of(on).await?;
        let mut tx = self.pool.begin().await?;
        let (d, inserted) = self
            .ports
            .decisions
            .insert_decision(
                &mut tx,
                item,
                on,
                title.trim(),
                decider.trim(),
                due,
                reversibility,
                idem,
                c.now,
            )
            .await?;
        if inserted {
            self.activity
                .record(
                    &mut tx,
                    self.rec(
                        c,
                        "decision",
                        &d.id().to_string(),
                        "required",
                        serde_json::json!({"on": on, "title": d.title(), "decider": d.decider(), "reversibility": d.reversibility()}),
                    ),
                )
                .await?;
        }
        tx.commit().await?;
        Ok((d, inserted))
    }

    /// Every decision mutation reads the row FOR UPDATE inside its own transaction and saves under its version,
    /// so a check that started before a relink, or a relink racing a close, is refused rather than overwriting.
    pub async fn link_record(
        &self,
        c: &Ctx,
        id: i64,
        repo: &str,
        path: &str,
        record_id: &str,
        pr_url: Option<String>,
    ) -> Result<RequiredDecision, HandlerError> {
        let r = RecordRef::new(repo, path, record_id)?;
        let pr = pr_url.as_deref().map(PrUrl::parse).transpose()?;
        let mut tx = self.pool.begin().await?;
        let mut d = self
            .ports
            .decisions
            .get_decision_for_update(&mut tx, id)
            .await?;
        d.link(r, pr)?;
        self.ports.decisions.save_decision(&mut tx, &d).await?;
        let r = d.record().expect("just linked");
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "decision",
                    &id.to_string(),
                    "linked",
                    serde_json::json!({"repo": r.repo(), "record_id": r.record_id(), "path": r.path(), "pr_url": d.pr_url()}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.ports.decisions.get_decision(id).await?)
    }

    /// `for_record` and `observed_version` are what the CLI saw when it ran the check; a result for a reference or
    /// version that has changed since is refused with conflict (kb/ADR-0004 R3). Comparing the version closes the
    /// relink-A-to-B-to-A hole that comparing the record id alone leaves open.
    ///
    /// `is_admin` is the caller's current role, decided by the route (kb/ADR-0010 R2). Only an admin may store the
    /// satisfying result (kb/ADR-0018 R3), from any token kind (storing a result is not an override). A non-admin's is
    /// refused before the decision is read, so the answer says nothing about its version, reference, closure or
    /// existence, and nothing is written. Unchecked is refused from anyone (422); every other non-satisfying result is
    /// stored from any caller: it can only add a reason. A result that says the same as the stored one
    /// (`same_outcome`, which ignores `commit`) only moves `checked_at`: no activity entry and no version bump.
    pub async fn record_check(
        &self,
        c: &Ctx,
        id: i64,
        for_record: &str,
        observed_version: i64,
        check: CheckResult,
        is_admin: bool,
    ) -> Result<RequiredDecision, HandlerError> {
        if matches!(check, CheckResult::Unchecked) {
            return Err(HandlerError::Validation(
                "a check result cannot be unchecked".into(),
            ));
        }
        if satisfies(&check) && !is_admin {
            return Err(HandlerError::Forbidden);
        }
        let mut tx = self.pool.begin().await?;
        let mut d = self
            .ports
            .decisions
            .get_decision_for_update(&mut tx, id)
            .await?;
        // The version is the handler's to compare; the aggregate owns the remaining refusals, in this order: no record,
        // a stale reference, closed.
        if d.version() != observed_version {
            return Err(HandlerError::Conflict {
                current: d.version(),
            });
        }
        match d.record_check(for_record, check, c.now)? {
            // The scheduled check posts every open, linked decision every few minutes; logging and bumping an
            // unchanged result would bury people's entries in the activity log and refetch every client on each run.
            CheckOutcome::Unchanged => {
                self.ports
                    .decisions
                    .touch_decision_checked(&mut tx, id, c.now)
                    .await?;
            }
            CheckOutcome::Changed => {
                self.ports.decisions.save_decision(&mut tx, &d).await?;
                self.activity
                    .record(
                        &mut tx,
                        self.rec(
                            c,
                            "decision",
                            &id.to_string(),
                            "checked",
                            serde_json::json!({"check": d.check()}),
                        ),
                    )
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(self.ports.decisions.get_decision(id).await?)
    }

    /// Closes the decision as decided or withdrawn (kb/ADR-0013 R3). Every rule is the aggregate's
    /// (`RequiredDecision::close`); a refusal drops the transaction, so nothing is written. A system caller is refused
    /// before any write: nothing system-driven closes a decision, and `closed_via` cannot hold it.
    pub async fn close_decision(
        &self,
        c: &Ctx,
        id: i64,
        kind: ClosureKind,
        reason: &str,
    ) -> Result<RequiredDecision, HandlerError> {
        let via = match c.via {
            Via::Interactive => TokenKind::Interactive,
            Via::Automation => TokenKind::Automation,
            Via::System => return Err(HandlerError::Forbidden),
        };
        let mut tx = self.pool.begin().await?;
        let mut d = self
            .ports
            .decisions
            .get_decision_for_update(&mut tx, id)
            .await?;
        d.close(kind, reason, &c.actor, via, c.now)?;
        self.ports.decisions.save_decision(&mut tx, &d).await?;
        let reason = &d.closed().expect("just closed").reason;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "decision",
                    &id.to_string(),
                    "closed",
                    serde_json::json!({"reason": reason, "kind": kind}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.ports.decisions.get_decision(id).await?)
    }

    /// Edits the planner-side fields of an open decision (kb/ADR-0004 R2): title, decider, due and reversibility
    /// (`Some(None)` clears either; a reversibility change is the versioned, logged edit kb/ADR-0013 R1 asks for).
    /// The record reference, PR url and check are never touched (kb/ADR-0004 R1); `link_record` sets the reference. The row
    /// is read FOR UPDATE and saved under the caller's version: a stale version is a conflict, checked before the closed
    /// rule as in `record_check`. An edit that changes nothing writes nothing and leaves the version.
    pub async fn edit_decision(
        &self,
        c: &Ctx,
        id: i64,
        version: i64,
        title: Option<String>,
        decider: Option<String>,
        due: Option<Option<chrono::NaiveDate>>,
        reversibility: Option<Option<Reversibility>>,
    ) -> Result<RequiredDecision, HandlerError> {
        let e = DecisionEdit::new(title, decider, due, reversibility)?;
        let mut tx = self.pool.begin().await?;
        let mut d = self
            .ports
            .decisions
            .get_decision_for_update(&mut tx, id)
            .await?;
        if d.version() != version {
            tx.rollback().await?;
            return Err(HandlerError::Conflict {
                current: d.version(),
            });
        }
        let changes = match d.edit(e) {
            Ok(changes) => changes,
            Err(e) => {
                tx.rollback().await?;
                return Err(e.into());
            }
        };
        if changes.is_empty() {
            tx.rollback().await?;
            return Ok(d);
        }
        // Each changed field's old and new value goes into the entry, so a retitle of a satisfied decision stays
        // recoverable from the log.
        let mut fields = Vec::new();
        let mut from = serde_json::Map::new();
        let mut to = serde_json::Map::new();
        for ch in &changes {
            let (old, new) = match ch {
                FieldChange::Title { from: a, to: b } | FieldChange::Decider { from: a, to: b } => {
                    (serde_json::json!(a), serde_json::json!(b))
                }
                FieldChange::Due { from: a, to: b } => (serde_json::json!(a), serde_json::json!(b)),
                FieldChange::Reversibility { from: a, to: b } => {
                    (serde_json::json!(a), serde_json::json!(b))
                }
            };
            from.insert(ch.name().into(), old);
            to.insert(ch.name().into(), new);
            fields.push(ch.name());
        }
        self.ports.decisions.save_decision(&mut tx, &d).await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "decision",
                    &id.to_string(),
                    "edited",
                    serde_json::json!({"fields": fields, "from": from, "to": to}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.ports.decisions.get_decision(id).await?)
    }

    pub async fn get_decision(&self, id: i64) -> Result<RequiredDecision, HandlerError> {
        Ok(self.ports.decisions.get_decision(id).await?)
    }

    /// Every decision under an initiative: on it, its epics and their stories.
    pub async fn decisions_under_initiative(
        &self,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, HandlerError> {
        self.ports.initiatives.get_initiative(initiative).await?;
        Ok(self
            .ports
            .reads
            .list_decisions_under_initiative(initiative)
            .await?)
    }

    /// `open_with_record_only` keeps decisions that are open and carry a reference, whether or not `on` scopes the list.
    /// Without `on`, closed decisions are left out unless `include_closed`; a list scoped by `on` always has them (unless
    /// `open_with_record_only`, which wins over `include_closed`).
    pub async fn list_decisions(
        &self,
        on: Option<&Key>,
        open_with_record_only: bool,
        include_closed: bool,
    ) -> Result<Vec<RequiredDecision>, HandlerError> {
        Ok(match on {
            Some(k) => {
                let mut ds = self.ports.decisions.list_decisions_on(k).await?;
                if open_with_record_only {
                    ds.retain(|d| {
                        d.closed().is_none()
                            && d.record()
                                .is_some_and(|r| self.settings.repos.iter().any(|x| x == r.repo()))
                    });
                }
                ds
            }
            None => {
                let mut ds = self
                    .ports
                    .decisions
                    .list_all_decisions(
                        include_closed && !open_with_record_only,
                        open_with_record_only,
                    )
                    .await?;
                // The scheduled check reads only configured repositories: a record elsewhere waits, unlisted, until its
                // repository is configured.
                if open_with_record_only {
                    ds.retain(|d| {
                        d.record()
                            .is_some_and(|r| self.settings.repos.iter().any(|x| x == r.repo()))
                    });
                }
                ds
            }
        })
    }
}

/// The scope the queue covers. `On` and `Initiative` always include closed decisions.
#[derive(Debug, Clone)]
pub enum DecisionScope {
    All { include_closed: bool },
    On(Key),
    Initiative(Key),
}

impl Handlers {
    /// Spec §1: decisions and the readiness of every todo story, read in one REPEATABLE READ READ ONLY snapshot, then
    /// grouped and ordered by the pure queue. `now` is the caller's `Ctx.now`; its date decides what is overdue.
    pub async fn decision_queue(
        &self,
        scope: &DecisionScope,
        now: DateTime<Utc>,
    ) -> Result<(Vec<QueueEntry>, QueueSummary), HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = self.decision_queue_in(&mut tx, scope, now).await;
        end_read(tx, out).await
    }

    /// One decision's queue entry, from the same code as the queue (scoped to the decision's item, which holds every
    /// decision gating that item's stories), on one snapshot.
    ///
    /// This reads every unfinished story's readiness (a fixed number of statements, whatever the scope), because the
    /// entry's held and under-way stories come from it. Do not turn it into a per-story read.
    pub async fn decision_context(
        &self,
        id: i64,
        now: DateTime<Utc>,
    ) -> Result<QueueEntry, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let d = self.ports.decisions.get_decision_in(&mut tx, id).await?;
            let (entries, _) = self
                .decision_queue_in(&mut tx, &DecisionScope::On(d.on().clone()), now)
                .await?;
            entries
                .into_iter()
                .find(|e| e.decision.id() == id)
                .ok_or(HandlerError::NotFound)
        }
        .await;
        end_read(tx, out).await
    }

    /// The reads run over every unfinished story whatever the scope: a decision's affected stories can sit anywhere
    /// below its item, and the board read costs the same fixed statements either way. Unfinished (delivery not done,
    /// kb/ADR-0018 R5) is what makes a todo story "held" and a started one "under way"; `list_stories_in` is the
    /// board's own path, so the queue's readiness is the gate's. Goals (active on the stories, draft on one more read),
    /// lineage and urgency are read on the same snapshot, with statement counts independent of how many there are.
    async fn decision_queue_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: &DecisionScope,
        now: DateTime<Utc>,
    ) -> Result<(Vec<QueueEntry>, QueueSummary), HandlerError> {
        let ds = match scope {
            DecisionScope::All { include_closed } => {
                self.ports
                    .decisions
                    .list_all_decisions_in(tx, *include_closed)
                    .await?
            }
            DecisionScope::On(k) => self.ports.decisions.list_decisions_on_in(tx, k).await?,
            DecisionScope::Initiative(k) => {
                self.ports
                    .reads
                    .list_decisions_under_initiative_in(tx, k)
                    .await?
            }
        };
        let urg = self.urgency_in(tx, now).await?;
        let stories = self
            .list_stories_in(tx, &StoryFilter::default(), false, DoneWindow::None, &urg)
            .await?;
        let keys: Vec<Key> = stories.iter().map(|s| s.story.key.clone()).collect();
        let mut draft = self.ports.reads.draft_goals_of_stories(tx, &keys).await?;
        // `Key` is not `Ord`, so dedupe with a set; `unnest($1)` does not care about order.
        let items: Vec<Key> = ds
            .iter()
            .map(|d| d.on().clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let lineage = self.ports.reads.decision_lineage_in(tx, &items).await?;
        let affected: Vec<queue::AffectedStory> = stories
            .into_iter()
            .filter(|s| s.story.delivery != DeliveryState::Done)
            .map(|s| queue::AffectedStory {
                draft_goals: draft
                    .remove(&s.story.key)
                    .map(|g| g.into_iter().map(|(id, o, _)| (id, o)).collect())
                    .unwrap_or_default(),
                active_goals: s
                    .story
                    .goals
                    .iter()
                    .map(|g| (g.id, g.outcome.clone()))
                    .collect(),
                urgent: !s.story.urgent_sources.is_empty(),
                key: s.story.key,
                title: s.story.title,
                kind: s.story.kind,
                delivery: s.story.delivery,
                owner: s.story.owner,
                reasons: s.readiness.reasons,
            })
            .collect();
        let inp = queue::ClassifyInputs {
            stories: &affected,
            lineage: &lineage,
            first_bucket: self.settings.horizon_buckets.first().map(String::as_str),
            repos: &self.settings.repos,
        };
        Ok(queue::order(ds, &inp, now.date_naive()))
    }
}
