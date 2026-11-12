//! Promises (kb/ADR-0025) in the handlers: setting, changing and clearing one (R2), and the per-snapshot clock every
//! promise classification reads.
use super::*;
use crate::domain::promise::{Clock, PromiseError, Request, change, may_promise};
use reeve_protocol::SetPromise;

/// A domain refusal as the person reads it; `PromiseError` owns the wording.
fn refused(e: PromiseError) -> HandlerError {
    HandlerError::Validation(e.to_string())
}

/// R1: an initiative is never promised. No route reaches here with one; a direct caller gets a refusal, not a panic.
fn promisable(item: ItemType) -> Result<(), HandlerError> {
    if item == ItemType::Initiative {
        return Err(HandlerError::Validation(
            "an initiative cannot be promised".into(),
        ));
    }
    Ok(())
}

/// The clock `promise_clock_in` reads, built from every sprint already read on the caller's snapshot (the sprint views
/// read them all anyway): today, and the active and the draft sprint's planned ends.
pub(super) fn promise_clock_of(all: &[crate::domain::sprint::Sprint], now: DateTime<Utc>) -> Clock {
    let ends =
        |st: reeve_protocol::SprintState| all.iter().find(|s| s.state == st).map(|s| s.ends_on);
    Clock {
        today: now.date_naive(),
        active_ends: ends(reeve_protocol::SprintState::Active),
        draft_ends: ends(reeve_protocol::SprintState::Draft),
    }
}

impl Handlers {
    /// Today (the UTC date of `now`) and the active and draft sprints' planned ends, read once on the caller's snapshot
    /// in one statement. Every story and epic assembler classifies promises against this one value, beside `urgency_in`.
    pub async fn promise_clock_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        now: DateTime<Utc>,
    ) -> Result<Clock, HandlerError> {
        let (active_ends, draft_ends) = self.ports.reads.planned_ends_in(tx).await?;
        Ok(Clock {
            today: now.date_naive(),
            active_ends,
            draft_ends,
        })
    }

    /// Sets, changes or replaces the promise on a story or an epic (kb/ADR-0025 R2). Interactive only; takes and
    /// changes no version; logged as `promised` with the promise it replaced. An unchanged date and recipient writes and
    /// logs nothing.
    pub async fn set_promise(
        &self,
        c: &Ctx,
        item: ItemType,
        key: &Key,
        body: SetPromise,
    ) -> Result<(), HandlerError> {
        interactive(c)?;
        let mut tx = self.pool.begin().await?;
        self.set_promise_in(&mut tx, c, item, key, body).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Inside the caller's transaction: lock the row, apply the rules, write, and record the entry last. `previous` is
    /// the promise read under the row lock. An unchanged date and recipient writes and logs nothing. GRA-632 composes it
    /// with clearing an urgent mark (kb/ADR-0024 R4).
    pub(super) async fn set_promise_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        item: ItemType,
        key: &Key,
        body: SetPromise,
    ) -> Result<(), HandlerError> {
        interactive(c)?;
        promisable(item)?;
        let target = self
            .ports
            .promises
            .lock_for_promise(tx, item, key)
            .await?
            .ok_or(HandlerError::NotFound)?;
        if let Some(d) = target.delivery {
            may_promise(key, d).map_err(refused)?;
        }
        // `change` reads today from the clock and stamps `at`: both come from `c.now`, so they are the same UTC date.
        let clock = self.promise_clock_in(tx, c.now).await?;
        let req = Request::from(body);
        let p = change(target.promise.as_ref(), req, &c.actor, c.now, &clock).map_err(refused)?;
        // Nothing changes (a retry or a double submit): no write, no entry, and the promise keeps its `by`.
        if target
            .promise
            .as_ref()
            .is_some_and(|q| q.on() == p.on() && q.to() == p.to())
        {
            return Ok(());
        }
        if !self.ports.promises.set_promise(tx, item, key, &p).await? {
            return Err(HandlerError::NotFound);
        }
        let name = item_name(item);
        let previous = target
            .promise
            .as_ref()
            .map(|q| serde_json::json!({"on": q.on(), "to": q.to()}));
        self.activity
            .record(
                tx,
                self.rec(
                    c,
                    name,
                    &key.to_string(),
                    "promised",
                    serde_json::json!({"item_type": name, "on": p.on(), "to": p.to(), "previous": previous}),
                ),
            )
            .await?;
        Ok(())
    }

    /// Clears the promise on a story or an epic (R2). Interactive only; allowed on a done story (R2 refuses only
    /// setting). No entry when there was no promise; NotFound when no item of that type has the key.
    pub async fn clear_promise(
        &self,
        c: &Ctx,
        item: ItemType,
        key: &Key,
    ) -> Result<(), HandlerError> {
        interactive(c)?;
        let mut tx = self.pool.begin().await?;
        self.clear_promise_in(&mut tx, c, item, key).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Inside the caller's transaction: lock the row, clear, and record the entry last, with the promise read under the
    /// lock as `previous`. GRA-632 composes it with an urgent mark's actions.
    pub(super) async fn clear_promise_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        item: ItemType,
        key: &Key,
    ) -> Result<(), HandlerError> {
        interactive(c)?;
        promisable(item)?;
        let target = self
            .ports
            .promises
            .lock_for_promise(tx, item, key)
            .await?
            .ok_or(HandlerError::NotFound)?;
        let Some(prev) = target.promise else {
            return Ok(());
        };
        self.ports.promises.clear_promise(tx, item, key).await?;
        let name = item_name(item);
        self.activity
            .record(
                tx,
                self.rec(
                    c,
                    name,
                    &key.to_string(),
                    "unpromised",
                    serde_json::json!({"item_type": name, "previous": {"on": prev.on(), "to": prev.to()}}),
                ),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sprint::Sprint;
    use chrono::{NaiveDate, TimeZone};
    use reeve_protocol::SprintState;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }
    fn sprint(id: i64, state: SprintState, ends_on: NaiveDate) -> Sprint {
        Sprint {
            id,
            state,
            starts_on: d(10, 1),
            ends_on,
            opened_at: None,
            closed_at: None,
            version: 1,
        }
    }

    /// `promise_clock_of` follows `planned_ends_in`'s rule: the active and the draft sprint's planned ends, closed
    /// sprints ignored whatever their ends, None for a missing one; today is the UTC date of `now`.
    #[test]
    fn promise_clock_of_takes_the_active_and_draft_ends_only() {
        let now = Utc.with_ymd_and_hms(2026, 11, 12, 23, 30, 0).unwrap();
        let all = vec![
            sprint(1, SprintState::Closed, d(11, 30)),
            sprint(2, SprintState::Active, d(11, 24)),
            sprint(3, SprintState::Closed, d(12, 20)),
            sprint(4, SprintState::Draft, d(12, 8)),
        ];
        assert_eq!(
            promise_clock_of(&all, now),
            Clock {
                today: d(11, 12),
                active_ends: Some(d(11, 24)),
                draft_ends: Some(d(12, 8)),
            }
        );
        let closed_only = vec![
            sprint(1, SprintState::Closed, d(11, 30)),
            sprint(3, SprintState::Closed, d(12, 20)),
        ];
        assert_eq!(
            promise_clock_of(&closed_only, now),
            Clock {
                today: d(11, 12),
                active_ends: None,
                draft_ends: None,
            }
        );
        assert_eq!(
            promise_clock_of(&[sprint(4, SprintState::Draft, d(12, 8))], now).draft_ends,
            Some(d(12, 8))
        );
    }
}
