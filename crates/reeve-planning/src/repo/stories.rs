use super::*;

#[async_trait]
impl StoryRepo for PgPlanningRepo {
    async fn insert_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Story,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("INSERT INTO stories (key, title, kind, epic_key, description, criteria, owner, labels, rank, delivery, resolution, external_ref, version, idempotency_key, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,1,$13,$14,$14) ON CONFLICT (idempotency_key) DO NOTHING")
            .bind(s.key().to_string()).bind(s.title()).bind(kind_str(s.kind())).bind(s.epic().map(|k| k.to_string())).bind(s.description()).bind(criteria_json(s.criteria())).bind(s.owner()).bind(s.labels().to_vec()).bind(s.rank()).bind(delivery_str(s.delivery())).bind(s.resolution().map(resolution_str)).bind(s.external_ref()).bind(idem).bind(now).execute(&mut **tx).await?;
        Ok(r.rows_affected() == 1)
    }
    async fn get_story(&self, k: &Key) -> Result<Story, RepoError> {
        let r = sqlx::query(&format!("SELECT {STORY_COLS} FROM stories WHERE key = $1"))
            .bind(k.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(RepoError::NotFound)?;
        Ok(story_from(&r)?)
    }
    async fn get_story_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
    ) -> Result<Story, RepoError> {
        let r = sqlx::query(&format!(
            "SELECT {STORY_COLS} FROM stories WHERE key = $1 FOR UPDATE"
        ))
        .bind(k.to_string())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RepoError::NotFound)?;
        Ok(story_from(&r)?)
    }
    async fn list_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        f: &StoryFilter,
        done_since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Story>, RepoError> {
        let rows = sqlx::query(&format!("SELECT {STORY_COLS} FROM stories WHERE ($1::text IS NULL OR epic_key = $1) AND ($2::text IS NULL OR owner = $2) AND ($3::text IS NULL OR delivery = $3) AND (NOT $4 OR (epic_key IS NULL AND delivery <> 'done')) AND ($5::text IS NULL OR $5 = ANY(labels)) AND ($6::timestamptz IS NULL OR delivery <> 'done' OR done_at >= $6) AND (NOT $7 OR delivery <> 'done') ORDER BY CASE delivery WHEN 'todo' THEN 0 WHEN 'in_progress' THEN 1 WHEN 'in_review' THEN 2 WHEN 'testing' THEN 3 ELSE 4 END, rank, length(key), key"))
            .bind(f.epic.as_ref().map(|k| k.to_string())).bind(&f.owner).bind(f.delivery.map(delivery_str)).bind(f.untriaged).bind(&f.label).bind(done_since).bind(f.unfinished).fetch_all(&mut **tx).await?;
        Ok(rows.iter().map(story_from).collect::<Result<Vec<_>, _>>()?)
    }
    async fn similar_stories(
        &self,
        text: &str,
        min: f32,
        done_since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Story>, RepoError> {
        let mut tx = self.pool.begin().await?;
        // set_config(.., true) is SET LOCAL with a bindable value: the `%` operator reads this threshold.
        sqlx::query("SELECT set_config('pg_trgm.similarity_threshold', $1, true)")
            .bind(min.to_string())
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query(&format!(
            "SELECT {STORY_COLS} FROM stories WHERE title % $1 AND (delivery <> 'done' OR done_at >= $2) \
             ORDER BY similarity(title, $1) DESC, created_at DESC, length(key) DESC, key DESC LIMIT $3"
        ))
        .bind(text)
        .bind(done_since)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.rollback().await?;
        Ok(rows.iter().map(story_from).collect::<Result<Vec<_>, _>>()?)
    }
    async fn update_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Story,
        rank: Option<i32>,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        // rank is written only when the caller supplied one: a content edit that read the story before a rerank must not
        // restore the stale rank (re-ranks change neighbours' ranks without bumping their version).
        let res = sqlx::query("UPDATE stories SET title=$2, kind=$3, epic_key=$4, description=$5, criteria=$6, owner=$7, labels=$8, rank=COALESCE($9, rank), version=version+1, updated_at=$10, external_ref=$12 WHERE key=$1 AND version=$11")
            .bind(s.key().to_string()).bind(s.title()).bind(kind_str(s.kind())).bind(s.epic().map(|k| k.to_string())).bind(s.description()).bind(criteria_json(s.criteria())).bind(s.owner()).bind(s.labels().to_vec()).bind(rank).bind(now).bind(expected).bind(s.external_ref()).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT version FROM stories WHERE key = $1")
            .bind(s.key().to_string())
            .fetch_optional(&mut **tx)
            .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn set_delivery(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Story,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let k = story.key();
        let res = sqlx::query("UPDATE stories SET delivery=$2, resolution=$3, owner=COALESCE(owner, $4), version=version+1, updated_at=$5, done_at = CASE WHEN $2 <> 'done' THEN NULL WHEN delivery = 'done' THEN done_at ELSE $5 END, started_at = CASE WHEN $2 IN ('in_progress', 'in_review', 'testing') AND delivery IN ('todo', 'done') THEN $5 ELSE started_at END WHERE key=$1 AND version=$6")
            .bind(k.to_string()).bind(delivery_str(story.delivery())).bind(story.resolution().map(resolution_str)).bind(story.owner()).bind(now).bind(expected).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT version FROM stories WHERE key = $1")
            .bind(k.to_string())
            .fetch_optional(&mut **tx)
            .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn todo_column_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Key>, RepoError> {
        // FOR NO KEY UPDATE, not FOR UPDATE: a rank update touches no key column, and FOR UPDATE would conflict with the
        // FOR KEY SHARE locks add_block's story_blocks foreign-key checks take on both stories, which can deadlock (40P01).
        let ks: Vec<String> = sqlx::query_scalar("SELECT key FROM (SELECT key, rank FROM stories WHERE delivery = 'todo' ORDER BY key FOR NO KEY UPDATE) t ORDER BY rank, length(key), key")
            .fetch_all(&mut **tx)
            .await?;
        Ok(ks.iter().map(|k| key(k)).collect())
    }
    async fn set_ranks(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ranks: &[(Key, i32)],
    ) -> Result<(), RepoError> {
        let keys: Vec<String> = ranks.iter().map(|(k, _)| k.to_string()).collect();
        let values: Vec<i32> = ranks.iter().map(|(_, r)| *r).collect();
        sqlx::query("UPDATE stories s SET rank = v.rank FROM (SELECT * FROM unnest($1::text[], $2::int[]) AS u(key, rank)) v WHERE s.key = v.key AND s.rank <> v.rank")
            .bind(keys)
            .bind(values)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn touch_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query(
            "UPDATE stories SET version=version+1, updated_at=$2 WHERE key=$1 AND version=$3",
        )
        .bind(k.to_string())
        .bind(now)
        .bind(expected)
        .execute(&mut **tx)
        .await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT version FROM stories WHERE key = $1")
            .bind(k.to_string())
            .fetch_optional(&mut **tx)
            .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn add_block(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("INSERT INTO story_blocks (story_key, blocker_key) VALUES ($1,$2) ON CONFLICT DO NOTHING").bind(story.to_string()).bind(blocker.to_string()).execute(&mut **tx).await?;
        Ok(r.rows_affected() == 1)
    }
    async fn remove_block(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("DELETE FROM story_blocks WHERE story_key = $1 AND blocker_key = $2")
            .bind(story.to_string())
            .bind(blocker.to_string())
            .execute(&mut **tx)
            .await?;
        Ok(r.rows_affected() == 1)
    }
}
