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
                (status, body.to_string())
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
        {"key": "TST-3", "item": "story", "kind": "bug", "title": "Move the database", "matched": ["description"]},
        {"key": "TST-1", "item": "initiative", "kind": null, "title": "Leave the old host", "matched": ["title", "external_ref"]}
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
        ok(queue_body())
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
        "suggested_at":null,"suggested_by":null,"verdict":null,"verdict_note":null,"version":1,"links":[]})
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

/// The active sprint's `--json` view has neither `at_close` nor `changed_since_close`, and reads no activity.
#[test]
fn an_active_sprint_shows_no_counts_at_close() {
    let (url, server) = serve(vec![(
        "GET /api/v1/sprints",
        200,
        ok(
            serde_json::json!({"active":view_json("active", counts_json([(12, 1), (13, 0)])),"draft":null,"closed":[]}),
        ),
    )]);
    let out = reeve_at(&url, &["--json", "sprint", "show"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["value"].get("at_close").is_none(), "{v}");
    assert!(v["value"].get("changed_since_close").is_none(), "{v}");
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
