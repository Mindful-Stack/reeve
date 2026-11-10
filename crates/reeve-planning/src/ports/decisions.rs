use super::*;

/// The required-decision rows.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait DecisionRepo: Send + Sync {
    /// Returns (decision, inserted). When the idempotency key already exists, returns the existing row and false.
    async fn insert_decision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        on: ItemType,
        on_key: &Key,
        title: &str,
        decider: &str,
        due: Option<chrono::NaiveDate>,
        reversibility: Option<reeve_protocol::Reversibility>,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<(RequiredDecision, bool), RepoError>;
    async fn get_decision(&self, id: i64) -> Result<RequiredDecision, RepoError>;
    async fn get_decision_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<RequiredDecision, RepoError>;
    async fn list_decisions_on(&self, on_key: &Key) -> Result<Vec<RequiredDecision>, RepoError>;
    /// `list_decisions_on` on the caller's transaction.
    async fn list_decisions_on_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        on_key: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// Every decision, oldest first: closed ones only when `include_closed`, and only those carrying a record when
    /// `with_record_only`.
    async fn list_all_decisions(
        &self,
        include_closed: bool,
        with_record_only: bool,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// `list_all_decisions(include_closed, false)` on the caller's transaction.
    async fn list_all_decisions_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        include_closed: bool,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// Writes the row only if `version` still equals `d.version`, then bumps it; Conflict otherwise.
    async fn save_decision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        d: &RequiredDecision,
    ) -> Result<(), RepoError>;
    /// Sets `checked_at` only, leaving the version as it is: for a check whose result equals the stored one. The caller
    /// holds the row lock (`get_decision_for_update`) and has compared the version.
    async fn touch_decision_checked(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        at: DateTime<Utc>,
    ) -> Result<(), RepoError>;
}
