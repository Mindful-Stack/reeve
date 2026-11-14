#![allow(dead_code)] // each test binary uses a different subset of the helpers
use reeve_server::*;
use testcontainers_modules::{postgres::Postgres, testcontainers::ContainerAsync};

#[cfg(feature = "static-oidc")]
pub mod clients;

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
    /// The instant this harness issued its tokens at, in whole microseconds: PostgreSQL keeps microseconds, so a
    /// time stored from this instant (or from a fixed clock started at it) reads back equal.
    pub started: chrono::DateTime<chrono::Utc>,
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
    // Whole microseconds, as PostgreSQL stores them: `started` and a clock fixed at it have no finer part.
    let now = chrono::SubsecRound::trunc_subsecs(chrono::Utc::now(), 6);
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
        started: now,
    }
}

impl Harness {
    /// This harness on a fixed clock that starts at `started` (whole microseconds) and moves only by
    /// `state.clock.advance`. The router is rebuilt from the changed state, so both share the clock.
    pub fn with_fixed_clock(mut self) -> Self {
        self.state.clock = Clock::fixed(self.started);
        self.app = app(self.state.clone());
        self
    }

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
    let now = h.state.clock.now();
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

/// The standard harness with `[mcp] enabled = true`, added to the configuration text so that parsing and validation
/// see it.
pub async fn mcp_harness() -> Harness {
    harness_with(|cfg| format!("{cfg}\n[mcp]\nenabled = true\n")).await
}

/// The `_meta` every 2026-07-28 request carries.
pub fn mcp_meta() -> serde_json::Value {
    serde_json::json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "reeve-tests", "version": "0"},
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// A 2026-07-28 request to `/mcp` as the specification's examples show one: the per-request `_meta` and the mirrored
/// headers. Written from the same reading of the specification as the server; `tests/mcp_conformance.rs` (the SDK's
/// client over a real port) is the check that the reading is right.
pub fn mcp_modern(
    token: &str,
    id: i64,
    method: &str,
    mut params: serde_json::Value,
) -> axum::http::Request<axum::body::Body> {
    params["_meta"] = mcp_meta();
    let mut req = axum::http::Request::post("/mcp")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", method);
    if let Some(name) = params.get("name").and_then(|n| n.as_str()) {
        req = req.header("mcp-name", name);
    }
    let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    req.body(axum::body::Body::from(body.to_string())).unwrap()
}

/// A 2025-11-25 message: no `_meta` version; the version header on everything but `initialize`. `id` `None` makes it
/// a notification.
pub fn mcp_legacy(
    token: &str,
    id: Option<i64>,
    method: &str,
    params: serde_json::Value,
) -> axum::http::Request<axum::body::Body> {
    let mut req = axum::http::Request::post("/mcp")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    if method != "initialize" {
        req = req.header("mcp-protocol-version", "2025-11-25");
    }
    let mut body = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
    if let Some(id) = id {
        body["id"] = serde_json::json!(id);
    }
    req.body(axum::body::Body::from(body.to_string())).unwrap()
}

/// Sends a request and returns the status, the headers and the body as text (empty for no body).
pub async fn send(
    app: &axum::Router,
    req: axum::http::Request<axum::body::Body>,
) -> (u16, axum::http::HeaderMap, String) {
    use http_body_util::BodyExt;
    let res = call_raw(app, req).await;
    let (status, headers) = (res.status().as_u16(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}
