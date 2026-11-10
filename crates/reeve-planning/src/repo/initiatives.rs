use super::*;

#[async_trait]
impl InitiativeRepo for PgPlanningRepo {
    async fn insert_initiative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        i: &Initiative,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("INSERT INTO initiatives (key, title, brief, horizon, target_month, owner, external_ref, discovery_at, discovery_by, version, idempotency_key, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$9,$10,$11,1,$7,$8,$8) ON CONFLICT (idempotency_key) DO NOTHING")
            .bind(i.key().to_string()).bind(i.title()).bind(i.brief()).bind(i.horizon()).bind(i.target_month()).bind(i.owner()).bind(idem).bind(now).bind(i.external_ref()).bind(i.discovery().map(|d| d.at)).bind(i.discovery().map(|d| d.by.as_str())).execute(&mut **tx).await?;
        Ok(r.rows_affected() == 1)
    }
    async fn get_initiative(&self, k: &Key) -> Result<Initiative, RepoError> {
        get_initiative_on(&self.pool, k).await
    }
    async fn get_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
    ) -> Result<Initiative, RepoError> {
        get_initiative_on(&mut **tx, k).await
    }
    async fn list_initiatives_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Initiative>, RepoError> {
        Ok(sqlx::query(&format!(
            "SELECT {INITIATIVE_COLS} FROM initiatives ORDER BY created_at, {BY_KEY_NUMBER}"
        ))
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(initiative_from)
        .collect())
    }
    async fn update_initiative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        i: &Initiative,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE initiatives SET title=$2, brief=$3, horizon=$4, target_month=$5, owner=$6, discovery_at=$7, discovery_by=$8, version=version+1, updated_at=$9, external_ref=$11 WHERE key=$1 AND version=$10")
            .bind(i.key().to_string()).bind(i.title()).bind(i.brief()).bind(i.horizon()).bind(i.target_month()).bind(i.owner()).bind(i.discovery().map(|d| d.at)).bind(i.discovery().map(|d| d.by.as_str())).bind(now).bind(expected).bind(i.external_ref()).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> =
            sqlx::query_scalar("SELECT version FROM initiatives WHERE key = $1")
                .bind(i.key().to_string())
                .fetch_optional(&mut **tx)
                .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
}
