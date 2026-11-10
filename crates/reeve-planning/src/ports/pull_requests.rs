use super::*;

/// The pull-request rows. Nothing here writes delivery, resolution or a story's version (kb/ADR-0002 R4).
#[async_trait]
pub trait PullRequestRepo: Send + Sync {
    /// Conflict-safe upsert keyed by (repo, number): `INSERT ... ON CONFLICT DO NOTHING RETURNING` (a returned row
    /// means Linked); otherwise lock the existing row `FOR UPDATE` and decide with `pull_request::decide`, writing
    /// only on Updated/Relinked. Never touches the story row (its version and `updated_at` stay as they were).
    async fn upsert_pull_request(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        pr: &PullRequest,
    ) -> Result<PrChange, RepoError>;
}
