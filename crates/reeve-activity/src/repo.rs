use crate::domain::*;
use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ActivityError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Writers hand in their own transaction so the entry commits with the change it describes.
/// Named `append`, not `record`, so it never collides with the consumer ports' `record` methods on the same object.
#[async_trait]
pub trait ActivityWriter: Send + Sync {
    async fn append(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        entry: ActivityEntry,
    ) -> Result<i64, ActivityError>;
}

#[async_trait]
pub trait ActivityReader: Send + Sync {
    async fn list(&self, filter: ActivityFilter) -> Result<Vec<ActivityEntry>, ActivityError>;
    /// The newest committed entry id, or 0 when the log is empty.
    async fn max_id(&self) -> Result<i64, ActivityError>;
    /// One page of the whole log on the caller's transaction (the export's snapshot): at most `limit` entries whose
    /// id is above `after_id`, oldest first. A page shorter than `limit` is the last.
    async fn list_all_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<ActivityEntry>, ActivityError>;
}

/// One struct, several trait views: its own writer and reader plus the consumer-defined ports of identity and planning.
pub trait ActivityStore:
    ActivityWriter + ActivityReader + reeve_identity::IdentityActivity + reeve_planning::ActivitySink
{
}

pub(crate) struct PgActivityStore {
    pool: PgPool,
}
impl PgActivityStore {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
impl ActivityStore for PgActivityStore {}

/// The only place an activity row is written. The transaction-level advisory lock is taken immediately before the
/// INSERT and held until the caller's transaction ends, so concurrent writers commit in id order: a `BIGSERIAL` id is
/// allocated at insert, not at commit, and without the lock a later id could become visible before an earlier one,
/// which a stream resuming `after_id` would skip forever (PostgreSQL docs, "Advisory Locks").
async fn insert_entry(
    tx: &mut Transaction<'_, Postgres>,
    e: &ActivityEntry,
) -> Result<i64, ActivityError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_activity'))")
        .execute(&mut **tx)
        .await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO activity (at, actor, via, entity_type, entity_key, action, payload) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
    )
    .bind(e.at)
    .bind(&e.actor)
    .bind(e.via.as_str())
    .bind(&e.entity_type)
    .bind(&e.entity_key)
    .bind(&e.action)
    .bind(&e.payload)
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

#[async_trait]
impl ActivityWriter for PgActivityStore {
    async fn append(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: ActivityEntry,
    ) -> Result<i64, ActivityError> {
        insert_entry(tx, &e).await
    }
}

#[async_trait]
impl reeve_identity::IdentityActivity for PgActivityStore {
    async fn record(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: reeve_identity::IdentityEvent,
    ) -> Result<(), sqlx::Error> {
        let entry = ActivityEntry {
            id: None,
            at: e.at,
            actor: e.actor,
            via: e.via,
            entity_type: "user".into(),
            entity_key: e.subject,
            action: e.action.into(),
            payload: e.payload,
        };
        self.append(tx, entry)
            .await
            .map(|_| ())
            .map_err(|ActivityError::Db(e)| e)
    }
}

#[async_trait]
impl reeve_planning::ActivitySink for PgActivityStore {
    async fn record(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        r: reeve_planning::ActivityRecord,
    ) -> Result<(), reeve_planning::RepoError> {
        let entry = ActivityEntry {
            id: None,
            at: r.at,
            actor: r.actor,
            via: r.via,
            entity_type: r.entity_type.into(),
            entity_key: r.entity_key,
            action: r.action.into(),
            payload: r.payload,
        };
        self.append(tx, entry)
            .await
            .map(|_| ())
            .map_err(|ActivityError::Db(e)| reeve_planning::RepoError::Db(e))
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    at: chrono::DateTime<chrono::Utc>,
    actor: String,
    via: String,
    entity_type: String,
    entity_key: String,
    action: String,
    payload: serde_json::Value,
}

fn entries(rows: Vec<Row>) -> Result<Vec<ActivityEntry>, ActivityError> {
    let entries = rows
        .into_iter()
        .map(|r| {
            let via = Via::parse(&r.via)
                .ok_or_else(|| sqlx::Error::Decode(format!("unknown via {:?}", r.via).into()))?;
            Ok(ActivityEntry {
                id: Some(r.id),
                at: r.at,
                actor: r.actor,
                via,
                entity_type: r.entity_type,
                entity_key: r.entity_key,
                action: r.action,
                payload: r.payload,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(entries)
}

#[async_trait]
impl ActivityReader for PgActivityStore {
    async fn list(&self, f: ActivityFilter) -> Result<Vec<ActivityEntry>, ActivityError> {
        // A resume replays oldest first; a plain listing shows the newest first so a capped feed keeps the latest.
        let order = if f.after_id.is_some() { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT id, at, actor, via, entity_type, entity_key, action, payload FROM activity
             WHERE ($1::text IS NULL OR entity_type = $1) AND ($2::text IS NULL OR entity_key = $2)
               AND ($3::text IS NULL OR action = $3) AND ($4::timestamptz IS NULL OR at >= $4)
               AND ($6::bigint IS NULL OR id > $6)
             ORDER BY id {order} LIMIT $5"
        );
        let rows: Vec<Row> = sqlx::query_as(&sql)
            .bind(f.entity_type)
            .bind(f.entity_key)
            .bind(f.action)
            .bind(f.since)
            .bind(f.limit.unwrap_or(500))
            .bind(f.after_id)
            .fetch_all(&self.pool)
            .await?;
        entries(rows)
    }

    async fn list_all_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<ActivityEntry>, ActivityError> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, at, actor, via, entity_type, entity_key, action, payload FROM activity
             WHERE id > $1 ORDER BY id ASC LIMIT $2",
        )
        .bind(after_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        entries(rows)
    }

    async fn max_id(&self) -> Result<i64, ActivityError> {
        let id: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM activity")
            .fetch_one(&self.pool)
            .await?;
        Ok(id)
    }
}

pub fn make_activity_store(pool: PgPool) -> Arc<dyn ActivityStore> {
    Arc::new(PgActivityStore::new(pool))
}
