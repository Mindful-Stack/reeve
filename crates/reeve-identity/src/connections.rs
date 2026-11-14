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
    domain::{IdentityConfig, hash_secret, new_secret, random_secret, reason_text},
    ports::{IdentityActivity, IdentityEvent},
    repo::TokenStore,
};
use chrono::{DateTime, Duration, Utc};
use reeve_protocol::{RevokedReason, Scope, Via};
use sqlx::{PgPool, Postgres, Transaction};

/// How long a connection lasts from its approval (kb/ADR-0029 R5).
pub(crate) const CONNECTION_DAYS: i64 = 30;
/// The longest an MCP access token lives (kb/ADR-0029 R5); migration 2005 holds the database to it as well.
pub(crate) const ACCESS_TOKEN_HOURS: i64 = 1;
/// The client of a connection made by `reeve-server mcp-token-issue` (a developer command of test builds), and
/// the name listings show for it.
pub(crate) const DEVELOPER_CLIENT_ID: &str = "urn:reeve:developer-command";
pub(crate) const DEVELOPER_CLIENT_NAME: &str = "Developer command";

/// What "live" means: not revoked, and not yet at its end. The same predicate is repeated in SQL, in
/// `revoke_connections` and `live_connection_owners` in `repo.rs`; a change here changes them too.
pub(crate) fn is_live(
    ends_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    revoked_at.is_none() && ends_at > now
}

/// A connection as the store hands it out. Today only the refresh grant reads it; the listing of a person's
/// connections (kb/ADR-0029 R9) arrives in the next pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub id: i64,
    pub owner: String,
    pub client_id: String,
    pub client_name: String,
    pub scope: Scope,
    pub approved_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// When the client last refreshed: the newest `used_at` of the connection's refresh tokens.
    pub refreshed_at: Option<DateTime<Utc>>,
    /// When and why it was revoked; `None` while it is not. Who did it is in the activity entry.
    pub revoked: Option<(DateTime<Utc>, RevokedReason)>,
}

impl Connection {
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        is_live(self.ends_at, self.revoked.map(|(at, _)| at), now)
    }
}

/// What the token route answers a successful grant with (RFC 6749 section 5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds until the access token expires: one hour, or less when the connection ends sooner.
    pub expires_in: i64,
    /// The scope granted, as the answer names it (`granted_scope`).
    pub scope: &'static str,
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
    let expires_at = (now + Duration::hours(ACCESS_TOKEN_HOURS)).min(ends_at);
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
        scope: granted_scope(scope),
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

/// Who acts when no person does (a reused refresh token). The webhook's entries name `github` the same way.
pub(crate) const SYSTEM: &str = "system";
/// Who acts in an operator command, as the other operator commands name it.
const OPERATOR: &str = "operator";

/// One `connection_revoked` entry per connection in `revoked`: the one writer of that action, for every way a
/// connection is revoked. Called last in its transaction.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_revocations(
    activity: &dyn IdentityActivity,
    tx: &mut Transaction<'_, Postgres>,
    actor: &str,
    via: Via,
    owner: &str,
    revoked: &[(i64, String)],
    reason: RevokedReason,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    for (id, client_id) in revoked {
        activity
            .record(
                tx,
                IdentityEvent {
                    at: now,
                    actor: actor.into(),
                    via,
                    subject: owner.into(),
                    action: "connection_revoked",
                    payload: serde_json::json!({"connection_id": id, "client_id": client_id, "reason": reason_text(reason)}),
                },
            )
            .await?;
    }
    Ok(())
}

/// A refresh request as the token route read it.
pub struct RefreshRequest<'a> {
    pub refresh_token: &'a str,
    /// Optional at a refresh (RFC 6749 section 6); compared with the connection's client when sent.
    pub client_id: Option<&'a str>,
    /// The `scope` parameter, when sent.
    pub scope: Option<&'a str>,
    /// Whether the `resource` parameter was absent or named this endpoint.
    pub resource_matches: bool,
}

/// Whether a requested scope stays within the granted one (OAuth 2.1 section 4.3.1: "MUST NOT include any scope
/// not originally granted"). Omitted means the granted scope; `offline_access` asks for nothing.
fn within(requested: Option<&str>, granted: Scope) -> bool {
    requested.is_none_or(|scope| {
        scope.split(' ').all(|name| match name {
            "read" | "offline_access" => true,
            "write" => granted == Scope::Write,
            _ => false,
        })
    })
}

/// The `scope` a token answer names for a connection's scope: a write connection may also read, so it names both,
/// as the metadata lists them.
fn granted_scope(scope: Scope) -> &'static str {
    match scope {
        Scope::Read => "read",
        Scope::Write => "read write",
    }
}

