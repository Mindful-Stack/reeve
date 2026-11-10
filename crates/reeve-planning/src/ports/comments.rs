use super::*;

/// The comment rows.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait CommentRepo: Send + Sync {
    /// `at` is the comment's own date: the caller's `now`, or the original date of an imported comment.
    async fn insert_comment(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: Option<&Key>,
        decision: Option<i64>,
        author: &str,
        body: &str,
        external_ref: Option<&str>,
        idempotency_key: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(i64, bool), RepoError>;
    async fn list_comments_for_story(
        &self,
        story: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
    /// `list_comments_for_story` on the caller's transaction.
    async fn list_comments_for_story_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
    async fn list_comments_for_decision(
        &self,
        id: i64,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
    /// A comment on an initiative, an epic or a story, by its key; a repeated idempotency key answers the first
    /// comment's id and false.
    async fn insert_item_comment(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        on: &Key,
        author: &str,
        body: &str,
        idempotency_key: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(i64, bool), RepoError>;
    async fn list_comments_for_item(
        &self,
        item: ItemType,
        on: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
}
