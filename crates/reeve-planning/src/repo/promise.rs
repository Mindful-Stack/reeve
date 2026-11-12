//! The SQL behind the promise methods (kb/ADR-0025). Free functions the `PromiseRepo` impl delegates to. No statement
//! names `version` or `updated_at`, and nothing here reads the clock.
use super::*;
use crate::domain::promise::{Promisable, Promise, PromiseItem};

#[async_trait]
impl PromiseRepo for PgPlanningRepo {
    async fn lock_for_promise(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: PromiseItem,
        key: &Key,
    ) -> Result<Option<PromiseTarget>, RepoError> {
        lock_for_promise(tx, item, key).await
    }
    async fn set_promise(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: PromiseItem,
        key: &Key,
        p: &Promise,
    ) -> Result<bool, RepoError> {
        set_promise(tx, item, key, p).await
    }
    async fn clear_promise(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: PromiseItem,
        key: &Key,
    ) -> Result<bool, RepoError> {
        clear_promise(tx, item, key).await
    }
}

fn table(item: PromiseItem) -> &'static str {
    match item {
        PromiseItem::Epic => "epics",
        PromiseItem::Story => "stories",
    }
}

async fn lock_for_promise(
    tx: Tx<'_, '_>,
    item: PromiseItem,
    k: &Key,
) -> Result<Option<PromiseTarget>, RepoError> {
    let delivery = match item {
        PromiseItem::Story => "delivery",
        PromiseItem::Epic => "NULL::text AS delivery",
    };
    let row = sqlx::query(&format!(
        "SELECT {delivery}, promised_on, promised_by, promised_at, promised_to FROM {} WHERE key = $1 FOR NO KEY UPDATE",
        table(item)
    ))
    .bind(k.to_string())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let item = match item {
        PromiseItem::Story => Promisable::Story(delivery_of(r.try_get::<&str, _>("delivery")?)?),
        PromiseItem::Epic => Promisable::Epic,
    };
    let promise = promise_cols(&r, "promised_")?;
    Ok(Some(PromiseTarget { item, promise }))
}

async fn set_promise(
    tx: Tx<'_, '_>,
    item: PromiseItem,
    k: &Key,
    p: &Promise,
) -> Result<bool, RepoError> {
    let r = sqlx::query(&format!(
        "UPDATE {} SET promised_on = $2, promised_by = $3, promised_at = $4, promised_to = $5 WHERE key = $1",
        table(item)
    ))
    .bind(k.to_string())
    .bind(p.on())
    .bind(p.by())
    .bind(p.at())
    .bind(p.to())
    .execute(&mut **tx)
    .await?;
    Ok(r.rows_affected() == 1)
}

async fn clear_promise(tx: Tx<'_, '_>, item: PromiseItem, k: &Key) -> Result<bool, RepoError> {
    let r = sqlx::query(&format!(
        "UPDATE {} SET promised_on = NULL, promised_by = NULL, promised_at = NULL, promised_to = NULL WHERE key = $1 AND promised_on IS NOT NULL",
        table(item)
    ))
    .bind(k.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(r.rows_affected() == 1)
}
