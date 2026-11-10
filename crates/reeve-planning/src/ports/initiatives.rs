use super::*;

/// The initiative rows.
#[async_trait]
pub trait InitiativeRepo: Send + Sync {
    /// Returns false when a row with this idempotency key already exists (nothing inserted). Inserts write every field
    /// of the item, its discovery mark and external reference included (the import creates items in their state).
    async fn insert_initiative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        i: &Initiative,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError>;
    async fn get_initiative(&self, key: &Key) -> Result<Initiative, RepoError>;
    /// `get_initiative` on the caller's transaction.
    async fn get_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Initiative, RepoError>;
    /// Every initiative, oldest first (ties by key number), on the caller's transaction.
    async fn list_initiatives_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Initiative>, RepoError>;
    async fn update_initiative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        i: &Initiative,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
}
