//! The stored registrations behind their port (kb/ADR-0010 R2): a private Postgres adapter reached through
//! `make_registered_clients`.
use crate::registrations::{RegisteredClient, RegisteredClients, retention};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;

struct PgRegisteredClients {
    pool: PgPool,
}

#[async_trait]
impl RegisteredClients for PgRegisteredClients {
    async fn find(&self, client_id: &str) -> Result<Option<RegisteredClient>, sqlx::Error> {
        sqlx::query_as(
            "SELECT client_id, client_name, redirect_uris, created_at, last_used_at \
             FROM registered_clients WHERE client_id = $1",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await
    }

    async fn insert_capped(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        client: &RegisteredClient,
        cap: i64,
    ) -> Result<bool, sqlx::Error> {
        // One registration at a time: the count below and the insert are then one step, so the cap holds. The
        // first statement that touches the table, in a READ COMMITTED transaction, as the access store's lock is.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_registered_clients'))")
            .execute(&mut **tx)
            .await?;
        // Expiry (R3). Both conditions are on the row's own columns, so a row approved while this waits for it
        // is judged again as it then is, and kept.
        let (unused_before, used_before) = retention(client.created_at);
        sqlx::query(
            "DELETE FROM registered_clients \
             WHERE (last_used_at IS NULL AND created_at <= $1) OR last_used_at <= $2",
        )
        .bind(unused_before)
        .bind(used_before)
        .execute(&mut **tx)
        .await?;
        let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM registered_clients")
            .fetch_one(&mut **tx)
            .await?;
        if stored >= cap {
            // The cap (R3): the oldest row nobody has approved a connection for. `last_used_at IS NULL` is
            // repeated on the row deleted, so a row approved since the subquery chose it is not deleted.
            let dropped = sqlx::query(
                "DELETE FROM registered_clients WHERE last_used_at IS NULL AND client_id = \
                 (SELECT client_id FROM registered_clients WHERE last_used_at IS NULL \
                  ORDER BY created_at, client_id LIMIT 1)",
            )
            .execute(&mut **tx)
            .await?
            .rows_affected();
            if dropped == 0 {
                return Ok(false);
            }
        }
        sqlx::query(
            "INSERT INTO registered_clients (client_id, client_name, redirect_uris, created_at, last_used_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&client.client_id)
        .bind(&client.client_name)
        .bind(&client.redirect_uris)
        .bind(client.created_at)
        .bind(client.last_used_at)
        .execute(&mut **tx)
        .await?;
        Ok(true)
    }

    async fn mark_used(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        // GREATEST skips a NULL, and keeps the newer instant when two approvals commit out of the order their
        // instants were read.
        let marked = sqlx::query(
            "UPDATE registered_clients SET last_used_at = GREATEST(last_used_at, $2) WHERE client_id = $1",
        )
        .bind(client_id)
        .bind(now)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        Ok(marked == 1)
    }
}

pub fn make_registered_clients(pool: PgPool) -> Arc<dyn RegisteredClients> {
    Arc::new(PgRegisteredClients { pool })
}
