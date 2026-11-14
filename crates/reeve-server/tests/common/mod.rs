#![allow(dead_code)] // each test binary uses a different subset of the helpers
use reeve_server::*;
use testcontainers_modules::{postgres::Postgres, testcontainers::ContainerAsync};

/// The `[github] webhook_secret` of the standard harness.
pub const WEBHOOK_SECRET: &str = "0123456789abcdef0123";

pub struct Harness {
    pub db: reeve_testdb::TestDb,
    pub app: axum::Router,
    pub state: AppState,
    pub url: String,
    pub victim_token: String,
    pub victim_email: String,
    pub admin_token: String,
    pub bot_token: String,
}

/// The standard harness: loopback public URL and the static provider signing in as `dan@example.com`.
pub async fn harness() -> Harness {
    harness_with(|cfg| cfg).await
}

/// A harness whose configuration text is edited before it is parsed.
pub async fn harness_with(edit: impl FnOnce(String) -> String) -> Harness {
    build(edit, |_| {}).await
}

/// The standard harness served as `https://reeve.example.com`, so cookies are `Secure` and `__Host-` prefixed.
///
/// `ServerConfig::validate` refuses the static provider off loopback, and the Google provider would contact Google
/// in `build_state`, so neither can be configured through the TOML. Instead the state is built from the standard
/// (loopback, static) configuration and only then is its `public_url` replaced. This relaxes the test harness's
/// construction path alone; `validate` and `build_state` are unchanged.
pub async fn https_harness() -> Harness {
    build(
        |cfg| cfg,
        |c| c.public_url = Some("https://reeve.example.com".into()),
    )
    .await
}

/// The standard harness with its parsed configuration changed before the state is built.
pub async fn harness_patched(patch: impl FnOnce(&mut ServerConfig)) -> Harness {
    build(|cfg| cfg, patch).await
}

async fn build(
    edit: impl FnOnce(String) -> String,
    patch: impl FnOnce(&mut ServerConfig),
) -> Harness {
    build_on(
        reeve_testdb::database(&composed_migrator()).await,
        edit,
        patch,
    )
    .await
}

/// The standard harness on a PostgreSQL container of its own, returned next to it: for tests that control the
/// server itself (stop it, run commands inside it with `docker exec`).
pub async fn harness_in_own_container() -> (ContainerAsync<Postgres>, Harness) {
    let (c, db) = reeve_testdb::own_container(&composed_migrator()).await;
    (c, build_on(db, |cfg| cfg, |_| {}).await)
}

async fn build_on(
    db: reeve_testdb::TestDb,
    edit: impl FnOnce(String) -> String,
    patch: impl FnOnce(&mut ServerConfig),
) -> Harness {
    let url = db.url.clone();
    let pool = pool_options().connect(&url).await.unwrap();
    run_migrations(&pool).await.unwrap(); // idempotent: the database is already migrated
    let config = ServerConfig::parse(&edit(format!(
        r#"database_url = "{url}"
key_prefix = "TST"
public_url = "http://127.0.0.1:7474"
allowed_domains = ["example.com"]
admins = ["dan@example.com"]
[oidc]
provider = "static"
static_email = "dan@example.com"
[[repos]]
name = "orion"
adr_dir = "docs/adr"
[github]
webhook_secret = "{WEBHOOK_SECRET}"
repos = ["Acme/portal"]
legacy_prefixes = ["SCRUM"]"#
    )))
    .unwrap();
    let identity = config.identity();
    let policy = reeve_identity::AccessPolicy::floor_only(&identity);
    let tokens = reeve_identity::make_token_store(pool.clone());
    let now = chrono::Utc::now();
    let activity = reeve_activity::make_activity_store(pool.clone());
    let ident_log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = activity.clone();
    let admin_token = reeve_identity::issue_token(
        &pool,
        &*tokens,
        &*ident_log,
        &policy,
        "test",
        reeve_protocol::Via::Interactive,
        "dan@example.com",
        reeve_protocol::TokenKind::Interactive,
        "t",
        None,
        now,
    )
    .await
    .unwrap()
    .secret;
    let bot_token = reeve_identity::issue_token(
        &pool,
        &*tokens,
        &*ident_log,
        &policy,
        "test",
        reeve_protocol::Via::Interactive,
        "dan@example.com",
        reeve_protocol::TokenKind::Automation,
        "bot",
        None,
        now,
    )
    .await
    .unwrap()
    .secret;
    let victim_token = reeve_identity::issue_token(
        &pool,
        &*tokens,
        &*ident_log,
        &policy,
        "test",
        reeve_protocol::Via::Interactive,
        "bob@example.com",
        reeve_protocol::TokenKind::Interactive,
        "v",
        None,
        now,
    )
    .await
    .unwrap()
    .secret;
    let mut state = build_state(config, pool).await.unwrap();
    let mut config = (*state.config).clone();
    patch(&mut config);
    state.config = std::sync::Arc::new(config);
    let app = app(state.clone());
    Harness {
        db,
        app,
        state,
        url,
        victim_token,
        victim_email: "bob@example.com".into(),
        admin_token,
        bot_token,
    }
}

