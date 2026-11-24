//! The SQL behind `BriefTemplateRepo`. Only `brief_templates` is named here, and this is the only file that names
//! it (the isolation lint): no item table, no join, and nothing reads the clock.
use super::*;
use crate::domain::brief_template::BriefTemplate;
use reeve_protocol::BriefTemplateLevel;

type Row = (i64, String, String, String, bool, i64);

fn stored(
    (id, level, name, body, is_default, version): Row,
) -> Result<StoredBriefTemplate, sqlx::Error> {
    let level =
        BriefTemplateLevel::from_wire(&level).ok_or_else(|| bad("brief template level", &level))?;
    Ok(StoredBriefTemplate {
        id,
        template: BriefTemplate::from_persisted(level, name, body),
        is_default,
        version,
    })
}

#[async_trait]
impl BriefTemplateRepo for PgPlanningRepo {
    async fn lock_brief_templates(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<(), RepoError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_brief_templates'))")
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn insert_brief_template(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        t: &BriefTemplate,
    ) -> Result<i64, RepoError> {
        Ok(sqlx::query_scalar(
            "INSERT INTO brief_templates (level, name, name_key, body) VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(t.level().wire())
        .bind(t.name())
        .bind(t.name_key())
        .bind(t.body())
        .fetch_one(&mut **tx)
        .await?)
    }
    async fn save_brief_template(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        t: &BriefTemplate,
    ) -> Result<(), RepoError> {
        let done = sqlx::query(
            "UPDATE brief_templates SET name = $2, name_key = $3, body = $4, version = version + 1 WHERE id = $1",
        )
        .bind(id)
        .bind(t.name())
        .bind(t.name_key())
        .bind(t.body())
        .execute(&mut **tx)
        .await?;
        if done.rows_affected() == 0 {
            return Err(RepoError::NotFound);
        }
        Ok(())
    }
    async fn delete_brief_template(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<(), RepoError> {
        let done = sqlx::query("DELETE FROM brief_templates WHERE id = $1")
            .bind(id)
            .execute(&mut **tx)
            .await?;
        if done.rows_affected() == 0 {
            return Err(RepoError::NotFound);
        }
        Ok(())
    }
    async fn set_default_brief_template(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        level: BriefTemplateLevel,
        id: Option<i64>,
    ) -> Result<(), RepoError> {
        // Two statements, clear then set: one UPDATE that did both could set the new mark before it cleared the
        // old one, and the partial unique index is checked row by row.
        sqlx::query(
            "UPDATE brief_templates SET is_default = false WHERE level = $1 AND is_default",
        )
        .bind(level.wire())
        .execute(&mut **tx)
        .await?;
        if let Some(id) = id {
            let done = sqlx::query(
                "UPDATE brief_templates SET is_default = true WHERE id = $1 AND level = $2",
            )
            .bind(id)
            .bind(level.wire())
            .execute(&mut **tx)
            .await?;
            if done.rows_affected() == 0 {
                return Err(RepoError::NotFound);
            }
        }
        Ok(())
    }
    async fn brief_templates_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        level: Option<BriefTemplateLevel>,
    ) -> Result<Vec<StoredBriefTemplate>, RepoError> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, level, name, body, is_default, version FROM brief_templates \
             WHERE ($1::text IS NULL OR level = $1) ORDER BY id",
        )
        .bind(level.map(BriefTemplateLevel::wire))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .map(stored)
            .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }
}
