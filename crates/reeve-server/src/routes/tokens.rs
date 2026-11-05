use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_identity::{TokenSummary, Via};
use reeve_protocol::*;
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};

fn dto(t: TokenSummary) -> TokenDto {
    TokenDto {
        id: t.id,
        kind: t.kind,
        label: t.label,
        created_at: t.created_at,
        expires_at: t.expires_at,
        revoked_at: t.revoked_at,
    }
}

pub(super) fn via_of(c: &Caller) -> Via {
    if c.0.token_kind == TokenKind::Automation {
        Via::Automation
    } else {
        Via::Interactive
    }
}

#[utoipa::path(get, path = "/tokens", responses((status = 200, body = Response<Vec<TokenDto>>)))]
async fn list(
    State(s): State<AppState>,
    c: Caller,
) -> Result<Json<Response<Vec<TokenDto>>>, AppError> {
    let rows = reeve_identity::list_tokens(&*s.tokens, &c.0.email).await?;
    Ok(ok(rows.into_iter().map(dto).collect()))
}

#[utoipa::path(post, path = "/tokens", request_body = IssueTokenRequest, responses((status = 200, body = Response<IssuedTokenDto>), (status = 403, description = "automation tokens cannot issue tokens")))]
async fn issue(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<IssueTokenRequest>,
) -> Result<Json<Response<IssuedTokenDto>>, AppError> {
    if c.0.token_kind == TokenKind::Automation {
        return Err(AppError::Forbidden);
    }
    if b.label.chars().count() > 100 {
        return Err(AppError::Validation(
            "label must be at most 100 characters".into(),
        ));
    }
    let expires_at = match b.expires_in_days {
        None => None,
        Some(d) if (1..=365).contains(&d) => Some(c.1 + chrono::Duration::days(d.into())),
        Some(_) => {
            return Err(AppError::Validation(
                "expires_in_days must be between 1 and 365".into(),
            ));
        }
    };
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let issued = reeve_identity::issue_token(
        &s.pool,
        &*s.tokens,
        &*log,
        &s.identity,
        &c.0.email,
        Via::Interactive,
        &c.0.email,
        b.kind,
        &b.label,
        expires_at,
        c.1,
    )
    .await?;
    // Built from the write itself: nothing after the commit can fail and lose the one-time secret.
    Ok(ok(IssuedTokenDto {
        token: TokenDto {
            id: issued.id,
            kind: b.kind,
            label: b.label,
            created_at: c.1,
            expires_at,
            revoked_at: None,
        },
        secret: issued.secret,
    }))
}

#[utoipa::path(delete, path = "/tokens/{id}", params(("id" = i64, Path)), responses((status = 200, body = Response<Empty>)))]
async fn revoke(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<Empty>>, AppError> {
    let log: Arc<dyn reeve_identity::IdentityActivity> = s.activity.clone();
    let changed =
        reeve_identity::revoke_token(&s.pool, &*s.tokens, &*log, &c.0.email, via_of(&c), id, c.1)
            .await?;
    if !changed {
        return Err(AppError::NotFound);
    }
    Ok(ok(Empty {}))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, issue))
        .routes(routes!(revoke))
}
