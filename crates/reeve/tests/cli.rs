//! CLI tests that need no Docker: argument grammar and what the CLI puts on the wire.
use assert_cmd::Command;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

/// Accepts one connection within five seconds; a CLI that rejected its arguments never connects, so fail instead of hanging.
fn accept(listener: &TcpListener) -> std::net::TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "the CLI never connected (it rejected its arguments before sending a request?)"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("accept failed: {e}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
}

/// Reads one request: its request line and its body (by content-length).
fn read_request(stream: &std::net::TcpStream) -> (String, String) {
    let mut r = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        if h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body).unwrap();
    (line.trim().to_string(), String::from_utf8(body).unwrap())
}

/// Answers `answers` in order, one request per connection (every answer closes it). Each names the request it expects
/// (`POST /api/v1/decisions/7/check`); any other request is answered 500 and fails the test. Status 0 reads the request
/// and closes the connection without answering (a transport failure for the CLI). Yields the bodies sent.
fn serve(
    answers: Vec<(&'static str, u16, serde_json::Value)>,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    serve_lazy(
        answers
            .into_iter()
            .map(|(want, status, body)| (want, status, lazy(move || body)))
            .collect(),
    )
}

/// An answer body built when its request is handled, not when the test starts.
type Lazy = Box<dyn FnOnce() -> serde_json::Value + Send>;

fn lazy(f: impl FnOnce() -> serde_json::Value + Send + 'static) -> Lazy {
    Box::new(f)
}

/// `lazy`, built 2 ms after the request arrives: a mark it stamps is later than any clock the CLI read before sending
/// by whole milliseconds, which the expiry's millisecond arithmetic cannot round away, so a CLI that measures from such
/// a clock says "expires in 15 d" every run.
fn late(f: impl FnOnce() -> serde_json::Value + Send + 'static) -> Lazy {
    lazy(move || {
        std::thread::sleep(std::time::Duration::from_millis(2));
        f()
    })
}

/// `serve`, with each body built when its request arrives: a mark the fake server answers is timestamped as the real
/// server would stamp it, after the CLI sent its request.
fn serve_lazy(
    answers: Vec<(&'static str, u16, Lazy)>,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut bodies = vec![];
        for (want, status, body) in answers {
            let stream = accept(&listener);
            let (line, sent) = read_request(&stream);
            let expected = line.starts_with(&format!("{want} "));
            if expected && status == 0 {
                drop(stream);
                bodies.push(sent);
                continue;
            }
            let (status, body) = if expected {
                (status, body().to_string())
            } else {
                (
                    500,
                    r#"{"outcome":"error","code":"internal","message":"unexpected request"}"#
                        .to_string(),
                )
            };
            let mut w = stream;
            write!(
                w,
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            assert!(expected, "expected `{want}`, got `{line}`");
            bodies.push(sent);
        }
        bodies
    });
    (url, handle)
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A household with repository `kb` whose origin's default branch holds `adrs/0001.md`, accepted.
fn household() -> tempfile::TempDir {
    let hh = tempfile::tempdir().unwrap();
    household_with(hh.path(), "");
    hh
}

/// Makes `<dir>/kb` a clone (origin at `<dir>/origin.git`) whose default branch holds `adrs/0001.md`, accepted, with
/// `extra_front_matter` (whole lines, each ending in a newline) added to the record's front matter.
fn household_with(dir: &std::path::Path, extra_front_matter: &str) {
    let kb = dir.join("kb");
    let origin = dir.join("origin.git");
    std::fs::create_dir_all(kb.join("adrs")).unwrap();
    git(&kb, &["init", "-q", "-b", "main"]);
    git(&kb, &["config", "user.email", "t@example.com"]);
    git(&kb, &["config", "user.name", "t"]);
    std::fs::write(
        kb.join("adrs/0001.md"),
        format!(
            "---\nid: kb/ADR-0001\nstatus: accepted\nsuperseded_by:\n{extra_front_matter}---\n# Queue\n"
        ),
    )
    .unwrap();
    git(&kb, &["add", "."]);
    git(&kb, &["commit", "-q", "-m", "accept"]);
    git(
        &kb,
        &["clone", "-q", "--bare", ".", origin.to_str().unwrap()],
    );
    git(&kb, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&kb, &["fetch", "-q", "origin"]);
    git(&kb, &["remote", "set-head", "origin", "-a"]);
}

/// Decision 7 as the server returns it, linked to `kb:<path>`.
fn decision(path: &str, check: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"id":7,"on":"TST-1","on_type":"story","title":"Pick","decider":"dan@example.com","due":null,"reversibility":null,
        "record":{"repo":"kb","path":path,"record_id":"kb/ADR-0001"},"pr_url":null,"check":check,
        "checked_at":null,"closed":null,"created_at":"2026-10-26T09:00:00Z","version":2})
}

fn ok(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"outcome":"ok","value":value})
}

fn forbidden() -> serde_json::Value {
    serde_json::json!({"outcome":"error","code":"forbidden","message":"forbidden"})
}

fn reeve_at(url: &str, args: &[&str]) -> std::process::Output {
    let cfg = tempfile::tempdir().unwrap();
    Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", url)
        .env("REEVE_TOKEN", "reeve_test")
        .env("REEVE_CONFIG_DIR", cfg.path())
        .env_remove("REEVE_ASCII")
        .args(args)
        .timeout(Duration::from_secs(60))
        .output()
        .unwrap()
}

/// Serves one request with a `not_found` envelope and returns the request line and body the CLI sent.
fn capture(args: &[&str]) -> (String, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let stream = accept(&listener);
        let (line, body) = read_request(&stream);
        let resp = r#"{"outcome":"error","code":"not_found","message":"not found"}"#;
        let mut w = stream;
        write!(
            w,
            "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{resp}",
            resp.len()
        )
        .unwrap();
        (line, body)
    });
    let cfg = tempfile::tempdir().unwrap();
    Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", &url)
        .env("REEVE_TOKEN", "reeve_test")
        .env("REEVE_CONFIG_DIR", cfg.path())
        .env_remove("REEVE_ASCII")
        .args(args)
        .timeout(std::time::Duration::from_secs(30))
        .output()
        .unwrap();
    server.join().unwrap()
}

/// External references (`S2-4` is not `Key` grammar) reach the server unchanged, in paths and in bodies;
/// distinguishes a CLI that still parses `Key` and rejects them before contacting the server.
#[test]
fn key_arguments_accept_external_references_and_pass_them_through() {
    let (line, _) = capture(&["story", "show", "S2-4"]);
    assert!(line.contains("/stories/S2-4 "), "{line}");
    let (line, body) = capture(&[
        "story",
        "create",
        "--idempotency-key",
        "x",
        "--title",
        "T",
        "--epic",
        "SCRUM-10",
    ]);
    assert!(
        line.starts_with("POST ") && line.contains("/stories "),
        "{line}"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["epic"], "SCRUM-10");
    let (_, body) = capture(&[
        "decision",
        "require",
        "--idempotency-key",
        "d",
        "--on",
        "SCRUM-11",
        "--title",
        "Pick",
        "--decider",
        "dan@example.com",
    ]);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["on"], "SCRUM-11");
}

/// `story edit --kind` reaches the wire as `"kind":"bug"` in the PATCH body.
#[test]
fn story_edit_sends_kind() {
    let (line, body) = capture(&["story", "edit", "TST-1", "--version", "1", "--kind", "bug"]);
    assert!(line.starts_with("PATCH "), "{line}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["kind"], "bug", "{body}");
}

/// `story edit --description` reaches the wire, with no read first.
#[test]
fn story_edit_sends_description() {
    let (line, body) = capture(&[
        "story",
        "edit",
        "TST-1",
        "--version",
        "1",
        "--description",
        "Why it matters",
    ]);
    assert!(line.starts_with("PATCH "), "{line}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["description"], "Why it matters", "{body}");
    assert!(v.get("labels").is_none(), "{body}");
}

/// Labels are replaced as a list on the wire, so `--add-label` and `--remove-label` read the story's labels and send
/// them edited: order kept, a label already there not doubled, a removed one gone.
#[test]
fn story_edit_adds_and_removes_labels() {
    let story = serde_json::json!({
        "key": "TST-1", "title": "T", "kind": "story", "epic": null, "description": "", "criteria": [],
        "owner": null, "labels": ["migrated", "legacy-import"], "rank": 0, "blocked_by": [], "delivery": "todo",
        "resolution": null, "external_ref": null, "version": 4, "prs": []
    });
    let (url, server) = serve(vec![
        ("GET /api/v1/stories/TST-1", 200, ok(story.clone())),
        ("PATCH /api/v1/stories/TST-1", 200, ok(story)),
    ]);
    let out = reeve_at(
        &url,
        &[
            "story",
            "edit",
            "TST-1",
            "--version",
            "4",
            "--add-label",
            "needs-design",
            "--add-label",
            "migrated",
            "--remove-label",
            "legacy-import",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bodies = server.join().unwrap();
    let v: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(
        v["labels"],
        serde_json::json!(["migrated", "needs-design"]),
        "{}",
        bodies[1]
    );
    assert_eq!(v["version"], 4);
}

/// A 429 from the proxy is reported as `rate_limited`, never `internal`, so an agent waits and retries.
#[test]
fn a_rate_limit_is_reported_as_rate_limited() {
    let (url, server) = serve(vec![(
        "POST /api/v1/stories",
        429,
        serde_json::json!({"message": "slow down"}),
    )]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "story",
            "create",
            "--idempotency-key",
            "k",
            "--title",
            "T",
        ],
    );
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["outcome"], "error");
    assert_eq!(v["code"], "rate_limited", "{v}");
    assert!(v.get("retry_after_secs").is_none(), "{v}");
    let out = {
        let (url, server) = serve(vec![(
            "POST /api/v1/stories",
            429,
            serde_json::json!({"message": "slow down"}),
        )]);
        let o = reeve_at(
            &url,
            &["story", "create", "--idempotency-key", "k", "--title", "T"],
        );
        server.join().unwrap();
        o
    };
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("rate limited") && err.contains("retry the same command"),
        "{err}"
    );

    // With a Retry-After, --json carries it in seconds.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let stream = accept(&listener);
        read_request(&stream);
        let mut w = stream;
        write!(w, "HTTP/1.1 429 Too Many Requests\r\nretry-after: 12\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").unwrap();
    });
    let out = reeve_at(&url, &["--json", "story", "show", "TST-1"]);
    server.join().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["code"], "rate_limited", "{v}");
    assert_eq!(v["retry_after_secs"], 12, "{v}");
}

/// A string outside the loose grammar is still a usage error, before any request.
#[test]
fn a_malformed_key_argument_is_a_usage_error() {
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", "http://127.0.0.1:9")
        .env("REEVE_TOKEN", "reeve_test")
        .args(["story", "show", "scrum-11"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("invalid value"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The planning skills reclassify stories through `story edit --kind`.
#[test]
fn story_edit_lists_kind() {
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .args(["story", "edit", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("--kind <KIND>"), "{help}");
    assert!(help.contains("story, bug, chore"), "{help}");
}

const HELD_LINE: &str =
    "held: accepted on the default branch; stored by the next scheduled check run by an admin";

/// A 403 to an `accepted` post is `held`: counted apart, exit 0. Distinguishes a CLI that counts it as failed (exit 1)
/// and one that never posted what it read (the post body must carry the record's own status).
#[test]
fn decision_check_reports_a_refused_accepted_post_as_held() {
    let hh = household();
    let (url, server) = serve(vec![
        (
            "GET /api/v1/decisions?open_with_record=true",
            200,
            ok(serde_json::json!([decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"unchecked"})
            )])),
        ),
        ("POST /api/v1/decisions/7/check", 403, forbidden()),
    ]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "check",
            "--household",
            hh.path().to_str().unwrap(),
        ],
    );
    let bodies = server.join().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let posted: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(posted["check"]["kind"], "accepted", "{posted}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["outcome"], "ok", "{v}");
    assert_eq!(v["value"]["checked"], 0, "{v}");
    assert_eq!(v["value"]["held"], 1, "{v}");
    assert_eq!(v["value"]["unreadable"], 0, "{v}");
    assert_eq!(v["value"]["failed"], 0, "{v}");
    assert_eq!(v["value"]["results"][0]["outcome"], "held", "{v}");
    assert!(stderr.contains(HELD_LINE), "{stderr}");
}

/// R2: a real `decision check` against a record with `reversibility: one-way` posts it in the planner's own words.
/// Catches a checker that drops the field somewhere between the front matter and the post.
#[test]
fn decision_check_posts_the_records_reversibility() {
    let hh = tempfile::tempdir().unwrap();
    household_with(hh.path(), "reversibility: one-way\n");
    let (url, server) = serve(vec![
        (
            "GET /api/v1/decisions?open_with_record=true",
            200,
            ok(serde_json::json!([decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"unchecked"})
            )])),
        ),
        ("POST /api/v1/decisions/7/check", 403, forbidden()),
    ]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "check",
            "--household",
            hh.path().to_str().unwrap(),
        ],
    );
    let bodies = server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let posted: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(posted["check"]["kind"], "accepted", "{posted}");
    assert_eq!(posted["check"]["reversibility"], "one_way", "{posted}");
}

/// The text-mode summary counts the held result apart.
#[test]
fn decision_check_text_summary_counts_held() {
    let hh = household();
    let (url, server) = serve(vec![
        (
            "GET /api/v1/decisions?open_with_record=true",
            200,
            ok(serde_json::json!([decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"unchecked"})
            )])),
        ),
        ("POST /api/v1/decisions/7/check", 403, forbidden()),
    ]);
    let out = reeve_at(
        &url,
        &[
            "decision",
            "check",
            "--household",
            hh.path().to_str().unwrap(),
        ],
    );
    server.join().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("1 held"), "{stdout}");
}

/// A 403 to any other result is still `failed` and exits 1 (the record is absent: the CLI posts not_found).
#[test]
fn a_refused_not_found_post_is_still_failed() {
    let hh = household();
    let (url, server) = serve(vec![
        (
            "GET /api/v1/decisions?open_with_record=true",
            200,
            ok(serde_json::json!([decision(
                "adrs/0002.md",
                serde_json::json!({"kind":"unchecked"})
            )])),
        ),
        ("POST /api/v1/decisions/7/check", 403, forbidden()),
    ]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "check",
            "--household",
            hh.path().to_str().unwrap(),
        ],
    );
    let bodies = server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let posted: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(posted["check"]["kind"], "not_found", "{posted}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["value"]["held"], 0, "{v}");
    assert_eq!(v["value"]["failed"], 1, "{v}");
}

/// `decision link`: the link succeeded and the immediate check is held, so exit 0, the held line on stderr, and in JSON
/// `value` is the decision exactly as the server returned it (the shape skills read) with `immediate_check` beside it.
#[test]
fn decision_link_exits_0_when_its_immediate_check_is_held() {
    for json in [true, false] {
        let hh = household();
        let unchecked = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
        // The read-back answers a different version than the link did: `value` must be the read-back.
        let mut current = unchecked.clone();
        current["version"] = serde_json::json!(99);
        let (url, server) = serve(vec![
            ("POST /api/v1/decisions/7/link", 200, ok(unchecked.clone())),
            (
                "GET /api/v1/config",
                200,
                ok(
                    serde_json::json!({"key_prefix":"TST","horizon_buckets":["Now"],"repos":["kb"],"sign_in_configured":true}),
                ),
            ),
            ("POST /api/v1/decisions/7/check", 403, forbidden()),
            ("GET /api/v1/decisions/7", 200, ok(current.clone())),
        ]);
        let mut args = vec![
            "decision",
            "link",
            "7",
            "--record",
            "kb/ADR-0001",
            "--repo",
            "kb",
            "--path",
            "adrs/0001.md",
            "--household",
            hh.path().to_str().unwrap(),
        ];
        if json {
            args.insert(0, "--json");
        }
        let out = reeve_at(&url, &args);
        server.join().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(0), "json={json}: {stderr}");
        assert!(stderr.contains(HELD_LINE), "json={json}: {stderr}");
        if json {
            let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(v["outcome"], "ok", "{v}");
            assert_ne!(current, unchecked, "the fixture must tell the bodies apart");
            assert_eq!(
                v["value"], current,
                "value is the server's decision as read back"
            );
            assert_eq!(v["immediate_check"]["outcome"], "held", "{v}");
            assert_eq!(v["immediate_check"]["check"]["kind"], "accepted", "{v}");
        }
    }
}

