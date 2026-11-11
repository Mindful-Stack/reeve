use crate::state::AppState;
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
pub mod access;
pub mod activity;
pub mod attachments;
pub mod comments;
pub mod config;
pub mod decisions;
pub mod epics;
pub mod events;
pub mod export;
pub mod import;
pub mod initiatives;
pub mod plans;
pub mod search;
pub mod sprints;
pub mod stories;
pub mod tokens;
pub mod users;
pub mod war_room;
pub mod webhooks;

/// An optional key input (a parent, a filter, `before`) resolved through `Handlers::resolve_ref`.
pub(crate) async fn resolve_opt(
    h: &reeve_planning::Handlers,
    r: Option<&reeve_protocol::KeyRef>,
    item: reeve_protocol::ItemType,
) -> Result<Option<reeve_protocol::Key>, reeve_planning::HandlerError> {
    match r {
        Some(r) => Ok(Some(h.resolve_ref(r.as_str(), item).await?)),
        None => Ok(None),
    }
}

/// A list filter naming an item. A filter is a query, not an address: a reference that matches nothing (an external
/// reference not imported yet) selects nothing, exactly as an unknown instance key does, so it is `Nothing` (200, empty
/// list) and not `NotFound`. Path parameters and body fields keep 404, where naming a missing item is an error.
pub(crate) enum FilterRef {
    Unset,
    Is(reeve_protocol::Key),
    Nothing,
}

pub(crate) async fn resolve_filter(
    h: &reeve_planning::Handlers,
    r: Option<&reeve_protocol::KeyRef>,
    item: reeve_protocol::ItemType,
) -> Result<FilterRef, reeve_planning::HandlerError> {
    match resolve_opt(h, r, item).await {
        Ok(Some(k)) => Ok(FilterRef::Is(k)),
        Ok(None) => Ok(FilterRef::Unset),
        Err(reeve_planning::HandlerError::NotFound) => Ok(FilterRef::Nothing),
        Err(e) => Err(e),
    }
}

/// An edit's parent: `None` leaves it alone, `Some(None)` clears it, `Some(Some(r))` resolves `r`.
pub(crate) async fn resolve_edit(
    h: &reeve_planning::Handlers,
    r: Option<Option<reeve_protocol::KeyRef>>,
    item: reeve_protocol::ItemType,
) -> Result<Option<Option<reeve_protocol::Key>>, reeve_planning::HandlerError> {
    match r {
        Some(r) => Ok(Some(resolve_opt(h, r.as_ref(), item).await?)),
        None => Ok(None),
    }
}

/// Declares the bearer scheme and applies it to every operation; shared error responses are attached the same way.
struct Security;
impl utoipa::Modify for Security {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::{
            Content, Ref, ResponseBuilder,
            security::{HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme},
        };
        let comps = openapi.components.get_or_insert_with(Default::default);
        comps.add_security_scheme(
            "bearer",
            SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
        );
        let envelope = |desc: &str| {
            ResponseBuilder::new()
                .description(desc)
                .content(
                    "application/json",
                    Content::new(Some(Ref::from_schema_name("Response_Empty"))),
                )
                .build()
        };
        for item in openapi.paths.paths.values_mut() {
            for op in [
                &mut item.get,
                &mut item.post,
                &mut item.put,
                &mut item.patch,
                &mut item.delete,
            ]
            .into_iter()
            .flatten()
            {
                op.security
                    .get_or_insert_with(Vec::new)
                    .push(SecurityRequirement::new::<_, _, &str>("bearer", []));
                for (code, desc) in [
                    ("400", "validation error"),
                    ("401", "unauthenticated"),
                    ("404", "not found"),
                    ("409", "version conflict"),
                    ("422", "refused with reasons"),
                ] {
                    op.responses
                        .responses
                        .entry(code.to_string())
                        .or_insert_with(|| envelope(desc).into());
                }
            }
        }
    }
}

// `Empty` is only ever a generic argument and `DoneFilter` and `StorySort` only query parameters; utoipa registers none
// of them on its own, so listing them here keeps every `$ref` resolvable.
#[derive(OpenApi)]
#[openapi(
    info(title = "Reeve planner API", version = "0.1.0"),
    components(schemas(
        reeve_protocol::Empty,
        reeve_protocol::DoneFilter,
        reeve_protocol::StorySort
    ))
)]
struct ApiDoc;

