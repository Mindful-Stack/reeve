use super::*;

/// The urgent mark on an initiative, an epic or a story (kb/ADR-0017): stored on the marked row only, never part
/// of the item, never versioned.
#[async_trait]
pub trait UrgencyRepo: Send + Sync {
    /// Every stored urgent mark (active or expired) with its item's parent and done fact (kb/ADR-0017). One statement.
    async fn urgent_marks_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<MarkFact>, RepoError>;
    /// Writes the mark on one item, replacing any; touches no other column (not `version`, not `updated_at`).
    /// Ok(false) when no item of that type has the key.
    async fn set_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
        mark: &UrgentMark,
    ) -> Result<bool, RepoError>;
    /// Renews the mark a row still carries: sets its time and author only, keeping the reason the row holds when the
    /// write lands, and returns that reason. Ok(None) when the row carries no mark (or does not exist), so a renewal
    /// never revives a mark cleared meanwhile. Touches no other column.
    async fn renew_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
        at: DateTime<Utc>,
        by: &str,
    ) -> Result<Option<Option<String>>, RepoError>;
    /// Clears the mark; Ok(false) when the item carried none (or does not exist). Touches no other column.
    async fn clear_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
    ) -> Result<bool, RepoError>;
}