/// A link whose immediate check is refused for another result still exits 1, with the entry saying failed.
#[test]
fn decision_link_exits_1_when_a_refused_check_is_not_accepted() {
    let hh = household();
    let unchecked = decision("adrs/0002.md", serde_json::json!({"kind":"unchecked"}));
    let (url, server) = serve(vec![
        ("POST /api/v1/decisions/7/link", 200, ok(unchecked.clone())),
        (
            "GET /api/v1/config",
            200,
            ok(
                serde_json::json!({"key_prefix":"TST","horizon_buckets":["Now"],"repos":["kb"],"sign_in_configured":true}),
            ),
        ),
        ("POST /api/v1/decisions/7/check", 403, forbidden()),
        ("GET /api/v1/decisions/7", 200, ok(unchecked)),
    ]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "link",
            "7",
            "--record",
            "kb/ADR-0001",
            "--repo",
            "kb",
            "--path",
            "adrs/0002.md",
            "--household",
            hh.path().to_str().unwrap(),
        ],
    );
    server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["immediate_check"]["outcome"], "failed", "{v}");
}

/// `--strict` (the scheduled run's mode): a held result is a failure. This is the run whose token was an admin's when it
/// started (the script checked `/me`) and is refused now. Distinguishes a strict flag that is parsed but ignored.
#[test]
fn a_strict_check_fails_on_a_held_result() {
    let hh = household();
    let (url, server) = serve(vec![
        (
            "GET /api/v1/decisions?open_with_record=true",
            200,
            ok(serde_json::json!([decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"unchecked"})
            )])),
        ),
        ("POST /api/v1/decisions/7/check", 403, forbidden()),
    ]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "check",
            "--strict",
            "--household",
            hh.path().to_str().unwrap(),
        ],
    );
    server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["outcome"], "error", "{v}");
    assert_eq!(v["value"]["held"], 1, "{v}");
}

/// The link and its immediate check have happened even when the read-back fails, by an error envelope or in transport:
/// the answer is still the ok envelope, `value` is the link's own response, `immediate_check` is there, `refresh_error`
/// says why, and the exit status is 1. Distinguishes a CLI that loses the result (`?` on the transport error, or
/// `finish` printing only the error envelope).
#[test]
fn decision_link_keeps_its_result_when_the_read_back_fails() {
    let linked = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    for (status, answer, says) in [
        (
            500,
            serde_json::json!({"outcome":"error","code":"internal","message":"read-back boom"}),
            "read-back boom",
        ),
        (0, serde_json::Value::Null, ""),
    ] {
        let hh = household();
        let (url, server) = serve(vec![
            ("POST /api/v1/decisions/7/link", 200, ok(linked.clone())),
            (
                "GET /api/v1/config",
                200,
                ok(
                    serde_json::json!({"key_prefix":"TST","horizon_buckets":["Now"],"repos":["kb"],"sign_in_configured":true}),
                ),
            ),
            ("POST /api/v1/decisions/7/check", 403, forbidden()),
            ("GET /api/v1/decisions/7", status, answer),
        ]);
        let out = reeve_at(
            &url,
            &[
                "--json",
                "decision",
                "link",
                "7",
                "--record",
                "kb/ADR-0001",
                "--repo",
                "kb",
                "--path",
                "adrs/0001.md",
                "--household",
                hh.path().to_str().unwrap(),
            ],
        );
        server.join().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "status {status}: {stderr}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["outcome"], "ok", "{v}");
        assert_eq!(v["value"], linked, "the link's own response: {v}");
        assert_eq!(v["immediate_check"]["outcome"], "held", "{v}");
        let e = v["refresh_error"]
            .as_str()
            .unwrap_or_else(|| panic!("no refresh_error: {v}"));
        assert!(e.contains(says), "{e}");
        assert!(stderr.contains("could not be read back"), "{stderr}");
    }
}

/// The scheduled layout with a household named like its repository: `<root>/kb` is the household and `<root>/kb/kb`
/// the clone. The check must read the clone and post what it found (today it takes the household itself, finds no
/// clone there, posts nothing, and the fake never sees the check: "the CLI never connected").
#[test]
fn a_household_named_like_its_repository_reads_the_clone_inside_it() {
    let root = tempfile::tempdir().unwrap();
    let hh = root.path().join("kb");
    std::fs::create_dir_all(&hh).unwrap();
    household_with(&hh, "");
    let (url, server) = serve(vec![
        (
            "GET /api/v1/decisions?open_with_record=true",
            200,
            ok(serde_json::json!([decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"unchecked"})
            )])),
        ),
        (
            "POST /api/v1/decisions/7/check",
            200,
            ok(decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"accepted","commit":"c"}),
            )),
        ),
    ]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "check",
            "--household",
            hh.to_str().unwrap(),
        ],
    );
    let bodies = server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let posted: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(
        posted["check"]["kind"], "accepted",
        "the record was read from the clone: {posted}"
    );
}

/// REEVE_TOKEN is for the planner: the git the check runs (and the ssh, credential helper or lazy fetch git starts)
/// never has it in its environment. A `git` wrapper first on PATH records what it was given.
#[cfg(unix)]
#[test]
fn the_check_runs_git_without_the_token() {
    use std::os::unix::fs::PermissionsExt;
    let hh = household();
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("git-env");
    let real = String::from_utf8(
        std::process::Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let wrapper = bin.path().join("git");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\necho \"token=${{REEVE_TOKEN+set}}\" >>'{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (url, server) = serve(vec![
        (
            "GET /api/v1/decisions?open_with_record=true",
            200,
            ok(serde_json::json!([decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"unchecked"})
            )])),
        ),
        ("POST /api/v1/decisions/7/check", 403, forbidden()),
    ]);
    let cfg = tempfile::tempdir().unwrap();
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .env("PATH", path)
        .env("REEVE_URL", &url)
        .env("REEVE_TOKEN", "reeve_test")
        .env("REEVE_CONFIG_DIR", cfg.path())
        .args(["decision", "check", "--household"])
        .arg(hh.path())
        .timeout(Duration::from_secs(60))
        .output()
        .unwrap();
    server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(seen.lines().count() >= 3, "the check ran git: {seen}");
    assert!(
        seen.lines().all(|l| l == "token="),
        "git saw the token: {seen}"
    );
}

fn conflict() -> serde_json::Value {
    serde_json::json!({"outcome":"error","code":"conflict","message":"stale version"})
}

/// Runs `reeve --json decision check --strict` against `answers` and returns (exit status, stdout JSON, stderr).
fn strict_check(
    answers: Vec<(&'static str, u16, serde_json::Value)>,
) -> (Option<i32>, serde_json::Value, String) {
    let hh = household();
    let (url, server) = serve(answers);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "check",
            "--strict",
            "--household",
            hh.path().to_str().unwrap(),
        ],
    );
    server.join().unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (
        out.status.code(),
        v,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A decision relinked or closed between the run's list and its post is refused (409, or 400 `validation` "decision is
/// closed").
/// The refusal is right and the next run checks the new state, so the scheduled `--strict` run reports it `skipped` and
/// stays green; a red unit for it would teach operators to ignore red.
#[test]
fn a_decision_that_moved_on_during_the_run_is_skipped_not_failed() {
    let list = || {
        (
            "GET /api/v1/decisions?open_with_record=true",
            200,
            ok(serde_json::json!([decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"unchecked"})
            )])),
        )
    };
    let mut relinked = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    relinked["version"] = 3.into();
    let mut closed = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    closed["version"] = 3.into();
    closed["closed"] =
        serde_json::json!({"at":"2026-10-26T09:05:00Z","by":"mia@example.com","reason":"moot"});
    let closed_refusal =
        serde_json::json!({"outcome":"error","code":"validation","message":"decision is closed"});
    for (name, refusal, now) in [
        ("relinked", (409, conflict()), relinked),
        ("closed", (400, closed_refusal), closed),
    ] {
        let (code, v, stderr) = strict_check(vec![
            list(),
            ("POST /api/v1/decisions/7/check", refusal.0, refusal.1),
            ("GET /api/v1/decisions/7", 200, ok(now)),
        ]);
        assert_eq!(code, Some(0), "{name}: {v} {stderr}");
        assert_eq!(v["outcome"], "ok", "{name}: {v}");
        assert_eq!(v["value"]["skipped"], 1, "{name}: {v}");
        assert_eq!(v["value"]["failed"], 0, "{name}: {v}");
        assert_eq!(
            v["value"]["results"][0]["outcome"], "skipped",
            "{name}: {v}"
        );
        assert!(stderr.contains("next run"), "{name}: {stderr}");
    }
}

/// A refusal for a decision that has not moved on is still `failed` (exit 1): only a decision the server shows as
/// changed is skipped, and other errors are not looked at.
#[test]
fn a_refused_post_for_an_unchanged_decision_is_still_failed() {
    let list = (
        "GET /api/v1/decisions?open_with_record=true",
        200,
        ok(serde_json::json!([decision(
            "adrs/0001.md",
            serde_json::json!({"kind":"unchecked"})
        )])),
    );
    let (code, v, _) = strict_check(vec![
        list.clone(),
        ("POST /api/v1/decisions/7/check", 409, conflict()),
        (
            "GET /api/v1/decisions/7",
            200,
            ok(decision(
                "adrs/0001.md",
                serde_json::json!({"kind":"unchecked"}),
            )),
        ),
    ]);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["value"]["failed"], 1, "{v}");
    // A 422 for a decision that is still open is not explained by another version: a relink resets the check and
    // keeps a reference, so only a close makes a valid post invalid. It stays failed.
    let mut bumped = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    bumped["version"] = 3.into();
    let (code, v, _) = strict_check(vec![
        list.clone(),
        (
            "POST /api/v1/decisions/7/check",
            422,
            serde_json::json!({"outcome":"error","code":"validation","message":"bad status"}),
        ),
        ("GET /api/v1/decisions/7", 200, ok(bumped)),
    ]);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["value"]["failed"], 1, "{v}");
    assert_eq!(v["value"]["skipped"], 0, "{v}");
    let internal =
        serde_json::json!({"outcome":"error","code":"internal","message":"database down"});
    let (code, v, _) = strict_check(vec![
        list,
        ("POST /api/v1/decisions/7/check", 500, internal),
    ]);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["value"]["failed"], 1, "{v}");
    assert_eq!(v["value"]["skipped"], 0, "{v}");
}

/// `story unblock` is a DELETE of the edge with both keys in the path as given (an external reference passes through)
/// and no body; ok exits 0 and --json prints the envelope; forbidden exits 2; not found exits 1.
#[test]
fn story_unblock_deletes_the_edge() {
    let (url, server) = serve(vec![(
        "DELETE /api/v1/stories/TST-2/blocks/SCRUM-11",
        200,
        ok(serde_json::json!({})),
    )]);
    let out = reeve_at(
        &url,
        &["--json", "story", "unblock", "TST-2", "--by", "SCRUM-11"],
    );
    assert_eq!(server.join().unwrap(), vec![String::new()], "no body");
    assert_eq!(out.status.code(), Some(0));
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(printed["outcome"], "ok");
    let (url, server) = serve(vec![(
        "DELETE /api/v1/stories/TST-2/blocks/TST-3",
        200,
        ok(serde_json::json!({})),
    )]);
    let out = reeve_at(&url, &["story", "unblock", "TST-2", "--by", "TST-3"]);
    server.join().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "unblocked",
        "the text output"
    );
    let not_found = serde_json::json!({"outcome":"error","code":"not_found","message":"not found"});
    for (status, body, code) in [(403, forbidden(), 2), (404, not_found, 1)] {
        let (url, server) = serve(vec![(
            "DELETE /api/v1/stories/TST-2/blocks/TST-3",
            status,
            body,
        )]);
        let out = reeve_at(&url, &["story", "unblock", "TST-2", "--by", "TST-3"]);
        server.join().unwrap();
        assert_eq!(out.status.code(), Some(code), "{status}");
    }
}

/// `decision edit` sends the version and only the fields given: no `--due` sends no `due` key (a `null` would clear the
/// date), `--no-due` sends `"due": null`.
#[test]
fn decision_edit_sends_only_the_given_fields() {
    let sent = |args: &[&str]| {
        let (line, body) = capture(args);
        assert!(line.starts_with("PATCH /api/v1/decisions/7 "), "{line}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    };
    assert_eq!(
        sent(&[
            "decision",
            "edit",
            "7",
            "--version",
            "3",
            "--decider",
            "x@example.com"
        ]),
        serde_json::json!({"version": 3, "decider": "x@example.com"})
    );
    assert_eq!(
        sent(&["decision", "edit", "7", "--version", "3", "--no-due"]),
        serde_json::json!({"version": 3, "due": null})
    );
    assert_eq!(
        sent(&[
            "decision",
            "edit",
            "7",
            "--version",
            "3",
            "--title",
            "Pick",
            "--due",
            "2026-11-30"
        ]),
        serde_json::json!({"version": 3, "title": "Pick", "due": "2026-11-30"})
    );
}

/// ok exits 0 and --json prints the envelope; a conflict exits 1 with the error envelope; no change, or both --due and
/// --no-due, is a usage error (exit 1, clap's usage text) before any request.
#[test]
fn decision_edit_exit_statuses_and_usage() {
    let (url, server) = serve(vec![(
        "PATCH /api/v1/decisions/7",
        200,
        ok(decision(
            "adrs/0001.md",
            serde_json::json!({"kind":"unchecked"}),
        )),
    )]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "edit",
            "7",
            "--version",
            "3",
            "--title",
            "Pick",
        ],
    );
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["outcome"],
        "ok"
    );
    let (url, server) = serve(vec![(
        "PATCH /api/v1/decisions/7",
        409,
        serde_json::json!({"outcome":"error","code":"conflict","message":"version conflict: current is 4","current":4}),
    )]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "edit",
            "7",
            "--version",
            "2",
            "--title",
            "Pick",
        ],
    );
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap(),
        serde_json::json!({"outcome":"error","code":"conflict","message":"version conflict: current is 4","current":4}),
        "the conflict's envelope carries the current version"
    );
    for args in [
        &["decision", "edit", "7", "--version", "3"][..],
        &[
            "decision",
            "edit",
            "7",
            "--version",
            "3",
            "--due",
            "2026-11-30",
            "--no-due",
        ][..],
    ] {
        let out = Command::cargo_bin("reeve")
            .unwrap()
            .env("REEVE_URL", "http://127.0.0.1:9")
            .env("REEVE_TOKEN", "reeve_test")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        // A request to the closed port would also exit 1; only a usage error prints clap's usage line.
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("Usage:"),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// `reeve tui` with no stored login exits 1 with the login hint, before touching the terminal: no alternate-screen
/// sequence reaches stdout. `REEVE_TOKEN` is set to prove it is not used as a credential (kb/ADR-0011 R7).
#[test]
fn tui_without_a_stored_login_exits_1_without_entering_the_alternate_screen() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .arg("tui")
        .env("REEVE_CONFIG_DIR", dir.path())
        .env("REEVE_URL", "http://127.0.0.1:9")
        .env("REEVE_TOKEN", "reeve_automation_secret")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("reeve auth login"), "{stderr}");
    assert!(
        !out.stdout.windows(8).any(|w| w == b"\x1b[?1049h"),
        "entered the alternate screen"
    );
}

