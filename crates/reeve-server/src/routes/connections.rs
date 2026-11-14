//! `/connections`: the apps a person connected through MCP sign-in (kb/ADR-0029 R9). A person lists and revokes
//! their own; an admin lists and revokes anyone's (the record's later observation). Both routes work whether or
//! not `[mcp]` is on: a connection made while it was on stays visible and revocable.
use super::tokens::via_of;
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiPath, ApiQuery},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_identity::Connection;
use reeve_protocol::*;
use serde::Deserialize;
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ConnectionsQuery {
    /// Whose connections; the caller's own when absent. Another person's needs an admin.
    pub owner: Option<String>,
}

fn dto(c: Connection) -> ConnectionDto {
    ConnectionDto {
        client_host: c.client_host(),
        id: c.id,
        owner: c.owner,
        client_id: c.client_id,
        client_name: c.client_name,
        scope: c.scope,
        approved_at: c.approved_at,
        ends_at: c.ends_at,
        refreshed_at: c.refreshed_at,
        revoked_at: c.revoked.map(|(at, _)| at),
        revoked_reason: c.revoked.map(|(_, reason)| reason),
    }
}

#[utoipa::path(get, path = "/connections", params(ConnectionsQuery),
    responses((status = 200, body = Response<Vec<ConnectionDto>>), (status = 403, body = Response<Empty>, description = "another person's connections: admins only")))]
async fn list(
    State(s): State<AppState>,
    c: Caller,
    ApiQuery(q): ApiQuery<ConnectionsQuery>,
) -> Result<Json<Response<Vec<ConnectionDto>>>, AppError> {
    let rows = reeve_identity::list_connections(&*s.tokens, &c.0, q.owner.as_deref(), c.1).await?;
    Ok(ok(rows.into_iter().map(dto).collect()))
}

#[utoipa::path(delete, path = "/connections/{id}", params(("id" = i64, Path)), responses((status = 200, body = Response<Empty>)))]
async fn revoke(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<Empty>>, AppError> {
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let changed =
        reeve_identity::revoke_connection(&s.pool, &*s.tokens, &*log, &c.0, via_of(&c), id, c.1)
            .await?;
    if !changed {
        return Err(AppError::NotFound);
    }
    Ok(ok(Empty {}))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(revoke))
}
