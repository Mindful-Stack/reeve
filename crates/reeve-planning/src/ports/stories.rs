use super::*;

/// The story rows, their ranks and their blocking edges. Mutations take the caller's transaction.
#[async_trait]
pub trait StoryRepo: Send + Sync {
    async fn insert_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Story,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError>;
    async fn get_story(&self, key: &Key) -> Result<Story, RepoError>;
    async fn get_story_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Story, RepoError>;
    /// The stories matching `f`, on the caller's transaction: by delivery column, then rank, then key number. With
    /// `done_since`, a done story is kept only if it entered done at or after that instant (one with no `done_at` is
    /// not); stories in other states are unaffected.
    async fn list_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        f: &StoryFilter,
        done_since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Story>, RepoError>;
    /// Stories whose title is similar to `text` (pg_trgm `similarity` at or above `min`), not done or done at or after
    /// `done_since`; most similar first, then newest; at most `limit`. The caller owns the clock (kb/ADR-0010 R4).
    async fn similar_stories(
        &self,
        text: &str,
        min: f32,
        done_since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Story>, RepoError>;
    async fn update_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Story,
        rank: Option<i32>,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
    /// Writes a story's delivery and resolution after `Story::move_to`, stamping `started_at`/`done_at` from the locked
    /// row's state, guarded by `expected_version`. The owner is written as `COALESCE(owner, story.owner())`: a non-null
    /// stored owner is kept and `story.owner()` lands only when the row has none, so an owner change must not rely on
    /// this method. The version is a
    /// separate argument, not `story.version()`: it is the version the caller observed and checked, and the UPDATE
    /// matches the row only at that version, so a write that raced the caller is a conflict rather than overwritten.
    async fn set_delivery(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Story,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
    /// The todo column in rank order (rank, then shorter key, then key), every row locked FOR UPDATE in key order so
    /// two concurrent re-ranks queue instead of deadlocking.
    async fn todo_column_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Key>, RepoError>;
    /// Writes every (key, rank) pair in one statement. Ranks are ordering metadata: no version or `updated_at` changes.
    async fn set_ranks(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ranks: &[(Key, i32)],
    ) -> Result<(), RepoError>;
    /// Bumps one story's version (and `updated_at`) at the expected version; Conflict when it moved on.
    async fn touch_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
    async fn add_block(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, RepoError>;
    /// Deletes one edge; false when there was none (nothing deleted).
    async fn remove_block(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, RepoError>;
}