#[test]
fn tui_refuses_json() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .args(["--json", "tui"])
        .env("REEVE_CONFIG_DIR", dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("is not supported by `reeve tui`"),
        "{stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// `show` lists an initiative's or an epic's advisory findings in text; `findings` gives them as JSON.
#[test]
fn show_lists_findings_and_findings_gives_them_as_json() {
    let ini = serde_json::json!({
        "key": "TST-1", "title": "Leave the old host", "brief": "", "horizon": "Now", "target_month": null, "owner": null,
        "discovery_complete": null, "progress": null, "external_ref": null, "version": 1
    });
    let (url, server) = serve(vec![
        ("GET /api/v1/initiatives/TST-1", 200, ok(ini)),
        (
            "GET /api/v1/initiatives/TST-1/findings",
            200,
            ok(serde_json::json!([{"finding": "empty_brief"}])),
        ),
        (
            "GET /api/v1/decisions?initiative=TST-1",
            200,
            ok(serde_json::json!([])),
        ),
        (
            "GET /api/v1/items/TST-1/comments",
            200,
            ok(
                serde_json::json!([{"id": 4, "on_story": null, "on_decision": null, "on_initiative": "TST-1",
                "on_epic": null, "author": "anna@example.com", "body": "Should Orion go first?",
                "at": "2026-10-31T09:00:00Z", "external_ref": null}]),
            ),
        ),
    ]);
    let out = reeve_at(&url, &["initiative", "show", "TST-1"]);
    server.join().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("TST-1 Leave the old host"), "{text}");
    assert!(
        text.contains("findings (advisory") && text.contains("the brief is empty"),
        "{text}"
    );
    assert!(
        text.contains("comments:") && text.contains("anna@example.com: Should Orion go first?"),
        "{text}"
    );

    let (url, server) = serve(vec![(
        "GET /api/v1/epics/TST-2/findings",
        200,
        ok(serde_json::json!([{"finding": "discovery_complete_without_stories"}])),
    )]);
    let out = reeve_at(&url, &["--json", "epic", "findings", "TST-2"]);
    server.join().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["value"],
        serde_json::json!([{"finding": "discovery_complete_without_stories"}])
    );
}

/// Findings are advisory: when their request fails outright (here the connection drops), `show` still prints the item.
#[test]
fn show_prints_the_item_when_the_findings_request_fails() {
    let epic = serde_json::json!({
        "key": "TST-2", "title": "Hosting", "initiative": null, "brief": "", "discovery_complete": null,
        "external_ref": null, "version": 1
    });
    let (url, server) = serve(vec![
        ("GET /api/v1/epics/TST-2", 200, ok(epic)),
        (
            "GET /api/v1/epics/TST-2/findings",
            0,
            serde_json::Value::Null,
        ),
        (
            "GET /api/v1/items/TST-2/comments",
            200,
            ok(serde_json::json!([])),
        ),
    ]);
    let out = reeve_at(&url, &["epic", "show", "TST-2"]);
    server.join().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("TST-2 Hosting"));
}

/// Text output marks each key with its level's glyph; `--ascii` or `REEVE_ASCII` switch to the plain-text
/// fallback, and `--json` never carries either.
#[test]
fn keys_carry_a_glyph_with_an_ascii_fallback() {
    let story = serde_json::json!({
        "key": "TST-1", "title": "T", "kind": "chore", "epic": null, "description": "", "criteria": [],
        "owner": null, "labels": [], "rank": 0, "blocked_by": [], "delivery": "todo", "resolution": null,
        "external_ref": null, "version": 1, "prs": []
    });
    let run = |args: &[&str], env: Option<(&str, &str)>| {
        let (url, server) = serve(vec![("GET /api/v1/stories/TST-1", 200, ok(story.clone()))]);
        let cfg = tempfile::tempdir().unwrap();
        let mut c = Command::cargo_bin("reeve").unwrap();
        c.env("REEVE_URL", &url)
            .env("REEVE_TOKEN", "reeve_test")
            .env("REEVE_CONFIG_DIR", cfg.path())
            .env_remove("REEVE_ASCII")
            .args(args);
        if let Some((k, v)) = env {
            c.env(k, v);
        }
        let out = c.output().unwrap();
        server.join().unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert!(run(&["story", "show", "TST-1"], None).starts_with("○ TST-1 [todo]"));
    assert!(run(&["--ascii", "story", "show", "TST-1"], None).starts_with("[C] TST-1 [todo]"));
    assert!(run(&["story", "show", "TST-1"], Some(("REEVE_ASCII", "1"))).starts_with("[C] TST-1"));
    let json = run(&["--json", "story", "show", "TST-1"], None);
    assert!(
        !json.contains('○') && json.contains("\"key\":\"TST-1\""),
        "{json}"
    );
}

/// A brief or a description read from a file or from stdin (`-`) reaches the wire unchanged, whatever shell
/// quoting would have broken.
#[test]
fn briefs_and_descriptions_come_from_a_file_or_stdin_unchanged() {
    let md =
        "# Why\n\nIt's the `edge`: $(not run) and \"quotes\".\n- a list item\n#hash at the start\n";
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("brief.md");
    std::fs::write(&file, md).unwrap();
    let path = file.to_str().unwrap();

    let (_, body) = capture(&[
        "initiative",
        "edit",
        "TST-1",
        "--version",
        "1",
        "--brief-file",
        path,
    ]);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["brief"], md, "{body}");

    let (_, body) = capture(&[
        "story",
        "create",
        "--idempotency-key",
        "k",
        "--title",
        "T",
        "--description-file",
        path,
    ]);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["description"], md, "{body}");

    // `-` reads stdin
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let stream = accept(&listener);
        let (_, body) = read_request(&stream);
        let mut w = stream;
        let resp = r#"{"outcome":"error","code":"not_found","message":"not found"}"#;
        write!(w, "HTTP/1.1 404 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{resp}", resp.len()).unwrap();
        body
    });
    let cfg = tempfile::tempdir().unwrap();
    Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", &url)
        .env("REEVE_TOKEN", "reeve_test")
        .env("REEVE_CONFIG_DIR", cfg.path())
        .args([
            "epic",
            "edit",
            "TST-2",
            "--version",
            "1",
            "--brief-file",
            "-",
        ])
        .write_stdin(md)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert_eq!(v["brief"], md);

    // inline and file together is a usage error
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .args([
            "epic",
            "edit",
            "TST-2",
            "--version",
            "1",
            "--brief",
            "x",
            "--brief-file",
            path,
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

/// `reeve apply` sends a YAML (or JSON) plan as one request and prints the key each name came to.
#[test]
fn apply_sends_a_yaml_plan_in_one_request_and_lists_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plan.yaml");
    std::fs::write(
        &file,
        "id: refine-hosting\nstories:\n  - name: db\n    title: Move the database\n    criteria: [moved]\n  - name: dns\n    title: Point DNS\n    kind: chore\nblocks:\n  - {story: dns, blocked_by: db}\n",
    )
    .unwrap();
    let applied = serde_json::json!({
        "items": [
            {"name": "db", "item": "story", "key": "TST-3", "created": true},
            {"name": "dns", "item": "story", "key": "TST-4", "created": false}
        ],
        "decisions": [],
        "blocks": [{"story": "TST-4", "blocked_by": "TST-3", "created": true}]
    });
    let (url, server) = serve(vec![("POST /api/v1/plans", 200, ok(applied))]);
    let out = reeve_at(&url, &["apply", file.to_str().unwrap()]);
    let bodies = server.join().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(sent["id"], "refine-hosting");
    assert_eq!(sent["stories"][1]["kind"], "chore");
    assert_eq!(sent["blocks"][0]["blocked_by"], "db");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("db") && text.contains("● TST-3") && text.contains("created"),
        "{text}"
    );
    assert!(
        text.contains("○ TST-4") || text.contains("● TST-4"),
        "{text}"
    );
    assert!(text.contains("existing"), "{text}");

    // a misspelt field is refused before anything is sent
    std::fs::write(
        &file,
        "id: x\nstories:\n  - name: s\n    title: S\n    criterion: [x]\n",
    )
    .unwrap();
    let out = reeve_at("http://127.0.0.1:9", &["apply", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("criterion"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `reeve search` lists hits with their level glyph and where they matched; --json gives the hits.
#[test]
fn search_lists_hits_with_glyphs() {
    let hits = serde_json::json!([
        {"key": "TST-3", "item": "story", "kind": "bug", "title": "Move the database", "matched": ["description"], "exact": false},
        {"key": "TST-1", "item": "initiative", "kind": null, "title": "Leave the old host", "matched": ["title", "external_ref"], "exact": false}
    ]);
    let (url, server) = serve(vec![(
        "GET /api/v1/search?q=north+cloud",
        200,
        ok(hits.clone()),
    )]);
    let out = reeve_at(&url, &["search", "north", "cloud"]);
    server.join().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("▲ TST-3\tMove the database\tdescription"),
        "{text}"
    );
    assert!(
        text.contains("◆ TST-1\tLeave the old host\ttitle, external_ref"),
        "{text}"
    );
    let (url, server) = serve(vec![("GET /api/v1/search?q=north", 200, ok(hits.clone()))]);
    let out = reeve_at(&url, &["--json", "search", "north"]);
    server.join().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["value"], hits);
}

/// Linking a record in a repository the server does not check yet stores it, runs no immediate check (there is
/// no clone to read), says so, and exits 0; the scheduled check picks it up once the repository is configured.
#[test]
fn link_to_an_unconfigured_repository_is_kept_and_not_checked() {
    let mut d = decision(
        "docs/adr/0001-host.md",
        serde_json::json!({"kind":"unchecked"}),
    );
    d["record"]["repo"] = "infra".into();
    let config = serde_json::json!({"key_prefix":"TST","horizon_buckets":["Now"],"repos":["kb"],"sign_in_configured":true});
    let (url, server) = serve(vec![
        ("POST /api/v1/decisions/7/link", 200, ok(d.clone())),
        ("GET /api/v1/config", 200, ok(config)),
        ("GET /api/v1/decisions/7", 200, ok(d)),
    ]);
    let household = tempfile::tempdir().unwrap();
    let out = reeve_at(
        &url,
        &[
            "--json",
            "decision",
            "link",
            "7",
            "--record",
            "infra/ADR-0001",
            "--repo",
            "infra",
            "--path",
            "docs/adr/0001-host.md",
            "--household",
            household.path().to_str().unwrap(),
        ],
    );
    server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["immediate_check"]["outcome"], "repo_not_configured",
        "{v}"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("not configured"));
}

/// `story create --owner` and `story edit --owner` set a story's owner; `story edit --no-owner` clears it.
#[test]
fn story_owner_is_set_on_create_and_edit_and_cleared() {
    let (_, body) = capture(&[
        "story",
        "create",
        "--idempotency-key",
        "k",
        "--title",
        "T",
        "--owner",
        "anna@example.com",
    ]);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["owner"], "anna@example.com", "{body}");
    let (_, body) = capture(&[
        "story",
        "edit",
        "TST-1",
        "--version",
        "2",
        "--owner",
        "anna@example.com",
    ]);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["owner"], "anna@example.com", "{body}");
    let (_, body) = capture(&["story", "edit", "TST-1", "--version", "2", "--no-owner"]);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v.get("owner").is_some_and(|o| o.is_null()), "{body}");
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .args([
            "story",
            "edit",
            "TST-1",
            "--version",
            "2",
            "--owner",
            "a@example.com",
            "--no-owner",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

/// `comment add --item <KEY>` comments on any item; `--story` stays an alias; `comment ls --item` lists.
#[test]
fn comment_takes_any_item_by_key() {
    let (line, body) = capture(&[
        "comment",
        "add",
        "--idempotency-key",
        "k",
        "--item",
        "TST-2",
        "--body",
        "hi",
    ]);
    assert!(
        line.starts_with("POST /api/v1/items/TST-2/comments"),
        "{line}"
    );
    assert!(body.contains("\"body\":\"hi\""), "{body}");
    let (line, _) = capture(&[
        "comment",
        "add",
        "--idempotency-key",
        "k",
        "--story",
        "TST-3",
        "--body",
        "hi",
    ]);
    assert!(
        line.starts_with("POST /api/v1/items/TST-3/comments"),
        "{line}"
    );
    let (line, _) = capture(&["comment", "ls", "--item", "TST-2"]);
    assert!(
        line.starts_with("GET /api/v1/items/TST-2/comments"),
        "{line}"
    );
    let (line, _) = capture(&["comment", "ls", "--decision", "7"]);
    assert!(
        line.starts_with("GET /api/v1/decisions/7/comments"),
        "{line}"
    );
}

/// `decision ls --initiative` asks for every decision under an initiative; each line says where the decision
/// lives and what it gates.
#[test]
fn decisions_under_an_initiative_say_where_they_live() {
    let mut on_epic = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    on_epic["on"] = "TST-3".into();
    on_epic["on_type"] = "epic".into();
    let (url, server) = serve(vec![(
        "GET /api/v1/decisions?initiative=TST-1",
        200,
        ok(serde_json::json!([on_epic])),
    )]);
    let out = reeve_at(&url, &["decision", "ls", "--initiative", "TST-1"]);
    server.join().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("#7 on ■ TST-3"), "{text}");
    assert!(text.contains("gates the stories in this epic"), "{text}");
}

/// A conflict's current version reaches a caller: `--json` keeps the server's `current` (absent when the server sent
/// none, `-1` as sent), and text output says `current version: N` on stderr, except for `-1` (the server's message
/// already says to retry). Covers an initiative edit and an explicit-version story move: every command prints through
/// the same `finish`.
#[test]
fn a_conflict_reports_the_current_version() {
    let stale = |current: Option<i64>| {
        let mut v = serde_json::json!({"outcome":"error","code":"conflict","message":"version conflict: current is 4"});
        if let Some(c) = current {
            v["current"] = serde_json::json!(c);
        }
        v
    };
    let initiative: &[&str] = &[
        "initiative",
        "edit",
        "TST-1",
        "--version",
        "2",
        "--title",
        "T",
    ];
    let mv: &[&str] = &["story", "mv", "TST-2", "todo", "--version", "2"];
    for (route, args) in [
        ("PATCH /api/v1/initiatives/TST-1", initiative),
        ("POST /api/v1/stories/TST-2/move", mv),
    ] {
        for (current, want_line) in [(Some(4), true), (Some(-1), false), (None, false)] {
            let (url, server) = serve(vec![(route, 409, stale(current))]);
            let out = reeve_at(&url, args);
            server.join().unwrap();
            assert_eq!(out.status.code(), Some(1), "{args:?}");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains("version conflict"), "{stderr}");
            assert_eq!(
                stderr.contains("current version:"),
                want_line,
                "{args:?} {current:?}: {stderr}"
            );
            if want_line {
                assert!(stderr.contains("current version: 4"), "{stderr}");
            }

            let (url, server) = serve(vec![(route, 409, stale(current))]);
            let json_args: Vec<&str> = std::iter::once("--json")
                .chain(args.iter().copied())
                .collect();
            let out = reeve_at(&url, &json_args);
            server.join().unwrap();
            assert_eq!(out.status.code(), Some(1));
            let env: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(env["code"], "conflict");
            // absent when the server sent none (never `null`), as in the server's envelope
            assert_eq!(
                env.get("current"),
                current.map(serde_json::Value::from).as_ref(),
                "{env}"
            );
        }
    }
}

