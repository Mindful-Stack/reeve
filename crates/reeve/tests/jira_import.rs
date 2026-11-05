//! `reeve import jira` end to end: the real binary against an in-process server on a real database and
//! an in-test fake Jira serving the committed fixtures. Catches a run that writes during a rehearsal or while the switch
//! is off, logs overrides, is not re-runnable, drops provenance, an orphan epic or `owner_map`, or imports a partial
//! fetch.
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

const JIRA_EMAIL: &str = "jira@example.com";
const JIRA_TOKEN: &str = "jira-secret";
/// base64("jira@example.com:jira-secret")
const JIRA_AUTH: &str = "Basic amlyYUBleGFtcGxlLmNvbTpqaXJhLXNlY3JldA==";
const ADMIN: &str = "dan@example.com";

fn fixture(name: &str) -> Option<Value> {
    let path = format!("{}/tests/fixtures/jira/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).ok()?;
    Some(serde_json::from_str(&text).unwrap())
}

#[derive(Clone, Default)]
struct Fake {
    /// While set, the second issue page answers 500, on the first try and on the retry.
    fail_issue_page_2: Arc<AtomicBool>,
    /// Every request the fake received.
    requests: Arc<AtomicUsize>,
    /// The issue keys whose comments were requested, in order.
    comment_keys: Arc<Mutex<Vec<String>>>,
    /// While set, SCRUM-11 is served with this summary instead of the fixture's.
    scrum_11_summary: Arc<Mutex<Option<String>>>,
}

fn authorised(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(JIRA_AUTH)
}

async fn search(
    State(f): State<Fake>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    f.requests.fetch_add(1, Ordering::SeqCst);
    if !authorised(&headers) {
        return (StatusCode::UNAUTHORIZED, "bad credentials").into_response();
    }
    if q.contains_key("startAt") {
        return (StatusCode::BAD_REQUEST, "startAt is not supported here").into_response();
    }
    let jql = q.get("jql").cloned().unwrap_or_default();
    let kind = if jql.contains("JPD") {
        "ideas"
    } else if jql.contains("SCRUM") {
        "issues"
    } else {
        return (StatusCode::BAD_REQUEST, "unknown project").into_response();
    };
    let page = match q.get("nextPageToken").map(String::as_str) {
        None => 1,
        Some(t) if t == format!("{kind}-page-2") => 2,
        Some(_) => return (StatusCode::BAD_REQUEST, "bad token").into_response(),
    };
    if kind == "issues" && page == 2 && f.fail_issue_page_2.load(Ordering::SeqCst) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("Retry-After", "0")],
            "boom",
        )
            .into_response();
    }
    let mut body = fixture(&format!("{kind}_page{page}.json")).unwrap();
    if let Some(summary) = f.scrum_11_summary.lock().unwrap().clone() {
        for issue in body["issues"].as_array_mut().unwrap() {
            if issue["key"] == "SCRUM-11" {
                issue["fields"]["summary"] = json!(summary);
            }
        }
    }
    axum::Json(body).into_response()
}

