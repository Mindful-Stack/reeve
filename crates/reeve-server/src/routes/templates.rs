//! `/brief-templates`: named Markdown texts per level from which a new brief can be prefilled. Every member lists
//! and reads them; only an admin creates, edits, removes and marks the default, with either kind of token (the check
//! is `users::require_admin`, made before anything is read). No MCP tool shares these routes (kb/ADR-0029 R10), so
//! they call the planning handlers directly, without an action. Handler names are unique across the API on purpose,
//! so their operation ids need no path prefix (`routes::unique_operation_ids`).
use super::users::require_admin;
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
};
use axum::{Json, extract::State};
use reeve_protocol::*;
use serde::Deserialize;
use utoipa_axum::{router::OpenApiRouter, routes};

/// The query of `GET /api/v1/brief-templates`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BriefTemplateQuery {
    /// Only this level's templates.
    pub level: Option<BriefTemplateLevel>,
}

/// Every brief template with its body, or one level's: initiative templates first, then by name.
#[utoipa::path(get, path = "/brief-templates", params(BriefTemplateQuery), responses((status = 200, body = Response<Vec<BriefTemplateDto>>)))]
async fn list_brief_templates(
    State(s): State<AppState>,
    _c: Caller,
    ApiQuery(q): ApiQuery<BriefTemplateQuery>,
) -> Result<Json<Response<Vec<BriefTemplateDto>>>, AppError> {
    Ok(ok(s.handlers.brief_templates(q.level).await?))
}

/// Creates a brief template: a name of 1 to 100 characters on one line, unique at its level whatever its case, and
/// a Markdown body that is not blank. Creating one that exists with the same body answers it and changes nothing;
/// with another body it is a validation error (edit it instead). It is not the default until it is marked.
#[utoipa::path(post, path = "/brief-templates", request_body = CreateBriefTemplate,
    responses((status = 200, body = Response<BriefTemplateDto>), (status = 403, description = "admins only")))]
async fn create_brief_template(
    State(s): State<AppState>,
    c: Caller,
    ApiJson(b): ApiJson<CreateBriefTemplate>,
) -> Result<Json<Response<BriefTemplateDto>>, AppError> {
    require_admin(&c)?;
    Ok(ok(s
        .handlers
        .create_brief_template(&c.ctx(), b.level, &b.name, &b.body)
        .await?))
}

#[utoipa::path(get, path = "/brief-templates/{id}", params(("id" = i64, Path)), responses((status = 200, body = Response<BriefTemplateDto>)))]
async fn get_brief_template(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<BriefTemplateDto>>, AppError> {
    Ok(ok(s.handlers.brief_template(id).await?))
}

/// Changes the name, the body or both, under the version last read (409 with the current one otherwise). Briefs
/// already made from the template do not change.
#[utoipa::path(patch, path = "/brief-templates/{id}", params(("id" = i64, Path)), request_body = EditBriefTemplate,
    responses((status = 200, body = Response<BriefTemplateDto>), (status = 403, description = "admins only")))]
async fn edit_brief_template(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
    ApiJson(b): ApiJson<EditBriefTemplate>,
) -> Result<Json<Response<BriefTemplateDto>>, AppError> {
    require_admin(&c)?;
    Ok(ok(s
        .handlers
        .edit_brief_template(&c.ctx(), id, b.version, b.name, b.body)
        .await?))
}

/// Removes the template. Briefs already made from it do not change; if it was its level's default, the level has
/// none.
#[utoipa::path(delete, path = "/brief-templates/{id}", params(("id" = i64, Path)),
    responses((status = 200, body = Response<Empty>), (status = 403, description = "admins only")))]
async fn remove_brief_template(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<Empty>>, AppError> {
    require_admin(&c)?;
    s.handlers.remove_brief_template(&c.ctx(), id).await?;
    Ok(ok(Empty {}))
}

/// Makes the template its level's default, taking the mark from the level's previous default. A create that asks
/// for the level's default (`default_template`) starts from it; a create that asks for nothing starts empty.
#[utoipa::path(post, path = "/brief-templates/{id}/default", params(("id" = i64, Path)),
    responses((status = 200, body = Response<BriefTemplateDto>), (status = 403, description = "admins only")))]
async fn set_default_brief_template(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<BriefTemplateDto>>, AppError> {
    require_admin(&c)?;
    Ok(ok(s
        .handlers
        .mark_brief_template_default(&c.ctx(), id, true)
        .await?))
}

/// Takes the default mark off the template; its level then has no default.
#[utoipa::path(delete, path = "/brief-templates/{id}/default", params(("id" = i64, Path)),
    responses((status = 200, body = Response<BriefTemplateDto>), (status = 403, description = "admins only")))]
async fn clear_default_brief_template(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Response<BriefTemplateDto>>, AppError> {
    require_admin(&c)?;
    Ok(ok(s
        .handlers
        .mark_brief_template_default(&c.ctx(), id, false)
        .await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_brief_templates, create_brief_template))
        .routes(routes!(
            get_brief_template,
            edit_brief_template,
            remove_brief_template
        ))
        .routes(routes!(
            set_default_brief_template,
            clear_default_brief_template
        ))
}
