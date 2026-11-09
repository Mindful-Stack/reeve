//! The access tables behind a port (kb/ADR-0010 R2): a private Postgres adapter reached through `make_access_store`.
//! Writes take the caller's transaction so a change and its activity entry commit together (kb/ADR-0010 R5).
use crate::access::{AccessGrant, AccessKind, AccessPolicy};
use crate::domain::IdentityConfig;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;

/// An access write is `begin` -> `lock` -> `load_in` -> writes -> activity -> commit.
#[async_trait]
pub trait AccessStore: Send + Sync {
    /// The current policy, read on a connection of its own (no cache, kb/ADR-0015 R5).
    async fn load(&self, floor: &IdentityConfig) -> Result<AccessPolicy, sqlx::Error>;
    /// The policy as the caller's transaction sees it (an access write after `lock`; the export's snapshot).
    async fn load_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        floor: &IdentityConfig,
    ) -> Result<AccessPolicy, sqlx::Error>;
    /// Serialises access writes until the transaction ends (kb/ADR-0015 R3). It MUST be the first statement of a READ
    /// COMMITTED transaction (`pool.begin()`'s default), so the `load_in` after it sees every write committed before the
    /// lock was granted. Under REPEATABLE READ the snapshot would be taken before the wait and be stale.
    async fn lock(&self, tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error>;
    async fn insert_entry(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        kind: AccessKind,
        value: &str,
        added_by: &str,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;
    /// Whether a row was deleted.
    async fn delete_entry(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        kind: AccessKind,
        value: &str,
    ) -> Result<bool, sqlx::Error>;
    async fn insert_admin(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
        added_by: &str,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;
    /// Whether a row was deleted.
    async fn delete_admin(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
    ) -> Result<bool, sqlx::Error>;
}

/// One statement, so the three lists come from one snapshot.
const LOAD: &str = "SELECT kind, value, added_by, added_at FROM allowed_entries \
                    UNION ALL SELECT 'admin', email, added_by, added_at FROM admin_grants";

type Row = (String, String, String, DateTime<Utc>);

fn policy(floor: &IdentityConfig, rows: Vec<Row>) -> Result<AccessPolicy, sqlx::Error> {
    let (mut domains, mut emails, mut admins) = (vec![], vec![], vec![]);
    for (kind, value, added_by, added_at) in rows {
        let g = AccessGrant {
            value,
            added_by,
            added_at,
        };
        if kind == "admin" {
            admins.push(g);
        } else {
            match AccessKind::parse(&kind) {
                Some(AccessKind::Domain) => domains.push(g),
                Some(AccessKind::Email) => emails.push(g),
                // Unreachable while the kind CHECK holds; fails closed on purpose rather than guessing a list.
                None => {
                    return Err(sqlx::Error::Decode(
                        format!("unknown access kind {kind:?}").into(),
                    ));
                }
            }
        }
    }
    Ok(AccessPolicy::new(floor, domains, emails, admins))
}

pub(crate) struct PgAccessStore {
    pool: PgPool,
}

#[async_trait]
impl AccessStore for PgAccessStore {
    async fn load(&self, floor: &IdentityConfig) -> Result<AccessPolicy, sqlx::Error> {
        policy(floor, sqlx::query_as(LOAD).fetch_all(&self.pool).await?)
    }
    async fn load_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        floor: &IdentityConfig,
    ) -> Result<AccessPolicy, sqlx::Error> {
        policy(floor, sqlx::query_as(LOAD).fetch_all(&mut **tx).await?)
    }
    async fn lock(&self, tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
        // Same idiom as the activity writer's `hashtext('reeve_activity')`; a different key, taken first in an access write.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_access'))")
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn insert_entry(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        kind: AccessKind,
        value: &str,
        added_by: &str,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(kind.as_str())
        .bind(value)
        .bind(added_by)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
    async fn delete_entry(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        kind: AccessKind,
        value: &str,
    ) -> Result<bool, sqlx::Error> {
        Ok(
            sqlx::query("DELETE FROM allowed_entries WHERE kind = $1 AND value = $2")
                .bind(kind.as_str())
                .bind(value)
                .execute(&mut **tx)
                .await?
                .rows_affected()
                == 1,
        )
    }
    async fn insert_admin(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
        added_by: &str,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO admin_grants (email, added_by, added_at) VALUES ($1, $2, $3)")
            .bind(email)
            .bind(added_by)
            .bind(now)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn delete_admin(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
    ) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM admin_grants WHERE email = $1")
            .bind(email)
            .execute(&mut **tx)
            .await?
            .rows_affected()
            == 1)
    }
}

pub fn make_access_store(pool: PgPool) -> Arc<dyn AccessStore> {
    Arc::new(PgAccessStore { pool })
}
