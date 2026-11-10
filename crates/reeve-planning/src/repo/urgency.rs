//! The SQL behind the urgent-mark methods (kb/ADR-0017). Free functions the `UrgencyRepo` impl delegates to.
//! Nothing here reads the clock.
use super::*;

#[async_trait]
impl UrgencyRepo for PgPlanningRepo {
    async fn urgent_marks_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<MarkFact>, RepoError> {
        urgent_marks_in(tx).await
    }
    async fn set_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
        mark: &UrgentMark,
    ) -> Result<bool, RepoError> {
        set_urgent_mark(tx, item, key, mark).await
    }
    async fn renew_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
        at: DateTime<Utc>,
        by: &str,
    ) -> Result<Option<Option<String>>, RepoError> {
        renew_urgent_mark(tx, item, key, at, by).await
    }
    async fn clear_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
    ) -> Result<bool, RepoError> {
        clear_urgent_mark(tx, item, key).await
    }
}

fn table(item: ItemType) -> &'static str {
    match item {
        ItemType::Initiative => "initiatives",
        ItemType::Epic => "epics",
        ItemType::Story => "stories",
    }
}

async fn urgent_marks_in(tx: Tx<'_, '_>) -> Result<Vec<MarkFact>, RepoError> {
    let rows = sqlx::query(
        "SELECT 'initiative' AS item, key, urgent_at, urgent_by, urgent_reason, NULL::text AS parent, false AS done
           FROM initiatives WHERE urgent_at IS NOT NULL
         UNION ALL
         SELECT 'epic', key, urgent_at, urgent_by, urgent_reason, initiative_key, false
           FROM epics WHERE urgent_at IS NOT NULL
         UNION ALL
         SELECT 'story', key, urgent_at, urgent_by, urgent_reason, epic_key, delivery = 'done'
           FROM stories WHERE urgent_at IS NOT NULL",
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.iter()
        .map(|r| {
            let item = match r.get::<&str, _>("item") {
                "initiative" => ItemType::Initiative,
                "epic" => ItemType::Epic,
                "story" => ItemType::Story,
                other => return Err(bad("item type", other).into()),
            };
            Ok(MarkFact {
                item,
                key: key(r.get::<&str, _>("key")),
                mark: UrgentMark {
                    at: r.get::<DateTime<Utc>, _>("urgent_at"),
                    by: r.get("urgent_by"),
                    reason: r.get("urgent_reason"),
                },
                parent: r.get::<Option<String>, _>("parent").map(|p| key(&p)),
                done: r.get("done"),
            })
        })
        .collect()
}

async fn set_urgent_mark(
    tx: Tx<'_, '_>,
    item: ItemType,
    k: &Key,
    m: &UrgentMark,
) -> Result<bool, RepoError> {
    let r = sqlx::query(&format!(
        "UPDATE {} SET urgent_at = $2, urgent_by = $3, urgent_reason = $4 WHERE key = $1",
        table(item)
    ))
    .bind(k.to_string())
    .bind(m.at)
    .bind(&m.by)
    .bind(&m.reason)
    .execute(&mut **tx)
    .await?;
    Ok(r.rows_affected() == 1)
}

async fn renew_urgent_mark(
    tx: Tx<'_, '_>,
    item: ItemType,
    k: &Key,
    at: DateTime<Utc>,
    by: &str,
) -> Result<Option<Option<String>>, RepoError> {
    let r: Option<(Option<String>,)> = sqlx::query_as(&format!(
        "UPDATE {} SET urgent_at = $2, urgent_by = $3 WHERE key = $1 AND urgent_at IS NOT NULL RETURNING urgent_reason",
        table(item)
    ))
    .bind(k.to_string())
    .bind(at)
    .bind(by)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(r.map(|(reason,)| reason))
}

async fn clear_urgent_mark(tx: Tx<'_, '_>, item: ItemType, k: &Key) -> Result<bool, RepoError> {
    let r = sqlx::query(&format!(
        "UPDATE {} SET urgent_at = NULL, urgent_by = NULL, urgent_reason = NULL WHERE key = $1 AND urgent_at IS NOT NULL",
        table(item)
    ))
    .bind(k.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(r.rows_affected() == 1)
}
