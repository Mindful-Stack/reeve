use super::*;

/// Reads that join other aggregates' tables, and the export readers.
#[async_trait]
pub trait PlanningReads: Send + Sync {
    /// Readiness facts for many stories in a fixed number of statements, whatever their number, on the caller's
    /// transaction. A key with no story row has no entry.
    async fn readiness_inputs(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, ReadinessInput>, RepoError>;
    /// The blockers of many stories in one statement on the caller's transaction, each list in key-number order
    /// (`ABC-9` before `ABC-10`); a story without blockers has no entry.
    async fn blockers_of_many(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<Key>>, RepoError>;
    /// The stories with these keys, in the order given; a key with no row is skipped. One statement. Callers pass
    /// distinct keys.
    async fn stories_by_keys(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        keys: &[Key],
    ) -> Result<Vec<Story>, RepoError>;
    /// The pull requests linked to many stories in one statement on the caller's transaction, each list by repo then
    /// number; a story without pull requests has no entry.
    async fn pull_requests_for_many(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<PullRequest>>, RepoError>;
    /// The same lookup inside the caller's transaction (the import reads it under `lock_external_ref`).
    /// `find_by_idempotency` on the caller's transaction: a check made while it holds a lock takes no second connection.
    /// Initiatives, epics and stories whose title, brief or description, or external reference contains `text`
    /// (case-insensitive, taken literally: `%` and `_` are not wildcards), or whose key is `exact`; at most `limit`,
    /// in tiers: that key, an exact external reference, title matches, the rest; newest first within a tier.
    async fn search_items(
        &self,
        text: &str,
        exact: Option<&Key>,
        limit: i64,
    ) -> Result<Vec<SearchRow>, RepoError>;
    /// The story counts under one initiative (through its epics), on the caller's transaction.
    async fn story_counts_under_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<StoryCounts, RepoError>;
    /// Story counts for every initiative that has a story, in one statement on the caller's transaction; an initiative
    /// absent from the map has none.
    async fn story_counts_by_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<HashMap<Key, StoryCounts>, RepoError>;
    /// The decisions on an initiative, on its epics and on their stories, closed ones included, oldest first:
    /// each read where it lives, never copied.
    async fn list_decisions_under_initiative(
        &self,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// `list_decisions_under_initiative` on the caller's transaction.
    async fn list_decisions_under_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// The active sprint's goals each story serves (through itself, its epic or its epic's initiative), in one statement.
    async fn active_goals_of_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<(i64, String, reeve_protocol::GoalVia)>>, RepoError>;
    /// The stories a sprint's numbers need, in one statement: covered by `linked` keys (as story, epic or initiative),
    /// or completed in `[from, to)` (the upkeep share's denominator); `started_at` comes along for "in flight".
    async fn sprint_story_times(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        linked: &[Key],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoryTimes>, RepoError>;
    // The export readers: every row of one table, read on the caller's transaction so the export is one
    // snapshot. Items order by key number, decisions and comments by id, all ascending.
    async fn initiatives_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Initiative>, RepoError>;
    async fn epics_in(&self, tx: &mut Transaction<'_, Postgres>) -> Result<Vec<Epic>, RepoError>;
    async fn stories_in(&self, tx: &mut Transaction<'_, Postgres>)
    -> Result<Vec<Story>, RepoError>;
    async fn decisions_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    async fn comments_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
    /// The lineage `urgency` needs: epics whose key is in `epics` or whose initiative is in `initiatives`, and the
    /// stories under `epics` or under those initiatives' epics. Two statements; none when both inputs are empty.
    async fn urgency_lineage_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        epics: &[Key],
        initiatives: &[Key],
    ) -> Result<Lineage, RepoError>;
}