impl Harness {
    /// The router rebuilt from this harness's state with the client files read from `dir` (debug builds only).
    #[cfg(debug_assertions)]
    pub fn app_with_assets(&self, dir: &std::path::Path) -> axum::Router {
        let mut state = self.state.clone();
        state.assets = assets::Assets::Dir(dir.to_path_buf());
        app(state)
    }
}

pub async fn call(
    app: &axum::Router,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let mut req = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = req
        .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

/// Creates an epic under no initiative and marks its discovery complete, as the admin; returns its key. A story placed
/// under it is ready when its own checks pass (an item without an epic is untriaged, so most fixtures want one).
pub async fn ready_epic(h: &Harness, idem: &str) -> String {
    let (st, e) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/epics",
        Some(serde_json::json!({"idempotency_key": idem, "title": "Ready epic"})),
    )
    .await;
    assert_eq!(st, 200, "{e}");
    let key = e["value"]["key"].as_str().unwrap().to_string();
    let (st, d) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/epics/{key}/discovery"),
        Some(serde_json::json!({"version": e["value"]["version"], "complete": true})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    key
}

/// Sends a request as is and returns the raw response (status, headers, body untouched).
pub async fn call_raw(
    app: &axum::Router,
    req: axum::http::Request<axum::body::Body>,
) -> axum::http::Response<axum::body::Body> {
    use tower::ServiceExt;
    app.clone().oneshot(req).await.unwrap()
}

pub async fn body_json(res: axum::http::Response<axum::body::Body>) -> serde_json::Value {
    use http_body_util::BodyExt;
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Every `set-cookie` header value of a response.
pub fn cookies_of(res: &axum::http::Response<axum::body::Body>) -> Vec<String> {
    res.headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

/// An MCP token for `email`, issued on demand by the real `reeve_identity::issue_mcp_token` under the access policy
/// as a request would load it, expiring in one hour. Not issued in `build_on`: the standard harness's token and
/// activity counts stay as they were. This is a developer-minted token; a token issued through sign-in, with a
/// client and scopes, does not exist before GRA-658.
pub async fn issue_mcp_token(h: &Harness, email: &str) -> reeve_identity::IssuedToken {
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = h.state.activity.clone();
    let policy = h.state.access.load(&h.state.identity).await.unwrap();
    let now = chrono::Utc::now();
    reeve_identity::issue_mcp_token(
        &h.state.pool,
        &*h.state.tokens,
        &*log,
        &policy,
        "test",
        reeve_protocol::Via::Interactive,
        email,
        "mcp",
        now + chrono::Duration::hours(1),
        now,
    )
    .await
    .unwrap()
}
