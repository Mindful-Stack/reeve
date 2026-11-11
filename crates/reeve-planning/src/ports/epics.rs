use super::*;

/// The epic rows.
#[async_trait]
pub trait EpicRepo: Send + Sync {
    async fn insert_epic(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: &Epic,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError>;
    async fn get_epic(&self, key: &Key) -> Result<Epic, RepoError>;
    /// `get_epic` on the caller's transaction.
    async fn get_epic_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Epic, RepoError>;
    /// Every epic (of one initiative when given), oldest first (ties by key number), on the caller's transaction.
    async fn list_epics_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        initiative: Option<&Key>,
    ) -> Result<Vec<Epic>, RepoError>;
    /// The keys of one initiative's epics, in no particular order, on the caller's transaction (the story list's
    /// `initiative` filter reads keys only).
    async fn epic_keys_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        initiative: &Key,
    ) -> Result<Vec<Key>, RepoError>;
    async fn update_epic(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: &Epic,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
}
