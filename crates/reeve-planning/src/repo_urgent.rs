//! The SQL behind the urgent-mark methods of `PlanningRepo` (kb/ADR-0017). Nothing here reads the clock.
use crate::{
    domain::urgency::{Lineage, MarkFact, UrgentMark},
    ports::RepoError,
    repo::{bad, key},
};
use chrono::{DateTime, Utc};
use reeve_protocol::{ItemType, Key};
use sqlx::{Postgres, Row, Transaction};

type Tx<'a, 'b> = &'a mut Transaction<'b, Postgres>;

fn table(item: ItemType) -> &'static str {
    match item {
        ItemType::Initiative => "initiatives",
        ItemType::Epic => "epics",
        ItemType::Story => "stories",
    }
}

pub(crate) async fn urgent_marks_in(tx: Tx<'_, '_>) -> Result<Vec<MarkFact>, RepoError> {
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

pub(crate) async fn urgency_lineage_in(
    tx: Tx<'_, '_>,
    epics: &[Key],
    initiatives: &[Key],
) -> Result<Lineage, RepoError> {
    if epics.is_empty() && initiatives.is_empty() {
        return Ok(Lineage::default());
    }
    let e: Vec<String> = epics.iter().map(Key::to_string).collect();
    let i: Vec<String> = initiatives.iter().map(Key::to_string).collect();
    let epic_rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT key, initiative_key FROM epics WHERE key = ANY($1) OR initiative_key = ANY($2)",
    )
    .bind(&e)
    .bind(&i)
    .fetch_all(&mut **tx)
    .await?;
    let story_rows: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT s.key, s.epic_key, s.delivery = 'done' FROM stories s
          WHERE s.epic_key = ANY($1)
             OR s.epic_key IN (SELECT key FROM epics WHERE initiative_key = ANY($2))",
    )
    .bind(&e)
    .bind(&i)
    .fetch_all(&mut **tx)
    .await?;
    Ok(Lineage {
        epics: epic_rows
            .into_iter()
            .map(|(k, i)| (key(&k), i.map(|i| key(&i))))
            .collect(),
        stories: story_rows
            .into_iter()
            .map(|(s, e, d)| (key(&s), key(&e), d))
            .collect(),
    })
}

pub(crate) async fn set_urgent_mark(
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

pub(crate) async fn renew_urgent_mark(
    tx: Tx<'_, '_>,
    item: ItemType,
    k: &Key,
    m: &UrgentMark,
) -> Result<bool, RepoError> {
    let r = sqlx::query(&format!(
        "UPDATE {} SET urgent_at = $2, urgent_by = $3, urgent_reason = $4 WHERE key = $1 AND urgent_at IS NOT NULL",
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

pub(crate) async fn clear_urgent_mark(
    tx: Tx<'_, '_>,
    item: ItemType,
    k: &Key,
) -> Result<bool, RepoError> {
    let r = sqlx::query(&format!(
        "UPDATE {} SET urgent_at = NULL, urgent_by = NULL, urgent_reason = NULL WHERE key = $1 AND urgent_at IS NOT NULL",
        table(item)
    ))
    .bind(k.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(r.rows_affected() == 1)
}