async fn comments(State(f): State<Fake>, headers: HeaderMap, Path(key): Path<String>) -> Response {
    f.requests.fetch_add(1, Ordering::SeqCst);
    f.comment_keys.lock().unwrap().push(key.clone());
    if !authorised(&headers) {
        return (StatusCode::UNAUTHORIZED, "bad credentials").into_response();
    }
    let page = fixture(&format!("comments_{key}_page1.json"))
        .unwrap_or_else(|| json!({"startAt": 0, "maxResults": 100, "total": 0, "comments": []}));
    axum::Json(page).into_response()
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

fn server_config(db: &str, import: bool) -> reeve_server::ServerConfig {
    let mut text = format!(
        "database_url = \"{db}\"\nkey_prefix = \"REV\"\nallowed_domains = [\"example.com\"]\nadmins = [\"{ADMIN}\"]\n\
         horizon_buckets = [\"Now\", \"Next\", \"Upcoming\", \"Later\", \"Won't do\"]\n"
    );
    if import {
        text.push_str("[import]\nenabled = true\n");
    }
    toml::from_str(&text).unwrap()
}

/// One CLI run: exit code, stdout parsed as JSON (Null when it is not JSON), stdout and stderr as text.
struct Run {
    code: i32,
    json: Value,
    stdout: String,
    stderr: String,
}

struct Env {
    server: String,
    token: String,
    config_dir: std::path::PathBuf,
    mapping: std::path::PathBuf,
}

impl Env {
    async fn reeve(&self, server: &str, args: &[&str]) -> Run {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_reeve"));
        cmd.env("REEVE_URL", server)
            .env("REEVE_TOKEN", &self.token)
            .env("REEVE_CONFIG_DIR", &self.config_dir)
            .env("JIRA_TOKEN", JIRA_TOKEN)
            .arg("--json")
            .args(args)
            .kill_on_drop(true);
        let out = tokio::time::timeout(Duration::from_secs(120), cmd.output())
            .await
            .expect("the CLI must finish within two minutes")
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        Run {
            code: out.status.code().unwrap_or(-1),
            json: serde_json::from_str(stdout.trim()).unwrap_or(Value::Null),
            stdout,
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        }
    }
    async fn import(&self, server: &str, apply: bool) -> Run {
        let mapping = self.mapping.to_str().unwrap();
        let mut args = vec!["import", "jira", "--mapping", mapping];
        if apply {
            args.push("--apply");
        }
        self.reeve(server, &args).await
    }
    /// `GET /api/v1{path}` with the admin token; the envelope's value, or the whole envelope when it is not ok.
    async fn get(&self, path: &str) -> Value {
        let v: Value = reqwest::Client::new()
            .get(format!("{}/api/v1{path}", self.server))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["outcome"], "ok", "GET {path}: {v}");
        v["value"].clone()
    }
    async fn stories(&self) -> Vec<Value> {
        let r = self.reeve(&self.server, &["story", "ls"]).await;
        assert_eq!(r.code, 0, "{}", r.stderr);
        r.json["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["story"].clone())
            .collect()
    }
}

impl Env {
    /// The comments `story context` shows on a story.
    async fn comments(&self, key: &str) -> Value {
        let ctx = self.reeve(&self.server, &["story", "context", key]).await;
        assert_eq!(ctx.code, 0, "{}", ctx.stderr);
        ctx.json["value"]["comments"].clone()
    }
}

fn by_ref<'a>(items: &'a [Value], external_ref: &str) -> Option<&'a Value> {
    items.iter().find(|i| i["external_ref"] == external_ref)
}

