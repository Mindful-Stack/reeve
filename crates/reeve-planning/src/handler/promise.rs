//! Promises (kb/ADR-0025) in the handlers: the per-snapshot clock every promise classification reads.
use super::*;
use crate::domain::promise::Clock;

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
}
