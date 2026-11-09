//! Admin writes to sign-in access (kb/ADR-0015 R3, R4, R7). Each takes the lock, reads the policy inside its own
//! transaction and decides on that, never on the policy the request was authenticated with, so concurrent writes run
//! one after another. The role and token kind are checked here, so the rules hold for every caller.
use crate::access::{AccessKind, AccessPolicy, parse_domain, parse_email};
use crate::access_repo::AccessStore;
use crate::domain::{IdentityConfig, Principal, Role, TokenKind, normalise_email};
use crate::handler::AuthError;
use crate::ports::{IdentityActivity, IdentityEvent};
use chrono::{DateTime, Utc};
use reeve_protocol::Via;
use sqlx::{PgPool, Postgres, Transaction};

const SELF_LOCKOUT: &str = "you cannot remove your own access";
const IN_CONFIG: &str = "set in reeve.toml; change it there";

/// Interactive and admin by the request's principal: refuses early, before any validation or lock.
fn precheck(caller: &Principal) -> Result<(), AuthError> {
    if caller.token_kind == TokenKind::Interactive && caller.role == Role::Admin {
        Ok(())
    } else {
        Err(AuthError::Forbidden)
    }
}

/// Opens the write's transaction, takes the lock, reads the policy and re-checks the caller on it.
async fn begin<'a>(
    pool: &'a PgPool,
    store: &dyn AccessStore,
    floor: &IdentityConfig,
    caller: &Principal,
) -> Result<(Transaction<'a, Postgres>, AccessPolicy), AuthError> {
    let mut tx = pool.begin().await?;
    store.lock(&mut tx).await?;
    let policy = store.load_in(&mut tx, floor).await?;
    if policy.role_for(&caller.email) != Role::Admin {
        return Err(AuthError::Forbidden);
    }
    Ok((tx, policy))
}

fn event(
    caller: &Principal,
    now: DateTime<Utc>,
    subject: &str,
    action: &'static str,
    payload: serde_json::Value,
) -> IdentityEvent {
    IdentityEvent {
        at: now,
        actor: normalise_email(&caller.email),
        via: Via::Interactive,
        subject: subject.into(),
        action,
        payload,
    }
}

fn parse(kind: AccessKind, raw: &str) -> Result<String, AuthError> {
    match kind {
        AccessKind::Domain => parse_domain(raw),
        AccessKind::Email => parse_email(raw),
    }
    .map_err(AuthError::Validation)
}

/// Writes the events last, just before commit (the activity writer's lock is held from its insert to commit), and
/// answers the policy as committed.
async fn finish(
    mut tx: Transaction<'_, Postgres>,
    store: &dyn AccessStore,
    activity: &dyn IdentityActivity,
    floor: &IdentityConfig,
    events: Vec<IdentityEvent>,
) -> Result<AccessPolicy, AuthError> {
    let after = store.load_in(&mut tx, floor).await?;
    for e in events {
        activity.record(&mut tx, e).await?;
    }
    tx.commit().await?;
    Ok(after)
}

#[allow(clippy::too_many_arguments)]
pub async fn allow_entry(
    pool: &PgPool,
    store: &dyn AccessStore,
    activity: &dyn IdentityActivity,
    floor: &IdentityConfig,
    caller: &Principal,
    kind: AccessKind,
    value: &str,
    now: DateTime<Utc>,
) -> Result<AccessPolicy, AuthError> {
    precheck(caller)?;
    let value = parse(kind, value)?;
    let (mut tx, policy) = begin(pool, store, floor, caller).await?;
    if policy.in_table(kind, &value) {
        return Err(AuthError::Validation("already allowed".into()));
    }
    if policy.in_floor(kind, &value) {
        return Err(AuthError::Validation("already set in reeve.toml".into()));
    }
    store
        .insert_entry(&mut tx, kind, &value, &caller.email, now)
        .await?;
    let e = event(
        caller,
        now,
        &value,
        "access_allowed",
        serde_json::json!({"kind": kind.as_str()}),
    );
    finish(tx, store, activity, floor, vec![e]).await
}

