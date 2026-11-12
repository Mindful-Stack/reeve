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