/// Every warning line of a report, joined, for substring checks.
fn warning_lines(report: &Value) -> String {
    report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["lines"].as_array().unwrap().clone())
        .map(|l| l.as_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn import_jira_end_to_end() {
    tokio::time::timeout(Duration::from_secs(300), flow())
        .await
        .expect("the import flow must finish within five minutes");
}

async fn flow() {
    // A database, one server with the import switched on and one, on the same pool, without `[import]`.
    let pg = reeve_testdb::database(&reeve_server::composed_migrator()).await;
    let db = pg.url.clone();
    let pool = pg.pool.clone();
    let config = server_config(&db, true);
    let identity = config.identity();
    let tokens = reeve_identity::make_token_store(pool.clone());
    let activity = reeve_activity::make_activity_store(pool.clone());
    let ident_log: Arc<dyn reeve_identity::IdentityActivity> = activity.clone();
    let token = reeve_identity::issue_token(
        &pool,
        &*tokens,
        &*ident_log,
        &identity,
        "test",
        reeve_protocol::Via::Interactive,
        ADMIN,
        reeve_protocol::TokenKind::Interactive,
        "t",
        None,
        chrono::Utc::now(),
    )
    .await
    .unwrap()
    .secret;
    let on = serve(reeve_server::app(
        reeve_server::build_state(config, pool.clone())
            .await
            .unwrap(),
    ))
    .await;
    let off = serve(reeve_server::app(
        reeve_server::build_state(server_config(&db, false), pool.clone())
            .await
            .unwrap(),
    ))
    .await;

    // The fake Jira, and a mapping pointing at it.
    let fake = Fake::default();
    let jira = serve(
        Router::new()
            .route("/rest/api/3/search/jql", get(search))
            .route("/rest/api/3/issue/{key}/comment", get(comments))
            .with_state(fake.clone()),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let example = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../jira-import.example.toml"
    ))
    .unwrap();
    let mut mapping_text: String = example
        .lines()
        .map(|l| {
            if l.starts_with("site = ") {
                format!("site = \"{jira}\"")
            } else if l.starts_with("email = ") {
                format!("email = \"{JIRA_EMAIL}\"")
            } else if l.starts_with("idea_link_type = ") {
                "idea_link_type = \"Polaris work item link\"".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        mapping_text.trim_end().ends_with("= \"bob@example.com\""),
        "[owner_map] is the example's last table"
    );
    mapping_text.push_str("\n\"acc-carol\" = \"carol@example.com\"\n");
    let mapping = dir.path().join("jira-import.toml");
    std::fs::write(&mapping, mapping_text).unwrap();
    let env = Env {
        server: on.clone(),
        token,
        config_dir: dir.path().join("cli"),
        mapping,
    };

    // 1. A failed page, first, on the fresh database: the ideas and issue page 1 hold importable items, so an importer
    //    that wrote what it fetched before the failure would create rows here.
    fake.fail_issue_page_2.store(true, Ordering::SeqCst);
    let r = env.import(&on, true).await;
    assert_eq!(r.code, 1, "stdout {} stderr {}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("issues of SCRUM: search page 2") && r.stderr.contains("HTTP 500"),
        "{}",
        r.stderr
    );
    // The protocol's error envelope, with the page named in its extra fields.
    assert_eq!(
        (&r.json["outcome"], &r.json["code"]),
        (&json!("error"), &json!("internal")),
        "{}",
        r.json
    );
    assert_eq!(r.json["page"], "issues of SCRUM: search page 2");
    assert_eq!(r.json["status"], 500);
    assert_eq!(r.json["written"], false);
    assert!(env.stories().await.is_empty());
    assert_eq!(env.get("/initiatives").await, json!([]));
    assert_eq!(env.get("/epics").await, json!([]));
    assert_eq!(env.get("/activity?action=imported").await, json!([]));

    // 2. The switch off: refused before any Jira request.
    let before = fake.requests.load(Ordering::SeqCst);
    assert!(before > 0);
    let r = env.import(&off, true).await;
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(r.stderr.contains("[import] enabled"), "{}", r.stderr);
    assert_eq!(
        (&r.json["outcome"], &r.json["code"]),
        (&json!("error"), &json!("validation"))
    );
    assert_eq!(
        fake.requests.load(Ordering::SeqCst),
        before,
        "no Jira request"
    );

    // 3. Rehearsal: the report, and nothing written.
    fake.fail_issue_page_2.store(false, Ordering::SeqCst);
    fake.comment_keys.lock().unwrap().clear();
    let r = env.import(&on, false).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.json["outcome"], "ok", "{}", r.json);
    let report = &r.json["value"];
    let planned = &report["planned"];
    assert_eq!(
        (
            &planned["initiatives"],
            &planned["epics"],
            &planned["stories"],
            &planned["comments"]
        ),
        (&json!(2), &json!(3), &json!(6), &json!(1)),
        "{report}"
    );
    assert_eq!(
        planned["by_kind"],
        json!({"bug": 1, "chore": 1, "story": 4})
    );
    let warnings = warning_lines(report);
    for want in [
        "orphan: idea JPD-3 skipped (unmapped bucket",
        "orphan: no idea link",
        "skipped: story SCRUM-15 skipped",
    ] {
        assert!(warnings.contains(want), "{want:?} in {warnings}");
    }
    assert_eq!(
        report["link_types_seen"],
        json!({"Polaris work item link": 4, "Relates": 2})
    );
    assert_eq!(report["grandfathered_stories"], json!(["SCRUM-11"]));
    assert_eq!(report["marked_epics"], json!(["SCRUM-10"]));
    assert_eq!(report["marked_initiatives"], json!(["JPD-1"]));
    assert_eq!(report["written"], false);
    assert!(report["apply"].is_null());
    assert!(env.stories().await.is_empty());
    assert_eq!(env.get("/activity?action=imported").await, json!([]));
    // Comments are read for stories, bugs and tasks only, never for ideas or epics.
    assert_eq!(
        *fake.comment_keys.lock().unwrap(),
        [
            "SCRUM-11", "SCRUM-12", "SCRUM-13", "SCRUM-14", "SCRUM-15", "SCRUM-16", "SCRUM-17"
        ]
    );

    // 4. A create the server refuses: SCRUM-11 with a blank title fails validation, and its comment is never sent.
    *fake.scrum_11_summary.lock().unwrap() = Some("   ".into());
    let r = env.import(&on, true).await;
    assert_eq!(r.code, 1, "stdout {} stderr {}", r.stdout, r.stderr);
    let applied = &r.json["value"]["apply"];
    let failed = applied["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "{applied}");
    assert_eq!(
        (
            &failed[0]["jira_key"],
            &failed[0]["step"],
            &failed[0]["code"]
        ),
        (&json!("SCRUM-11"), &json!("story"), &json!("validation"))
    );
    assert_eq!(
        applied["skipped"],
        json!([["jira-comment:900", "skipped: parent SCRUM-11 failed"]])
    );
    assert_eq!(
        applied["created"],
        json!({"initiatives": 2, "epics": 3, "stories": 5, "comments": 0})
    );
    assert!(applied["stopped"].is_null());
    // One `imported` entry per create reached the server: none for SCRUM-11, none for its comment.
    let partial = env.get("/activity?action=imported&limit=500").await;
    assert_eq!(partial.as_array().unwrap().len(), 10, "{partial}");
    assert!(by_ref(&env.stories().await, "SCRUM-11").is_none());
    *fake.scrum_11_summary.lock().unwrap() = None;

    // 5. --apply: the rest created (the earlier run's items exist), with provenance.
    let r = env.import(&on, true).await;
    assert_eq!(r.code, 0, "stdout {} stderr {}", r.stdout, r.stderr);
    let applied = &r.json["value"]["apply"];
    assert_eq!(
        applied["created"],
        json!({"initiatives": 0, "epics": 0, "stories": 1, "comments": 1}),
        "{}",
        r.json
    );
    assert_eq!(
        applied["existing"],
        json!({"initiatives": 2, "epics": 3, "stories": 5, "comments": 0})
    );
    assert_eq!(applied["failed"], json!([]));
    assert_eq!(applied["skipped"], json!([]));
    assert_eq!(r.json["value"]["written"], true);

    let stories = env.stories().await;
    assert_eq!(stories.len(), 6);
    let story = |r: &str| {
        by_ref(&stories, r)
            .unwrap_or_else(|| panic!("{r} imported"))
            .clone()
    };
    let s11 = story("SCRUM-11");
    assert_eq!(s11["delivery"], "in_progress");
    assert!(
        s11["labels"]
            .as_array()
            .unwrap()
            .contains(&json!("legacy-import"))
    );
    assert_eq!(s11["external_ref"], "SCRUM-11");
    let s12 = story("SCRUM-12");
    assert_eq!(
        (&s12["delivery"], &s12["resolution"]),
        (&json!("done"), &json!("dropped"))
    );
    assert_eq!(
        story("SCRUM-14")["criteria"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["text"].as_str().unwrap())
            .take(2)
            .collect::<Vec<_>>(),
        ["A link is sent", "The link expires after a day"]
    );
    let s16 = story("SCRUM-16");
    assert_eq!(
        (&s16["delivery"], &s16["resolution"], &s16["owner"]),
        (
            &json!("done"),
            &json!("completed"),
            &json!("carol@example.com")
        )
    );
    assert_eq!(s16["kind"], "chore");
    let s17 = story("SCRUM-17");
    assert!(s17["owner"].is_null());
    assert!(by_ref(&stories, "SCRUM-15").is_none());

    let epics = env.get("/epics").await;
    let epics = epics.as_array().unwrap();
    let epic = |r: &str| {
        by_ref(epics, r)
            .unwrap_or_else(|| panic!("{r} imported"))
            .clone()
    };
    assert_eq!(s17["epic"], epic("SCRUM-20")["key"]);
    assert!(epic("SCRUM-20")["initiative"].is_null());
    assert!(epic("SCRUM-30")["initiative"].is_null());
    let initiatives = env.get("/initiatives").await;
    let initiatives = initiatives.as_array().unwrap();
    assert_eq!(initiatives.len(), 2);
    let jpd1 = by_ref(initiatives, "JPD-1").unwrap();
    let e10 = epic("SCRUM-10");
    assert_eq!(s11["epic"], e10["key"]);
    assert_eq!(e10["initiative"], jpd1["key"]);
    assert_eq!(e10["discovery_complete"]["by"], ADMIN);
    assert_eq!(jpd1["discovery_complete"]["by"], ADMIN);

    let key11 = s11["key"].as_str().unwrap();
    let context_comments = env.comments(key11).await;
    let comments = context_comments.as_array().unwrap().clone();
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert_eq!(comments[0]["external_ref"], "jira-comment:900");
    assert_eq!(comments[0]["author"], "Dana Example (dana@example.com)");
    assert_eq!(comments[0]["at"], "2026-09-01T10:15:30.123Z");
    assert_eq!(env.get("/activity?action=override").await, json!([]));
    assert_eq!(env.get("/stories/SCRUM-11").await["key"], key11);
    let imported = env.get("/activity?action=imported&limit=500").await;
    assert_eq!(
        imported.as_array().unwrap().len(),
        12,
        "one entry per created item"
    );

    // 6. --apply again, with SCRUM-11 renamed in Jira meanwhile: every item exists and nothing changes (the import
    //    is create-only; it never updates).
    *fake.scrum_11_summary.lock().unwrap() = Some("Renamed in Jira".into());
    let initiatives = env.get("/initiatives").await;
    let r = env.import(&on, true).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    let applied = &r.json["value"]["apply"];
    assert_eq!(
        applied["created"],
        json!({"initiatives": 0, "epics": 0, "stories": 0, "comments": 0})
    );
    assert_eq!(
        applied["existing"],
        json!({"initiatives": 2, "epics": 3, "stories": 6, "comments": 1})
    );
    assert_eq!(r.json["value"]["written"], false);
    let after = env.stories().await;
    assert_eq!(after, stories, "nothing changed");
    assert_eq!(by_ref(&after, "SCRUM-11").unwrap()["title"], s11["title"]);
    assert_ne!(s11["title"], "Renamed in Jira");
    assert_eq!(env.get("/epics").await.as_array().unwrap(), epics);
    assert_eq!(env.get("/initiatives").await, initiatives);
    assert_eq!(env.comments(key11).await, context_comments);
    assert_eq!(
        env.get("/activity?action=imported&limit=500").await,
        imported
    );
    assert_eq!(env.get("/activity?action=override").await, json!([]));
}
