//! The MCP sign-in flow for tests, step by step through the real router (`static-oidc` only): authorize with a
//! client published on the fake client host, the approval page's three API calls, and the token route.
//! The harness is on a fixed clock: time moves only by `f.h.state.clock.advance`.
//!
//! Not reproduced: a browser. The connect cookie is copied from the `Set-Cookie` the server sent and the session
//! is an API token put in the cookie header by hand. Nothing here proves that a browser stores or sends either
//! cookie; `web/tests/smoke.spec.ts` does, on http loopback.
use super::{clients::client_harness, *};
use axum::{body::Body, http::Request};
use serde_json::Value;

/// The client's redirect address: listed by the instance and by the client's document.
pub const CALLBACK: &str = "https://client.example/callback";
/// The harness's issuer and the resource of its MCP endpoint.
pub const ISSUER: &str = "http://127.0.0.1:7474";
pub const RESOURCE: &str = "http://127.0.0.1:7474/mcp";
/// RFC 7636, appendix B: a verifier and its S256 challenge.
pub const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
pub const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

pub struct Flow {
    pub h: Harness,
    pub host: reeve_identity::FakeClientHost,
    /// The client id of "Example App": `http://127.0.0.1:<port>/client.json`.
    pub client_id: String,
}

/// The MCP harness, on a fixed clock, with `CALLBACK` as the one redirect address and a client that lists it.
pub async fn flow() -> Flow {
    flow_with(&[CALLBACK]).await
}

/// The same with the instance's redirect list as given; the client's document lists `CALLBACK` only.
pub async fn flow_with(redirect_uris: &[&str]) -> Flow {
    let (h, host) = client_harness(redirect_uris).await;
    let client_id = host.publish("/client.json", "Example App", &[CALLBACK]);
    Flow {
        h: h.with_fixed_clock(),
        host,
        client_id,
    }
}

/// The parameters of a valid authorize request for this flow's client.
pub fn authorize_params(f: &Flow) -> Vec<(&'static str, String)> {
    vec![
        ("response_type", "code".to_string()),
        ("client_id", f.client_id.clone()),
        ("redirect_uri", CALLBACK.to_string()),
        ("code_challenge", CHALLENGE.to_string()),
        ("code_challenge_method", "S256".to_string()),
        ("resource", RESOURCE.to_string()),
        ("scope", "read write".to_string()),
        ("state", "s-1".to_string()),
    ]
}

