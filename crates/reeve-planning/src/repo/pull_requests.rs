use super::*;

#[async_trait]
impl PullRequestRepo for PgPlanningRepo {
    async fn upsert_pull_request(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        pr: &PullRequest,
    ) -> Result<PrChange, RepoError> {
        let incoming = story;
        let story = story.to_string();
        // A concurrent first delivery waits here on the unique index until the winner commits, then sees the
        // conflict and falls through to the locking path below: no unique violation ever reaches the caller.
        let inserted: Option<String> = sqlx::query_scalar(
            "INSERT INTO story_prs (story_key, repo, number, url, title, status, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (repo, number) DO NOTHING RETURNING story_key",
        )
        .bind(&story)
        .bind(&pr.repo)
        .bind(pr.number)
        .bind(&pr.url)
        .bind(&pr.title)
        .bind(pr_status_str(pr.status))
        .bind(pr.updated_at)
        .fetch_optional(&mut **tx)
        .await?;
        if inserted.is_some() {
            return Ok(PrChange::Linked);
        }
        let r = sqlx::query(
            "SELECT story_key, repo, number, url, title, status, updated_at FROM story_prs \
             WHERE repo = $1 AND number = $2 FOR UPDATE",
        )
        .bind(&pr.repo)
        .bind(pr.number)
        .fetch_one(&mut **tx)
        .await?;
        let stored_story = key(r.get::<&str, _>("story_key"));
        let stored = pull_request_from(&r)?;
        let change = decide(&stored_story, &stored, incoming, pr);
        if matches!(
            change,
            PrChange::Updated | PrChange::Touched | PrChange::Relinked { .. }
        ) {
            sqlx::query(
                "UPDATE story_prs SET story_key = $3, url = $4, title = $5, status = $6, updated_at = $7 \
                 WHERE repo = $1 AND number = $2",
            )
            .bind(&pr.repo)
            .bind(pr.number)
            .bind(&story)
            .bind(&pr.url)
            .bind(&pr.title)
            .bind(pr_status_str(pr.status))
            .bind(pr.updated_at)
            .execute(&mut **tx)
            .await?;
        }
        Ok(change)
    }
}
