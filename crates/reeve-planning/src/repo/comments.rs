use super::*;

#[async_trait]
#[allow(clippy::too_many_arguments)]
impl CommentRepo for PgPlanningRepo {
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
    ) -> Result<(i64, bool), RepoError> {
        // Two unique keys can conflict (external_ref for imports, idempotency_key for callers), so DO NOTHING has no target
        // and the existing row is looked up by whichever key was supplied.
        let inserted: Option<i64> = sqlx::query_scalar("INSERT INTO comments (story_key, decision_id, author, body, at, external_ref, idempotency_key) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING RETURNING id")
            .bind(story.map(|k| k.to_string())).bind(decision).bind(author).bind(body).bind(at).bind(external_ref).bind(idempotency_key).fetch_optional(&mut **tx).await?;
        if let Some(id) = inserted {
            return Ok((id, true));
        }
        let id: i64 = sqlx::query_scalar(
            "SELECT id FROM comments WHERE idempotency_key = $1 OR external_ref = $2 ORDER BY id LIMIT 1",
        )
        .bind(idempotency_key)
        .bind(external_ref)
        .fetch_one(&mut **tx)
        .await?;
        Ok((id, false))
    }
    async fn list_comments_for_story(
        &self,
        story: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        comments(&self.pool, "story_key = $1", story.to_string()).await
    }
    async fn list_comments_for_story_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        comments(&mut **tx, "story_key = $1", story.to_string()).await
    }
    async fn list_comments_for_decision(
        &self,
        id: i64,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        comments(&self.pool, "decision_id = $1::bigint", id.to_string()).await
    }
    async fn insert_item_comment(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        on: &Key,
        author: &str,
        body: &str,
        idempotency_key: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(i64, bool), RepoError> {
        let column = match item {
            ItemType::Initiative => "initiative_key",
            ItemType::Epic => "epic_key",
            ItemType::Story => "story_key",
        };
        let inserted: Option<i64> = sqlx::query_scalar(&format!("INSERT INTO comments ({column}, author, body, at, idempotency_key) VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING RETURNING id"))
            .bind(on.to_string()).bind(author).bind(body).bind(at).bind(idempotency_key).fetch_optional(&mut **tx).await?;
        if let Some(id) = inserted {
            return Ok((id, true));
        }
        let id: i64 = sqlx::query_scalar(
            "SELECT id FROM comments WHERE idempotency_key = $1 ORDER BY id LIMIT 1",
        )
        .bind(idempotency_key)
        .fetch_one(&mut **tx)
        .await?;
        Ok((id, false))
    }
    async fn list_comments_for_item(
        &self,
        item: ItemType,
        on: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        let column = match item {
            ItemType::Initiative => "initiative_key = $1",
            ItemType::Epic => "epic_key = $1",
            ItemType::Story => "story_key = $1",
        };
        comments(&self.pool, column, on.to_string()).await
    }
}
