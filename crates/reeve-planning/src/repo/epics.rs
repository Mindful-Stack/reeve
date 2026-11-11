use super::*;

#[async_trait]
impl EpicRepo for PgPlanningRepo {
    async fn insert_epic(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: &Epic,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("INSERT INTO epics (key, title, initiative_key, brief, external_ref, discovery_at, discovery_by, version, idempotency_key, created_at, updated_at) VALUES ($1,$2,$3,$4,$7,$8,$9,1,$5,$6,$6) ON CONFLICT (idempotency_key) DO NOTHING")
            .bind(e.key().to_string()).bind(e.title()).bind(e.initiative().map(|k| k.to_string())).bind(e.brief()).bind(idem).bind(now).bind(e.external_ref()).bind(e.discovery().map(|d| d.at)).bind(e.discovery().map(|d| d.by.as_str())).execute(&mut **tx).await?;
        Ok(r.rows_affected() == 1)
    }
    async fn get_epic(&self, k: &Key) -> Result<Epic, RepoError> {
        get_epic_on(&self.pool, k).await
    }
    async fn get_epic_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
    ) -> Result<Epic, RepoError> {
        get_epic_on(&mut **tx, k).await
    }
    async fn list_epics_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        initiative: Option<&Key>,
    ) -> Result<Vec<Epic>, RepoError> {
        Ok(sqlx::query(&format!("SELECT {EPIC_COLS} FROM epics WHERE ($1::text IS NULL OR initiative_key = $1) ORDER BY created_at, {BY_KEY_NUMBER}")).bind(initiative.map(|k| k.to_string())).fetch_all(&mut **tx).await?.iter().map(epic_from).collect())
    }
    async fn epic_keys_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        initiative: &Key,
    ) -> Result<Vec<Key>, RepoError> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT key FROM epics WHERE initiative_key = $1")
                .bind(initiative.to_string())
                .fetch_all(&mut **tx)
                .await?;
        Ok(rows.iter().map(|k| key(k)).collect())
    }
    async fn update_epic(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: &Epic,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE epics SET title=$2, initiative_key=$3, brief=$4, discovery_at=$5, discovery_by=$6, version=version+1, updated_at=$7, external_ref=$9 WHERE key=$1 AND version=$8")
            .bind(e.key().to_string()).bind(e.title()).bind(e.initiative().map(|k| k.to_string())).bind(e.brief()).bind(e.discovery().map(|d| d.at)).bind(e.discovery().map(|d| d.by.as_str())).bind(now).bind(expected).bind(e.external_ref()).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT version FROM epics WHERE key = $1")
            .bind(e.key().to_string())
            .fetch_optional(&mut **tx)
            .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
}
