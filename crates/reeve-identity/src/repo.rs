use crate::connections::Connection;
use crate::domain::*;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reeve_protocol::{RevokedReason, Scope};
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TokenRow {
    pub id: i64,
    pub owner: String,
    pub kind: String,
    /// The stored audience text (`api` or `mcp`); parsed by the authenticate functions.
    pub audience: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub owner_disabled: bool,
    /// The connection an MCP token belongs to, with what `authenticate_mcp` needs of it; all `None` for an API
    /// token.
    pub connection_id: Option<i64>,
    pub connection_client_id: Option<String>,
    pub connection_scope: Option<String>,
    pub connection_ends_at: Option<DateTime<Utc>>,
    pub connection_revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TokenSummary {
    pub id: i64,
    pub kind: TokenKind,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct UserRow {
    pub email: String,
    pub display_name: String,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
}

/// Mutations take the caller's transaction so the handler can commit them with their activity entry.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait TokenStore: Send + Sync {
    async fn ensure_user(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error>;
    /// An API token; an MCP token needs a connection and is written by `insert_mcp_token`.
    async fn insert_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        kind: TokenKind,
        hash: &str,
        label: &str,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error>;
    /// A new connection (kb/ADR-0029 R5); its id. The caller holds the owner's `users` row lock.
    async fn insert_connection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        client_id: &str,
        client_name: &str,
        scope: Scope,
        approved_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error>;
    /// An MCP token of `connection_id`: interactive, audience `mcp`, always with an expiry.
    async fn insert_mcp_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        connection_id: i64,
        hash: &str,
        label: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error>;
    /// Locks the user's row until the transaction ends and says whether the user is disabled; `None` when there is
    /// no such user. The first statement of every transaction that touches a person's connections, MCP tokens
    /// or refresh tokens: whoever holds it is alone with them, so issuing can never slip past a disable or a
    /// revocation.
    async fn lock_user(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
    ) -> Result<Option<bool>, sqlx::Error>;
    /// A new, unused refresh token of the connection, stored as its hash.
    async fn insert_refresh_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        connection_id: i64,
        hash: &str,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;
    /// Deletes the owner's connections that ended or were revoked before `before`, their tokens and refresh tokens
    /// with them (the foreign keys cascade). This is what bounds the rows of spent refresh tokens. The caller
    /// holds the owner's `users` row lock.
    async fn delete_ended_connections(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        before: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error>;
    /// The refresh token with this hash: its id, its connection and the connection's owner. A plain read: the
    /// three never change, and the owner must be known before the owner's row can be locked.
    async fn find_refresh_token(
        &self,
        hash: &str,
    ) -> Result<Option<(i64, i64, String)>, sqlx::Error>;
    /// Marks the refresh token used, if it is not yet. Returns whether this call marked it: `false` means it was
    /// used before.
    async fn use_refresh_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error>;
    /// One connection, read in the caller's transaction without a lock of its own: a caller that holds the
    /// owner's `users` row is alone with it; a caller that does not yet reads it to learn the owner.
    async fn connection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Option<Connection>, sqlx::Error>;
    /// Revokes the owner's live connections, or only the one named, and returns each one's id and client id, by
    /// id. A connection that is revoked already or has ended is left as it is. The caller holds the owner's
    /// `users` row lock.
    async fn revoke_connections(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        only: Option<i64>,
        reason: RevokedReason,
        now: DateTime<Utc>,
    ) -> Result<Vec<(i64, String)>, sqlx::Error>;
    /// Deletes the connection's MCP token rows that have expired by `now`.
    async fn delete_expired_mcp_tokens(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        connection_id: i64,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;
    async fn find_by_hash(&self, hash: &str) -> Result<Option<TokenRow>, sqlx::Error>;
    async fn revoke_all_for(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error>;
    async fn set_disabled(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
        disabled: bool,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error>;
    async fn set_display_name(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
        name: &str,
    ) -> Result<(), sqlx::Error>;
    /// The owner's API tokens, newest first. MCP tokens are not listed here; connections get their own listing
    /// (kb/ADR-0029 R9, GRA-669).
    async fn list_for(&self, owner: &str) -> Result<Vec<TokenSummary>, sqlx::Error>;
    /// Revokes one API token if it belongs to `owner` and is not yet revoked. Returns whether a row changed. An MCP
    /// token is not found here (a connection is revoked as a whole); `revoke_all_for` covers every audience.
    async fn revoke_one(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error>;
    async fn list_users(&self) -> Result<Vec<UserRow>, sqlx::Error>;
    async fn find_user(&self, email: &str) -> Result<Option<UserRow>, sqlx::Error>;
}

pub(crate) struct PgTokenStore {
    pool: PgPool,
}
impl PgTokenStore {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TokenStore for PgTokenStore {
    async fn ensure_user(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO users (email, created_at) VALUES ($1, $2) ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email RETURNING disabled",
        )
        .bind(email)
        .bind(now)
        .fetch_one(&mut **tx)
        .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn insert_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        kind: TokenKind,
        hash: &str,
        label: &str,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let kind = match kind {
            TokenKind::Interactive => "interactive",
            TokenKind::Automation => "automation",
        };
        sqlx::query_scalar("INSERT INTO tokens (owner, kind, audience, hash, label, created_at, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id")
            .bind(owner).bind(kind).bind(Audience::Api.as_str()).bind(hash).bind(label).bind(now).bind(expires_at).fetch_one(&mut **tx).await
    }
    async fn find_by_hash(&self, hash: &str) -> Result<Option<TokenRow>, sqlx::Error> {
        // A plain read, no lock: every `/mcp` call runs it. The connection's columns are NULL for an API token.
        sqlx::query_as(
            "SELECT t.id, t.owner, t.kind, t.audience, t.expires_at, t.revoked_at, u.disabled AS owner_disabled, \
             t.connection_id, c.client_id AS connection_client_id, c.scope AS connection_scope, \
             c.ends_at AS connection_ends_at, c.revoked_at AS connection_revoked_at \
             FROM tokens t JOIN users u ON u.email = t.owner LEFT JOIN connections c ON c.id = t.connection_id \
             WHERE t.hash = $1",
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn insert_connection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        client_id: &str,
        client_name: &str,
        scope: Scope,
        approved_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO connections (owner, client_id, client_name, scope, approved_at, ends_at) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
        )
        .bind(owner).bind(client_id).bind(client_name).bind(scope_text(scope)).bind(approved_at).bind(ends_at)
        .fetch_one(&mut **tx)
        .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn insert_mcp_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        connection_id: i64,
        hash: &str,
        label: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO tokens (owner, kind, audience, hash, label, created_at, expires_at, connection_id) VALUES ($1,'interactive','mcp',$2,$3,$4,$5,$6) RETURNING id",
        )
        .bind(owner).bind(hash).bind(label).bind(now).bind(expires_at).bind(connection_id)
        .fetch_one(&mut **tx)
        .await
    }
    async fn lock_user(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
    ) -> Result<Option<bool>, sqlx::Error> {
        // FOR UPDATE conflicts with every row-level lock mode (PostgreSQL 16, table 13.3), so it waits for a
        // disable in progress and then reads the committed flag.
        sqlx::query_scalar("SELECT disabled FROM users WHERE email = $1 FOR UPDATE")
            .bind(email)
            .fetch_optional(&mut **tx)
            .await
    }
    async fn insert_refresh_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        connection_id: i64,
        hash: &str,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO refresh_tokens (connection_id, hash, created_at) VALUES ($1, $2, $3)",
        )
        .bind(connection_id)
        .bind(hash)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
    async fn delete_ended_connections(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        before: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        // A connection stopped at its end or at its revocation, whichever came first.
        Ok(sqlx::query("DELETE FROM connections WHERE owner = $1 AND LEAST(ends_at, COALESCE(revoked_at, ends_at)) < $2")
            .bind(owner).bind(before)
            .execute(&mut **tx)
            .await?
            .rows_affected())
    }
    async fn find_refresh_token(
        &self,
        hash: &str,
    ) -> Result<Option<(i64, i64, String)>, sqlx::Error> {
        sqlx::query_as(
            "SELECT r.id, r.connection_id, c.owner FROM refresh_tokens r JOIN connections c ON c.id = r.connection_id WHERE r.hash = $1",
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
    }
    async fn use_refresh_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        // Read Committed re-evaluates the WHERE after waiting for a concurrent update (PostgreSQL 16, 13.2.1), so
        // exactly one transaction ever gets the row back.
        let marked: Option<i64> = sqlx::query_scalar(
            "UPDATE refresh_tokens SET used_at = $2 WHERE id = $1 AND used_at IS NULL RETURNING id",
        )
        .bind(id)
        .bind(now)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(marked.is_some())
    }
    async fn connection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Option<Connection>, sqlx::Error> {
        let row = sqlx::query(&format!(
            "SELECT {CONNECTION} FROM connections c WHERE c.id = $1"
        ))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
        row.as_ref().map(connection_from).transpose()
    }
    async fn revoke_connections(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        only: Option<i64>,
        reason: RevokedReason,
        now: DateTime<Utc>,
    ) -> Result<Vec<(i64, String)>, sqlx::Error> {
        let mut rows: Vec<(i64, String)> = sqlx::query_as(
            "UPDATE connections SET revoked_at = $3, revoked_reason = $4 \
             WHERE owner = $1 AND ($2::bigint IS NULL OR id = $2) AND revoked_at IS NULL AND ends_at > $3 \
             RETURNING id, client_id",
        )
        .bind(owner).bind(only).bind(now).bind(reason_text(reason))
        .fetch_all(&mut **tx)
        .await?;
        rows.sort();
        Ok(rows)
    }
    async fn delete_expired_mcp_tokens(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        connection_id: i64,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM tokens WHERE connection_id = $1 AND expires_at <= $2")
            .bind(connection_id)
            .bind(now)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn revoke_all_for(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        Ok(
            sqlx::query(
                "UPDATE tokens SET revoked_at = $2 WHERE owner = $1 AND revoked_at IS NULL",
            )
            .bind(owner)
            .bind(now)
            .execute(&mut **tx)
            .await?
            .rows_affected(),
        )
    }
    async fn set_disabled(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
        disabled: bool,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        // The WHERE makes an unchanged flag touch no row, so rows_affected is "did the flag change (or the user appear)".
        Ok(sqlx::query("INSERT INTO users (email, disabled, created_at) VALUES ($1, $2, $3) ON CONFLICT (email) DO UPDATE SET disabled = EXCLUDED.disabled WHERE users.disabled IS DISTINCT FROM EXCLUDED.disabled")
            .bind(email)
            .bind(disabled)
            .bind(now)
            .execute(&mut **tx)
            .await?
            .rows_affected()
            == 1)
    }
    async fn set_display_name(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        email: &str,
        name: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET display_name = $2 WHERE email = $1 AND display_name <> $2")
            .bind(email)
            .bind(name)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn list_for(&self, owner: &str) -> Result<Vec<TokenSummary>, sqlx::Error> {
        use sqlx::Row;
        let rows = sqlx::query("SELECT id, kind, label, created_at, expires_at, revoked_at FROM tokens WHERE owner = $1 AND audience = $2 ORDER BY created_at DESC, id DESC")
            .bind(owner)
            .bind(Audience::Api.as_str())
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| {
                let text: String = r.try_get("kind")?;
                let kind = kind_from(&text).ok_or_else(|| {
                    sqlx::Error::Decode(format!("unknown token kind {text:?}").into())
                })?;
                Ok(TokenSummary {
                    id: r.try_get("id")?,
                    kind,
                    label: r.try_get("label")?,
                    created_at: r.try_get("created_at")?,
                    expires_at: r.try_get("expires_at")?,
                    revoked_at: r.try_get("revoked_at")?,
                })
            })
            .collect()
    }
    async fn revoke_one(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query(
            "UPDATE tokens SET revoked_at = $3 WHERE id = $2 AND owner = $1 AND audience = $4 AND revoked_at IS NULL",
        )
        .bind(owner)
        .bind(id)
        .bind(now)
        .bind(Audience::Api.as_str())
        .execute(&mut **tx)
        .await?
        .rows_affected()
            == 1)
    }
    async fn list_users(&self) -> Result<Vec<UserRow>, sqlx::Error> {
        let rows = sqlx::query_as(USERS).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(user_row).collect())
    }
    async fn find_user(&self, email: &str) -> Result<Option<UserRow>, sqlx::Error> {
        let row = sqlx::query_as(
            "SELECT email, display_name, disabled, created_at FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_row))
    }
}

const USERS: &str = "SELECT email, display_name, disabled, created_at FROM users ORDER BY email";

fn user_row(
    (email, display_name, disabled, created_at): (String, String, bool, DateTime<Utc>),
) -> UserRow {
    UserRow {
        email,
        display_name,
        disabled,
        created_at,
    }
}

/// The columns of a connection, with when it last refreshed, for a query whose table alias is `c`.
const CONNECTION: &str = "c.id, c.owner, c.client_id, c.client_name, c.scope, c.approved_at, c.ends_at, c.revoked_at, c.revoked_reason, \
     (SELECT max(r.used_at) FROM refresh_tokens r WHERE r.connection_id = c.id) AS refreshed_at";

/// The one place a stored connection becomes a `Connection`, as `list_for` builds a `TokenSummary`; text no code
/// writes is a decode error, not a guess.
fn connection_from(r: &sqlx::postgres::PgRow) -> Result<Connection, sqlx::Error> {
    use sqlx::Row;
    let unknown =
        |what: &str, text: &str| sqlx::Error::Decode(format!("unknown {what} {text:?}").into());
    let scope: String = r.try_get("scope")?;
    let revoked_at: Option<DateTime<Utc>> = r.try_get("revoked_at")?;
    let reason: Option<String> = r.try_get("revoked_reason")?;
    let revoked = match (revoked_at, reason) {
        (None, None) => None,
        (Some(at), Some(text)) => Some((
            at,
            reason_from(&text).ok_or_else(|| unknown("revoked_reason", &text))?,
        )),
        // The table sets both or neither; a row that says otherwise is not read as live.
        _ => {
            return Err(sqlx::Error::Decode(
                "revoked_at and revoked_reason are not set together".into(),
            ));
        }
    };
    Ok(Connection {
        id: r.try_get("id")?,
        owner: r.try_get("owner")?,
        client_id: r.try_get("client_id")?,
        client_name: r.try_get("client_name")?,
        scope: scope_from(&scope).ok_or_else(|| unknown("scope", &scope))?,
        approved_at: r.try_get("approved_at")?,
        ends_at: r.try_get("ends_at")?,
        refreshed_at: r.try_get("refreshed_at")?,
        revoked,
    })
}

/// Every user row, read on the caller's transaction (the export's snapshot). Only the user columns: no token is read.
pub(crate) async fn users_in(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Vec<UserRow>, sqlx::Error> {
    let rows = sqlx::query_as(USERS).fetch_all(&mut **tx).await?;
    Ok(rows.into_iter().map(user_row).collect())
}

pub fn make_token_store(pool: PgPool) -> Arc<dyn TokenStore> {
    Arc::new(PgTokenStore::new(pool))
}
