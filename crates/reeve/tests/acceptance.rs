// crates/reeve/tests/acceptance.rs
//! The design's five-minute flow, driven through the real binaries against a real server and a real git repository.
use assert_cmd::Command;
use std::{path::Path, process::Command as Std};

fn git(dir: &Path, args: &[&str]) {
    let st = Std::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(st.success(), "git {args:?}");
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Std::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Multi-thread runtime: the CLI subprocesses block their calling thread while the in-process server must keep serving.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_minute_flow() {
    tokio::time::timeout(std::time::Duration::from_secs(300), flow())
        .await
        .expect("the flow must finish within five minutes");
}

async fn flow() {
    // 1. Database and server.
    let pg = reeve_testdb::database(&reeve_server::composed_migrator()).await;
    let url = pg.url.clone();
    let pool = pg.pool.clone();
    let config: reeve_server::ServerConfig = toml::from_str(&format!("database_url = \"{url}\"\nkey_prefix = \"ACC\"\nallowed_domains = [\"example.com\"]\nadmins = [\"dan@example.com\"]\n[[repos]]\nname = \"knowledge-base\"\nadr_dir = \"knowledge/adrs\"\n[[repos]]\nname = \"missing-repo\"\nadr_dir = \"adrs\"\n")).unwrap();
    let identity = config.identity();
    let tokens = reeve_identity::make_token_store(pool.clone());
    let now = chrono::Utc::now();
    let activity = reeve_activity::make_activity_store(pool.clone());
    let ident_log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = activity.clone();
    let human = reeve_identity::issue_token(
        &pool,
        &*tokens,
        &*ident_log,
        &reeve_identity::AccessPolicy::floor_only(&identity),
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
    let bot = reeve_identity::issue_token(
        &pool,
        &*tokens,
        &*ident_log,
        &reeve_identity::AccessPolicy::floor_only(&identity),
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
    // A member: allowlisted by domain, not in `admins`.
    let member = reeve_identity::issue_token(
        &pool,
        &*tokens,
        &*ident_log,
        &reeve_identity::AccessPolicy::floor_only(&identity),
        "test",
        reeve_protocol::Via::Interactive,
        "mia@example.com",
        reeve_protocol::TokenKind::Interactive,
        "m",
        None,
        now,
    )
    .await
    .unwrap()
    .secret;
    let app = reeve_server::app(reeve_server::build_state(config, pool).await.unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // 2. A household checkout with a knowledge-base repository that has an origin.
    let household = tempfile::tempdir().unwrap();
    let kb = household.path().join("knowledge-base");
    std::fs::create_dir_all(kb.join("knowledge/adrs")).unwrap();
    git(&kb, &["init", "-q", "-b", "main"]);
    git(&kb, &["config", "user.email", "t@example.com"]);
    git(&kb, &["config", "user.name", "t"]);
    std::fs::write(kb.join("README.md"), "kb\n").unwrap();
    git(&kb, &["add", "."]);
    git(&kb, &["commit", "-q", "-m", "init"]);
    let origin = tempfile::tempdir().unwrap();
    git(
        &kb,
        &[
            "clone",
            "-q",
            "--bare",
            ".",
            origin.path().to_str().unwrap(),
        ],
    );
    git(
        &kb,
        &["remote", "add", "origin", origin.path().to_str().unwrap()],
    );
    git(&kb, &["fetch", "-q", "origin"]);
    git(&kb, &["remote", "set-head", "origin", "-a"]);

    let cfg_dir = tempfile::tempdir().unwrap();
    let reeve = |token: &str| {
        let mut c = Command::cargo_bin("reeve").unwrap();
        c.env("REEVE_URL", &base)
            .env("REEVE_CONFIG_DIR", cfg_dir.path())
            .env("REEVE_TOKEN", token)
            .arg("--json")
            .timeout(std::time::Duration::from_secs(60));
        c
    };
    let ok = |out: std::process::Output| -> serde_json::Value {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };

    // --version identifies the planner (kb/ADR-0005 R1).
    let v = Command::cargo_bin("reeve")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&v.stdout).trim(),
        format!("reeve-planner {}", env!("CARGO_PKG_VERSION"))
    );

    // `auth login` stores a pasted token only after `GET /me` with that token says it is interactive. It reads stdin
    // (piped, so no browser opens) and ignores REEVE_TOKEN; its own config directory keeps the rest of the flow apart.
    let login_dir = tempfile::tempdir().unwrap();
    let login = |token: &str, args: &[&str]| {
        let mut c = Command::cargo_bin("reeve").unwrap();
        c.env("REEVE_CONFIG_DIR", login_dir.path())
            .env_remove("REEVE_URL")
            .env_remove("REEVE_TOKEN")
            .arg("--json")
            .args(args)
            .write_stdin(format!("{token}\n"))
            .timeout(std::time::Duration::from_secs(60));
        c.output().unwrap()
    };
    let signed_in = ok(login(&human, &["auth", "login", "--url", &base]));
    assert_eq!(
        signed_in["value"]["email"], "dan@example.com",
        "{signed_in}"
    );
    assert_eq!(signed_in["value"]["kind"], "interactive");
    // With REEVE_TOKEN exported the stored token is shadowed, so login says so on stderr (and still stores it).
    let shadowed = Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_CONFIG_DIR", login_dir.path())
        .env_remove("REEVE_URL")
        .env("REEVE_TOKEN", &bot)
        .args(["--json", "auth", "login", "--url", &base])
        .write_stdin(format!("{human}\n"))
        .timeout(std::time::Duration::from_secs(60))
        .output()
        .unwrap();
    assert!(shadowed.status.success());
    assert!(
        String::from_utf8_lossy(&shadowed.stderr).contains("REEVE_TOKEN is set"),
        "{}",
        String::from_utf8_lossy(&shadowed.stderr)
    );
    let refused = login(&bot, &["auth", "login", "--url", &base]);
    assert_eq!(
        refused.status.code(),
        Some(1),
        "login refuses an automation token"
    );
    let status = ok(login("", &["auth", "status"]));
    assert_eq!(
        status["value"]["token_kind"], "interactive",
        "a refused login leaves the stored token alone: {status}"
    );
    assert_eq!(status["value"]["email"], "dan@example.com");

    // 3. Hierarchy.
    let ini = ok(reeve(&human)
        .args([
            "initiative",
            "create",
            "--idempotency-key",
            "i1",
            "--title",
            "Orion",
            "--horizon",
            "Now",
        ])
        .output()
        .unwrap());
    let ini_key = ini["value"]["key"].as_str().unwrap().to_string();
    // Owner and target month are set, then cleared with the --no- flags.
    let set = ok(reeve(&human)
        .args([
            "initiative",
            "edit",
            &ini_key,
            "--version",
            "1",
            "--owner",
            "dan@example.com",
            "--target-month",
            "2027-03",
        ])
        .output()
        .unwrap());
    assert_eq!(set["value"]["owner"], "dan@example.com", "{set}");
    assert_eq!(set["value"]["target_month"], "2027-03", "{set}");
    // A stale version is a conflict whose envelope says the version to re-read from.
    let stale = reeve(&human)
        .args([
            "initiative",
            "edit",
            &ini_key,
            "--version",
            "1",
            "--no-owner",
        ])
        .output()
        .unwrap();
    assert_eq!(stale.status.code(), Some(1));
    let stale: serde_json::Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(
        (stale["code"].as_str(), &stale["current"]),
        (Some("conflict"), &set["value"]["version"]),
        "{stale}"
    );
    let cleared = ok(reeve(&human)
        .args([
            "initiative",
            "edit",
            &ini_key,
            "--version",
            &set["value"]["version"].to_string(),
            "--no-owner",
            "--no-target-month",
        ])
        .output()
        .unwrap());
    assert!(cleared["value"]["owner"].is_null(), "{cleared}");
    assert!(cleared["value"]["target_month"].is_null(), "{cleared}");
    let ini_version = cleared["value"]["version"].to_string();
    let epic = ok(reeve(&human)
        .args([
            "epic",
            "create",
            "--idempotency-key",
            "e1",
            "--title",
            "Orion v2",
            "--initiative",
            &ini_key,
        ])
        .output()
        .unwrap());
    let epic_key = epic["value"]["key"].as_str().unwrap().to_string();
    let story = ok(reeve(&human)
        .args([
            "story",
            "create",
            "--idempotency-key",
            "s1",
            "--title",
            "Booking flow",
            "--epic",
            &epic_key,
            "--criterion",
            "booking confirms in under 5s",
        ])
        .output()
        .unwrap());
    let key = story["value"]["key"].as_str().unwrap().to_string();

    // The duplicate check: a reworded title finds the story; a too short text is a failure (exit 1), not a refusal.
    let similar = reeve(&human)
        .args(["story", "similar", "Flow", "for", "booking"])
        .output()
        .unwrap();
    assert_eq!(similar.status.code(), Some(0), "{similar:?}");
    let hits: serde_json::Value = serde_json::from_slice(&similar.stdout).unwrap();
    assert_eq!(hits["value"][0]["key"], key.as_str(), "{hits}");
    let short = reeve(&human)
        .args(["story", "similar", "short"])
        .output()
        .unwrap();
    assert_eq!(short.status.code(), Some(1), "a too short text fails");
    let body: serde_json::Value = serde_json::from_slice(&short.stdout).unwrap();
    assert_eq!(body["code"], "validation", "{body}");
    ok(reeve(&human)
        .args([
            "initiative",
            "discovery",
            &ini_key,
            "--version",
            &ini_version,
            "--complete",
        ])
        .output()
        .unwrap());
    ok(reeve(&human)
        .args([
            "epic",
            "discovery",
            &epic_key,
            "--version",
            "1",
            "--complete",
        ])
        .output()
        .unwrap());
    let d = ok(reeve(&human)
        .args([
            "decision",
            "require",
            "--idempotency-key",
            "d1",
            "--on",
            &key,
            "--title",
            "Pick the queue",
            "--decider",
            "dan@example.com",
        ])
        .output()
        .unwrap());
    let did = d["value"]["id"].as_i64().unwrap().to_string();

    // 4. Refused with the reason "no record".
    let refused = reeve(&human)
        .args(["story", "mv", &key, "in-progress"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2), "refusal exits 2");
    let body: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(body["outcome"], "refused");
    assert_eq!(body["reasons"][0]["reason"], "decision_no_record");

    // 5. The agent drafts the record on a branch (a PR in real life) and links it: proposed (PR linked).
    std::fs::write(
        kb.join("knowledge/adrs/0001-queue.md"),
        "---\nid: kb/ADR-0001\nstatus: proposed\nsuperseded_by:\n---\n# Queue\n",
    )
    .unwrap();
    git(&kb, &["checkout", "-q", "-b", "adr/0001"]);
    git(&kb, &["add", "."]);
    git(&kb, &["commit", "-q", "-m", "propose"]);
    git(&kb, &["checkout", "-q", "main"]);
    // A PR url that is not http(s) is refused by the server, and the CLI reports the server's reason.
    let bad = reeve(&bot)
        .args([
            "decision",
            "link",
            &did,
            "--record",
            "kb/ADR-0001",
            "--repo",
            "knowledge-base",
            "--path",
            "knowledge/adrs/0001-queue.md",
            "--pr",
            "javascript:alert(1)",
            "--household",
            household.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!bad.status.success(), "a javascript: PR url is refused");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&bad.stdout),
        String::from_utf8_lossy(&bad.stderr)
    );
    assert!(said.contains("http(s)"), "{said}");
    ok(reeve(&bot)
        .args([
            "decision",
            "link",
            &did,
            "--record",
            "kb/ADR-0001",
            "--repo",
            "knowledge-base",
            "--path",
            "knowledge/adrs/0001-queue.md",
            "--pr",
            "https://example.com/pr/1",
            "--household",
            household.path().to_str().unwrap(),
        ])
        .output()
        .unwrap());
    let r = ok(reeve(&human)
        .args(["story", "ready", &key])
        .output()
        .unwrap());
    assert_eq!(
        r["value"]["reasons"][0]["reason"],
        "decision_proposed_pr_open"
    );

    // Automation may not override: the refusal names the rule, alongside the unmet readiness reason.
    let denied = reeve(&bot)
        .args(["story", "mv", &key, "in-progress", "--override", "demo"])
        .output()
        .unwrap();
    assert_eq!(denied.status.code(), Some(2));
    let denied: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(denied["outcome"], "refused");
    let names: Vec<&str> = denied["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["decision_proposed_pr_open", "automation_cannot_override"]
    );
    // A person may override, and the override is logged. The CLI has no activity command, so the log is read from the store.
    let moved = ok(reeve(&human)
        .args([
            "story",
            "mv",
            &key,
            "in-progress",
            "--override",
            "demo for the board",
        ])
        .output()
        .unwrap());
    assert_eq!(moved["value"]["delivery"], "in_progress");
    let overrides = reeve_activity::ActivityReader::list(
        &*activity,
        reeve_activity::ActivityFilter {
            action: Some("override".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(overrides.len(), 1, "only the person's override is logged");
    assert_eq!(overrides[0].entity_key, key);
    assert_eq!(overrides[0].payload["reason"], "demo for the board");
    ok(reeve(&human)
        .args(["story", "mv", &key, "todo"])
        .output()
        .unwrap());

    // Urgent marks through the CLI (kb/ADR-0017). The flow's own override is already logged, so the override count is
    // compared before and after; marks and clears are counted on the epic.
    let count = |action: &'static str, entity: Option<String>| {
        let activity = activity.clone();
        async move {
            reeve_activity::ActivityReader::list(
                &*activity,
                reeve_activity::ActivityFilter {
                    action: Some(action.into()),
                    entity_key: entity,
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .len()
        }
    };
    let plain = |token: &str| {
        let mut c = Command::cargo_bin("reeve").unwrap();
        c.env("REEVE_URL", &base)
            .env("REEVE_CONFIG_DIR", cfg_dir.path())
            .env("REEVE_TOKEN", token)
            .timeout(std::time::Duration::from_secs(60));
        c
    };
    let show = |token: &str| {
        let out = plain(token).args(["story", "show", &key]).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(
            text.lines().any(|l| l.contains(key.as_str())),
            "story show printed nothing useful: {text:?}"
        );
        text
    };
    let before_override = count("override", None).await;
    let before_marked = count("urgent_marked", Some(epic_key.clone())).await;
    let before_cleared = count("urgent_cleared", Some(epic_key.clone())).await;

    let marked = plain(&human)
        .args(["epic", "urgent", &epic_key, "--reason", "acceptance"])
        .output()
        .unwrap();
    assert!(
        marked.status.success(),
        "{}",
        String::from_utf8_lossy(&marked.stderr)
    );
    let shown = show(&human);
    let line = shown.lines().nth(1).unwrap_or_default();
    assert!(
        line.starts_with(&format!("  urgent · via epic {epic_key}")),
        "{shown}"
    );
    let room = ok(reeve(&human).args(["war-room", "ls"]).output().unwrap());
    let rows = room["value"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{room}");
    assert_eq!(rows[0]["item"]["key"], epic_key.as_str(), "{room}");
    let emptied = plain(&human)
        .args(["war-room", "clear", "--yes"])
        .output()
        .unwrap();
    assert!(
        emptied.status.success(),
        "{}",
        String::from_utf8_lossy(&emptied.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&emptied.stdout).trim(),
        "Cleared 1 mark.\nThe war room is empty."
    );
    let shown = show(&human);
    assert!(
        !shown.lines().any(|l| l.trim_start().starts_with("urgent")),
        "{shown}"
    );
    assert_eq!(
        count("urgent_marked", Some(epic_key.clone())).await,
        before_marked + 1
    );
    assert_eq!(
        count("urgent_cleared", Some(epic_key.clone())).await,
        before_cleared + 1
    );
    assert_eq!(
        count("override", None).await,
        before_override,
        "marking and clearing never write an override"
    );
    let refused = reeve(&bot)
        .args(["story", "urgent", &key])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    let refused: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(refused["outcome"], "refused");
    let names: Vec<&str> = refused["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reason"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["interactive_only"]);
    assert_eq!(
        count("urgent_marked", Some(key.clone())).await,
        0,
        "a refused mark writes nothing"
    );

    // 6. The record is accepted. Committed locally but not pushed, the default branch on origin still lacks it, so the
    // check must not see it: the story stays blocked. Pushed, the next check accepts it.
    git(&kb, &["merge", "-q", "adr/0001"]);
    std::fs::write(
        kb.join("knowledge/adrs/0001-queue.md"),
        "---\nid: kb/ADR-0001\nstatus: accepted\nsuperseded_by:\n---\n# Queue\n",
    )
    .unwrap();
    git(&kb, &["commit", "-q", "-am", "accept"]);
    let hh = household.path().to_str().unwrap();
    let local = ok(reeve(&bot)
        .args(["decision", "check", "--household", hh])
        .output()
        .unwrap());
    assert_eq!(local["value"]["checked"], 1);
    assert_eq!(
        local["value"]["results"][0]["check"]["kind"],
        "proposed_pr_open"
    );
    let blocked = ok(reeve(&human)
        .args(["story", "ready", &key])
        .output()
        .unwrap());
    assert_eq!(blocked["value"]["ready"], false);
    assert_eq!(
        blocked["value"]["reasons"][0]["reason"],
        "decision_proposed_pr_open"
    );
    let not_yet = ok(reeve(&bot)
        .args(["story", "ls", "--ready"])
        .output()
        .unwrap());
    assert!(
        !not_yet["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["story"]["key"] == key)
    );
    git(&kb, &["push", "-q", "origin", "main"]);
    // Run from inside the repository's own checkout with `--household .`: the checker resolves `.` before comparing
    // its basename with the repository name, so the current checkout is recognised as the repository.
    let pushed = ok(reeve(&bot)
        .current_dir(&kb)
        .args(["decision", "check", "--household", "."])
        .output()
        .unwrap());
    assert_eq!(pushed["value"]["checked"], 1);
    assert_eq!(pushed["value"]["unreadable"], 0);
    assert_eq!(pushed["value"]["results"][0]["check"]["kind"], "accepted");
    let origin_main = git_out(origin.path(), &["rev-parse", "main"]);
    assert_eq!(
        pushed["value"]["results"][0]["check"]["commit"],
        origin_main
    );
    let ready = ok(reeve(&human)
        .args(["story", "ready", &key])
        .output()
        .unwrap());
    assert_eq!(ready["value"]["ready"], true);
    let ls = ok(reeve(&bot)
        .args(["story", "ls", "--ready"])
        .output()
        .unwrap());
    assert!(
        ls["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["story"]["key"] == key)
    );
    ok(reeve(&bot)
        .args(["story", "mv", &key, "in-progress"])
        .output()
        .unwrap());
    // Work is under way: the agent comments, then the story is completed from active work (the second delivery rule).
    ok(reeve(&bot)
        .args([
            "comment",
            "add",
            "--idempotency-key",
            "c1",
            "--story",
            &key,
            "--body",
            "queue chosen, wiring it up",
        ])
        .output()
        .unwrap());
    // A retried comment with the same key adds nothing (kb/ADR-0005 R3).
    ok(reeve(&bot)
        .args([
            "comment",
            "add",
            "--idempotency-key",
            "c1",
            "--story",
            &key,
            "--body",
            "queue chosen, wiring it up",
        ])
        .output()
        .unwrap());
    let comments = ok(reeve(&bot)
        .args(["comment", "ls", "--story", &key])
        .output()
        .unwrap());
    assert_eq!(comments["value"].as_array().unwrap().len(), 1, "{comments}");
    assert!(
        comments["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["body"] == "queue chosen, wiring it up")
    );
    let done = ok(reeve(&bot)
        .args(["story", "mv", &key, "done", "--resolution", "completed"])
        .output()
        .unwrap());
    assert_eq!(done["value"]["delivery"], "done");
    assert_eq!(done["value"]["resolution"], "completed");

    // 7. The context bundle hands the accepted decision to an agent.
    let ctx = ok(reeve(&bot)
        .args(["story", "context", &key])
        .output()
        .unwrap());
    assert_eq!(
        ctx["value"]["required_decisions"][0]["check"]["kind"],
        "accepted"
    );
    assert_eq!(
        ctx["value"]["required_decisions"][0]["record"]["path"],
        "knowledge/adrs/0001-queue.md"
    );
    // Text mode must name the record file so an agent can open it from the Markdown.
    let mut text = Command::cargo_bin("reeve").unwrap();
    text.env("REEVE_URL", &base)
        .env("REEVE_CONFIG_DIR", cfg_dir.path())
        .env("REEVE_TOKEN", &human);
    let out = text.args(["story", "context", &key]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("knowledge/adrs/0001-queue.md"));

    // 7b. A screenshot on the story reaches the agent: listed in the context, downloaded byte for byte, privately.
    let png = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one.png"
    ))
    .unwrap();
    let http = reqwest::Client::new();
    let up: serde_json::Value = http
        .post(format!("{base}/api/v1/attachments?filename=one.png"))
        .bearer_auth(&bot)
        .header("content-type", "application/octet-stream")
        .body(png.clone())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = up["value"]["id"].as_str().unwrap().to_string();
    let claim = http
        .post(format!("{base}/api/v1/stories/{key}/comments"))
        .bearer_auth(&bot)
        .json(&serde_json::json!({"body": format!("![one](/api/v1/attachments/{id})"), "attachments": [id]}))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status(), 200);
    let ctx = ok(reeve(&bot)
        .args(["story", "context", &key])
        .output()
        .unwrap());
    assert_eq!(ctx["value"]["attachments"][0]["id"], id.as_str());
    assert_eq!(ctx["value"]["attachments"][0]["owner"]["item"], "story");
    let dir = tempfile::tempdir().unwrap();
    let got = ok(reeve(&bot)
        .args([
            "attachment",
            "get",
            &id,
            "--dir",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap());
    let path = dir.path().join(format!("{id}.png"));
    assert_eq!(got["value"]["path"], path.to_str().unwrap());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        png,
        "a clean PNG comes back unchanged"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let missing = reeve(&bot)
        .args([
            "attachment",
            "get",
            "00000000-0000-4000-8000-000000000000",
            "--dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(v["code"], "not_found");

    // 8. A configured repository with no clone in the household: the checker posts nothing, reports the decision
    // unreadable and exits 1, so a misconfigured host never records "not found".
    let d2 = ok(reeve(&human)
        .args([
            "decision",
            "require",
            "--idempotency-key",
            "d2",
            "--on",
            &ini_key,
            "--title",
            "Pick the region",
            "--decider",
            "dan@example.com",
        ])
        .output()
        .unwrap());
    let d2 = d2["value"]["id"].as_i64().unwrap().to_string();
    let linked = reeve(&human)
        .args([
            "decision",
            "link",
            &d2,
            "--record",
            "kb/ADR-0002",
            "--repo",
            "missing-repo",
            "--path",
            "adrs/0002-region.md",
            "--household",
            hh,
        ])
        .output()
        .unwrap();
    assert_eq!(
        linked.status.code(),
        Some(1),
        "the immediate check could not read the record"
    );
    let unread = reeve(&bot)
        .args(["decision", "check", "--household", hh, "--only", &d2])
        .output()
        .unwrap();
    assert_eq!(unread.status.code(), Some(1));
    let unread: serde_json::Value = serde_json::from_slice(&unread.stdout).unwrap();
    assert_eq!(unread["value"]["unreadable"], 1, "{unread}");
    assert_eq!(unread["value"]["checked"], 0);
    assert!(unread["value"]["results"][0]["check"].is_null(), "{unread}");
    let shown = ok(reeve(&human)
        .args(["decision", "show", &d2])
        .output()
        .unwrap());
    assert_eq!(
        shown["value"]["check"]["kind"], "unchecked",
        "nothing was posted"
    );

    // 9. A member links a record that is accepted on the default branch. The link succeeds and its immediate check runs
    // (kb/ADR-0004 R3), but only an admin's check stores accepted (kb/ADR-0018 R3): it is held, exit 0, and the story
    // waits for the scheduled run, which uses an admin's automation token. The scheduled script tells the two apart
    // through `auth status`; pin the field it reads.
    let status_of = |t: &str| {
        String::from_utf8(reeve(t).args(["auth", "status"]).output().unwrap().stdout).unwrap()
    };
    assert!(
        status_of(&bot).contains("\"role\":\"admin\""),
        "{}",
        status_of(&bot)
    );
    assert!(
        status_of(&member).contains("\"role\":\"member\""),
        "{}",
        status_of(&member)
    );
    // The chore sits under a discovery-complete epic: an item without an epic is untriaged, and this step is about the
    // decision's check, so the decision must be the chore's only reason.
    let broker_epic = ok(reeve(&member)
        .args([
            "epic",
            "create",
            "--idempotency-key",
            "m-e",
            "--title",
            "Messaging",
        ])
        .output()
        .unwrap());
    let broker_epic_key = broker_epic["value"]["key"].as_str().unwrap().to_string();
    ok(reeve(&human)
        .args([
            "epic",
            "discovery",
            &broker_epic_key,
            "--version",
            "1",
            "--complete",
        ])
        .output()
        .unwrap());
    let chore = ok(reeve(&member)
        .args([
            "story",
            "create",
            "--idempotency-key",
            "m1",
            "--title",
            "Wire the broker",
            "--kind",
            "chore",
            "--epic",
            &broker_epic_key,
            "--criterion",
            "messages arrive",
        ])
        .output()
        .unwrap());
    let chore_key = chore["value"]["key"].as_str().unwrap().to_string();
    let d3 = ok(reeve(&member)
        .args([
            "decision",
            "require",
            "--idempotency-key",
            "d3",
            "--on",
            &chore_key,
            "--title",
            "Pick the broker",
            "--decider",
            "mia@example.com",
        ])
        .output()
        .unwrap());
    let d3 = d3["value"]["id"].as_i64().unwrap().to_string();
    std::fs::write(
        kb.join("knowledge/adrs/0003-broker.md"),
        "---\nid: kb/ADR-0003\nstatus: accepted\nsuperseded_by:\n---\n# Broker\n",
    )
    .unwrap();
    git(&kb, &["add", "."]);
    git(&kb, &["commit", "-q", "-m", "broker"]);
    git(&kb, &["push", "-q", "origin", "main"]);
    let held = ok(reeve(&member)
        .args([
            "decision",
            "link",
            &d3,
            "--record",
            "kb/ADR-0003",
            "--repo",
            "knowledge-base",
            "--path",
            "knowledge/adrs/0003-broker.md",
            "--household",
            hh,
        ])
        .output()
        .unwrap());
    assert_eq!(held["immediate_check"]["outcome"], "held", "{held}");
    assert_eq!(
        held["immediate_check"]["check"]["kind"], "accepted",
        "{held}"
    );
    assert_eq!(
        held["value"]["check"]["kind"], "unchecked",
        "the server stored nothing: {held}"
    );
    let waiting = ok(reeve(&member)
        .args(["story", "ready", &chore_key])
        .output()
        .unwrap());
    assert_eq!(waiting["value"]["ready"], false);
    assert_eq!(
        waiting["value"]["reasons"][0]["reason"], "decision_unchecked",
        "{waiting}"
    );
    // The scheduled run's `--strict` against the real 403: a held result fails the run (exit 1) instead of passing it.
    let strict = reeve(&member)
        .args([
            "decision",
            "check",
            "--strict",
            "--household",
            hh,
            "--only",
            &d3,
        ])
        .output()
        .unwrap();
    assert_eq!(
        strict.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&strict.stderr)
    );
    let strict: serde_json::Value = serde_json::from_slice(&strict.stdout).unwrap();
    assert_eq!(strict["value"]["held"], 1, "{strict}");
    let stored = ok(reeve(&bot)
        .args(["decision", "check", "--household", hh, "--only", &d3])
        .output()
        .unwrap());
    assert_eq!(stored["value"]["checked"], 1, "{stored}");
    assert_eq!(stored["value"]["held"], 0, "{stored}");
    let ready = ok(reeve(&member)
        .args(["story", "ready", &chore_key])
        .output()
        .unwrap());
    assert_eq!(ready["value"]["ready"], true, "{ready}");

    // Export: the interactive admin writes the document (not the envelope) to a file; a member and an automation
    // token are refused by the server, which exits 2.
    let out_dir = tempfile::tempdir().unwrap();
    let file = out_dir.path().join("e.json");
    let exported = reeve(&human)
        .args(["export", "--out", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        exported.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(
        doc.get("outcome").is_none(),
        "the document, not the envelope"
    );
    assert_eq!(doc["key_prefix"], "ACC");
    assert!(
        doc["stories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["key"] == key.as_str()),
        "{doc}"
    );
    fn has_key(v: &serde_json::Value, name: &str) -> bool {
        match v {
            serde_json::Value::Object(o) => o.iter().any(|(k, x)| k == name || has_key(x, name)),
            serde_json::Value::Array(a) => a.iter().any(|x| has_key(x, name)),
            _ => false,
        }
    }
    assert!(!has_key(&doc, "hash"));
    let denied = reeve(&member).args(["export"]).output().unwrap();
    assert_eq!(denied.status.code(), Some(2), "a member is refused");
    let denied: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(denied["code"], "forbidden");
    let denied = reeve(&bot).args(["export"]).output().unwrap();
    assert_eq!(
        denied.status.code(),
        Some(2),
        "an automation token is refused"
    );

    // 10. Sprints and goals (kb/ADR-0014). Text-mode commands read stderr for the refusal's words.
    let refused_with = |out: std::process::Output, reason: &str| {
        assert_eq!(
            out.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["reasons"][0]["reason"], reason, "{v}");
    };
    let add_goal = [
        "goal",
        "add",
        "--idempotency-key",
        "g1",
        "--outcome",
        "Customers book sessions",
        "--check",
        "a customer books one",
    ];
    // Automation may not write goals (kb/ADR-0014 R5).
    refused_with(
        reeve(&bot).args(add_goal).output().unwrap(),
        "interactive_only",
    );
    let said = plain(&bot).args(add_goal).output().unwrap();
    assert_eq!(said.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&said.stderr).contains("only an interactive user may do this"),
        "{}",
        String::from_utf8_lossy(&said.stderr)
    );
    let goal = ok(reeve(&human).args(add_goal).output().unwrap());
    let gid = goal["value"]["goal"]["id"].as_i64().unwrap().to_string();
    let n = ok(reeve(&human).args(["sprint", "open"]).output().unwrap());
    assert_eq!(n["value"]["state"], "active", "{n}");
    let n_id = n["value"]["id"].as_i64().unwrap().to_string();
    // Work for the goal: story a linked directly, story b through its epic.
    let booking = ok(reeve(&human)
        .args([
            "epic",
            "create",
            "--idempotency-key",
            "e2",
            "--title",
            "Booking",
        ])
        .output()
        .unwrap());
    let booking = booking["value"]["key"].as_str().unwrap().to_string();
    let new_story = |idem: &str, title: &str, epic: &str| {
        let s = ok(reeve(&human)
            .args([
                "story",
                "create",
                "--idempotency-key",
                idem,
                "--title",
                title,
                "--epic",
                epic,
                "--criterion",
                "it works",
            ])
            .output()
            .unwrap());
        s["value"]["key"].as_str().unwrap().to_string()
    };
    let a = new_story("sa", "Book a slot", &epic_key);
    let b = new_story("sb", "Cancel a slot", &booking);
    ok(reeve(&bot)
        .args(["goal", "link", &gid, &a, &booking])
        .output()
        .unwrap());
    let serving = ok(reeve(&bot)
        .args(["story", "ls", "--goal", "any"])
        .output()
        .unwrap());
    let serving: Vec<&str> = serving["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["story"]["key"].as_str().unwrap())
        .collect();
    assert!(
        serving.contains(&a.as_str()) && serving.contains(&b.as_str()),
        "{serving:?}"
    );
    assert!(!serving.contains(&key.as_str()), "{serving:?}");
    let mut b_version = 0;
    for s in [&a, &b] {
        let moved = ok(reeve(&human)
            .args(["story", "mv", s, "in-progress", "--override", "sprint demo"])
            .output()
            .unwrap());
        b_version = moved["value"]["version"].as_i64().unwrap();
    }
    // Confidence is a person's; an agent suggests one, which the view shows beside it.
    refused_with(
        reeve(&bot)
            .args(["goal", "confidence", &gid, "at_risk"])
            .output()
            .unwrap(),
        "interactive_only",
    );
    let c = ok(reeve(&human)
        .args(["goal", "confidence", &gid, "at_risk", "slow start"])
        .output()
        .unwrap());
    assert_eq!(c["value"]["confidence"], "at_risk", "{c}");
    ok(reeve(&bot)
        .args(["goal", "suggest", &gid, "at_risk", "2 in review"])
        .output()
        .unwrap());
    let shown = ok(reeve(&bot).args(["sprint", "show"]).output().unwrap());
    let g = &shown["value"]["sprint"]["goals"][0];
    assert_eq!(
        (&g["suggested_confidence"], &g["suggested_note"]),
        (
            &serde_json::json!("at_risk"),
            &serde_json::json!("2 in review")
        ),
        "{shown}"
    );
    assert_eq!(
        shown["value"]["counts"]["goals"][0]["in_flight"], 2,
        "{shown}"
    );
    let text = plain(&bot).args(["sprint", "show"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&text.stdout).contains(
            "Counts the work completed in this sprint; work started but not finished shows as in flight."
        ),
        "{}",
        String::from_utf8_lossy(&text.stdout)
    );
    ok(reeve(&human)
        .args(["sprint", "close", "--verdict", &format!("{gid}=met")])
        .output()
        .unwrap());
    let ls = ok(reeve(&bot).args(["sprint", "ls"]).output().unwrap());
    let closed = &ls["value"]["closed"][0];
    assert_eq!(closed["state"], "closed", "{ls}");
    assert_eq!(closed["goals"][0]["verdict"], "met", "{ls}");
    let unchanged = ok(reeve(&bot)
        .args(["sprint", "show", &n_id])
        .output()
        .unwrap());
    assert_eq!(
        unchanged["value"]["changed_since_close"], false,
        "right after the close the derived counts are the counts at close: {unchanged}"
    );
    assert_eq!(
        unchanged["value"]["at_close"]["goals"][0]["blocked_decisions"], 1,
        "a closed sprint always carries its counts at close: {unchanged}"
    );
    // Every write to a closed sprint's goals is refused, a suggestion included.
    refused_with(
        reeve(&bot)
            .args(["goal", "suggest", &gid, "at_risk", "2 in review"])
            .output()
            .unwrap(),
        "sprint_closed",
    );
    let said = plain(&bot)
        .args(["goal", "suggest", &gid, "at_risk", "2 in review"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&said.stderr).contains("the sprint is closed"),
        "{}",
        String::from_utf8_lossy(&said.stderr)
    );

    // Two sprints: a completes in N+1, so it counts there (as upkeep: N+1 has no goals) and stays in flight in N. b moves
    // to an epic no goal of N covers: N's derived in-flight drops to 1, its counts at close keep 2 (kb/ADR-0014 R6).
    let n1 = ok(reeve(&human).args(["sprint", "open"]).output().unwrap());
    assert_eq!(n1["value"]["warnings"][0], "no_goals", "{n1}");
    ok(reeve(&human)
        .args(["story", "mv", &a, "done", "--resolution", "completed"])
        .output()
        .unwrap());
    let now = ok(reeve(&bot).args(["sprint", "show"]).output().unwrap());
    assert_eq!(
        (
            &now["value"]["counts"]["completed"],
            &now["value"]["counts"]["upkeep_completed"]
        ),
        (&serde_json::json!(1), &serde_json::json!(1)),
        "{now}"
    );
    // a completing after the close drops N's live blocked-by-decisions (its decision no longer blocks unfinished work),
    // which is no drift: not changed since close, no "(at close:", and the text shows the value at close from the entry.
    let live = ok(reeve(&bot)
        .args(["sprint", "show", &n_id])
        .output()
        .unwrap());
    assert_eq!(live["value"]["changed_since_close"], false, "{live}");
    assert_eq!(
        live["value"]["counts"]["goals"][0]["in_flight"], 2,
        "{live}"
    );
    assert_eq!(
        live["value"]["counts"]["goals"][0]["blocked_decisions"], 0,
        "the live value moved: {live}"
    );
    let text = plain(&bot)
        .args(["sprint", "show", &n_id])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(!text.contains("(at close:"), "{text}");
    assert!(text.contains("blocked by 1 decision at close"), "{text}");
    ok(reeve(&human)
        .args([
            "story",
            "edit",
            &b,
            "--version",
            &b_version.to_string(),
            "--epic",
            &epic_key,
        ])
        .output()
        .unwrap());
    let then = ok(reeve(&bot)
        .args(["sprint", "show", &n_id])
        .output()
        .unwrap());
    assert_eq!(
        then["value"]["counts"]["goals"][0]["in_flight"], 1,
        "{then}"
    );
    assert_eq!(
        then["value"]["at_close"]["goals"][0]["in_flight"], 2,
        "{then}"
    );
    assert_eq!(then["value"]["changed_since_close"], true, "{then}");
    let text = plain(&bot)
        .args(["sprint", "show", &n_id])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("in flight: 1 (at close: 2)"), "{text}");

    // 9. `story ls` filters on the server by text, kind, initiative, urgency and openness, and orders by relevance
    // (kb/ADR-0021). Every query below carries a text no earlier story matches, so the earlier rows stay out.
    let key_of = |v: serde_json::Value| v["value"]["key"].as_str().unwrap().to_string();
    let zi = key_of(ok(reeve(&human)
        .args([
            "initiative",
            "create",
            "--idempotency-key",
            "zi",
            "--title",
            "Savanna",
            "--horizon",
            "Now",
            "--external-ref",
            "ZOO-1",
        ])
        .output()
        .unwrap()));
    let ze = key_of(ok(reeve(&human)
        .args([
            "epic",
            "create",
            "--idempotency-key",
            "ze",
            "--title",
            "Grassland",
            "--initiative",
            &zi,
        ])
        .output()
        .unwrap()));
    let filed = |idem: &str, extra: &[&str]| {
        let mut args = vec!["story", "create", "--idempotency-key", idem];
        args.extend_from_slice(extra);
        key_of(ok(reeve(&human).args(&args).output().unwrap()))
    };
    let zebra_bug = filed(
        "zb",
        &["--title", "Zebra crossing", "--kind", "bug", "--epic", &ze],
    );
    let zebra_story = filed("zs", &["--title", "Zebra stripes"]);
    let zebra_done = filed("zd", &["--title", "Zebra herd"]);
    let started_ref = filed("zp", &["--title", "ZOO-7 started"]);
    ok(reeve(&human)
        .args([
            "story",
            "mv",
            &started_ref,
            "in-progress",
            "--override",
            "demo",
        ])
        .output()
        .unwrap());
    let mentions_ref = filed("zt", &["--title", "About ZOO-7"]);
    let exact_ref = filed("zx", &["--title", "Imported", "--external-ref", "ZOO-7"]);
    ok(reeve(&human)
        .args([
            "story",
            "mv",
            &zebra_done,
            "done",
            "--resolution",
            "dropped",
        ])
        .output()
        .unwrap());
    ok(reeve(&human)
        .args(["story", "urgent", &zebra_story])
        .output()
        .unwrap());
    let ls = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["story", "ls"];
        all.extend_from_slice(args);
        ok(reeve(&bot).args(&all).output().unwrap())["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["story"]["key"].as_str().unwrap().to_string())
            .collect()
    };
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    assert_eq!(
        sorted(ls(&["--text", "zebra"])),
        sorted(vec![
            zebra_bug.clone(),
            zebra_story.clone(),
            zebra_done.clone()
        ])
    );
    assert_eq!(
        sorted(ls(&["--text", "zebra", "--open"])),
        sorted(vec![zebra_bug.clone(), zebra_story.clone()])
    );
    assert_eq!(
        ls(&["--text", "zebra", "--kind", "bug"]),
        vec![zebra_bug.clone()]
    );
    assert_eq!(ls(&["--initiative", &zi]), vec![zebra_bug.clone()]);
    assert_eq!(ls(&["--initiative", "ZOO-1"]), vec![zebra_bug.clone()]);
    assert_eq!(
        ls(&["--text", "zebra", "--urgent"]),
        vec![zebra_story.clone()]
    );
    assert_eq!(
        ls(&["--text", "zoo-7"]),
        vec![mentions_ref.clone(), exact_ref.clone(), started_ref.clone()],
        "rank order by default"
    );
    assert_eq!(
        ls(&["--text", "zoo-7", "--sort", "relevance"]),
        vec![exact_ref, mentions_ref, started_ref],
        "the exact reference first, then the title matches in state order"
    );
    let no_text = reeve(&bot)
        .args(["story", "ls", "--sort", "relevance"])
        .output()
        .unwrap();
    assert_eq!(
        no_text.status.code(),
        Some(1),
        "relevance without text fails"
    );
    let body: serde_json::Value = serde_json::from_slice(&no_text.stdout).unwrap();
    assert_eq!(body["code"], "validation", "{body}");
}
