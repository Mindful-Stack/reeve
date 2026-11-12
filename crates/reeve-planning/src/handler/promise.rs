//! Promises (kb/ADR-0025) in the handlers: setting, changing and clearing one (R2), and the per-snapshot clock every
//! promise classification reads.
use super::*;
use crate::domain::promise::{Clock, PromiseError, PromiseItem, Request, change};
use reeve_protocol::SetPromise;

/// A domain refusal as the person reads it; `PromiseError` owns the wording.
fn refused(e: PromiseError) -> HandlerError {
    HandlerError::Validation(e.to_string())
}

/// R1: an initiative is never promised. No route reaches here with one; a direct caller gets a refusal before the
/// port is touched.
fn promisable(item: ItemType) -> Result<PromiseItem, HandlerError> {
    PromiseItem::of(item)
        .ok_or_else(|| HandlerError::Validation("an initiative cannot be promised".into()))
}

impl Handlers {
    /// Today (the UTC date of `now`) and the active and draft sprints' planned ends, read once on the caller's snapshot
    /// in one statement and assembled by `Clock::at`. Every story and epic assembler classifies promises against this
    /// one value, beside `urgency_in`.
    pub async fn promise_clock_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        now: DateTime<Utc>,
    ) -> Result<Clock, HandlerError> {
        let open = self.ports.sprints.planned_ends_in(tx).await?;
        Ok(Clock::at(now, &open))
    }

    /// Sets, changes or replaces the promise on a story or an epic (kb/ADR-0025 R2). Interactive only; takes and
    /// changes no version; logged as `promised` with the promise it replaced. An unchanged date and recipient writes and
    /// logs nothing (`change` returns None).
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
        let which = promisable(item)?;
        let target = self
            .ports
            .promises
            .lock_for_promise(tx, which, key)
            .await?
            .ok_or(HandlerError::NotFound)?;
        // `change` judges dates by the clock's today and stamps `at`: both come from `c.now` (its contract).
        let clock = self.promise_clock_in(tx, c.now).await?;
        let req = Request::from(body);
        let Some(p) = change(
            key,
            target.item,
            target.promise.as_ref(),
            req,
            &c.actor,
            c.now,
            &clock,
        )
        .map_err(refused)?
        else {
            // Nothing changes (a retry or a double submit): no write, no entry, and the promise keeps its `by`.
            return Ok(());
        };
        if !self.ports.promises.set_promise(tx, which, key, &p).await? {
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

    /// Clears the promise on a story or an epic (R2), logged as `promise_cleared`. Interactive only; allowed on a done
    /// story (R2 refuses only setting). No entry when there was no promise; NotFound when no item of that type has the key.
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
        let which = promisable(item)?;
        let target = self
            .ports
            .promises
            .lock_for_promise(tx, which, key)
            .await?
            .ok_or(HandlerError::NotFound)?;
        let Some(prev) = target.promise else {
            return Ok(());
        };
        self.ports.promises.clear_promise(tx, which, key).await?;
        let name = item_name(item);
        self.activity
            .record(
                tx,
                self.rec(
                    c,
                    name,
                    &key.to_string(),
                    "promise_cleared",
                    serde_json::json!({"item_type": name, "previous": {"on": prev.on(), "to": prev.to()}}),
                ),
            )
            .await?;
        Ok(())
    }
}