/// `initiative edit` sends only the fields given: `--no-owner` and `--no-target-month` send `null` (which clears),
/// leaving a flag out sends no key; a value and its `--no-` flag together is a usage error before any request.
#[test]
fn initiative_edit_sends_only_the_given_fields() {
    let sent = |args: &[&str]| {
        let (line, body) = capture(args);
        assert!(
            line.starts_with("PATCH /api/v1/initiatives/TST-1 "),
            "{line}"
        );
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    };
    let edit = |extra: &[&'static str]| -> Vec<&'static str> {
        let mut v = vec!["initiative", "edit", "TST-1", "--version", "3"];
        v.extend_from_slice(extra);
        v
    };
    assert_eq!(
        sent(&edit(&["--no-owner"])),
        serde_json::json!({"version": 3, "owner": null})
    );
    assert_eq!(
        sent(&edit(&["--no-target-month"])),
        serde_json::json!({"version": 3, "target_month": null})
    );
    assert_eq!(
        sent(&edit(&[
            "--owner",
            "x@example.com",
            "--target-month",
            "2027-03"
        ])),
        serde_json::json!({"version": 3, "owner": "x@example.com", "target_month": "2027-03"})
    );
    assert_eq!(
        sent(&edit(&["--title", "T"])),
        serde_json::json!({"version": 3, "title": "T"})
    );
    for extra in [
        &["--owner", "x@example.com", "--no-owner"][..],
        &["--target-month", "2027-03", "--no-target-month"][..],
    ] {
        let out = Command::cargo_bin("reeve")
            .unwrap()
            .env("REEVE_URL", "http://127.0.0.1:9")
            .env("REEVE_TOKEN", "reeve_test")
            .args(edit(extra))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{extra:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("Usage:"),
            "{extra:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// `--mine` resolves the caller with `GET /me`; a refusal there exits 2 with the server's envelope, like every other
/// command the server refuses, and any other error keeps its code and exits 1.
#[test]
fn mine_reports_the_servers_answer_to_me() {
    let unauthenticated =
        serde_json::json!({"outcome":"error","code":"unauthenticated","message":"unauthenticated"});
    let internal =
        serde_json::json!({"outcome":"error","code":"internal","message":"internal error"});
    type Answers = Vec<(&'static str, u16, serde_json::Value)>;
    let cases: Vec<(Answers, &[&str], i32, &str)> = vec![
        (
            vec![("GET /api/v1/me", 401, unauthenticated.clone())],
            &["--json", "story", "ls", "--mine"],
            2,
            "unauthenticated",
        ),
        (
            vec![
                ("GET /api/v1/decisions", 200, ok(serde_json::json!([]))),
                ("GET /api/v1/me", 403, forbidden()),
            ],
            &["--json", "decision", "ls", "--mine"],
            2,
            "forbidden",
        ),
        // the list's own refusal is the answer: no `/me` after it (`serve` would refuse that connection, exit 1)
        (
            vec![("GET /api/v1/decisions", 403, forbidden())],
            &["--json", "decision", "ls", "--mine"],
            2,
            "forbidden",
        ),
        (
            vec![("GET /api/v1/me", 500, internal.clone())],
            &["--json", "story", "ls", "--mine"],
            1,
            "internal",
        ),
    ];
    for (answers, args, want, code) in cases {
        let (url, server) = serve(answers);
        let out = reeve_at(&url, args);
        server.join().unwrap();
        assert_eq!(out.status.code(), Some(want), "{args:?}");
        let env: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(env["code"], code, "{args:?}: {env}");
    }
}

/// kb/ADR-0013 R3: `--decided` and `--withdrawn` each send their kind with the reason; a server validation answer
/// (decided on a one-way decision) exits 1 with the server's message. Asserts on the body the CLI sent, not on the
/// scripted answer, so a CLI that drops or hard-codes the kind fails.
#[test]
fn decision_close_sends_the_kind_its_flag_names() {
    let mut closed = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    closed["closed"] = serde_json::json!({"at":"2026-11-03T09:00:00Z","by":"dan@example.com","reason":"x",
        "kind":"withdrawn","via":"interactive"});
    for (flag, reason, kind) in [
        ("--decided", "flat fee", "decided"),
        ("--withdrawn", "dropped", "withdrawn"),
    ] {
        let (url, server) = serve(vec![(
            "POST /api/v1/decisions/7/close",
            200,
            ok(closed.clone()),
        )]);
        let out = reeve_at(&url, &["decision", "close", "7", flag, reason]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{flag}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let sent: serde_json::Value = serde_json::from_str(&server.join().unwrap()[0]).unwrap();
        assert_eq!(
            sent,
            serde_json::json!({"kind": kind, "reason": reason}),
            "{flag}"
        );
    }

    let message = "a one-way decision is decided by its accepted record; withdraw it if it is no longer needed";
    let (url, server) = serve(vec![(
        "POST /api/v1/decisions/7/close",
        400,
        serde_json::json!({"outcome":"error","code":"validation","message":message}),
    )]);
    let out = reeve_at(&url, &["decision", "close", "7", "--decided", "flat fee"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `close` needs exactly one of `--decided` and `--withdrawn`: neither or both is a usage error (exit 1) and sends
/// nothing (the port is closed, so only clap's usage line tells a usage error from a refused connection).
#[test]
fn decision_close_needs_exactly_one_outcome() {
    for args in [
        &["decision", "close", "7"][..],
        &[
            "decision",
            "close",
            "7",
            "--decided",
            "a",
            "--withdrawn",
            "b",
        ][..],
        &["decision", "close", "7", "--reason", "a"][..],
    ] {
        let out = Command::cargo_bin("reeve")
            .unwrap()
            .env("REEVE_URL", "http://127.0.0.1:9")
            .env("REEVE_TOKEN", "reeve_test")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("Usage:"),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// kb/ADR-0013 R1: `require --reversibility one-way` sends `one_way`; without `--due` the CLI warns on stderr and the
/// stdout JSON is the server's envelope, unchanged. With `--due` there is no warning.
#[test]
fn decision_require_sends_reversibility_and_warns_without_a_due_date() {
    let base = [
        "--json",
        "decision",
        "require",
        "--idempotency-key",
        "k1",
        "--on",
        "TST-1",
        "--title",
        "Pick",
        "--decider",
        "dan@example.com",
    ];
    let created = ok(decision(
        "adrs/0001.md",
        serde_json::json!({"kind":"unchecked"}),
    ));
    let (url, server) = serve(vec![("POST /api/v1/decisions", 200, created.clone())]);
    let mut args = base.to_vec();
    args.extend(["--reversibility", "one-way"]);
    let out = reeve_at(&url, &args);
    let sent: serde_json::Value = serde_json::from_str(&server.join().unwrap()[0]).unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(sent["reversibility"], "one_way");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no decide-by date; the queue will flag it"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap(),
        created,
        "the warning is on stderr only"
    );

    let (url, server) = serve(vec![("POST /api/v1/decisions", 200, created)]);
    let mut args = base.to_vec();
    args.extend(["--due", "2026-11-30", "--reversibility", "two-way"]);
    let out = reeve_at(&url, &args);
    let sent: serde_json::Value = serde_json::from_str(&server.join().unwrap()[0]).unwrap();
    assert_eq!(sent["reversibility"], "two_way");
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("no decide-by date"),
        "a dated decision gets no warning"
    );
}

/// kb/ADR-0013 R1: `edit --reversibility` sends the value, `--no-reversibility` sends null; both together is a usage
/// error that sends nothing.
#[test]
fn decision_edit_sends_reversibility_and_refuses_both_flags() {
    let sent = |args: &[&str]| {
        let (line, body) = capture(args);
        assert!(line.starts_with("PATCH /api/v1/decisions/7 "), "{line}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    };
    assert_eq!(
        sent(&[
            "decision",
            "edit",
            "7",
            "--version",
            "3",
            "--no-reversibility"
        ]),
        serde_json::json!({"version": 3, "reversibility": null})
    );
    assert_eq!(
        sent(&[
            "decision",
            "edit",
            "7",
            "--version",
            "3",
            "--reversibility",
            "two-way"
        ]),
        serde_json::json!({"version": 3, "reversibility": "two_way"})
    );
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", "http://127.0.0.1:9")
        .env("REEVE_TOKEN", "reeve_test")
        .args([
            "decision",
            "edit",
            "7",
            "--version",
            "3",
            "--reversibility",
            "one-way",
            "--no-reversibility",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage:"));
}

fn queue_body() -> serde_json::Value {
    let mut blocking = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    blocking["reversibility"] = serde_json::json!("one_way");
    let mut closed = decision("adrs/0002.md", serde_json::json!({"kind":"unchecked"}));
    closed["id"] = serde_json::json!(8);
    closed["closed"] = serde_json::json!({"at":"2026-11-03T09:00:00Z","by":"dan@example.com",
        "reason":"flat fee","kind":"decided","via":"interactive"});
    serde_json::json!({
        "entries": [
            {"group":"blocking","decision":blocking,"blocks":[
                {"key":"TST-1","title":"One"},{"key":"TST-2","title":"Two"}]},
            {"group":"closed","decision":closed,"blocks":[]}
        ],
        "summary": {"open":3,"blocking":1,"blocked_stories":2,"overdue":4}
    })
}

/// What the CLI prints for `queue_body()`: it re-serialises the typed queue, so an older server's response comes back
/// with every field the lanes change added at its default.
fn queue_body_defaulted() -> serde_json::Value {
    let mut b = queue_body();
    for e in b["entries"].as_array_mut().unwrap() {
        e["lane"] = serde_json::json!("open");
        e["needed_in"] = serde_json::json!("none");
        e["needed_in_reason"] = serde_json::json!(null);
        e["next_step"] = serde_json::json!(null);
        e["under_way"] = serde_json::json!([]);
        for st in e["blocks"].as_array_mut().unwrap() {
            st["kind"] = serde_json::json!("story");
            st["delivery"] = serde_json::json!("todo");
            st["owner"] = serde_json::json!(null);
            st["urgent"] = serde_json::json!(false);
            st["needed_in"] = serde_json::json!("none");
        }
    }
    b["summary"]["review"] = serde_json::json!(0);
    b["summary"]["this_sprint"] = serde_json::json!(0);
    b["summary"]["next_sprint"] = serde_json::json!(0);
    b
}

/// The same queue as a server with the lanes change sends it: every new field set, plus one this build does not know.
fn queue_body_with_lanes() -> serde_json::Value {
    let mut b = queue_body();
    let e = &mut b["entries"][0];
    e["lane"] = serde_json::json!("this_sprint");
    e["needed_in"] = serde_json::json!("this_sprint");
    e["needed_in_reason"] = serde_json::json!({"kind":"goal","goal_id":3,"outcome":"Ship it"});
    e["next_step"] =
        serde_json::json!({"kind":"review_record","pr_url":"https://github.com/acme/kb/pull/9"});
    e["added_later"] = serde_json::json!(true);
    for (i, st) in e["blocks"].as_array_mut().unwrap().iter_mut().enumerate() {
        st["kind"] = serde_json::json!("story");
        st["delivery"] = serde_json::json!("todo");
        st["owner"] = serde_json::json!(null);
        st["urgent"] = serde_json::json!(i == 0);
        st["needed_in"] = serde_json::json!("this_sprint");
    }
    e["under_way"] = serde_json::json!([{"key":"TST-3","title":"Three","kind":"bug","delivery":"in_progress",
        "owner":"o@x","urgent":false,"needed_in":"next_sprint"}]);
    e["decision"]["record"]["browse_url"] =
        serde_json::json!("https://github.com/acme/kb/blob/HEAD/adrs/0001.md");
    b["summary"]["review"] = serde_json::json!(1);
    b["summary"]["this_sprint"] = serde_json::json!(2);
    b["summary"]["next_sprint"] = serde_json::json!(3);
    b
}

/// A server that sends the new queue fields reads as before (the CLI prints the same lines), in text and as served in
/// JSON; the old-response fixture above proves the other direction.
#[test]
fn decision_ls_queue_reads_a_queue_with_lanes() {
    let (url, server) = serve(vec![(
        "GET /api/v1/decisions/queue",
        200,
        ok(queue_body_with_lanes()),
    )]);
    let out = reeve_at(&url, &["decision", "ls", "--queue"]);
    server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0], "open: 3 · blocking: 1 (2 stories) · overdue: 4",
        "{text}"
    );
    assert!(lines[1].starts_with("[blocking] #7 "), "{text}");
    assert!(lines[1].ends_with("blocks: 2"), "{text}");
    assert!(lines[2].starts_with("[closed] #8 "), "{text}");

    // as served in JSON, every new field survives the round trip and the unknown one is dropped
    let (url, server) = serve(vec![(
        "GET /api/v1/decisions/queue",
        200,
        ok(queue_body_with_lanes()),
    )]);
    let out = reeve_at(&url, &["--json", "decision", "ls", "--queue"]);
    server.join().unwrap();
    let got: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut want = ok(queue_body_with_lanes());
    want["value"]["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("added_later");
    // the closed entry carries no new field in the body, so it comes back at the defaults
    want["value"]["entries"][1] = queue_body_defaulted()["entries"][1].clone();
    assert_eq!(got, want);
}

/// `ls --queue` reads `/decisions/queue` with the same scope query and, with `--json`, prints the envelope as served;
/// as text the first line is the summary and each entry shows its group and block count. `--mine` conflicts.
#[test]
fn decision_ls_queue_reads_the_queue() {
    let (url, server) = serve(vec![(
        "GET /api/v1/decisions/queue?on=TST-1",
        200,
        ok(queue_body()),
    )]);
    let out = reeve_at(
        &url,
        &["--json", "decision", "ls", "--queue", "--on", "TST-1"],
    );
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap(),
        ok(queue_body_defaulted())
    );

    let (url, server) = serve(vec![(
        "GET /api/v1/decisions/queue?initiative=TST-1",
        200,
        ok(queue_body()),
    )]);
    let out = reeve_at(
        &url,
        &["decision", "ls", "--queue", "--initiative", "TST-1"],
    );
    server.join().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0], "open: 3 · blocking: 1 (2 stories) · overdue: 4",
        "{text}"
    );
    assert!(lines[1].starts_with("[blocking] #7 "), "{text}");
    assert!(lines[1].ends_with("blocks: 2"), "{text}");
    assert!(lines[2].starts_with("[closed] #8 "), "{text}");
    assert!(lines[2].ends_with("blocks: 0"), "{text}");

    let (url, server) = serve(vec![("GET /api/v1/decisions/queue", 200, ok(queue_body()))]);
    reeve_at(&url, &["decision", "ls", "--queue"]);
    server.join().unwrap();

    let out = Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", "http://127.0.0.1:9")
        .env("REEVE_TOKEN", "reeve_test")
        .args(["decision", "ls", "--queue", "--mine"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage:"));
}

/// Plain `ls` renders reversibility and the three closed shapes from the server's fields.
#[test]
fn decision_ls_renders_reversibility_and_closure() {
    let mut one_way = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    one_way["reversibility"] = serde_json::json!("one_way");
    let mut two_way = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
    two_way["id"] = serde_json::json!(8);
    two_way["reversibility"] = serde_json::json!("two_way");
    let closed = |id: i64, kind: serde_json::Value| {
        let mut d = decision("adrs/0001.md", serde_json::json!({"kind":"unchecked"}));
        d["id"] = serde_json::json!(id);
        d["closed"] = serde_json::json!({"at":"2026-11-03T09:00:00Z","by":"dan@example.com",
            "reason":"because","kind":kind,"via":null});
        d
    };
    let rows = serde_json::json!([
        one_way,
        two_way,
        closed(9, serde_json::json!("decided")),
        closed(10, serde_json::json!("withdrawn")),
        closed(11, serde_json::Value::Null)
    ]);
    let (url, server) = serve(vec![("GET /api/v1/decisions", 200, ok(rows))]);
    let out = reeve_at(&url, &["decision", "ls"]);
    server.join().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains(" reversibility: one-way"), "{text}");
    assert!(lines[1].contains(" reversibility: two-way"), "{text}");
    assert!(!lines[2].contains("reversibility"), "{text}");
    assert!(lines[2].contains(" closed: decided: because"), "{text}");
    assert!(lines[3].contains(" closed: withdrawn: because"), "{text}");
    assert!(lines[4].contains(" closed: because"), "{text}");
    assert!(!lines[4].contains("closed: decided") && !lines[4].contains("closed: withdrawn"));
}

fn goal_json(id: i64, sprint_id: i64) -> serde_json::Value {
    serde_json::json!({"id":id,"sprint_id":sprint_id,"outcome":"Bookings work","check":"a customer books","owner":"dan@example.com",
        "notes":"","confidence":null,"confidence_note":null,"confidence_at":null,"suggested_confidence":null,"suggested_note":null,
        "suggested_at":null,"suggested_by":null,"verdict":null,"verdict_note":null,"version":1,"created_at":"2026-11-03T09:00:00Z","links":[]})
}

fn sprint_json(id: i64, state: &str, version: i64) -> serde_json::Value {
    serde_json::json!({"id":id,"state":state,"starts_on":"2026-11-03","ends_on":"2026-11-17","opened_at":null,"closed_at":null,
        "version":version,"goals":[goal_json(12, id)],"warnings":[]})
}

fn counts_json(in_flight_reordered: [(i64, u32); 2]) -> serde_json::Value {
    let g = |(id, f): (i64, u32)| serde_json::json!({"goal_id":id,"completed":1,"in_flight":f,"todo":0,"in_review":0,"blocked_decisions":0});
    serde_json::json!({"goals":[g(in_flight_reordered[0]), g(in_flight_reordered[1])],"completed":3,"upkeep_completed":1})
}

fn view_json(state: &str, counts: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"sprint":sprint_json(4, state, 2),"counts":counts,"upkeep_line":20,"demo":[]})
}

/// A bad `--verdict` is a usage error before any request (exit 1: this CLI keeps exit 2 for the server's refusals).
#[test]
fn a_malformed_verdict_is_a_usage_error() {
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", "http://127.0.0.1:9")
        .env("REEVE_TOKEN", "reeve_test")
        .args(["sprint", "close", "--verdict", "12=maybe"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("partly_met"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `sprint close` reads the active sprint's id and version, then posts every verdict with its note.
#[test]
fn sprint_close_reads_the_active_version_and_posts_the_verdicts() {
    let (url, server) = serve(vec![
        (
            "GET /api/v1/sprints",
            200,
            ok(
                serde_json::json!({"active":view_json("active", counts_json([(12, 0), (13, 0)])),"draft":null,"closed":[]}),
            ),
        ),
        (
            "POST /api/v1/sprints/4/close",
            200,
            ok(sprint_json(4, "closed", 3)),
        ),
    ]);
    let out = reeve_at(
        &url,
        &[
            "--json",
            "sprint",
            "close",
            "--verdict",
            "12=partly_met:one booking",
            "--verdict",
            "13=met",
        ],
    );
    let bodies = server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sent: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"version":2,"verdicts":[{"goal_id":12,"verdict":"partly_met","note":"one booking"},{"goal_id":13,"verdict":"met","note":null}]})
    );
}

/// A closed sprint's view reads its `closed` entry (limit 1): `--json` always carries `at_close` and
/// `changed_since_close`, false for the same numbers in another goal order; a changed number shows both in text.
#[test]
fn sprint_show_adds_at_close_only_when_a_number_changed() {
    let closed_entry = |counts: serde_json::Value| {
        ok(
            serde_json::json!([{"id":9,"at":"2026-11-17T09:00:00Z","actor":"dan@example.com","via":"interactive",
            "entity_type":"sprint","entity_key":"4","action":"closed","payload":{"counts":counts}}]),
        )
    };
    let show = |at_close: serde_json::Value, args: &[&str]| {
        let (url, server) = serve(vec![
            (
                "GET /api/v1/sprints/4",
                200,
                ok(view_json("closed", counts_json([(12, 1), (13, 0)]))),
            ),
            (
                "GET /api/v1/activity?entity_type=sprint&entity_key=4&action=closed&limit=1",
                200,
                closed_entry(at_close),
            ),
        ]);
        let mut a = vec!["sprint", "show", "4"];
        a.extend_from_slice(args);
        let out = reeve_at(&url, &a);
        server.join().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let same = show(counts_json([(13, 0), (12, 1)]), &["--json"]);
    let v: serde_json::Value = serde_json::from_str(&same).unwrap();
    assert_eq!(v["value"]["changed_since_close"], false, "{v}");
    assert_eq!(v["value"]["at_close"]["goals"][1]["in_flight"], 1, "{v}");
    assert_eq!(v["value"]["counts"]["goals"][0]["in_flight"], 1, "{v}");
    let changed = show(counts_json([(13, 0), (12, 2)]), &["--json"]);
    let v: serde_json::Value = serde_json::from_str(&changed).unwrap();
    assert_eq!(v["value"]["changed_since_close"], true, "{v}");
    assert_eq!(v["value"]["at_close"]["goals"][1]["in_flight"], 2, "{v}");
    assert_eq!(v["value"]["counts"]["goals"][0]["in_flight"], 1, "{v}");
    let text = show(counts_json([(13, 0), (12, 2)]), &[]);
    assert!(text.contains("in flight: 1 (at close: 2)"), "{text}");
    assert!(
        text.contains("Counts the work completed in this sprint; work started but not finished shows as in flight."),
        "{text}"
    );
}

/// `goal unlink` sends one DELETE per key and stops at the first failure, exiting with its code.
#[test]
fn goal_unlink_stops_at_the_first_failure() {
    let refused = serde_json::json!({"outcome":"refused","reasons":[{"reason":"sprint_closed"}],"overridable":false});
    let (url, server) = serve(vec![
        (
            "DELETE /api/v1/goals/12/links/TST-1",
            200,
            ok(goal_json(12, 4)),
        ),
        ("DELETE /api/v1/goals/12/links/TST-2", 422, refused),
    ]);
    let out = reeve_at(&url, &["goal", "unlink", "12", "TST-1", "TST-2", "TST-3"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unlinked: TST-1; stopped at TST-2"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("the sprint is closed"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `story ls --goal` passes the filter through; `goal rm` sends its version as a query; confidence words are snake_case.
#[test]
fn goal_arguments_reach_the_wire() {
    let (line, _) = capture(&["story", "ls", "--goal", "any", "--ready"]);
    assert!(
        line.contains("goal=any") && line.contains("ready=true"),
        "{line}"
    );
    let (line, _) = capture(&["goal", "rm", "12", "--version", "3"]);
    assert!(
        line.starts_with("DELETE /api/v1/goals/12?version=3 "),
        "{line}"
    );
    let (line, body) = capture(&["goal", "suggest", "12", "at_risk", "2 in review"]);
    assert!(
        line.starts_with("POST /api/v1/goals/12/suggestion "),
        "{line}"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"confidence":"at_risk","note":"2 in review"})
    );
    let (line, body) = capture(&["goal", "link", "12", "TST-1", "SCRUM-4"]);
    assert!(line.starts_with("POST /api/v1/goals/12/links "), "{line}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v, serde_json::json!({"items":["TST-1","SCRUM-4"]}));
}

/// The active sprint's `--json` view has neither `at_close` nor `changed_since_close`, and reads no activity. It is read
/// again by its id, because `GET /sprints` leaves the active sprint's promises out (kb/ADR-0025 R8).
#[test]
fn an_active_sprint_shows_no_counts_at_close() {
    let mut by_id = view_json("active", counts_json([(12, 1), (13, 0)]));
    by_id["promises"] = serde_json::json!([{"key":"ABC-12","title":"Export","on":"2026-11-10","to":"Acme",
        "holder":"own","epic_key":null,"state":"open","done_on":null,"days":null}]);
    by_id["promises_after"] = "2026-11-02".into();
    let answers = || {
        vec![
            (
                "GET /api/v1/sprints",
                200,
                ok(
                    serde_json::json!({"active":view_json("active", counts_json([(12, 1), (13, 0)])),"draft":null,"closed":[]}),
                ),
            ),
            ("GET /api/v1/sprints/4", 200, ok(by_id.clone())),
        ]
    };
    let (url, server) = serve(answers());
    let out = reeve_at(&url, &["--json", "sprint", "show"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["value"].get("at_close").is_none(), "{v}");
    assert!(v["value"].get("changed_since_close").is_none(), "{v}");
    assert_eq!(v["value"]["promises"][0]["key"], "ABC-12", "{v}");

    let (url, server) = serve(answers());
    let out = reeve_at(&url, &["sprint", "show"]);
    server.join().unwrap();
    let text = out_text(&out);
    assert!(text.contains("Promises due 3–17 Nov:"), "{text}");
    assert!(
        text.contains("ABC-12 Export · to Acme · Promised by 10 Nov"),
        "{text}"
    );
}

/// A `closed` entry without counts is a clear error, not a view without its counts at close.
#[test]
fn a_closed_entry_without_counts_is_an_error() {
    let (url, server) = serve(vec![
        (
            "GET /api/v1/sprints/4",
            200,
            ok(view_json("closed", counts_json([(12, 1), (13, 0)]))),
        ),
        (
            "GET /api/v1/activity?entity_type=sprint&entity_key=4&action=closed&limit=1",
            200,
            ok(
                serde_json::json!([{"id":9,"at":"2026-11-17T09:00:00Z","actor":"dan@example.com","via":"interactive",
                "entity_type":"sprint","entity_key":"4","action":"closed","payload":{}}]),
            ),
        ),
    ]);
    let out = reeve_at(&url, &["sprint", "show", "4"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("carries no readable counts"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A conflict on open says to run the command again (the CLI read the version itself); `--json` keeps the envelope.
#[test]
fn a_conflict_on_open_says_to_run_it_again() {
    let conflict_at = |json: bool| {
        let (url, server) = serve(vec![
            (
                "GET /api/v1/sprints",
                200,
                ok(
                    serde_json::json!({"active":null,"draft":sprint_json(5, "draft", 1),"closed":[]}),
                ),
            ),
            (
                "POST /api/v1/sprints/5/open",
                409,
                serde_json::json!({"outcome":"error","code":"conflict","message":"version conflict","current":2}),
            ),
        ]);
        let mut args = vec!["sprint", "open"];
        if json {
            args.insert(0, "--json");
        }
        let out = reeve_at(&url, &args);
        server.join().unwrap();
        assert_eq!(out.status.code(), Some(1));
        out
    };
    let text = conflict_at(false);
    let err = String::from_utf8_lossy(&text.stderr);
    assert!(
        err.contains("the sprint changed; run the command again"),
        "{err}"
    );
    assert!(!err.contains("current version"), "{err}");
    let json = conflict_at(true);
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["current"], 2, "{v}");
}

fn urgent_story(sources: Vec<reeve_protocol::ItemRefDto>) -> reeve_protocol::StoryDto {
    reeve_protocol::StoryDto {
        key: "ABC-1".parse().unwrap(),
        title: "Match".into(),
        kind: reeve_protocol::Kind::Story,
        epic: None,
        description: String::new(),
        criteria: vec![reeve_protocol::CriterionDto {
            text: "It works".into(),
            done: false,
        }],
        owner: None,
        labels: vec![],
        rank: 1,
        blocked_by: vec![],
        delivery: reeve_protocol::DeliveryState::Todo,
        resolution: None,
        external_ref: None,
        version: 1,
        prs: vec![],
        goals: vec![],
        urgent_mark: None,
        urgent_sources: sources,
        promise: None,
    }
}

/// `story show` of a story that is urgent through its epic: the urgent line is right after the first line, and the rest
/// of the output is the story's usual output.
#[test]
fn story_show_prints_the_urgent_line_after_the_first_line() {
    let src = vec![reeve_protocol::ItemRefDto {
        key: "ABC-40".parse().unwrap(),
        item: reeve_protocol::ItemType::Epic,
    }];
    let (url, server) = serve(vec![(
        "GET /api/v1/stories/ABC-1",
        200,
        ok(serde_json::to_value(urgent_story(src)).unwrap()),
    )]);
    let out = reeve_at(&url, &["story", "show", "ABC-1"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains(" ! [todo] Match"), "{text}");
    assert_eq!(lines[1], "  urgent · via epic ABC-40", "{text}");
    assert!(lines[2].starts_with("  kind: Story"), "{text}");
    assert_eq!(lines[3], "  [ ] It works", "{text}");
    assert_eq!(lines.len(), 4, "{text}");
}

/// `ls` shows the `!` after the key and never the urgent lines.
#[test]
fn initiative_ls_marks_an_urgent_initiative_with_a_bang_and_no_urgent_line() {
    let i = reeve_protocol::InitiativeDto {
        key: "ABC-3".parse().unwrap(),
        title: "Leave".into(),
        brief: String::new(),
        horizon: "Now".into(),
        target_month: None,
        owner: None,
        discovery_complete: None,
        progress: None,
        external_ref: None,
        version: 1,
        urgent_mark: None,
        urgent_sources: vec![reeve_protocol::ItemRefDto {
            key: "ABC-40".parse().unwrap(),
            item: reeve_protocol::ItemType::Epic,
        }],
    };
    let (url, server) = serve(vec![(
        "GET /api/v1/initiatives",
        200,
        ok(serde_json::json!([i])),
    )]);
    let out = reeve_at(&url, &["initiative", "ls"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains("ABC-3 ! Leave"), "{text}");
    assert!(!text.contains("urgent ·"), "{text}");
}

/// `epic show` prints the urgent line right after its first line, ahead of the best-effort findings and comments.
#[test]
fn epic_show_prints_the_urgent_line_after_the_first_line() {
    let e = reeve_protocol::EpicDto {
        key: "ABC-2".parse().unwrap(),
        title: "Hosting".into(),
        initiative: None,
        brief: String::new(),
        discovery_complete: None,
        external_ref: None,
        version: 1,
        urgent_mark: None,
        urgent_sources: vec![reeve_protocol::ItemRefDto {
            key: "ABC-3".parse().unwrap(),
            item: reeve_protocol::ItemType::Initiative,
        }],
        promise: None,
    };
    let (url, server) = serve(vec![
        (
            "GET /api/v1/epics/ABC-2",
            200,
            ok(serde_json::to_value(e).unwrap()),
        ),
        (
            "GET /api/v1/epics/ABC-2/findings",
            200,
            ok(serde_json::json!([])),
        ),
        (
            "GET /api/v1/items/ABC-2/comments",
            200,
            ok(serde_json::json!([])),
        ),
    ]);
    let out = reeve_at(&url, &["epic", "show", "ABC-2"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains(" ! Hosting"), "{text}");
    assert_eq!(lines[1], "  urgent · via initiative ABC-3", "{text}");
    assert_eq!(lines.len(), 2, "{text}");
}

/// `story context` prints the story's urgent line right after the story's own first line.
#[test]
fn story_context_prints_the_urgent_line_after_the_storys_first_line() {
    let src = vec![reeve_protocol::ItemRefDto {
        key: "ABC-40".parse().unwrap(),
        item: reeve_protocol::ItemType::Epic,
    }];
    let ctx = serde_json::json!({
        "story": urgent_story(src), "epic": null, "initiative": null,
        "required_decisions": [], "blockers": [], "comments": [],
        "readiness": {"ready": true, "reasons": []}, "goals": []
    });
    let (url, server) = serve(vec![("GET /api/v1/stories/ABC-1/context", 200, ok(ctx))]);
    let out = reeve_at(&url, &["story", "context", "ABC-1"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.contains("[todo] Match"))
        .unwrap();
    assert!(lines[at].contains(" !"), "{text}");
    assert_eq!(lines[at + 1], "  urgent · via epic ABC-40", "{text}");
}

/// `story show` and `story context` measure a mark's expiry from a clock read after the answer: the fake server stamps
/// a fresh 14-day mark while handling the GET (as when another client marks the story while the read is in flight), so
/// a CLI that reads its clock before the request says 15 d.
#[test]
fn story_show_and_context_measure_a_fresh_mark_after_the_answer() {
    let (url, server) = serve_lazy(vec![(
        "GET /api/v1/stories/TST-1",
        200,
        late(|| ok(marked_story_json("TST-1", None))),
    )]);
    let out = reeve_at(&url, &["story", "show", "TST-1"]);
    server.join().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("· expires in 14 d"), "{text}");

    let (url, server) = serve_lazy(vec![(
        "GET /api/v1/stories/TST-1/context",
        200,
        late(|| {
            ok(serde_json::json!({
                "story": marked_story_json("TST-1", None), "epic": null, "initiative": null,
                "required_decisions": [], "blockers": [], "comments": [],
                "readiness": {"ready": true, "reasons": []}, "goals": []
            }))
        }),
    )]);
    let out = reeve_at(&url, &["story", "context", "TST-1"]);
    server.join().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("· expires in 14 d"), "{text}");
}

/// A story as the urgent route answers a mark: urgent by its own mark, which carries `reason`.
fn marked_story_json(key: &str, reason: Option<&str>) -> serde_json::Value {
    let at = chrono::Utc::now();
    let mut s = urgent_story(vec![reeve_protocol::ItemRefDto {
        key: key.parse().unwrap(),
        item: reeve_protocol::ItemType::Story,
    }]);
    s.key = key.parse().unwrap();
    s.urgent_mark = Some(reeve_protocol::UrgentMarkDto {
        at,
        by: "dana@example.com".into(),
        reason: reason.map(Into::into),
        expires_at: at + chrono::Duration::days(14),
        active: true,
        done: false,
    });
    serde_json::to_value(s).unwrap()
}

/// Marking posts the trimmed reason to the story's urgent route and prints the mark line then the urgent line; a CLI
/// that sends the reason untrimmed, or prints the whole story, fails. The fake server stamps the mark when it handles
/// the POST, as the real one does, so a CLI that measures the expiry from a clock read before the request says 15 d.
#[test]
fn story_urgent_posts_the_trimmed_reason() {
    let (url, server) = serve_lazy(vec![(
        "POST /api/v1/stories/TST-1/urgent",
        200,
        late(|| ok(marked_story_json("TST-1", Some("Customer X blocked")))),
    )]);
    let out = reeve_at(
        &url,
        &[
            "story",
            "urgent",
            "TST-1",
            "--reason",
            "  Customer X blocked ",
        ],
    );
    let bodies = server.join().unwrap();
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(sent, serde_json::json!({"reason":"Customer X blocked"}));
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        format!("{} !", glyph(reeve::glyphs::Level::Story, "TST-1")),
        "{text}"
    );
    assert_eq!(
        lines[1], "  urgent · marked by dana, today · 'Customer X blocked' · expires in 14 d",
        "{text}"
    );
}

/// A blank reason is no reason: `null`, as the web sends it. Epics use their own path, and the answer's first line is
/// the epic's mark line with the `!` its own source earns.
#[test]
fn epic_urgent_with_a_blank_reason_posts_null() {
    let e = reeve_protocol::EpicDto {
        key: "ABC-40".parse().unwrap(),
        title: "Hosting".into(),
        initiative: None,
        brief: String::new(),
        discovery_complete: None,
        external_ref: None,
        version: 1,
        urgent_mark: None,
        urgent_sources: vec![reeve_protocol::ItemRefDto {
            key: "ABC-40".parse().unwrap(),
            item: reeve_protocol::ItemType::Epic,
        }],
        promise: None,
    };
    let (url, server) = serve(vec![(
        "POST /api/v1/epics/ABC-40/urgent",
        200,
        ok(serde_json::to_value(e).unwrap()),
    )]);
    let out = reeve_at(&url, &["epic", "urgent", "ABC-40", "--reason", "   "]);
    let bodies = server.join().unwrap();
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(sent, serde_json::json!({"reason":null}));
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        text.lines().next().unwrap(),
        format!("{} !", glyph(reeve::glyphs::Level::Epic, "ABC-40")),
        "{text}"
    );
}

/// `--clear` is a bodiless DELETE on the same path and prints one fixed line.
#[test]
fn initiative_urgent_clear_sends_delete() {
    let init = serde_json::json!({"key":"ABC-2","title":"Leave","brief":"","horizon":"Now","target_month":null,
        "owner":null,"discovery_complete":null,"progress":null,"external_ref":null,"version":1,
        "urgent_mark":null,"urgent_sources":[]});
    let (url, server) = serve(vec![(
        "DELETE /api/v1/initiatives/ABC-2/urgent",
        200,
        ok(init),
    )]);
    let out = reeve_at(&url, &["initiative", "urgent", "ABC-2", "--clear"]);
    assert_eq!(server.join().unwrap(), vec![String::new()], "no body");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "urgent mark cleared on ABC-2\n"
    );
}

/// `--clear` with `--reason` is a clap conflict at every level, before any request (nothing listens on the port).
#[test]
fn urgent_clear_with_reason_is_a_usage_error() {
    for (level, key) in [
        ("initiative", "ABC-2"),
        ("epic", "ABC-40"),
        ("story", "TST-1"),
    ] {
        let out = Command::cargo_bin("reeve")
            .unwrap()
            .env("REEVE_URL", "http://127.0.0.1:9")
            .env("REEVE_TOKEN", "reeve_test")
            .args([level, "urgent", key, "--clear", "--reason", "x"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{level}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("cannot be used with"), "{level}: {err}");
        assert!(err.contains("--reason"), "{level}: {err}");
    }
}

/// Each level marks with POST and clears with DELETE on its own plural path; `serve` answers 500 and fails the test on
/// any other request line.
#[test]
fn urgent_uses_post_to_mark_and_delete_to_clear_at_every_level() {
    for (level, plural, key) in [
        ("initiative", "initiatives", "ABC-2"),
        ("epic", "epics", "ABC-40"),
        ("story", "stories", "TST-1"),
    ] {
        // A reason, so marking posts without reading the item first.
        let marks: [(&[&str], &str); 2] = [(&["--reason", "x"], "POST"), (&["--clear"], "DELETE")];
        for (extra, verb) in marks {
            let want: &'static str =
                Box::leak(format!("{verb} /api/v1/{plural}/{key}/urgent").into_boxed_str());
            let (url, server) = serve(vec![(
                want,
                404,
                serde_json::json!({"outcome":"error","code":"not_found","message":"not found"}),
            )]);
            let mut args = vec![level, "urgent", key];
            args.extend(extra);
            reeve_at(&url, &args);
            server.join().unwrap();
        }
    }
}

/// An automation token's refusal exits 2 and the human text goes to stderr, none to stdout.
#[test]
fn urgent_refused_interactive_only_exits_2() {
    let body = serde_json::json!({"outcome":"refused","reasons":[{"reason":"interactive_only"}],"overridable":false});
    let (url, server) = serve(vec![
        (
            "GET /api/v1/stories/TST-1",
            200,
            ok(marked_story_json("TST-1", None)),
        ),
        ("POST /api/v1/stories/TST-1/urgent", 403, body),
    ]);
    let out = reeve_at(&url, &["story", "urgent", "TST-1"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(2));
    // `render::reason(&Reason::InteractiveOnly)`; `render` is private to the crate, so the wording is spelled out.
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("only an interactive user may do this"),
        "{err}"
    );
    assert!(out.stdout.is_empty());
}

/// Marking again with no `--reason` keeps the current one: the item is read, then its reason is resent. A CLI that posts
/// null (wiping the reason) or skips the read fails.
#[test]
fn story_urgent_without_reason_resends_the_current_reason() {
    let (url, server) = serve(vec![
        (
            "GET /api/v1/stories/TST-1",
            200,
            ok(marked_story_json("TST-1", Some("old"))),
        ),
        (
            "POST /api/v1/stories/TST-1/urgent",
            200,
            ok(marked_story_json("TST-1", Some("old"))),
        ),
    ]);
    let out = reeve_at(&url, &["story", "urgent", "TST-1"]);
    let bodies = server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(bodies[0], "", "the read has no body");
    let sent: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(sent, serde_json::json!({"reason":"old"}));
}

/// `--reason ''` clears the reason: no read, a null reason posted. `serve` answers one request, so a read before the
/// POST would get the POST's expected line wrong and fail.
#[test]
fn story_urgent_with_an_empty_reason_posts_null_without_reading() {
    let (url, server) = serve(vec![(
        "POST /api/v1/stories/TST-1/urgent",
        200,
        ok(marked_story_json("TST-1", None)),
    )]);
    let out = reeve_at(&url, &["story", "urgent", "TST-1", "--reason", ""]);
    let bodies = server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(sent, serde_json::json!({"reason":null}));
}

/// A read that is not Ok is the command's outcome and nothing is posted: `serve` closes after the GET, so a POST would
/// fail to connect and change the exit code and stderr.
#[test]
fn epic_urgent_with_a_failed_read_posts_nothing() {
    let (url, server) = serve(vec![("GET /api/v1/epics/ABC-40", 403, forbidden())]);
    let out = reeve_at(&url, &["epic", "urgent", "ABC-40"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out_text(&out), "");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("forbidden"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---- war-room ls | clear | re-mark ----
//
// `serve` closes its listener after the last answer, so an unexpected POST makes the CLI fail: the exit-0 assert on
// a GET-only test is what proves nothing was posted.

/// A war-room row. `at_d` and `exp_d` are days from now: half days for a row built when the test starts, so floor and
/// ceiling are unambiguous; whole days are fine for a row built when its request is handled (`late`), since the CLI
/// reads its clock after the answer.
fn wr_row(
    key: &str,
    item: &str,
    initiative: Option<(&str, &str)>,
    (at_d, exp_d): (f64, f64),
    active: bool,
    done: bool,
) -> serde_json::Value {
    let off =
        |d: f64| chrono::Utc::now() + chrono::Duration::milliseconds((d * 86_400_000.0) as i64);
    serde_json::json!({
        "item": {"key": key, "item": item},
        "title": format!("T {key}"),
        "initiative": initiative.map(|(k, t)| serde_json::json!({"key": k, "title": t, "horizon": "h"})),
        "mark": {"at": off(at_d), "by": "dana@example.com", "reason": null,
                 "expires_at": off(exp_d), "active": active, "done": done}
    })
}

fn wr_active(key: &str, item: &str) -> serde_json::Value {
    wr_row(key, item, None, (-2.5, 9.5), true, false)
}
fn wr_expired(key: &str) -> serde_json::Value {
    wr_row(key, "story", None, (-12.5, -2.5), false, false)
}
fn wr_expired_done(key: &str) -> serde_json::Value {
    wr_row(key, "story", None, (-12.5, -2.5), false, true)
}

fn glyph(level: reeve::glyphs::Level, key: &str) -> String {
    reeve::glyphs::mark_with(level, key, false)
}

fn out_text(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

/// Rows print in the server's order with the initiative named only for epics and stories.
#[test]
fn war_room_ls_lists_rows() {
    let rows = serde_json::json!([
        wr_row(
            "ABC-40",
            "epic",
            Some(("ABC-2", "Payments")),
            (-2.5, 9.5),
            true,
            false
        ),
        wr_row("ABC-1", "story", None, (-12.5, -2.5), false, false),
    ]);
    let (url, server) = serve(vec![("GET /api/v1/war-room", 200, ok(rows))]);
    let out = reeve_at(&url, &["war-room", "ls"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out_text(&out),
        format!(
            "{} T ABC-40 · in Payments · marked by dana, 2 d ago · expires in 10 d\n{} T ABC-1 · marked by dana, 12 d ago · expired 2 d ago\n",
            glyph(reeve::glyphs::Level::Epic, "ABC-40"),
            glyph(reeve::glyphs::Level::Story, "ABC-1"),
        )
    );
}

#[test]
fn war_room_ls_empty() {
    let (url, server) = serve(vec![(
        "GET /api/v1/war-room",
        200,
        ok(serde_json::json!([])),
    )]);
    let out = reeve_at(&url, &["war-room", "ls"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out_text(&out), "The war room is empty.\n");
}

#[test]
fn war_room_clear_without_yes_rehearses() {
    let rows = serde_json::json!([wr_active("ABC-1", "story"), wr_expired("ABC-2")]);
    let (url, server) = serve(vec![("GET /api/v1/war-room", 200, ok(rows))]);
    let out = reeve_at(&url, &["war-room", "clear"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out_text(&out),
        format!(
            "{} T ABC-1 · marked by dana, 2 d ago · expires in 10 d\n{} T ABC-2 · marked by dana, 12 d ago · expired 2 d ago\nWould clear 2 marks; run again with --yes.\n",
            glyph(reeve::glyphs::Level::Story, "ABC-1"),
            glyph(reeve::glyphs::Level::Story, "ABC-2"),
        )
    );
}

#[test]
fn war_room_re_mark_without_yes_names_what_it_left_out() {
    let rows = serde_json::json!([
        wr_active("ABC-1", "story"),
        wr_expired("ABC-2"),
        wr_expired_done("ABC-3")
    ]);
    let (url, server) = serve(vec![("GET /api/v1/war-room", 200, ok(rows))]);
    let out = reeve_at(&url, &["war-room", "re-mark"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out_text(&out),
        format!(
            "{} T ABC-2 · marked by dana, 12 d ago · expired 2 d ago\nWould re-mark 1 mark for another cycle (1 on a done story left out); run again with --yes.\n",
            glyph(reeve::glyphs::Level::Story, "ABC-2")
        )
    );
}

/// `--yes` posts exactly the listed items, in order, and prints the server's new war room.
#[test]
fn war_room_clear_yes_posts_exactly_the_listed_items() {
    let rows = serde_json::json!([wr_active("ABC-40", "epic"), wr_expired("ABC-1")]);
    let (url, server) = serve(vec![
        ("GET /api/v1/war-room", 200, ok(rows)),
        (
            "POST /api/v1/war-room/clear",
            200,
            ok(serde_json::json!([])),
        ),
    ]);
    let out = reeve_at(&url, &["war-room", "clear", "--yes"]);
    let bodies = server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    // Parsed: the body's key order is serde_json's (sorted), the array order is the GET's.
    let sent: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"items":[{"key":"ABC-40","item":"epic"},{"key":"ABC-1","item":"story"}]})
    );
    assert_eq!(out_text(&out), "Cleared 2 marks.\nThe war room is empty.\n");
}

/// The `renew` alias posts only the expired, not-done item to the renew route.
#[test]
fn war_room_renew_alias_posts_only_expired_not_done() {
    let rows = serde_json::json!([
        wr_active("ABC-1", "story"),
        wr_expired("ABC-2"),
        wr_expired_done("ABC-3")
    ]);
    let (url, server) = serve(vec![
        ("GET /api/v1/war-room", 200, ok(rows)),
        (
            "POST /api/v1/war-room/renew",
            200,
            ok(serde_json::json!([])),
        ),
    ]);
    let out = reeve_at(&url, &["war-room", "renew", "--yes"]);
    let bodies = server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let sent: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"items":[{"key":"ABC-2","item":"story"}]})
    );
    assert_eq!(
        out_text(&out),
        "Re-marked 1 mark for another cycle.\nThe war room is empty.\n"
    );
}

/// `story ls` marks an urgent story's key column with ` !`, and keeps four tab-separated columns.
#[test]
fn story_ls_marks_an_urgent_story_in_the_first_column() {
    let src = vec![reeve_protocol::ItemRefDto {
        key: "ABC-1".parse().unwrap(),
        item: reeve_protocol::ItemType::Story,
    }];
    let row = serde_json::json!({
        "story": urgent_story(src),
        "readiness": {"ready": true, "reasons": []}
    });
    let (url, server) = serve(vec![(
        "GET /api/v1/stories",
        200,
        ok(serde_json::json!([row])),
    )]);
    let out = reeve_at(&url, &["story", "ls"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = out_text(&out);
    let cols: Vec<&str> = text.trim_end().split('\t').collect();
    assert_eq!(cols.len(), 4, "{text}");
    assert_eq!(
        cols[0],
        format!("{} !", glyph(reeve::glyphs::Level::Story, "ABC-1")),
        "{text}"
    );
}

/// `--json story urgent` prints the envelope with the marked story as its value.
#[test]
fn story_urgent_json_prints_the_envelope() {
    let (url, server) = serve(vec![
        (
            "GET /api/v1/stories/TST-1",
            200,
            ok(marked_story_json("TST-1", None)),
        ),
        (
            "POST /api/v1/stories/TST-1/urgent",
            200,
            ok(marked_story_json("TST-1", None)),
        ),
    ]);
    let out = reeve_at(&url, &["--json", "story", "urgent", "TST-1"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let env: serde_json::Value = serde_json::from_str(out_text(&out).trim()).unwrap();
    assert_eq!(env["outcome"], "ok");
    assert_eq!(env["value"]["key"], "TST-1");
}

/// `re-mark --yes` (not the alias) posts to the renew route and prints the server's answer like `ls`. The fake server
/// stamps the renewed mark when it handles the POST, so a CLI that measures the expiry from a clock read before its
/// requests says 15 d.
#[test]
fn war_room_re_mark_yes_prints_the_servers_answer_like_ls() {
    let rows = serde_json::json!([wr_expired("ABC-2")]);
    let (url, server) = serve_lazy(vec![
        ("GET /api/v1/war-room", 200, lazy(move || ok(rows))),
        (
            "POST /api/v1/war-room/renew",
            200,
            late(|| {
                ok(serde_json::json!([wr_row(
                    "ABC-2",
                    "story",
                    None,
                    (0.0, 14.0),
                    true,
                    false
                )]))
            }),
        ),
    ]);
    let out = reeve_at(&url, &["war-room", "re-mark", "--yes"]);
    let bodies = server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let sent: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"items":[{"key":"ABC-2","item":"story"}]})
    );
    assert_eq!(
        out_text(&out),
        format!(
            "Re-marked 1 mark for another cycle.\n{} T ABC-2 · marked by dana, today · expires in 14 d\n",
            glyph(reeve::glyphs::Level::Story, "ABC-2"),
        )
    );
}

/// A refused read posts nothing: `serve` closes after the GET, so an attempted POST would change the outcome.
#[test]
fn war_room_clear_yes_with_a_refused_get_posts_nothing() {
    let (url, server) = serve(vec![("GET /api/v1/war-room", 403, forbidden())]);
    let out = reeve_at(&url, &["war-room", "clear", "--yes"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out_text(&out), "");
    assert!(String::from_utf8_lossy(&out.stderr).contains("forbidden"));
}

/// Nothing selected: the GET only, exit 0, and the note about done stories.
#[test]
fn war_room_re_mark_with_only_done_expired_says_so() {
    let rows = serde_json::json!([wr_expired_done("ABC-3")]);
    let (url, server) = serve(vec![("GET /api/v1/war-room", 200, ok(rows))]);
    let out = reeve_at(&url, &["war-room", "re-mark", "--yes"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out_text(&out),
        "No expired marks to re-mark (1 on a done story left out).\n"
    );
}

#[test]
fn war_room_clear_json_without_yes_prints_the_get_envelope() {
    let rows = serde_json::json!([wr_active("ABC-1", "story")]);
    let (url, server) = serve(vec![("GET /api/v1/war-room", 200, ok(rows.clone()))]);
    let out = reeve_at(&url, &["--json", "war-room", "clear"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let env: serde_json::Value = serde_json::from_str(out_text(&out).trim()).unwrap();
    assert_eq!(env["outcome"], "ok");
    assert_eq!(env["value"], rows);
}

/// The client keeps no copy of the server's 100-item limit: 101 are posted and the refusal is printed.
#[test]
fn war_room_clear_101_rows_are_all_posted_and_the_server_refusal_printed() {
    let rows: Vec<_> = (1..=101)
        .map(|n| wr_active(&format!("ABC-{n}"), "story"))
        .collect();
    let refusal =
        serde_json::json!({"outcome":"error","code":"validation","message":"at most 100 items"});
    let (url, server) = serve(vec![
        ("GET /api/v1/war-room", 200, ok(serde_json::json!(rows))),
        ("POST /api/v1/war-room/clear", 422, refusal),
    ]);
    let out = reeve_at(&url, &["war-room", "clear", "--yes"]);
    let bodies = server.join().unwrap();
    let sent: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    assert_eq!(sent["items"].as_array().unwrap().len(), 101);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("at most 100 items"));
    assert_eq!(out_text(&out), "");
}

/// Answers one request with `status`, `content_type` and `body`; returns the request line.
fn serve_raw(
    status: u16,
    content_type: &'static str,
    body: &'static [u8],
) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let h = std::thread::spawn(move || {
        let stream = accept(&listener);
        let (line, _) = read_request(&stream);
        let mut w = stream;
        write!(
            w,
            "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        w.write_all(body).unwrap();
        line
    });
    (url, h)
}

const ATTACHMENT_ID: &str = "0193a6a2-7b1c-7d2e-9f00-000000000001";

fn get_into(url: &str, dir: &std::path::Path) -> std::process::Output {
    let cfg = tempfile::tempdir().unwrap();
    Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", url)
        .env("REEVE_TOKEN", "reeve_test")
        .env("REEVE_CONFIG_DIR", cfg.path())
        .args(["attachment", "get", ATTACHMENT_ID, "--json", "--dir"])
        .arg(dir)
        .timeout(Duration::from_secs(60))
        .output()
        .unwrap()
}

/// A missing --dir is created (parents too) owner-only, and the extension comes from the answer's type.
#[test]
fn attachment_get_creates_a_private_directory() {
    let (url, server) = serve_raw(200, "image/webp", b"RIFF....WEBP");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("a").join("b");
    let out = get_into(&url, &dir);
    assert_eq!(
        server.join().unwrap(),
        format!("GET /api/v1/attachments/{ATTACHMENT_ID} HTTP/1.1")
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let want = dir.join(format!("{ATTACHMENT_ID}.webp"));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["value"]["path"], want.to_str().unwrap());
    assert_eq!(std::fs::read(&want).unwrap(), b"RIFF....WEBP");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

/// An answer that is not PNG, JPEG or WebP (kb/ADR-0023 R1) is an error and nothing is written.
#[test]
fn attachment_get_refuses_another_content_type() {
    let (url, server) = serve_raw(200, "text/html", b"<html>");
    let tmp = tempfile::tempdir().unwrap();
    let out = get_into(&url, tmp.path());
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["code"], "internal");
    assert!(v["message"].as_str().unwrap().contains("text/html"), "{v}");
    assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
}

/// --dir is required: the CLI has no shared default location for private screenshots.
#[test]
fn attachment_get_requires_a_directory() {
    let out = Command::cargo_bin("reeve")
        .unwrap()
        .env("REEVE_URL", "http://127.0.0.1:9")
        .env("REEVE_TOKEN", "reeve_test")
        .args(["attachment", "get", ATTACHMENT_ID, "--json"])
        .timeout(Duration::from_secs(60))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--dir"));
}

/// A story as the promise routes answer it: ABC-12, promised on `on` by its own promise to `to`, open and due this
/// sprint (the server's derivation, never the CLI's).
fn promised_story(on: &str, to: Option<&str>) -> serde_json::Value {
    let mut s = urgent_story(vec![]);
    s.key = "ABC-12".parse().unwrap();
    s.promise = Some(reeve_protocol::StoryPromiseDto {
        own: Some(reeve_protocol::PromiseFactDto {
            on: on.parse().unwrap(),
            by: "dana@example.com".into(),
            at: "2026-11-12T09:00:00Z".parse().unwrap(),
            to: to.map(Into::into),
        }),
        epic: None,
        on: on.parse().unwrap(),
        holder: reeve_protocol::PromiseHolder::Own,
        state: Some(reeve_protocol::PromiseState::Open),
        due: Some(reeve_protocol::PromiseDue::ThisSprint),
        days: None,
    });
    serde_json::to_value(s).unwrap()
}

/// ABC-12 with its own promise (30 Nov, to Beta) and its epic ABC-7's (18 Nov, to Acme): the epic's earlier date counts,
/// due this sprint.
fn story_with_both_promises() -> serde_json::Value {
    let mut s = promised_story("2026-11-30", Some("Beta"));
    s["promise"]["epic"] = serde_json::json!({"key":"ABC-7","fact":{
        "on":"2026-11-18","by":"dana@example.com","at":"2026-11-12T09:00:00Z","to":"Acme"}});
    s["promise"]["on"] = "2026-11-18".into();
    s["promise"]["holder"] = "epic".into();
    s
}

/// ABC-12 promised only through its epic ABC-7 (18 Nov), due this sprint.
fn story_promised_via_epic() -> serde_json::Value {
    let mut s = story_with_both_promises();
    s["promise"]["own"] = serde_json::Value::Null;
    s
}

/// `story promise` sends a shortcut, a date, or only a recipient, and nothing it was not given (the server refuses
/// unknown fields, and a missing `to` keeps the recipient while `""` clears it).
#[test]
fn story_promise_sends_a_shortcut_a_date_or_only_a_recipient() {
    let cases: [(&[&str], serde_json::Value); 4] = [
        (
            &["--on", "this-sprint", "--to", "Acme"],
            serde_json::json!({"shortcut":"this_sprint","to":"Acme"}),
        ),
        (
            &["--on", "2026-11-20"],
            serde_json::json!({"on":"2026-11-20"}),
        ),
        (&["--to", "Beta"], serde_json::json!({"to":"Beta"})),
        (&["--to", ""], serde_json::json!({"to":""})),
    ];
    for (args, want) in cases {
        let (url, server) = serve(vec![(
            "POST /api/v1/stories/ABC-12/promise",
            200,
            ok(promised_story("2026-11-24", Some("Acme"))),
        )]);
        let mut all = vec!["--json", "story", "promise", "ABC-12"];
        all.extend_from_slice(args);
        let out = reeve_at(&url, &all);
        let bodies = server.join().unwrap();
        let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
        assert_eq!(sent, want, "{args:?}");
        // The body is one the server's own type accepts (it refuses unknown fields).
        serde_json::from_value::<reeve_protocol::SetPromise>(sent).unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        let env: serde_json::Value = serde_json::from_str(out_text(&out).trim()).unwrap();
        assert_eq!(env["outcome"], "ok");
        assert_eq!(env["value"]["promise"]["on"], "2026-11-24", "{env}");
    }
    let (url, server) = serve(vec![(
        "POST /api/v1/stories/ABC-12/promise",
        200,
        ok(promised_story("2026-11-24", Some("Acme"))),
    )]);
    let out = reeve_at(
        &url,
        &[
            "story",
            "promise",
            "ABC-12",
            "--on",
            "this-sprint",
            "--to",
            "Acme",
        ],
    );
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out_text(&out),
        format!(
            "{} · Promised by 24 Nov · this sprint · to Acme\n",
            glyph(reeve::glyphs::Level::Story, "ABC-12")
        )
    );
}

/// The answer confirms what was written. `--to ''` clears the recipient, so none is named. When the epic's earlier
/// date counts, the chip names the epic and the drawer line follows with both promises and the story's own recipient.
#[test]
fn story_promise_answers_with_what_was_written() {
    let story = glyph(reeve::glyphs::Level::Story, "ABC-12");
    let (url, server) = serve(vec![(
        "POST /api/v1/stories/ABC-12/promise",
        200,
        ok(promised_story("2026-11-24", None)),
    )]);
    let out = reeve_at(&url, &["story", "promise", "ABC-12", "--to", ""]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out_text(&out),
        format!("{story} · Promised by 24 Nov · this sprint\n")
    );

    let (url, server) = serve(vec![(
        "POST /api/v1/stories/ABC-12/promise",
        200,
        ok(story_with_both_promises()),
    )]);
    let out = reeve_at(
        &url,
        &[
            "story",
            "promise",
            "ABC-12",
            "--on",
            "2026-11-30",
            "--to",
            "Beta",
        ],
    );
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out_text(&out),
        format!(
            "{story} · Promised by 18 Nov · this sprint · via ABC-7\n  Promised by 30 Nov 2026 to Beta. Epic ABC-7 is promised by 18 Nov 2026 to Acme; the earlier date counts.\n"
        )
    );
}

/// `--clear` makes one DELETE and words its answer from it alone: whether it cleared, and the story after. A cleared
/// promise; an own promise cleared while the epic's still applies; no own promise while the epic's applies; nothing.
#[test]
fn story_promise_clear_prints_what_was_cleared() {
    let mut cleared = urgent_story(vec![]);
    cleared.key = "ABC-12".parse().unwrap();
    let cleared = serde_json::to_value(cleared).unwrap();
    let story = glyph(reeve::glyphs::Level::Story, "ABC-12");
    let answer = |cleared: bool, story: serde_json::Value| {
        ok(serde_json::json!({"cleared": cleared, "story": story}))
    };
    let rows: [(serde_json::Value, String); 4] = [
        (
            answer(true, cleared.clone()),
            format!("{story} · promise cleared\n"),
        ),
        (
            answer(true, story_promised_via_epic()),
            format!("{story} · own promise cleared; still promised by 18 Nov via ABC-7\n"),
        ),
        (
            answer(false, story_promised_via_epic()),
            format!("{story} · no own promise to clear; promised by 18 Nov via ABC-7\n"),
        ),
        (
            answer(false, cleared.clone()),
            format!("{story} · no promise to clear\n"),
        ),
    ];
    for (body, want) in rows {
        let (url, server) = serve(vec![("DELETE /api/v1/stories/ABC-12/promise", 200, body)]);
        let out = reeve_at(&url, &["story", "promise", "ABC-12", "--clear"]);
        let bodies = server.join().unwrap();
        assert_eq!(bodies, vec![String::new()], "one bodiless DELETE, no read");
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(out_text(&out), want);
    }

    // `--json` prints the DELETE's envelope.
    let (url, server) = serve(vec![(
        "DELETE /api/v1/stories/ABC-12/promise",
        200,
        answer(true, story_promised_via_epic()),
    )]);
    let out = reeve_at(&url, &["--json", "story", "promise", "ABC-12", "--clear"]);
    server.join().unwrap();
    let env: serde_json::Value = serde_json::from_str(&out_text(&out)).unwrap();
    assert_eq!(env["value"]["cleared"], true, "{env}");
    assert_eq!(env["value"]["story"]["promise"]["holder"], "epic", "{env}");

    // A DELETE that fails says so.
    let missing =
        serde_json::json!({"outcome":"error","code":"not_found","message":"no story ABC-12"});
    let (url, server) = serve(vec![(
        "DELETE /api/v1/stories/ABC-12/promise",
        404,
        missing,
    )]);
    let out = reeve_at(&url, &["story", "promise", "ABC-12", "--clear"]);
    assert_eq!(server.join().unwrap().len(), 1);
    assert_ne!(out.status.code(), Some(0));
}

/// Usage errors exit 1 (2 is kept for refusals) and never reach the server; each is clap's own message for its cause.
#[test]
fn story_promise_usage_errors_exit_1() {
    let rows: [(&[&str], &str); 9] = [
        (
            &[
                "story",
                "promise",
                "ABC-12",
                "--on",
                "2026-11-20",
                "--clear",
            ],
            "cannot be used with",
        ),
        (
            &["story", "promise", "ABC-12", "--to", "Acme", "--clear"],
            "cannot be used with",
        ),
        (
            &["epic", "promise", "ABC-7", "--on", "this-sprint", "--clear"],
            "cannot be used with",
        ),
        (
            &["story", "promise", "ABC-12"],
            "required arguments were not provided",
        ),
        (
            &["epic", "promise", "ABC-7"],
            "required arguments were not provided",
        ),
        (
            &["story", "promise", "ABC-12", "--on", "2026-13-01"],
            "invalid value",
        ),
        (
            &["story", "promise", "ABC-12", "--on", "someday"],
            "invalid value",
        ),
        (
            &["story", "promise", "ABC-12", "--on", "2026-1-5"],
            "invalid value",
        ),
        (
            &["epic", "promise", "ABC-7", "--on", "2026-11-5"],
            "invalid value",
        ),
    ];
    for (args, want) in rows {
        let out = reeve_at("http://127.0.0.1:9", args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(want), "{args:?}: {err}");
    }
}

/// A refusal (`interactive_only`) exits 2 with the reason; a validation answer is an error, exit 1, with the server's
/// message.
#[test]
fn a_refused_promise_exits_2_and_prints_the_server_text() {
    let refusal = serde_json::json!({"outcome":"refused","reasons":[{"reason":"interactive_only"}],"overridable":false});
    let (url, server) = serve(vec![("POST /api/v1/stories/ABC-12/promise", 422, refusal)]);
    let out = reeve_at(&url, &["story", "promise", "ABC-12", "--on", "2026-11-20"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("only an interactive user may do this"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let invalid = serde_json::json!({"outcome":"error","code":"validation","message":"the date is in the past"});
    let (url, server) = serve(vec![("POST /api/v1/stories/ABC-12/promise", 400, invalid)]);
    let out = reeve_at(&url, &["story", "promise", "ABC-12", "--on", "2026-11-20"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("the date is in the past"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn promised_epic(past: bool) -> serde_json::Value {
    serde_json::to_value(reeve_protocol::EpicDto {
        key: "ABC-7".parse().unwrap(),
        title: "Hosting".into(),
        initiative: None,
        brief: String::new(),
        discovery_complete: None,
        external_ref: None,
        version: 1,
        urgent_mark: None,
        urgent_sources: vec![],
        promise: Some(reeve_protocol::EpicPromiseDto {
            fact: reeve_protocol::PromiseFactDto {
                on: "2026-11-27".parse().unwrap(),
                by: "dana@example.com".into(),
                at: "2026-11-12T09:00:00Z".parse().unwrap(),
                to: Some("Acme".into()),
            },
            due: if past {
                None
            } else {
                Some(reeve_protocol::PromiseDue::NextSprint)
            },
            past,
        }),
    })
    .unwrap()
}

#[test]
fn epic_promise_works_the_same() {
    let (url, server) = serve(vec![(
        "POST /api/v1/epics/ABC-7/promise",
        200,
        ok(promised_epic(false)),
    )]);
    let out = reeve_at(
        &url,
        &[
            "epic",
            "promise",
            "ABC-7",
            "--on",
            "next-sprint",
            "--to",
            "Acme",
        ],
    );
    let bodies = server.join().unwrap();
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"shortcut":"next_sprint","to":"Acme"})
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out_text(&out),
        format!(
            "{} · Promised by 27 Nov · next sprint · to Acme\n",
            glyph(reeve::glyphs::Level::Epic, "ABC-7")
        )
    );

    let mut cleared = promised_epic(false);
    cleared["promise"] = serde_json::Value::Null;
    let epic = glyph(reeve::glyphs::Level::Epic, "ABC-7");
    for (was, want) in [
        (true, format!("{epic} · promise cleared\n")),
        (false, format!("{epic} · no promise to clear\n")),
    ] {
        let (url, server) = serve(vec![(
            "DELETE /api/v1/epics/ABC-7/promise",
            200,
            ok(serde_json::json!({"cleared": was, "epic": cleared.clone()})),
        )]);
        let out = reeve_at(&url, &["epic", "promise", "ABC-7", "--clear"]);
        assert_eq!(server.join().unwrap(), vec![String::new()]);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(out_text(&out), want);
    }

    let refusal = serde_json::json!({"outcome":"refused","reasons":[{"reason":"interactive_only"}],"overridable":false});
    let (url, server) = serve(vec![("POST /api/v1/epics/ABC-7/promise", 422, refusal)]);
    let out = reeve_at(&url, &["epic", "promise", "ABC-7", "--to", "Beta"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// `--promised this-sprint` reaches the wire as `promised=this_sprint`, and each row shows its chip.
#[test]
fn story_ls_promised_sends_the_parameter_and_shows_the_date() {
    let (line, _) = capture(&["story", "ls", "--promised", "this-sprint"]);
    assert!(line.contains("promised=this_sprint"), "{line}");
    let (line, _) = capture(&["story", "ls", "--promised", "any"]);
    assert!(line.contains("promised=any"), "{line}");

    let rows = serde_json::json!([{
        "story": promised_story("2026-11-24", Some("Acme")),
        "readiness": {"ready": true, "reasons": []},
    }]);
    let (url, server) = serve(vec![(
        "GET /api/v1/stories?promised=this_sprint",
        200,
        ok(rows),
    )]);
    let out = reeve_at(&url, &["story", "ls", "--promised", "this-sprint"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = out_text(&out);
    assert!(
        text.trim_end()
            .ends_with("Match · Promised by 24 Nov · this sprint"),
        "{text}"
    );
}

/// `story show` and `epic show` give the promise line right after the first line, and the first line ends with the
/// chip.
#[test]
fn show_prints_the_promise_chip_and_line() {
    let (url, server) = serve(vec![(
        "GET /api/v1/stories/ABC-12",
        200,
        ok(promised_story("2026-11-24", Some("Acme"))),
    )]);
    let out = reeve_at(&url, &["story", "show", "ABC-12"]);
    server.join().unwrap();
    let text = out_text(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].ends_with("[todo] Match · Promised by 24 Nov · this sprint"),
        "{text}"
    );
    assert_eq!(
        lines[1], "  Promised by 24 Nov 2026 to Acme · set by dana",
        "{text}"
    );

    let (url, server) = serve(vec![
        ("GET /api/v1/epics/ABC-7", 200, ok(promised_epic(true))),
        (
            "GET /api/v1/epics/ABC-7/findings",
            200,
            ok(serde_json::json!([])),
        ),
        (
            "GET /api/v1/items/ABC-7/comments",
            200,
            ok(serde_json::json!([])),
        ),
    ]);
    let out = reeve_at(&url, &["epic", "show", "ABC-7"]);
    server.join().unwrap();
    let text = out_text(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].contains(" Hosting · Past 27 Nov (version: 1)"),
        "{text}"
    );
    assert_eq!(
        lines[1], "  Promised by 27 Nov 2026 to Acme · set by dana · past",
        "{text}"
    );
}

/// `n` rows as `GET /stories` answers them, ABC-1 to ABC-n.
fn story_rows(n: usize) -> serde_json::Value {
    (1..=n)
        .map(|i| {
            let mut s = urgent_story(vec![]);
            s.key = format!("ABC-{i}").parse().unwrap();
            serde_json::json!({"story": s, "readiness": {"ready": true, "reasons": []}})
        })
        .collect()
}

fn stderr_text(out: &std::process::Output) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}

/// The lines of standard error that start with `more:`.
fn more_lines(out: &std::process::Output) -> Vec<String> {
    stderr_text(out)
        .lines()
        .filter(|l| l.starts_with("more:"))
        .map(str::to_string)
        .collect()
}

/// `--limit` and `--offset` reach the wire as `limit` and `offset`; after a page of exactly `--limit` rows the flags
/// for the next page are on standard error, and standard output holds the rows only.
#[test]
fn story_ls_sends_limit_and_offset_and_says_more_on_stderr_after_a_full_page() {
    let (url, server) = serve(vec![(
        "GET /api/v1/stories?limit=2&offset=2",
        200,
        ok(story_rows(2)),
    )]);
    let out = reeve_at(&url, &["story", "ls", "--limit", "2", "--offset", "2"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_text(&out));
    let text = out_text(&out);
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(!text.contains("more:"), "{text}");
    assert_eq!(more_lines(&out), vec!["more: --limit 2 --offset 4"]);

    // No --offset: none is sent, and the next page starts at the limit.
    let (url, server) = serve(vec![(
        "GET /api/v1/stories?limit=2",
        200,
        ok(story_rows(2)),
    )]);
    let out = reeve_at(&url, &["story", "ls", "--limit", "2"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_text(&out));
    assert_eq!(more_lines(&out), vec!["more: --limit 2 --offset 2"]);

    // --offset alone is sent as given: refusing it is the server's rule, not a second copy in the CLI.
    let (line, _) = capture(&["story", "ls", "--offset", "5"]);
    assert!(line.starts_with("GET /api/v1/stories?offset=5 "), "{line}");
}

/// A page shorter than the limit, an empty page, and a list read without a limit say nothing more.
#[test]
fn story_ls_says_nothing_more_after_a_short_page_or_without_a_limit() {
    for (want, args, rows) in [
        (
            "GET /api/v1/stories?limit=3",
            &["story", "ls", "--limit", "3"][..],
            2,
        ),
        (
            "GET /api/v1/stories?limit=3&offset=6",
            &["story", "ls", "--limit", "3", "--offset", "6"][..],
            0,
        ),
        ("GET /api/v1/stories", &["story", "ls"][..], 3),
    ] {
        let (url, server) = serve(vec![(want, 200, ok(story_rows(rows)))]);
        let out = reeve_at(&url, args);
        server.join().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            stderr_text(&out)
        );
        assert_eq!(more_lines(&out), Vec::<String>::new(), "{args:?}");
        assert!(!out_text(&out).contains("more:"), "{args:?}");
    }
}

/// With `--json` a full page prints the envelope, unchanged, on standard output and nothing about more pages on
/// either stream.
#[test]
fn story_ls_json_prints_only_the_envelope_after_a_full_page() {
    let rows = story_rows(2);
    let (url, server) = serve(vec![("GET /api/v1/stories?limit=2", 200, ok(rows.clone()))]);
    let out = reeve_at(&url, &["--json", "story", "ls", "--limit", "2"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_text(&out));
    let text = out_text(&out);
    assert_eq!(text.lines().count(), 1, "{text}");
    let printed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(printed, ok(rows), "the envelope the server sent");
    assert_eq!(more_lines(&out), Vec::<String>::new());
}

/// `--limit` and `--offset` take whole numbers: anything else is refused by clap, exit 1, before any request.
#[test]
fn story_ls_refuses_a_limit_or_offset_that_is_not_a_whole_number() {
    for (args, says) in [
        (
            &["story", "ls", "--limit", "x"][..],
            "invalid value 'x' for '--limit <LIMIT>'",
        ),
        (
            &["story", "ls", "--limit", "2", "--offset", "x"][..],
            "invalid value 'x' for '--offset <OFFSET>'",
        ),
        (&["story", "ls", "--limit", "-1"][..], "unexpected argument"),
    ] {
        let out = Command::cargo_bin("reeve")
            .unwrap()
            .env("REEVE_URL", "http://127.0.0.1:9")
            .env("REEVE_TOKEN", "reeve_test")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err = stderr_text(&out);
        // A request to the closed port would also exit 1; only clap writes these texts. A value-validation error
        // carries no usage line in clap 4.6 (`a_malformed_key_argument_is_a_usage_error` asserts the same way).
        assert!(err.contains(says), "{args:?}: {err}");
    }
}

/// The server's `validation` answer to `--limit 0` is printed as any error is, exit 1, with no `more:` line. It would
/// catch a `more:` line printed for an error (a CLI that decides from its flags, not from a page it was answered).
#[test]
fn story_ls_prints_a_validation_error_and_nothing_more() {
    let invalid = serde_json::json!({"outcome":"error","code":"validation","message":"limit must be 1..=100"});
    let (url, server) = serve(vec![("GET /api/v1/stories?limit=0", 400, invalid)]);
    let out = reeve_at(&url, &["story", "ls", "--limit", "0"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr_text(&out));
    assert_eq!(
        stderr_text(&out).trim_end(),
        "error (Validation): limit must be 1..=100"
    );
    assert_eq!(more_lines(&out), Vec::<String>::new());
    assert_eq!(out_text(&out), "");
}

/// A connection as the server lists it. `ends_at` is far in the future or the past, so what the text says about
/// it does not depend on the day the test runs.
fn connection_row(id: i64, ends_at: &str, revoked: Option<(&str, &str)>) -> serde_json::Value {
    serde_json::json!({
        "id": id, "owner": "dan@example.com", "client_id": "https://claude.ai/oauth/client.json",
        "client_host": "claude.ai", "client_name": "Claude", "scope": if id == 7 { "write" } else { "read" },
        "approved_at": "2020-01-01T09:00:00Z", "ends_at": ends_at, "refreshed_at": null,
        "revoked_at": revoked.map(|(at, _)| at), "revoked_reason": revoked.map(|(_, reason)| reason)
    })
}

/// Catches a listing sent anywhere but `GET /api/v1/connections`, `--owner` not sent as the query, `--json` that
/// is not the server's envelope, and a text line that lacks the id a person needs for `revoke`, the app, or why a
/// connection stopped.
#[test]
fn auth_connections_lists_connections() {
    let rows = serde_json::json!([
        connection_row(7, "2099-12-01T09:00:00Z", None),
        connection_row(
            8,
            "2099-12-01T09:00:00Z",
            Some(("2020-01-10T09:00:00Z", "refresh_token_reused"))
        ),
        connection_row(9, "2020-01-31T09:00:00Z", None),
    ]);
    let (url, server) = serve(vec![("GET /api/v1/connections", 200, ok(rows.clone()))]);
    let out = reeve_at(&url, &["--json", "auth", "connections"]);
    server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap(),
        ok(rows.clone())
    );

    let (url, server) = serve(vec![("GET /api/v1/connections", 200, ok(rows))]);
    let out = reeve_at(&url, &["auth", "connections"]);
    server.join().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "7\tclaude.ai (Claude)\tread and change\tends 2099-12-01",
            "8\tclaude.ai (Claude)\tread\trevoked 2020-01-10: the app's sign-in went out of step",
            "9\tclaude.ai (Claude)\tread\tended 2020-01-31",
        ]
    );

    // An admin names another person; the request line carries the query.
    let (url, server) = serve(vec![(
        "GET /api/v1/connections?owner=bob%40example.com",
        200,
        ok(serde_json::json!([])),
    )]);
    let out = reeve_at(&url, &["auth", "connections", "--owner", "bob@example.com"]);
    server.join().unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        "no connected apps"
    );
}

/// Catches a revoke sent to the wrong address or with the wrong id, and a refusal that exits as a success.
#[test]
fn auth_connections_revoke_deletes_that_connection() {
    let (url, server) = serve(vec![(
        "DELETE /api/v1/connections/7",
        200,
        ok(serde_json::json!({})),
    )]);
    let out = reeve_at(&url, &["auth", "connections", "revoke", "7"]);
    server.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        "revoked connection 7"
    );

    let not_found = serde_json::json!({"outcome":"error","code":"not_found","message":"not found"});
    let (url, server) = serve(vec![("DELETE /api/v1/connections/9", 404, not_found)]);
    let out = reeve_at(&url, &["--json", "auth", "connections", "revoke", "9"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["code"],
        "not_found"
    );

    // A missing or non-numeric id is a usage error. The message is clap's, so the refusal is the argument's and
    // not a failed connection (which exits 1 too).
    for (args, message) in [
        (vec!["auth", "connections", "revoke"], "required"),
        (
            vec!["auth", "connections", "revoke", "seven"],
            "invalid value",
        ),
    ] {
        let out = reeve_at("http://127.0.0.1:9", &args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }
}
