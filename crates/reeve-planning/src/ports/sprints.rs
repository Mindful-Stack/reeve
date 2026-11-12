use super::*;

/// Sprints, goals and goal links (kb/ADR-0014). Every mutation takes the caller's transaction and the handler's timestamp.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait SprintRepo: Send + Sync {
    /// The sprint in this state, if any (at most one draft and one active exist).
    async fn sprint_in_state(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        state: reeve_protocol::SprintState,
    ) -> Result<Option<Sprint>, RepoError>;
    /// Locks the sprint row; NotFound when absent.
    async fn sprint_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Sprint, RepoError>;
    async fn get_sprint_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Sprint, RepoError>;
    /// Every sprint, newest first.
    async fn sprints_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Sprint>, RepoError>;
    /// The active and the draft sprint (at most one each), in one statement: the planned ends the promise clock reads
    /// (`Clock::at`, kb/ADR-0025 R5).
    async fn planned_ends_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Sprint>, RepoError>;
    /// Inserts a draft; `Ok(None)` when a draft already exists (the partial unique index; `ON CONFLICT DO NOTHING`).
    async fn insert_draft(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        starts_on: NaiveDate,
        ends_on: NaiveDate,
        now: DateTime<Utc>,
    ) -> Result<Option<Sprint>, RepoError>;
    /// Writes state, dates and timestamps at `s.version`, bumping it; Conflict otherwise. A second active sprint
    /// surfaces as a database unique violation (23505) the handler maps to `sprint_active`.
    async fn save_sprint(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Sprint,
    ) -> Result<(), RepoError>;
    /// Returns (goal, inserted); a repeated idempotency key answers the first goal and false.
    async fn insert_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        sprint_id: i64,
        outcome: &str,
        check: &str,
        owner: &str,
        notes: &str,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<(Goal, bool), RepoError>;
    /// Locks the goal row; NotFound when absent.
    async fn goal_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Goal, RepoError>;
    async fn get_goal_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Goal, RepoError>;
    /// Writes every mutable field at `g.version`, bumping it; Conflict otherwise.
    async fn save_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        g: &Goal,
    ) -> Result<(), RepoError>;
    /// Deletes the goal and, by cascade, its links; NotFound when absent.
    async fn delete_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<(), RepoError>;
    /// The goals of these sprints, by sprint then id, in one statement.
    async fn goals_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        sprint_ids: &[i64],
    ) -> Result<Vec<Goal>, RepoError>;
    /// The goals with these ids, in one statement.
    async fn goals_by_ids_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ids: &[i64],
    ) -> Result<Vec<Goal>, RepoError>;
    /// False when the link already existed.
    async fn insert_goal_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_id: i64,
        item: &Key,
        by: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError>;
    /// False when there was no such link.
    async fn delete_goal_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_id: i64,
        item: &Key,
    ) -> Result<bool, RepoError>;
    /// The links of these goals, by goal then key number, in one statement.
    async fn links_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_ids: &[i64],
    ) -> Result<Vec<GoalLink>, RepoError>;
}
