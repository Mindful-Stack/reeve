//! `/access`: who may sign in and who is admin (kb/ADR-0015). Reading needs an admin; writing needs an admin with an
//! interactive token, which the identity handlers check themselves.
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_identity::{AccessPolicy, AccessRow};
use reeve_protocol::*;
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

fn kind(k: AccessKind) -> reeve_identity::AccessKind {
    match k {
        AccessKind::Domain => reeve_identity::AccessKind::Domain,
        AccessKind::Email => reeve_identity::AccessKind::Email,
    }
}

fn row(r: AccessRow) -> AccessEntryDto {
    let (source, added_by, added_at) = match r.source {
        reeve_identity::AccessSource::Config => (AccessSource::Config, None, None),
        reeve_identity::AccessSource::Database { added_by, added_at } => {
            (AccessSource::Database, Some(added_by), Some(added_at))
        }
    };
    AccessEntryDto {
        value: r.value,
        source,
        also_in_config: r.also_in_config,
        allowed: r.allowed,
        added_by,
        added_at,
    }
}

fn dto(p: &AccessPolicy) -> AccessDto {
    let v = p.view();
    AccessDto {
        domains: v.domains.into_iter().map(row).collect(),
        emails: v.emails.into_iter().map(row).collect(),
        admins: v.admins.into_iter().map(row).collect(),
    }
}

#[utoipa::path(get, path = "/access", responses((status = 200, body = Response<AccessDto>), (status = 403, description = "admins only")))]
async fn get_access(c: Caller) -> Result<Json<Response<AccessDto>>, AppError> {
    if c.0.role != Role::Admin {
        return Err(AppError::Forbidden);
    }
    Ok(ok(dto(c.policy())))
}

#[utoipa::path(post, path = "/access/entries", request_body = AllowEntryRequest,
    responses((status = 200, body = Response<AccessDto>), (status = 403, description = "admins with an interactive token only")))]
async fn allow_entry(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<AllowEntryRequest>,
) -> Result<Json<Response<AccessDto>>, AppError> {
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let p = reeve_identity::allow_entry(
        &s.pool,
        &*s.access,
        &*log,
        &s.identity,
        &c.0,
        kind(b.kind),
        &b.value,
        c.1,
    )
    .await?;
    Ok(ok(dto(&p)))
}

#[utoipa::path(delete, path = "/access/entries/{kind}/{value}", params(("kind" = AccessKind, Path), ("value" = String, Path)),
    responses((status = 200, body = Response<AccessDto>), (status = 403, description = "admins with an interactive token only")))]
async fn remove_entry(
    State(s): State<AppState>,
    c: Caller,
    ApiPath((k, value)): ApiPath<(AccessKind, String)>,
) -> Result<Json<Response<AccessDto>>, AppError> {
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let p = reeve_identity::remove_entry(
        &s.pool,
        &*s.access,
        &*log,
        &s.identity,
        &c.0,
        kind(k),
        &value,
        c.1,
    )
    .await?;
    Ok(ok(dto(&p)))
}

#[utoipa::path(post, path = "/access/admins", request_body = GrantAdminRequest,
    responses((status = 200, body = Response<AccessDto>), (status = 403, description = "admins with an interactive token only")))]
async fn grant_admin(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<GrantAdminRequest>,
) -> Result<Json<Response<AccessDto>>, AppError> {
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let p =
        reeve_identity::grant_admin(&s.pool, &*s.access, &*log, &s.identity, &c.0, &b.email, c.1)
            .await?;
    Ok(ok(dto(&p)))
}

#[utoipa::path(delete, path = "/access/admins/{email}", params(("email" = String, Path)),
    responses((status = 200, body = Response<AccessDto>), (status = 403, description = "admins with an interactive token only")))]
async fn revoke_admin(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(email): ApiPath<String>,
) -> Result<Json<Response<AccessDto>>, AppError> {
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let p =
        reeve_identity::revoke_admin(&s.pool, &*s.access, &*log, &s.identity, &c.0, &email, c.1)
            .await?;
    Ok(ok(dto(&p)))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_access))
        .routes(routes!(allow_entry))
        .routes(routes!(remove_entry))
        .routes(routes!(grant_admin))
        .routes(routes!(revoke_admin))
}
