use super::tokens::via_of;
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::ApiPath,
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

fn require_admin(c: &Caller) -> Result<(), AppError> {
    if c.0.role == Role::Admin {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

#[utoipa::path(get, path = "/users", responses((status = 200, body = Response<Vec<UserDto>>), (status = 403, description = "admins only")))]
async fn list(
    State(s): State<AppState>,
    c: Caller,
) -> Result<Json<Response<Vec<UserDto>>>, AppError> {
    require_admin(&c)?;
    let rows = reeve_identity::list_users(&*s.tokens).await?;
    Ok(ok(rows
        .into_iter()
        .map(|u| UserDto {
            role: c.policy().role_for(&u.email),
            email: u.email,
            display_name: u.display_name,
            disabled: u.disabled,
            created_at: u.created_at,
        })
        .collect()))
}

#[utoipa::path(post, path = "/users/{email}/disable", params(("email" = String, Path)), responses((status = 200, body = Response<Empty>), (status = 403, description = "admins only")))]
async fn disable(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(email): ApiPath<String>,
) -> Result<Json<Response<Empty>>, AppError> {
    require_admin(&c)?;
    if normalise_email(&email) == normalise_email(&c.0.email) {
        return Err(AppError::Validation("you cannot disable yourself".into()));
    }
    // Checked here, not in `disable_user`: the operator command `reeve-server user-disable` keeps plan A's semantics
    // (it may disable an address not yet seen). Symmetric with enable, so no row is written that enable would refuse.
    if !c.policy().is_allowed(&email) {
        return Err(AppError::Validation(
            "email is not allowed by configuration".into(),
        ));
    }
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    reeve_identity::disable_user(
        &s.pool,
        &*s.tokens,
        &*log,
        &c.0.email,
        via_of(&c),
        &email,
        c.1,
    )
    .await?;
    Ok(ok(Empty {}))
}

#[utoipa::path(post, path = "/users/{email}/enable", params(("email" = String, Path)), responses((status = 200, body = Response<Empty>), (status = 403, description = "admins only")))]
async fn enable(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(email): ApiPath<String>,
) -> Result<Json<Response<Empty>>, AppError> {
    require_admin(&c)?;
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    reeve_identity::enable_user(
        &s.pool,
        &*s.tokens,
        &*log,
        c.policy(),
        &c.0.email,
        via_of(&c),
        &email,
        c.1,
    )
    .await?;
    Ok(ok(Empty {}))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(disable))
        .routes(routes!(enable))
}
