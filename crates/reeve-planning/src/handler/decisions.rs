use super::*;
use crate::domain::decision::*;
use crate::domain::queue::{self, QueueEntry, QueueSummary};
use crate::domain::readiness::satisfies;
use reeve_protocol::{CheckResult, ClosureKind, DeliveryState, Key, Reversibility, TokenKind, Via};

/// A reference names a repository and a path inside it: relative, without a `..` component, and not starting with `-`,
/// so a checker never reads outside the repository or hands git an option. The repository need not be configured yet
///: the reference is stored, readiness says the record is in an unchecked repository, and the scheduled check,
/// which reads only configured repositories (kb/ADR-0009 R2), picks it up once the repository is configured. Its name
/// is one plain directory name, as a checker uses it. Returns the path in normal form, without empty and `.` components
/// (`./a//b` is `a/b`): git names tree entries that way, and the checker compares the stored path with that name. A path
/// that is nothing but such components names no file.
fn validate_reference(repo: &str, path: &str) -> Result<String, HandlerError> {
    if repo.starts_with(['-', '.'])
        || repo.contains(['/', '\\'])
        || repo.chars().any(char::is_whitespace)
    {
        return Err(HandlerError::Validation(format!(
            "repo `{repo}` is not a single repository name"
        )));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(HandlerError::Validation(format!(
            "path `{path}` must be relative to the repository"
        )));
    }
    if path.split(['/', '\\']).any(|c| c == "..") {
        return Err(HandlerError::Validation(format!(
            "path `{path}` must not contain a `..` component"
        )));
    }
    let normal = path
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect::<Vec<_>>()
        .join("/");
    if normal.is_empty() {
        return Err(HandlerError::Validation(format!(
            "path `{path}` names no file"
        )));
    }
    // Checked on the normal form, which is what git is handed: `./-x.md` is `-x.md`.
    if normal.starts_with('-') {
        return Err(HandlerError::Validation(format!(
            "path `{path}` must not start with `-`"
        )));
    }
    Ok(normal)
}

