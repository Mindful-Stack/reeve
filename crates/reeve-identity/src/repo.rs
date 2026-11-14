use crate::domain::*;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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
    async fn insert_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &str,
        kind: TokenKind,
        audience: Audience,
        hash: &str,
        label: &str,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error>;
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
    async fn list_for(&self, owner: &str) -> Result<Vec<TokenSummary>, sqlx::Error>;
    /// Revokes one token if it belongs to `owner` and is not yet revoked. Returns whether a row changed.
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
        audience: Audience,
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
            .bind(owner).bind(kind).bind(audience.as_str()).bind(hash).bind(label).bind(now).bind(expires_at).fetch_one(&mut **tx).await
    }
    async fn find_by_hash(&self, hash: &str) -> Result<Option<TokenRow>, sqlx::Error> {
        sqlx::query_as("SELECT t.id, t.owner, t.kind, t.audience, t.expires_at, t.revoked_at, u.disabled AS owner_disabled FROM tokens t JOIN users u ON u.email = t.owner WHERE t.hash = $1")
            .bind(hash).fetch_optional(&self.pool).await
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
        let rows = sqlx::query("SELECT id, kind, label, created_at, expires_at, revoked_at FROM tokens WHERE owner = $1 ORDER BY created_at DESC, id DESC")
            .bind(owner)
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
            "UPDATE tokens SET revoked_at = $3 WHERE id = $2 AND owner = $1 AND revoked_at IS NULL",
        )
        .bind(owner)
        .bind(id)
        .bind(now)
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