/// `params` with `name` set to `value` (replacing it), or removed when `value` is `None`.
pub fn with(
    mut params: Vec<(&'static str, String)>,
    name: &'static str,
    value: Option<&str>,
) -> Vec<(&'static str, String)> {
    params.retain(|(k, _)| *k != name);
    if let Some(value) = value {
        params.push((name, value.to_string()));
    }
    params
}

/// `params` with `name` sent a second time with `value`.
pub fn twice(
    mut params: Vec<(&'static str, String)>,
    name: &'static str,
    value: &str,
) -> Vec<(&'static str, String)> {
    params.push((name, value.to_string()));
    params
}

pub fn query_of(params: &[(&'static str, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish()
}

pub fn authorize_request(params: &[(&'static str, String)]) -> Request<Body> {
    Request::get(format!("/oauth/authorize?{}", query_of(params)))
        .body(Body::empty())
        .unwrap()
}

/// A request authorize accepted: its id (what the page's address carries) and the connect cookie exactly as the
/// server set it (`reeve_connect=<value>`, or the `__Host-` name on https).
pub struct Started {
    pub id: String,
    pub cookie: String,
}

/// A valid authorize request, which must be accepted.
pub async fn authorize(f: &Flow) -> Started {
    started(call_raw(&f.h.app, authorize_request(&authorize_params(f))).await)
}

/// What an accepted authorize answer carries; panics when it was not accepted.
pub fn started(res: axum::http::Response<Body>) -> Started {
    assert_eq!(res.status(), 303);
    let location = res.headers()["location"].to_str().unwrap().to_string();
    let id = location
        .strip_prefix("/connect?request=")
        .unwrap_or_else(|| panic!("authorize did not go to the page: {location}"))
        .to_string();
    let cookie = cookies_of(&res)
        .into_iter()
        .find(|c| c.contains("reeve_connect="))
        .expect("authorize sets the connect cookie");
    Started {
        id,
        cookie: cookie.split(';').next().unwrap().to_string(),
    }
}

/// A request as the page sends it: the client header, and the two cookies as given. `session` is an API token's
/// secret, sent under the http session cookie's name; `connect` is `Started::cookie`. Leave either out to send a
/// request without it.
pub fn page_request(
    method: &str,
    path: &str,
    session: Option<&str>,
    connect: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut cookies = vec![];
    if let Some(session) = session {
        cookies.push(format!("reeve_session={session}"));
    }
    if let Some(connect) = connect {
        cookies.push(connect.to_string());
    }
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("x-reeve-client", "web")
        .header("content-type", "application/json");
    if !cookies.is_empty() {
        req = req.header("cookie", cookies.join("; "));
    }
    req.body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
        .unwrap()
}

/// Sends a request; the status, the headers and the body as JSON (`Null` when it is empty or not JSON).
pub async fn answer_of(
    app: &axum::Router,
    req: Request<Body>,
) -> (u16, axum::http::HeaderMap, Value) {
    let (status, headers, body) = send(app, req).await;
    (
        status,
        headers,
        serde_json::from_str(&body).unwrap_or(Value::Null),
    )
}

/// Whether an answer may not be stored by any cache: what every answer of the sign-in routes must say.
pub fn is_no_store(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("cache-control")
        .is_some_and(|v| v == "no-store")
        && headers.get("pragma").is_some_and(|v| v == "no-cache")
}

pub fn request_path(s: &Started, action: &str) -> String {
    format!("/api/v1/connection-requests/{}{action}", s.id)
}

/// The page's read as `session` in the browser that started `s`.
pub async fn read(f: &Flow, session: &str, s: &Started) -> (u16, axum::http::HeaderMap, Value) {
    answer_of(
        &f.h.app,
        page_request(
            "GET",
            &request_path(s, ""),
            Some(session),
            Some(&s.cookie),
            None,
        ),
    )
    .await
}

/// The page's read, which must succeed: the approval value.
pub async fn approval(f: &Flow, session: &str, s: &Started) -> String {
    let (status, _, body) = read(f, session, s).await;
    assert_eq!(status, 200, "{body}");
    body["value"]["approval"].as_str().unwrap().to_string()
}

/// Approve as `session` with `scope` (`"read"` or `"write"`).
pub async fn approve(
    f: &Flow,
    session: &str,
    s: &Started,
    approval: &str,
    scope: &str,
) -> (u16, axum::http::HeaderMap, Value) {
    let body = serde_json::json!({"approval": approval, "scope": scope});
    answer_of(
        &f.h.app,
        page_request(
            "POST",
            &request_path(s, "/approve"),
            Some(session),
            Some(&s.cookie),
            Some(body),
        ),
    )
    .await
}

pub async fn refuse(
    f: &Flow,
    session: &str,
    s: &Started,
    approval: &str,
) -> (u16, axum::http::HeaderMap, Value) {
    let body = serde_json::json!({"approval": approval});
    answer_of(
        &f.h.app,
        page_request(
            "POST",
            &request_path(s, "/refuse"),
            Some(session),
            Some(&s.cookie),
            Some(body),
        ),
    )
    .await
}

/// Authorize, read and approve as `session`, each of which must succeed: the address the page would navigate to.
pub async fn approved(f: &Flow, session: &str, scope: &str) -> url::Url {
    let s = authorize(f).await;
    let value = approval(f, session, &s).await;
    let (status, _, body) = approve(f, session, &s, &value, scope).await;
    assert_eq!(status, 200, "{body}");
    url::Url::parse(body["value"]["redirect_to"].as_str().unwrap()).unwrap()
}

/// The one value of a query parameter of `url`; panics when it is sent twice.
pub fn param(url: &url::Url, name: &str) -> Option<String> {
    let mut values = url
        .query_pairs()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned());
    let first = values.next();
    assert!(values.next().is_none(), "{name} is sent twice in {url}");
    first
}

/// The address without its query: what must equal the client's redirect address.
pub fn without_query(url: &url::Url) -> String {
    let mut bare = url.clone();
    bare.set_query(None);
    bare.to_string()
}

/// The flow harness served as `https://reeve.example.com`, so cookies are `Secure` and `__Host-` prefixed (built
/// as `https_harness` is: from the loopback configuration, with `public_url` replaced afterwards).
pub async fn https_flow() -> Flow {
    let host = reeve_identity::FakeClientHost::start().await;
    let h = super::build(
        |cfg| {
            format!(
                "{cfg}\n[mcp]\nenabled = true\nredirect_uris = [\"{CALLBACK}\"]\nclient_hosts = [\"{}\"]\n",
                host.host()
            )
        },
        |c| c.public_url = Some("https://reeve.example.com".into()),
    )
    .await;
    let client_id = host.publish("/client.json", "Example App", &[CALLBACK]);
    Flow {
        h: h.with_fixed_clock(),
        host,
        client_id,
    }
}

/// `POST /oauth/token` with a form body.
pub fn token_request(form: &[(&str, &str)]) -> Request<Body> {
    raw_token_request(
        &url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish(),
    )
}

/// The same with the body as given: for what a form serialiser would never write.
pub fn raw_token_request(body: &str) -> Request<Body> {
    Request::post("/oauth/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub async fn token(f: &Flow, form: &[(&str, &str)]) -> (u16, axum::http::HeaderMap, Value) {
    answer_of(&f.h.app, token_request(form)).await
}

/// The form of a correct exchange of `code`, as an OAuth 2.0 client sends it (with `redirect_uri`) and with the
/// `resource` MCP asks clients to send.
pub fn code_form<'a>(f: &'a Flow, code: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", f.client_id.as_str()),
        ("code_verifier", VERIFIER),
        ("redirect_uri", CALLBACK),
        ("resource", RESOURCE),
    ]
}

/// A refresh as a client sends it: without `client_id`, which the standards make optional here.
pub fn refresh_form(refresh_token: &str) -> Vec<(&str, &str)> {
    vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ]
}

/// `form` with `name` set to `value` (replacing it), or removed when `value` is `None`.
pub fn form_with<'a>(
    mut form: Vec<(&'a str, &'a str)>,
    name: &'a str,
    value: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    form.retain(|(k, _)| *k != name);
    if let Some(value) = value {
        form.push((name, value));
    }
    form
}

/// An approved request's code: authorize, read and approve as `session`, each of which must succeed.
pub async fn code_for(f: &Flow, session: &str, scope: &str) -> String {
    param(&approved(f, session, scope).await, "code").expect("an approval answers a code")
}

/// What a client holds after connecting.
pub struct Connected {
    pub access_token: String,
    pub refresh_token: String,
    /// The connection's id, read from the database by the access token's hash.
    pub connection_id: i64,
}

/// The whole journey as `session` with `scope`, every step of which must succeed.
pub async fn connect(f: &Flow, session: &str, scope: &str) -> Connected {
    let code = code_for(f, session, scope).await;
    let (status, _, body) = token(f, &code_form(f, &code)).await;
    assert_eq!(status, 200, "{body}");
    let access_token = body["access_token"].as_str().unwrap().to_string();
    let connection_id: i64 = sqlx::query_scalar("SELECT connection_id FROM tokens WHERE hash = $1")
        .bind(reeve_identity::hash_secret(&access_token))
        .fetch_one(&f.h.state.pool)
        .await
        .unwrap();
    Connected {
        access_token,
        refresh_token: body["refresh_token"].as_str().unwrap().to_string(),
        connection_id,
    }
}

/// The status `/mcp` answers a `server/discover` carrying `access_token`: 200 when the token is accepted.
pub async fn mcp_status(f: &Flow, access_token: &str) -> u16 {
    send(
        &f.h.app,
        mcp_modern(access_token, 1, "server/discover", serde_json::json!({})),
    )
    .await
    .0
}

/// A person admitted by a database entry, not by the configured floor, with an interactive token: the one kind
/// of person a test can take access away from. Returns the token's secret.
pub async fn guest(h: &Harness) -> String {
    set_guest_allowed(h, true).await;
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = h.state.activity.clone();
    let policy = h.state.access.load(&h.state.identity).await.unwrap();
    reeve_identity::issue_token(
        &h.state.pool,
        &*h.state.tokens,
        &*log,
        &policy,
        "test",
        reeve_protocol::Via::Interactive,
        "guest@other.org",
        reeve_protocol::TokenKind::Interactive,
        "guest",
        None,
        h.started,
    )
    .await
    .unwrap()
    .secret
}

pub async fn set_guest_allowed(h: &Harness, allowed: bool) {
    let query = if allowed {
        sqlx::query("INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email', 'guest@other.org', 'dan@example.com', $1)")
            .bind(h.state.clock.now())
    } else {
        sqlx::query("DELETE FROM allowed_entries WHERE value = 'guest@other.org'")
    };
    query.execute(&h.state.pool).await.unwrap();
}

pub type Answered = tokio::task::JoinHandle<(u16, axum::http::HeaderMap, Value)>;

/// Starts a request against the router without waiting for its answer.
pub fn start(f: &Flow, req: Request<Body>) -> Answered {
    let app = f.h.app.clone();
    tokio::spawn(async move { answer_of(&app, req).await })
}

/// Waits until `n` backends of this test's database wait for a lock. Bounded: when the request under test never
/// waits for the lock the test holds, this panics instead of hanging.
pub async fn until_waiting(h: &Harness, n: i64) {
    let waiting = async {
        loop {
            let now: i64 = sqlx::query_scalar(
                "SELECT count(DISTINCT l.pid) FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid WHERE NOT l.granted AND a.datname = current_database()",
            )
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
            if now >= n {
                break;
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
        .await
        .expect("no request waited for the lock this test holds: the flow does not take it, or takes it later than the test assumes");
}

/// Whether another transaction holds a lock on the row `id` of `table`, asked without waiting.
pub async fn locked(h: &Harness, table: &str, id: i64) -> bool {
    let mut probe = h.state.pool.begin().await.unwrap();
    let got = sqlx::query(&format!(
        "SELECT 1 FROM {table} WHERE id = $1 FOR UPDATE NOWAIT"
    ))
    .bind(id)
    .fetch_optional(&mut *probe)
    .await;
    // Rolled back before answering: a probe that found the row free must not keep it locked.
    probe.rollback().await.unwrap();
    match got {
        Ok(Some(_)) => false,
        Ok(None) => panic!("{table} has no row {id}"),
        Err(e) => {
            let code = e
                .as_database_error()
                .and_then(|d| d.code())
                .map(|c| c.into_owned());
            assert_eq!(
                code.as_deref(),
                Some("55P03"),
                "lock_not_available expected: {e}"
            );
            true
        }
    }
}

/// The ids of the token row and the refresh token row a client holds.
pub async fn rows_of(h: &Harness, c: &Connected) -> (i64, i64) {
    let id = |sql: &'static str, secret: &str| {
        let (pool, hash) = (h.state.pool.clone(), reeve_identity::hash_secret(secret));
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(hash)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    (
        id("SELECT id FROM tokens WHERE hash = $1", &c.access_token).await,
        id(
            "SELECT id FROM refresh_tokens WHERE hash = $1",
            &c.refresh_token,
        )
        .await,
    )
}
