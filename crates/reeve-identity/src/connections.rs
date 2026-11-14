//! Connections (kb/ADR-0029 R5, R9): an approved, exchanged request to connect an MCP client: a person, a client,
//! a scope, and an end thirty days after the approval. Every MCP token belongs to one.
//!
//! The lock rule of every transaction, here and in `handler.rs`, that touches a person's connections, MCP tokens
//! or refresh tokens: the person's `users` row first (`TokenStore::lock_user`, or the `ON CONFLICT DO UPDATE` of
//! `set_disabled` and `ensure_user`), the activity entries last. Whoever holds the row is alone with that
//! person's connections, so nothing in between needs an order (PostgreSQL 16, 13.3.4: "acquire locks on multiple
//! objects in a consistent order").
use crate::{
    access::AccessPolicy,
    access_repo::AccessStore,
    authorization::{AuthorizationRequests, RedeemRefusal},
    domain::{IdentityConfig, hash_secret, new_secret, random_secret},
    repo::TokenStore,
};
use chrono::{DateTime, Duration, Utc};
use reeve_protocol::Scope;
use sqlx::{PgPool, Postgres, Transaction};

/// How long a connection lasts from its approval (kb/ADR-0029 R5).
pub(crate) const CONNECTION_DAYS: i64 = 30;
/// The client of a connection made by `reeve-server mcp-token-issue` (a developer command of test builds), and
/// the name listings show for it.
pub(crate) const DEVELOPER_CLIENT_ID: &str = "urn:reeve:developer-command";
pub(crate) const DEVELOPER_CLIENT_NAME: &str = "Developer command";

/// The one place "live" is decided: not revoked, and not yet at its end.
pub(crate) fn is_live(
    ends_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    revoked_at.is_none() && ends_at > now
}

/// What the token route answers a successful grant with (RFC 6749 section 5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds until the access token expires: one hour, or less when the connection ends sooner.
    pub expires_in: i64,
    /// The scope granted: the connection's.
    pub scope: Scope,
}

/// Why a grant was refused, as RFC 6749 section 5.2 and RFC 8707 name it. `InvalidGrant` carries a fixed reason
/// for the operator's log; nothing a caller sent is ever in it.
#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    #[error("invalid_grant: {0}")]
    InvalidGrant(&'static str),
    #[error("invalid_target")]
    InvalidTarget,
    #[error("invalid_scope")]
    InvalidScope,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// A code exchange as the token route read it.
pub struct ExchangeRequest<'a> {
    pub code: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: Option<&'a str>,
    pub resource: Option<&'a str>,
    pub verifier: &'a str,
}

/// What both grants need to issue: the pool, the token store, and how to load the access policy when it is
/// needed (never cached, kb/ADR-0015 R5).
pub struct Issuing<'a> {
    pub pool: &'a PgPool,
    pub store: &'a dyn TokenStore,
    pub access: &'a dyn AccessStore,
    pub floor: &'a IdentityConfig,
}

impl Issuing<'_> {
    async fn policy(&self) -> Result<AccessPolicy, sqlx::Error> {
        self.access.load(self.floor).await
    }
}

/// A new access token and a new refresh token of `connection`. The caller holds the owner's `users` row lock.
#[allow(clippy::too_many_arguments)]
async fn issue_pair(
    store: &dyn TokenStore,
    tx: &mut Transaction<'_, Postgres>,
    owner: &str,
    connection: i64,
    label: &str,
    scope: Scope,
    ends_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<TokenPair, sqlx::Error> {
    // kb/ADR-0029 R5: at most one hour, and never past the connection's end.
    let expires_at = (now + Duration::hours(1)).min(ends_at);
    let access_token = new_secret();
    store
        .insert_mcp_token(
            tx,
            owner,
            connection,
            &hash_secret(&access_token),
            label,
            expires_at,
            now,
        )
        .await?;
    let refresh_token = random_secret("reeve_refresh_");
    store
        .insert_refresh_token(tx, connection, &hash_secret(&refresh_token), now)
        .await?;
    Ok(TokenPair {
        access_token,
        refresh_token,
        expires_in: (expires_at - now).num_seconds(),
        scope,
    })
}

/// The authorization code grant (kb/ADR-0029 R4, R5). The code is redeemed first, in memory: checked against its
/// client, redirect address, resource and verifier before it is spent, and spent exactly once. Then the access
/// policy is loaded and, in one transaction, the approver's row is locked and must be enabled; the approver's
/// connections that stopped more than thirty days ago are deleted; the connection, an MCP token and a refresh
/// token are created. No activity entry: the approval wrote one. If a step after the redeem fails, the code is
/// spent and the person connects again.
pub async fn exchange_code(
    with: Issuing<'_>,
    request: ExchangeRequest<'_>,
    requests: &AuthorizationRequests,
    now: DateTime<Utc>,
) -> Result<TokenPair, GrantError> {
    let grant = requests
        .redeem(request.code, request.client_id, request.redirect_uri, request.resource, request.verifier, now)
        .map_err(|refusal| match refusal {
            RedeemRefusal::InvalidGrant => GrantError::InvalidGrant(
                "the code is unknown, used or expired, or was presented with another client, redirect address or verifier",
            ),
            RedeemRefusal::InvalidTarget => GrantError::InvalidTarget,
        })?;
    if !with.policy().await?.is_allowed(&grant.owner) {
        return Err(GrantError::InvalidGrant(
            "the approver is no longer allowed",
        ));
    }
    let mut tx = with.pool.begin().await?;
    if with.store.lock_user(&mut tx, &grant.owner).await? != Some(false) {
        return Err(GrantError::InvalidGrant("the approver is disabled"));
    }
    // Thirty days from the approval (R5), not from this exchange.
    let ends_at = grant.approved_at + Duration::days(CONNECTION_DAYS);
    with.store
        .delete_ended_connections(&mut tx, &grant.owner, now - Duration::days(CONNECTION_DAYS))
        .await?;
    let connection = with
        .store
        .insert_connection(
            &mut tx,
            &grant.owner,
            &grant.client_id,
            &grant.client_name,
            grant.scope,
            grant.approved_at,
            ends_at,
        )
        .await?;
    let pair = issue_pair(
        with.store,
        &mut tx,
        &grant.owner,
        connection,
        &grant.client_name,
        grant.scope,
        ends_at,
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(pair)
}