#[allow(clippy::too_many_arguments)]
pub async fn remove_entry(
    pool: &PgPool,
    store: &dyn AccessStore,
    activity: &dyn IdentityActivity,
    floor: &IdentityConfig,
    caller: &Principal,
    kind: AccessKind,
    value: &str,
    now: DateTime<Utc>,
) -> Result<AccessPolicy, AuthError> {
    precheck(caller)?;
    let value = normalise_email(value);
    let (mut tx, policy) = begin(pool, store, floor, caller).await?;
    if !policy.in_table(kind, &value) {
        return Err(if policy.in_floor(kind, &value) {
            AuthError::Validation(IN_CONFIG.into())
        } else {
            AuthError::NotFound
        });
    }
    let after = policy.without_entry(kind, &value);
    if !after.is_allowed(&caller.email) || after.role_for(&caller.email) != Role::Admin {
        return Err(AuthError::Validation(SELF_LOCKOUT.into()));
    }
    store.delete_entry(&mut tx, kind, &value).await?;
    let mut events = vec![event(
        caller,
        now,
        &value,
        "access_removed",
        serde_json::json!({"kind": kind.as_str()}),
    )];
    // kb/ADR-0015 R4: the grants this removal leaves unallowed go with it, each logged. A grant already inert
    // before the removal was orphaned some other way and stays until an admin revokes it.
    for g in policy.admin_grants() {
        if policy.is_allowed(&g.value) && !after.is_allowed(&g.value) {
            store.delete_admin(&mut tx, &g.value).await?;
            events.push(event(
                caller,
                now,
                &g.value,
                "admin_revoked",
                serde_json::json!({"reason": "access_removed"}),
            ));
        }
    }
    finish(tx, store, activity, floor, events).await
}

#[allow(clippy::too_many_arguments)]
pub async fn grant_admin(
    pool: &PgPool,
    store: &dyn AccessStore,
    activity: &dyn IdentityActivity,
    floor: &IdentityConfig,
    caller: &Principal,
    email: &str,
    now: DateTime<Utc>,
) -> Result<AccessPolicy, AuthError> {
    precheck(caller)?;
    let email = parse(AccessKind::Email, email)?;
    let (mut tx, policy) = begin(pool, store, floor, caller).await?;
    if policy.is_floor_admin(&email) {
        return Err(AuthError::Validation("already set in reeve.toml".into()));
    }
    let granted = policy.has_admin_grant(&email);
    if granted && policy.is_allowed(&email) {
        return Err(AuthError::Validation("already admin".into()));
    }
    let mut events = vec![];
    if !policy.is_allowed(&email) {
        store
            .insert_entry(&mut tx, AccessKind::Email, &email, &caller.email, now)
            .await?;
        events.push(event(
            caller,
            now,
            &email,
            "access_allowed",
            serde_json::json!({"kind": "email"}),
        ));
    }
    if !granted {
        store
            .insert_admin(&mut tx, &email, &caller.email, now)
            .await?;
        events.push(event(
            caller,
            now,
            &email,
            "admin_granted",
            serde_json::json!({}),
        ));
    }
    finish(tx, store, activity, floor, events).await
}

#[allow(clippy::too_many_arguments)]
pub async fn revoke_admin(
    pool: &PgPool,
    store: &dyn AccessStore,
    activity: &dyn IdentityActivity,
    floor: &IdentityConfig,
    caller: &Principal,
    email: &str,
    now: DateTime<Utc>,
) -> Result<AccessPolicy, AuthError> {
    precheck(caller)?;
    let email = normalise_email(email);
    if email == normalise_email(&caller.email) {
        return Err(AuthError::Validation(SELF_LOCKOUT.into()));
    }
    let (mut tx, policy) = begin(pool, store, floor, caller).await?;
    if !policy.has_admin_grant(&email) {
        return Err(if policy.is_floor_admin(&email) {
            AuthError::Validation(IN_CONFIG.into())
        } else {
            AuthError::NotFound
        });
    }
    store.delete_admin(&mut tx, &email).await?;
    let e = event(caller, now, &email, "admin_revoked", serde_json::json!({}));
    finish(tx, store, activity, floor, vec![e]).await
}