/// The refresh grant (kb/ADR-0029 R5): rotation, with reuse revoking the connection. One transaction: the
/// owner's row first, then the refresh token's mark. A token that was used before revokes the connection, with
/// its entry, and commits; every other refusal returns before the commit, so the dropped transaction rolls back
/// the used mark and the same token can be presented again.
pub async fn refresh_tokens(
    with: Issuing<'_>,
    activity: &dyn IdentityActivity,
    request: RefreshRequest<'_>,
    now: DateTime<Utc>,
) -> Result<TokenPair, GrantError> {
    let found = with
        .store
        .find_refresh_token(&hash_secret(request.refresh_token))
        .await?;
    let Some((id, connection_id, owner)) = found else {
        return Err(GrantError::InvalidGrant("unknown refresh token"));
    };
    let policy = with.policy().await?;
    let mut tx = with.pool.begin().await?;
    let disabled = with.store.lock_user(&mut tx, &owner).await?;
    let Some(connection) = with.store.connection(&mut tx, connection_id).await? else {
        return Err(GrantError::InvalidGrant("the connection is gone"));
    };
    if !with.store.use_refresh_token(&mut tx, id, now).await? {
        let revoked = with
            .store
            .revoke_connections(
                &mut tx,
                &owner,
                Some(connection.id),
                RevokedReason::RefreshTokenReused,
                now,
            )
            .await?;
        record_revocations(
            activity,
            &mut tx,
            SYSTEM,
            Via::System,
            &owner,
            &revoked,
            RevokedReason::RefreshTokenReused,
            now,
        )
        .await?;
        tx.commit().await?;
        return Err(GrantError::InvalidGrant(
            "a used refresh token was presented again: the connection is revoked",
        ));
    }
    // The checks. Each failure returns here; nothing has been committed.
    if !connection.is_live(now) {
        return Err(GrantError::InvalidGrant(
            "the connection is revoked or has ended",
        ));
    }
    if request
        .client_id
        .is_some_and(|sent| sent != connection.client_id)
    {
        return Err(GrantError::InvalidGrant(
            "the refresh token was issued to another client",
        ));
    }
    if !request.resource_matches {
        return Err(GrantError::InvalidTarget);
    }
    if !within(request.scope, connection.scope) {
        return Err(GrantError::InvalidScope);
    }
    if disabled != Some(false) {
        return Err(GrantError::InvalidGrant("the owner is disabled"));
    }
    if !policy.is_allowed(&owner) {
        return Err(GrantError::InvalidGrant("the owner is no longer allowed"));
    }
    let pair = issue_pair(
        with.store,
        &mut tx,
        &owner,
        connection.id,
        &connection.client_name,
        connection.scope,
        connection.ends_at,
        now,
    )
    .await?;
    with.store
        .delete_expired_mcp_tokens(&mut tx, connection.id, now)
        .await?;
    tx.commit().await?;
    Ok(pair)
}

/// Revokes every live connection of every person, with one `connection_revoked` entry each, by the operator via
/// the system: what an operator runs after a restore, which rewinds refresh-token rotation (reason `Restored`). In each transaction the
/// owners' rows are locked first, in order, and the entries are written last. The owners are read before the
/// transaction that locks them, so the read is repeated after each commit until no live connection is left: a
/// connection made in between, by a server that was not stopped, ends as well. Returns how many were revoked.
pub async fn revoke_all_connections(
    pool: &PgPool,
    store: &dyn TokenStore,
    activity: &dyn IdentityActivity,
    now: DateTime<Utc>,
) -> Result<usize, sqlx::Error> {
    let reason = RevokedReason::Restored;
    let mut count = 0;
    loop {
        let owners = store.live_connection_owners(now).await?;
        if owners.is_empty() {
            return Ok(count);
        }
        let mut tx = pool.begin().await?;
        for owner in &owners {
            store.lock_user(&mut tx, owner).await?;
        }
        let mut revoked = vec![];
        for owner in &owners {
            revoked.push((
                owner,
                store
                    .revoke_connections(&mut tx, owner, None, reason, now)
                    .await?,
            ));
        }
        for (owner, connections) in &revoked {
            record_revocations(
                activity,
                &mut tx,
                OPERATOR,
                Via::System,
                owner,
                connections,
                reason,
                now,
            )
            .await?;
        }
        tx.commit().await?;
        count += revoked
            .iter()
            .map(|(_, connections)| connections.len())
            .sum::<usize>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a refresh that can widen a read connection, and one that refuses what a client may send.
    #[test]
    fn a_requested_scope_stays_within_the_grant() {
        for (requested, granted, ok) in [
            (None, Scope::Read, true),
            (Some("read"), Scope::Read, true),
            (Some("read offline_access"), Scope::Read, true),
            (Some("write"), Scope::Read, false),
            (Some("read write"), Scope::Read, false),
            (Some("read write"), Scope::Write, true),
            (Some("read"), Scope::Write, true),
            (Some("admin"), Scope::Write, false),
            (Some("read  write"), Scope::Write, false),
        ] {
            assert_eq!(
                within(requested, granted),
                ok,
                "{requested:?} within {granted:?}"
            );
        }
    }
}