/// How long `GET /health` waits for the database before answering 503. Below `reeve-server healthcheck`'s own timeout
/// (`ops::HEALTHCHECK_TIMEOUT`, 5 s), so the client always gets an answer, and below compose.yml's healthcheck
/// `timeout` (10 s).
pub const HEALTH_DB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// `GET /health`, unauthenticated: 200 `{"ok":true}` when the database answers `SELECT 1` within `HEALTH_DB_TIMEOUT`,
/// 503 `{"ok":false}` otherwise. Outside `/api/v1`, so not in the OpenAPI document.
async fn health(State(s): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    let ping = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&s.pool);
    match tokio::time::timeout(HEALTH_DB_TIMEOUT, ping).await {
        Ok(Ok(1)) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"ok": false})),
        ),
    }
}

/// Nest under the prefix *before* splitting so the document's paths carry `/api/v1` and generated clients hit real routes.
fn api_router() -> OpenApiRouter<AppState> {
    let v1: OpenApiRouter<AppState> = OpenApiRouter::new()
        .merge(initiatives::router())
        .merge(epics::router())
        .merge(stories::router())
        .merge(decisions::router())
        .merge(comments::router())
        .merge(attachments::router())
        .merge(activity::router())
        .merge(events::router())
        .merge(tokens::router())
        .merge(users::router())
        .merge(access::router())
        .merge(config::router())
        .merge(import::router())
        .merge(export::router())
        .merge(plans::router())
        .merge(search::router())
        .merge(sprints::router())
        .merge(war_room::router());
    OpenApiRouter::with_openapi(ApiDoc::openapi()).nest("/api/v1", v1)
}
/// The served document: split from the merged routers, then the bearer scheme and shared responses are applied to every operation.
fn document() -> utoipa::openapi::OpenApi {
    let (_, mut o) = api_router().split_for_parts();
    utoipa::Modify::modify(&Security, &mut o);
    unique_operation_ids(&mut o);
    o
}
/// utoipa names an operation after its handler, and several modules have a `list` or `get`. Generated clients key operations
/// by id (openapi-typescript's `operations` interface), so a shared id silently gives one path another path's types. Each
/// shared id is prefixed with its path below `/api/v1` (`/stories/{key}` + `get` → `stories_key_get`; a path already ending in the
/// id is used as is).
fn unique_operation_ids(o: &mut utoipa::openapi::OpenApi) {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for item in o.paths.paths.values() {
        for op in item_ops(item) {
            if let Some(id) = &op.operation_id {
                *seen.entry(id.clone()).or_default() += 1;
            }
        }
    }
    for (path, item) in o.paths.paths.iter_mut() {
        let prefix = path
            .trim_start_matches("/api/v1/")
            .split('/')
            .map(|seg| seg.trim_start_matches('{').trim_end_matches('}'))
            .collect::<Vec<_>>()
            .join("_");
        for op in [
            &mut item.get,
            &mut item.post,
            &mut item.put,
            &mut item.patch,
            &mut item.delete,
        ]
        .into_iter()
        .flatten()
        {
            if let Some(id) = &mut op.operation_id
                && seen.get(id.as_str()).is_some_and(|n| *n > 1)
            {
                // `/epics/{key}/discovery` + `discovery` → `epics_key_discovery`, not `..._discovery_discovery`.
                *id = if prefix == *id || prefix.ends_with(&format!("_{id}")) {
                    prefix.clone()
                } else {
                    format!("{prefix}_{id}")
                };
            }
        }
    }
}
fn item_ops(
    item: &utoipa::openapi::path::PathItem,
) -> impl Iterator<Item = &utoipa::openapi::path::Operation> {
    [&item.get, &item.post, &item.put, &item.patch, &item.delete]
        .into_iter()
        .flatten()
}
pub fn app(state: AppState) -> Router {
    let (api, _) = api_router().split_for_parts();
    let openapi = document();
    Router::new()
        .route("/health", get(health))
        .merge(api)
        .route(
            "/api/openapi.json",
            get(move || async move { Json(openapi) }),
        )
        .nest("/auth", crate::auth::routes::router())
        .nest("/webhooks", webhooks::router())
        .fallback(crate::assets::serve)
        .method_not_allowed_fallback(|| async { crate::error::AppError::MethodNotAllowed })
        .with_state(state)
}
pub fn openapi_json() -> serde_json::Value {
    serde_json::to_value(document()).unwrap()
}
