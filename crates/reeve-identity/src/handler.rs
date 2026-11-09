use crate::{
    domain::*,
    ports::*,
    repo::{TokenStore, TokenSummary, UserRow},
};
use chrono::{DateTime, Utc};
use reeve_protocol::Via;
use sqlx::PgPool;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("email not allowed by configuration")]
    NotAllowed,
    #[error("user is disabled")]
    Disabled,
    #[error("invalid request: {0}")]
    Validation(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub struct IssuedToken {
    pub id: i64,
    pub secret: String,
}

/// Handlers own the clock: `now` is passed in, never read here. `actor` is who performs the operation (the operator or, in plan B, the user on their token page); `via` is the channel it came through.
#[allow(clippy::too_many_arguments)]
pub async fn issue_token(
    pool: &PgPool,
    store: &dyn TokenStore,
    activity: &dyn IdentityActivity,
    cfg: &IdentityConfig,
    actor: &str,
    via: Via,
    email: &str,
    kind: TokenKind,
    label: &str,
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<IssuedToken, AuthError> {
    let email = normalise_email(email);
    if !cfg.is_allowed(&email) {
        return Err(AuthError::NotAllowed);
    }
    if expires_at.is_some_and(|e| e <= now) {
        return Err(AuthError::Validation(
            "expires_at must be in the future".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    if store.ensure_user(&mut tx, &email, now).await? {
        return Err(AuthError::Disabled);
    }
    let secret = new_secret();
    let id = store
        .insert_token(
            &mut tx,
            &email,
            kind,
            &hash_secret(&secret),
            label,
            expires_at,
            now,
        )
        .await?;
    activity
        .record(
            &mut tx,
            IdentityEvent {
                at: now,
                actor: actor.into(),
                via,
                subject: email.clone(),
                action: "token_issued",
                payload: serde_json::json!({"kind": kind, "label": label, "token_id": id}),
            },
        )
        .await?;
    tx.commit().await?;
    Ok(IssuedToken { id, secret })
}

/// Resolves a bearer or session secret to its principal. Every refusal is logged at `warn` with its reason, the token
/// id and owner when a row was found, and the first eight hex digits of the secret's hash (never the secret), so a 401
/// can be told apart afterwards: not found, revoked, owner disabled, expired, owner not allowed, bad kind.
pub async fn authenticate(
    store: &dyn TokenStore,
    cfg: &IdentityConfig,
    secret: &str,
    now: DateTime<Utc>,
) -> Result<Principal, AuthError> {
    let hash = hash_secret(secret);
    let hash_prefix = &hash[..8];
    let Some(row) = store.find_by_hash(&hash).await? else {
        tracing::warn!(
            hash_prefix,
            reason = "token not found",
            "authentication refused"
        );
        return Err(AuthError::Unauthenticated);
    };
    let owner = normalise_email(&row.owner);
    let reason = if row.revoked_at.is_some() {
        Some("revoked")
    } else if row.owner_disabled {
        Some("owner disabled")
    } else if row.expires_at.is_some_and(|e| e <= now) {
        Some("expired")
    } else if !cfg.is_allowed(&owner) {
        Some("owner not allowed")
    } else {
        None
    };
    if let Some(reason) = reason {
        tracing::warn!(hash_prefix, token_id = row.id, %owner, reason, "authentication refused");
        return Err(AuthError::Unauthenticated);
    }
    let Some(token_kind) = kind_from(&row.kind) else {
        tracing::warn!(hash_prefix, token_id = row.id, %owner, reason = "bad kind", kind = %row.kind, "authentication refused");
        return Err(AuthError::Unauthenticated);
    };
    Ok(Principal {
        role: cfg.role_for(&owner),
        email: owner,
        token_kind,
    })
}

pub async fn revoke_all_for(
    pool: &PgPool,
    store: &dyn TokenStore,
    activity: &dyn IdentityActivity,
    actor: &str,
    via: Via,
    email: &str,
    now: DateTime<Utc>,
) -> Result<u64, AuthError> {
    let email = normalise_email(email);
    let mut tx = pool.begin().await?;
    let n = store.revoke_all_for(&mut tx, &email, now).await?;
    activity
        .record(
            &mut tx,
            IdentityEvent {
                at: now,
                actor: actor.into(),
                via,
                subject: email,
                action: "tokens_revoked",
                payload: serde_json::json!({"count": n}),
            },
        )
        .await?;
    tx.commit().await?;
    Ok(n)
}

/// Disable and revoke commit together (kb/ADR-0015 R6, kb/ADR-0010 R5). Works for a user not yet seen.
pub async fn disable_user(
    pool: &PgPool,
    store: &dyn TokenStore,
    activity: &dyn IdentityActivity,
    actor: &str,
    via: Via,
    email: &str,
    now: DateTime<Utc>,
) -> Result<(), AuthError> {
    let email = normalise_email(email);
    let mut tx = pool.begin().await?;
    store.set_disabled(&mut tx, &email, true, now).await?;
    let n = store.revoke_all_for(&mut tx, &email, now).await?;
    activity
        .record(
            &mut tx,
            IdentityEvent {
                at: now,
                actor: actor.into(),
                via,
                subject: email,
                action: "user_disabled",
                payload: serde_json::json!({"tokens_revoked": n}),
            },
        )
        .await?;
    tx.commit().await?;
    Ok(())
}

/// What the identity provider asserted about the person, after the server verified the ID token.
#[derive(Debug, Clone)]
pub struct VerifiedIdentity {
    pub email: String,
    pub email_verified: bool,
    pub name: Option<String>,
}

pub const SESSION_LABEL: &str = "web session";

/// Browser sign-in: the session is an interactive token, so everything plan A knows about tokens (expiry, revocation,
/// disable) applies to it unchanged. Refuses an unverified email before looking at the allowlist (kb/ADR-0015 R1).
pub async fn sign_in(
    pool: &PgPool,
    store: &dyn TokenStore,
    activity: &dyn IdentityActivity,
    cfg: &IdentityConfig,
    who: VerifiedIdentity,
    session_days: i64,
    now: DateTime<Utc>,
) -> Result<IssuedToken, AuthError> {
    if !(1..=365).contains(&session_days) {
        return Err(AuthError::Validation(
            "session_days must be between 1 and 365".into(),
        ));
    }
    if !who.email_verified {
        return Err(AuthError::NotAllowed);
    }
    let email = normalise_email(&who.email);
    if !cfg.is_allowed(&email) {
        return Err(AuthError::NotAllowed);
    }
    let mut tx = pool.begin().await?;
    if store.ensure_user(&mut tx, &email, now).await? {
        return Err(AuthError::Disabled);
    }
    if let Some(name) = who.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        store.set_display_name(&mut tx, &email, name).await?;
    }
    let secret = new_secret();
    let expires_at = now + chrono::Duration::days(session_days);
    let id = store
        .insert_token(
            &mut tx,
            &email,
            TokenKind::Interactive,
            &hash_secret(&secret),
            SESSION_LABEL,
            Some(expires_at),
            now,
        )
        .await?;
    activity
        .record(
            &mut tx,
            IdentityEvent {
                at: now,
                actor: email.clone(),
                via: Via::Interactive,
                subject: email.clone(),
                action: "signed_in",
                payload: serde_json::json!({"token_id": id, "expires_at": expires_at}),
            },
        )
        .await?;
    tx.commit().await?;
    Ok(IssuedToken { id, secret })
}

/// The owner's tokens, newest first; never includes the secret or its hash.
pub async fn list_tokens(
    store: &dyn TokenStore,
    owner: &str,
) -> Result<Vec<TokenSummary>, AuthError> {
    Ok(store.list_for(&normalise_email(owner)).await?)
}

/// Revokes one of the owner's own tokens. `Ok(false)` covers not found, not the owner's and already revoked alike,
/// so a caller cannot probe other people's token ids; an entry is logged only when a token actually changed.
pub async fn revoke_token(
    pool: &PgPool,
    store: &dyn TokenStore,
    activity: &dyn IdentityActivity,
    owner: &str,
    via: Via,
    id: i64,
    now: DateTime<Utc>,
) -> Result<bool, AuthError> {
    let owner = normalise_email(owner);
    let mut tx = pool.begin().await?;
    let changed = store.revoke_one(&mut tx, &owner, id, now).await?;
    if changed {
        activity
            .record(
                &mut tx,
                IdentityEvent {
                    at: now,
                    actor: owner.clone(),
                    via,
                    subject: owner.clone(),
                    action: "token_revoked",
                    payload: serde_json::json!({"token_id": id}),
                },
            )
            .await?;
    }
    tx.commit().await?;
    Ok(changed)
}

/// Re-enables sign-in. Tokens revoked by the disable stay revoked: the person signs in again (kb/ADR-0015 R6 says
/// what disable must do; nothing says enable restores, and restoring would resurrect automation tokens nobody re-approved).
/// Records `user_enabled` only when the flag actually changed. Refuses an email outside the allowlist, so `set_disabled`'s upsert cannot create a user row for a stranger.
#[allow(clippy::too_many_arguments)]
pub async fn enable_user(
    pool: &PgPool,
    store: &dyn TokenStore,
    activity: &dyn IdentityActivity,
    cfg: &IdentityConfig,
    actor: &str,
    via: Via,
    email: &str,
    now: DateTime<Utc>,
) -> Result<(), AuthError> {
    let email = normalise_email(email);
    if !cfg.is_allowed(&email) {
        return Err(AuthError::NotAllowed);
    }
    let mut tx = pool.begin().await?;
    if store.set_disabled(&mut tx, &email, false, now).await? {
        activity
            .record(
                &mut tx,
                IdentityEvent {
                    at: now,
                    actor: actor.into(),
                    via,
                    subject: email.clone(),
                    action: "user_enabled",
                    payload: serde_json::json!({}),
                },
            )
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Every user row, for the admin page.
pub async fn list_users(store: &dyn TokenStore) -> Result<Vec<UserRow>, AuthError> {
    Ok(store.list_users().await?)
}

/// Every user row, read on the caller's transaction, so the export's users belong to its one snapshot.
pub async fn list_users_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Vec<UserRow>, AuthError> {
    Ok(crate::repo::users_in(tx).await?)
}
