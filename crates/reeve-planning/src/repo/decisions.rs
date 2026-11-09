use super::*;

#[async_trait]
#[allow(clippy::too_many_arguments)]
impl DecisionRepo for PgPlanningRepo {
    async fn insert_decision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        on: ItemType,
        on_key: &Key,
        title: &str,
        decider: &str,
        due: Option<NaiveDate>,
        reversibility: Option<Reversibility>,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<(RequiredDecision, bool), RepoError> {
        let inserted = sqlx::query(&format!("INSERT INTO required_decisions (on_type, on_key, title, decider, due, reversibility, idempotency_key, created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (idempotency_key) DO NOTHING RETURNING {DECISION_COLS}"))
            .bind(item_type_str(on)).bind(on_key.to_string()).bind(title).bind(decider).bind(due).bind(reversibility.map(reversibility_str)).bind(idem).bind(now).fetch_optional(&mut **tx).await?;
        if let Some(r) = inserted {
            return Ok((decision_from(&r)?, true));
        }
        let r = sqlx::query(&format!(
            "SELECT {DECISION_COLS} FROM required_decisions WHERE idempotency_key = $1"
        ))
        .bind(idem)
        .fetch_one(&mut **tx)
        .await?;
        Ok((decision_from(&r)?, false))
    }
    async fn get_decision(&self, id: i64) -> Result<RequiredDecision, RepoError> {
        let r = sqlx::query(&format!(
            "SELECT {DECISION_COLS} FROM required_decisions WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(RepoError::NotFound)?;
        Ok(decision_from(&r)?)
    }
    async fn get_decision_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<RequiredDecision, RepoError> {
        let r = sqlx::query(&format!(
            "SELECT {DECISION_COLS} FROM required_decisions WHERE id = $1 FOR UPDATE"
        ))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RepoError::NotFound)?;
        Ok(decision_from(&r)?)
    }
    async fn list_decisions_on(&self, on_key: &Key) -> Result<Vec<RequiredDecision>, RepoError> {
        decisions_on(&self.pool, on_key).await
    }
    async fn list_decisions_on_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        on_key: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        decisions_on(&mut **tx, on_key).await
    }
    async fn list_all_decisions(
        &self,
        include_closed: bool,
        with_record_only: bool,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        all_decisions(&self.pool, include_closed, with_record_only).await
    }
    async fn list_all_decisions_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        include_closed: bool,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        all_decisions(&mut **tx, include_closed, false).await
    }
    async fn save_decision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        d: &RequiredDecision,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE required_decisions SET title=$2, decider=$3, due=$4, record_repo=$5, record_path=$6, record_id=$7, pr_url=$8, check_result=$9, checked_at=$10, closed_at=$11, closed_by=$12, closed_reason=$13, reversibility=$15, closed_kind=$16, closed_via=$17, version=version+1 WHERE id=$1 AND version=$14")
            .bind(d.id()).bind(d.title()).bind(d.decider()).bind(d.due()).bind(d.record().map(|r| r.repo())).bind(d.record().map(|r| r.path())).bind(d.record().map(|r| r.record_id())).bind(d.pr_url())
            .bind(serde_json::to_value(d.check()).unwrap()).bind(d.checked_at()).bind(d.closed().map(|c| c.at)).bind(d.closed().map(|c| &c.by)).bind(d.closed().map(|c| &c.reason)).bind(d.version()).bind(d.reversibility().map(reversibility_str)).bind(d.closed().and_then(|c| c.kind).map(closure_kind_str)).bind(d.closed().and_then(|c| c.via).map(token_kind_str)).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> =
            sqlx::query_scalar("SELECT version FROM required_decisions WHERE id = $1")
                .bind(d.id())
                .fetch_optional(&mut **tx)
                .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn touch_decision_checked(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        at: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE required_decisions SET checked_at = $2 WHERE id = $1")
            .bind(id)
            .bind(at)
            .execute(&mut **tx)
            .await?;
        if res.rows_affected() == 1 {
            Ok(())
        } else {
            Err(RepoError::NotFound)
        }
    }
}