/// A decision's PR url is rendered as a link, so it must be an absolute `http(s)://` URL with a host: a `javascript:`
/// or `data:` url stored here would run in the web client when clicked. Returns the trimmed url.
fn validate_pr_url(pr_url: Option<String>) -> Result<Option<String>, HandlerError> {
    let Some(raw) = pr_url else { return Ok(None) };
    let t = raw.trim();
    let ok = url::Url::parse(t)
        .map(|u| {
            matches!(u.scheme(), "http" | "https") && u.host_str().is_some_and(|h| !h.is_empty())
        })
        .unwrap_or(false);
    if !ok {
        return Err(HandlerError::Validation(format!(
            "pr_url `{t}` must be an absolute http(s) URL"
        )));
    }
    Ok(Some(t.to_string()))
}

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
            .repo
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
                        &d.id.to_string(),
                        "required",
                        serde_json::json!({"on": on, "title": d.title, "decider": d.decider, "reversibility": d.reversibility}),
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
        if repo.trim().is_empty() || path.trim().is_empty() || record_id.trim().is_empty() {
            return Err(HandlerError::Validation(
                "repo, path and record_id are required".into(),
            ));
        }
        let path = &validate_reference(repo.trim(), path.trim())?;
        let pr_url = validate_pr_url(pr_url)?;
        let mut tx = self.pool.begin().await?;
        let cur = self.repo.get_decision_for_update(&mut tx, id).await?;
        if cur.closed.is_some() {
            return Err(HandlerError::Validation("decision is closed".into()));
        }
        let d = cur.with_record(
            RecordRef {
                repo: repo.trim().into(),
                path: path.clone(),
                record_id: record_id.trim().into(),
            },
            pr_url,
        );
        self.repo.save_decision(&mut tx, &d).await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "decision",
                    &id.to_string(),
                    "linked",
                    serde_json::json!({"repo": repo.trim(), "record_id": record_id.trim(), "path": path, "pr_url": d.pr_url}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.repo.get_decision(id).await?)
    }

    /// `for_record` and `observed_version` are what the CLI saw when it ran the check; a result for a reference or
    /// version that has changed since is refused with conflict (kb/ADR-0004 R3). Comparing the version closes the
    /// relink-A-to-B-to-A hole that comparing the record id alone leaves open.
    ///
    /// `is_admin` is the caller's current role, decided by the route (kb/ADR-0010 R2). Only an admin may store the
    /// satisfying result (kb/ADR-0003 R3), from any token kind (storing a result is not an override). A non-admin's is
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
        let mut d = self.repo.get_decision_for_update(&mut tx, id).await?;
        if d.version != observed_version {
            return Err(HandlerError::Conflict { current: d.version });
        }
        match &d.record {
            None => {
                return Err(HandlerError::Validation(
                    "decision has no record reference".into(),
                ));
            }
            Some(r) if r.record_id != for_record => {
                return Err(HandlerError::Conflict { current: d.version });
            }
            _ => {}
        }
        if d.closed.is_some() {
            return Err(HandlerError::Validation("decision is closed".into()));
        }
        // The scheduled check posts every open, linked decision every few minutes; logging and bumping an unchanged
        // result would bury people's entries in the activity log and refetch every client on each run.
        if same_outcome(&d.check, &check) {
            self.repo.touch_decision_checked(&mut tx, id, c.now).await?;
            tx.commit().await?;
            return Ok(self.repo.get_decision(id).await?);
        }
        d.check = check;
        d.checked_at = Some(c.now);
        self.repo.save_decision(&mut tx, &d).await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "decision",
                    &id.to_string(),
                    "checked",
                    serde_json::json!({"check": d.check}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.repo.get_decision(id).await?)
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
        let mut d = self.repo.get_decision_for_update(&mut tx, id).await?;
        d.close(kind, reason, &c.actor, via, c.now)?;
        self.repo.save_decision(&mut tx, &d).await?;
        let reason = &d.closed.as_ref().expect("just closed").reason;
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
        Ok(self.repo.get_decision(id).await?)
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
        let title = title.map(|t| t.trim().to_string());
        let decider = decider.map(|d| d.trim().to_string());
        if title.as_deref() == Some("") || decider.as_deref() == Some("") {
            return Err(HandlerError::Validation(
                "title and decider must not be blank".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let mut d = self.repo.get_decision_for_update(&mut tx, id).await?;
        if d.version != version {
            tx.rollback().await?;
            return Err(HandlerError::Conflict { current: d.version });
        }
        if d.closed.is_some() {
            tx.rollback().await?;
            return Err(HandlerError::Validation("decision is closed".into()));
        }
        // Each changed field's old and new value goes into the entry, so a retitle of a satisfied decision stays
        // recoverable from the log.
        let mut fields = Vec::new();
        let mut from = serde_json::Map::new();
        let mut to = serde_json::Map::new();
        if let Some(t) = title
            && t != d.title
        {
            from.insert("title".into(), serde_json::json!(d.title));
            to.insert("title".into(), serde_json::json!(t));
            d.title = t;
            fields.push("title");
        }
        if let Some(x) = decider
            && x != d.decider
        {
            from.insert("decider".into(), serde_json::json!(d.decider));
            to.insert("decider".into(), serde_json::json!(x));
            d.decider = x;
            fields.push("decider");
        }
        if let Some(due) = due
            && due != d.due
        {
            from.insert("due".into(), serde_json::json!(d.due));
            to.insert("due".into(), serde_json::json!(due));
            d.due = due;
            fields.push("due");
        }
        if let Some(r) = reversibility
            && r != d.reversibility
        {
            from.insert("reversibility".into(), serde_json::json!(d.reversibility));
            to.insert("reversibility".into(), serde_json::json!(r));
            d.reversibility = r;
            fields.push("reversibility");
        }
        if fields.is_empty() {
            tx.rollback().await?;
            return Ok(d);
        }
        self.repo.save_decision(&mut tx, &d).await?;
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
        Ok(self.repo.get_decision(id).await?)
    }

    pub async fn get_decision(&self, id: i64) -> Result<RequiredDecision, HandlerError> {
        Ok(self.repo.get_decision(id).await?)
    }

    /// `open_with_record_only` keeps decisions that are open and carry a reference, whether or not `on` scopes the list.
    /// Without `on`, closed decisions are left out unless `include_closed`; a list scoped by `on` always has them (unless
    /// `open_with_record_only`, which wins over `include_closed`).
    /// Every decision under an initiative: on it, its epics and their stories.
    pub async fn decisions_under_initiative(
        &self,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, HandlerError> {
        self.repo.get_initiative(initiative).await?;
        Ok(self
            .repo
            .list_decisions_under_initiative(initiative)
            .await?)
    }

    pub async fn list_decisions(
        &self,
        on: Option<&Key>,
        open_with_record_only: bool,
        include_closed: bool,
    ) -> Result<Vec<RequiredDecision>, HandlerError> {
        Ok(match on {
            Some(k) => {
                let mut ds = self.repo.list_decisions_on(k).await?;
                if open_with_record_only {
                    ds.retain(|d| {
                        d.closed.is_none()
                            && d.record
                                .as_ref()
                                .is_some_and(|r| self.settings.repos.contains(&r.repo))
                    });
                }
                ds
            }
            None => {
                let mut ds = self
                    .repo
                    .list_all_decisions(
                        include_closed && !open_with_record_only,
                        open_with_record_only,
                    )
                    .await?;
                // The scheduled check reads only configured repositories: a record elsewhere waits, unlisted, until its
                // repository is configured.
                if open_with_record_only {
                    ds.retain(|d| {
                        d.record
                            .as_ref()
                            .is_some_and(|r| self.settings.repos.contains(&r.repo))
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

    /// The fold runs over every todo story whatever the scope: a decision's blocked stories can sit anywhere below its
    /// item, and the board read costs the same fixed statements either way. `delivery: Todo` is what makes "blocked"
    /// mean "cannot start"; `list_stories_in` is the board's own path, so the queue's readiness is the gate's.
    async fn decision_queue_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: &DecisionScope,
        now: DateTime<Utc>,
    ) -> Result<(Vec<QueueEntry>, QueueSummary), HandlerError> {
        let ds = match scope {
            DecisionScope::All { include_closed } => {
                self.repo.list_all_decisions_in(tx, *include_closed).await?
            }
            DecisionScope::On(k) => self.repo.list_decisions_on_in(tx, k).await?,
            DecisionScope::Initiative(k) => {
                self.repo.list_decisions_under_initiative_in(tx, k).await?
            }
        };
        let todo = StoryFilter {
            delivery: Some(DeliveryState::Todo),
            ..Default::default()
        };
        let stories = self.list_stories_in(tx, &todo, false, None).await?;
        Ok(queue::order(ds, queue::blocks(&stories), now.date_naive()))
    }
}
