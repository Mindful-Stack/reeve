use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::ApiQuery,
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_activity::{ActivityEntry, ActivityFilter};
use reeve_protocol::{ActivityEntryDto, Me, Response};
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ActivityQuery {
    pub entity_type: Option<String>,
    /// A literal match on the logged key: an instance key only (`ABC-12`, a decision id, an email). A Jira reference is
    /// not resolved, since an entry records the instance key.
    pub entity_key: Option<String>,
    pub action: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/activity", params(ActivityQuery), responses((status = 200, body = Response<Vec<ActivityEntryDto>>)))]
async fn list(
    State(s): State<AppState>,
    _c: Caller,
    ApiQuery(q): ApiQuery<ActivityQuery>,
) -> Result<Json<Response<Vec<ActivityEntryDto>>>, AppError> {
    if let Some(l) = q.limit
        && !(1..=500).contains(&l)
    {
        return Err(AppError::Validation(
            "limit must be between 1 and 500".into(),
        ));
    }
    let rows = s
        .activity
        .list(ActivityFilter {
            entity_type: q.entity_type,
            entity_key: q.entity_key,
            action: q.action,
            since: q.since,
            limit: q.limit,
            after_id: None,
        })
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    Ok(ok(rows.into_iter().map(to_dto).collect()))
}

/// The wire form of a stored entry; shared by the list and the event stream.
pub(crate) fn to_dto(e: ActivityEntry) -> ActivityEntryDto {
    ActivityEntryDto {
        id: e.id.expect("persisted entries have ids"),
        at: e.at,
        actor: e.actor,
        via: e.via,
        entity_type: e.entity_type,
        entity_key: e.entity_key,
        action: e.action,
        payload: e.payload,
    }
}
#[utoipa::path(get, path = "/me", responses((status = 200, body = Response<Me>)))]
async fn me(_s: State<AppState>, c: Caller) -> Result<Json<Response<Me>>, AppError> {
    Ok(ok(Me {
        email: c.0.email.clone(),
        role: c.0.role,
        token_kind: c.0.token_kind,
    }))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(me))
}
