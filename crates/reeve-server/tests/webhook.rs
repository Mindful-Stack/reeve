mod common;
use axum::body::Body;
use common::*;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};

const REPO: &str = "Acme/portal";

fn hmac_sha256(secret: &str, body: &str) -> String {
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    m.update(body.as_bytes());
    format!("sha256={}", hex::encode(m.finalize().into_bytes()))
}

/// A fixture with its placeholders substituted.
fn payload(fixture: &str, branch: &str, repo: &str, number: i64, updated: &str) -> String {
    let raw = match fixture {
        "pr_opened" => include_str!("fixtures/github/pr_opened.json"),
        "pr_closed_merged" => include_str!("fixtures/github/pr_closed_merged.json"),
        "pr_converted_to_draft" => include_str!("fixtures/github/pr_converted_to_draft.json"),
        "pr_closed_unmerged" => include_str!("fixtures/github/pr_closed_unmerged.json"),
        "pr_reopened" => include_str!("fixtures/github/pr_reopened.json"),
        "pr_from_fork" => include_str!("fixtures/github/pr_from_fork.json"),
        "pr_from_deleted_fork" => include_str!("fixtures/github/pr_from_deleted_fork.json"),
        "ping" => include_str!("fixtures/github/ping.json"),
        other => panic!("no fixture {other}"),
    };
    let body = raw
        .replace("BRANCH", branch)
        .replace("REPO", repo)
        .replace("NUMBER", &number.to_string())
        .replace("UPDATED", updated);
    serde_json::from_str::<Value>(&body).expect("fixture is JSON once substituted");
    body
}

/// Posts to the webhook with the given event and signature header (none: no header).
async fn post(h: &Harness, event: &str, body: &str, signature: Option<&str>) -> (u16, Value) {
    let mut req = axum::http::Request::builder()
        .method("POST")
        .uri("/webhooks/github")
        .header("content-type", "application/json")
        .header("x-github-event", event)
        .header("x-github-delivery", "72d3162e-cc78-11e3-81ab-4c9367dc0958");
    if let Some(sig) = signature {
        req = req.header("x-hub-signature-256", sig);
    }
    let res = call_raw(&h.app, req.body(Body::from(body.to_string())).unwrap()).await;
    let status = res.status().as_u16();
    use http_body_util::BodyExt;
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, v)
}

async fn post_signed(h: &Harness, event: &str, body: &str) -> (u16, Value) {
    post(h, event, body, Some(&hmac_sha256(WEBHOOK_SECRET, body))).await
}

async fn create(h: &Harness, path: &str, body: Value) -> String {
    let (st, v) = call(&h.app, Some(&h.admin_token), "POST", path, Some(body)).await;
    assert_eq!(st, 200, "{v}");
    v["value"]["key"].as_str().unwrap().to_string()
}

/// A todo story with no epic and no criteria.
async fn seed_story(h: &Harness, idem: &str) -> String {
    create(
        h,
        "/api/v1/stories",
        json!({"idempotency_key":idem,"title":"Login","kind":"story","criteria":[],"labels":[]}),
    )
    .await
}

async fn get_story(h: &Harness, key: &str) -> Value {
    let (st, v) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/stories/{key}"),
        None,
    )
    .await;
    assert_eq!(st, 200, "{v}");
    v["value"].clone()
}

async fn activity(h: &Harness, query: &str) -> Vec<Value> {
    let (st, v) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/activity?limit=500&{query}"),
        None,
    )
    .await;
    assert_eq!(st, 200, "{v}");
    v["value"].as_array().unwrap().clone()
}

async fn pr_rows(h: &Harness) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM story_prs")
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn webhook_links_by_branch_updates_status_and_never_moves() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let branch = format!("{key}-fix-login");
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload("pr_opened", &branch, REPO, 42, "2026-10-26T10:00:00Z"),
    )
    .await;
    assert_eq!((st, v["result"].as_str()), (202, Some("linked")), "{v}");
    let s = get_story(&h, &key).await;
    assert_eq!(s["prs"][0]["status"], "open");
    assert_eq!(s["prs"][0]["number"], 42);
    assert_eq!(s["prs"][0]["repo"], "acme/portal", "stored lower-cased");
    assert_eq!(s["prs"][0]["title"], "Fix the login redirect");
    assert_eq!(s["prs"][0]["url"], "https://github.com/Acme/portal/pull/42");
    // a path segment before the key and a lowercase key resolve too
    let (st, _) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_converted_to_draft",
            &format!("feature/{}", branch.to_lowercase()),
            REPO,
            42,
            "2026-10-26T10:30:00Z",
        ),
    )
    .await;
    assert_eq!(st, 202);
    assert_eq!(get_story(&h, &key).await["prs"][0]["status"], "draft");
    let (st, _) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_closed_merged",
            &branch,
            REPO,
            42,
            "2026-10-26T11:00:00Z",
        ),
    )
    .await;
    assert_eq!(st, 202);
    let s = get_story(&h, &key).await;
    assert_eq!(s["prs"][0]["status"], "merged");
    assert_eq!(s["delivery"], "todo");
    assert_eq!(s["version"], 1);
}

/// Catches one pull request spelled two ways by GitHub (`Mindful-Stack/Reeve`, `mindful-stack/reeve`) stored as two
/// rows: the repository is lower-cased before the allowlist check, the row and every lookup.
#[tokio::test]
async fn webhook_treats_repository_names_case_insensitively() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let branch = format!("{key}-fix");
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_opened",
            &branch,
            "ACME/Portal",
            9,
            "2026-10-26T10:00:00Z",
        ),
    )
    .await;
    assert_eq!((st, v["result"].as_str()), (202, Some("linked")), "{v}");
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_closed_merged",
            &branch,
            "acme/portal",
            9,
            "2026-10-26T11:00:00Z",
        ),
    )
    .await;
    assert_eq!((st, v["result"].as_str()), (202, Some("updated")), "{v}");
    assert_eq!(pr_rows(&h).await, 1, "no duplicate link");
    let s = get_story(&h, &key).await;
    assert_eq!(s["prs"].as_array().unwrap().len(), 1);
    assert_eq!(s["prs"][0]["repo"], "acme/portal");
    assert_eq!(s["prs"][0]["status"], "merged");
}

#[tokio::test]
async fn webhook_never_moves_a_story_that_could_move() {
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let ik = create(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"i","title":"I","horizon":"Now","owner":"dan@example.com"}),
    )
    .await;
    let ek = create(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"e","title":"E","initiative":ik}),
    )
    .await;
    for path in [
        format!("/api/v1/initiatives/{ik}"),
        format!("/api/v1/epics/{ek}"),
    ] {
        let (_, cur) = call(&h.app, t, "GET", &path, None).await;
        let (st, d) = call(
            &h.app,
            t,
            "POST",
            &format!("{path}/discovery"),
            Some(json!({"version":cur["value"]["version"],"complete":true})),
        )
        .await;
        assert_eq!(st, 200, "{d}");
    }
    let story = |idem: &str| json!({"idempotency_key":idem,"title":idem,"kind":"story","epic":ek,"criteria":["works"],"labels":[]});
    let ready = create(&h, "/api/v1/stories", story("ready")).await;
    let active = create(&h, "/api/v1/stories", story("active")).await;
    let (_, r) = call(
        &h.app,
        t,
        "GET",
        &format!("/api/v1/stories/{ready}/ready"),
        None,
    )
    .await;
    assert_eq!(r["value"]["ready"], true, "{r}");
    let (st, m) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{active}/move"),
        Some(json!({"version":1,"to":"in_progress"})),
    )
    .await;
    assert_eq!(st, 200, "{m}");

    // The baseline is taken after seeding, so the seeding move is in it.
    let moved = activity(&h, "action=moved").await.len();
    let overrides = activity(&h, "action=override").await.len();
    assert_eq!(moved, 1, "the seeding move is counted");
    let before = [get_story(&h, &ready).await, get_story(&h, &active).await];

    for (n, key) in [(1, &ready), (2, &active)] {
        let (st, v) = post_signed(
            &h,
            "pull_request",
            &payload(
                "pr_closed_merged",
                &format!("{key}-work"),
                REPO,
                n,
                "2026-10-26T11:00:00Z",
            ),
        )
        .await;
        assert_eq!((st, v["result"].as_str()), (202, Some("linked")), "{v}");
    }
    for (key, was) in [&ready, &active].into_iter().zip(&before) {
        let now = get_story(&h, key).await;
        assert_eq!(now["prs"][0]["status"], "merged");
        for field in ["delivery", "resolution", "version"] {
            assert_eq!(now[field], was[field], "{key} {field}");
        }
    }
    assert_eq!(before[0]["delivery"], "todo");
    assert_eq!(before[1]["delivery"], "in_progress");
    assert_eq!(activity(&h, "action=moved").await.len(), moved);
    assert_eq!(activity(&h, "action=override").await.len(), overrides);
}

#[tokio::test]
async fn webhook_rejects_bad_signatures_and_ignores_what_it_must() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let body = payload(
        "pr_opened",
        &format!("{key}-x"),
        REPO,
        7,
        "2026-10-26T10:00:00Z",
    );
    let good = hmac_sha256(WEBHOOK_SECRET, &body);

    // wrong 32-byte MAC
    let zeros = format!("sha256={}", hex::encode([0u8; 32]));
    assert_eq!(post(&h, "pull_request", &body, Some(&zeros)).await.0, 401);
    // a changed body with the original (now stale) signature
    let tampered = body.replace("\"number\": 7", "\"number\": 8");
    assert_ne!(tampered, body);
    assert_eq!(
        post(&h, "pull_request", &tampered, Some(&good)).await.0,
        401
    );
    // missing header, malformed headers
    assert_eq!(post(&h, "pull_request", &body, None).await.0, 401);
    assert_eq!(
        post(&h, "pull_request", &body, Some("sha256=zz")).await.0,
        401
    );
    assert_eq!(
        post(&h, "pull_request", &body, Some(&good["sha256=".len()..]))
            .await
            .0,
        401
    );
    assert_eq!(pr_rows(&h).await, 0);

    // a signed event that is not a pull request: accepted, nothing stored
    let (st, _) = post_signed(&h, "issues", &body).await;
    assert_eq!(st, 202);
    assert_eq!(pr_rows(&h).await, 0);
    // ping: 200; a ping must be signed too
    let ping = payload("ping", "", "", 0, "");
    let (st, v) = post_signed(&h, "ping", &ping).await;
    assert_eq!((st, &v["ok"]), (200, &json!(true)));
    assert_eq!(post(&h, "ping", &ping, None).await.0, 401);
    // a signed body that is not a pull_request payload
    assert_eq!(post_signed(&h, "pull_request", "[1,2]").await.0, 400);

    // a foreign repository whose branch names a seeded story: the story is untouched
    let before = get_story(&h, &key).await;
    let entries = activity(&h, &format!("entity_key={key}")).await.len();
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_opened",
            &format!("{key}-x"),
            "Other/repo",
            7,
            "2026-10-26T10:00:00Z",
        ),
    )
    .await;
    assert_eq!(st, 202, "{v}");
    let after = get_story(&h, &key).await;
    assert_eq!(after["prs"], json!([]));
    assert_eq!(after["version"], before["version"]);
    assert_eq!(
        activity(&h, &format!("entity_key={key}")).await.len(),
        entries
    );
    // the configured repository matches case-insensitively
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_opened",
            &format!("{key}-x"),
            "acme/PORTAL",
            9,
            "2026-10-26T10:00:00Z",
        ),
    )
    .await;
    assert_eq!((st, v["result"].as_str()), (202, Some("linked")), "{v}");
    assert_eq!(pr_rows(&h).await, 1);

    // an unknown branch, a non-existent key, an unknown legacy key: accepted, nothing stored
    for branch in ["main", "TST-9999-gone", "SCRUM-404-old", "OTHER-1"] {
        let (st, v) = post_signed(
            &h,
            "pull_request",
            &payload("pr_opened", branch, REPO, 10, "2026-10-26T10:00:00Z"),
        )
        .await;
        assert_eq!(st, 202, "{branch}: {v}");
    }
    assert_eq!(pr_rows(&h).await, 1);
}

#[tokio::test]
async fn webhook_answers_404_when_not_configured() {
    let h = harness_with(|cfg| {
        cfg.replace(
            &format!(
                "[github]\nwebhook_secret = \"{WEBHOOK_SECRET}\"\nrepos = [\"Acme/portal\"]\nlegacy_prefixes = [\"SCRUM\"]"
            ),
            "",
        )
    })
    .await;
    assert!(h.state.config.github.is_none());
    let body = payload("ping", "", "", 0, "");
    assert_eq!(post_signed(&h, "ping", &body).await.0, 404);
}

#[tokio::test]
async fn webhook_is_idempotent_under_redelivery_and_resolves_legacy_prefixes() {
    let h = harness_with(|cfg| format!("{cfg}\n[import]\nenabled = true")).await;
    let key = seed_story(&h, "s1").await;
    let opened = payload(
        "pr_opened",
        &format!("{key}-x"),
        REPO,
        5,
        "2026-10-26T10:00:00Z",
    );
    for _ in 0..2 {
        assert_eq!(post_signed(&h, "pull_request", &opened).await.0, 202);
    }
    assert_eq!(pr_rows(&h).await, 1);
    let linked = activity(&h, &format!("entity_key={key}&action=pr_linked")).await;
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["actor"], "github");
    assert_eq!(linked[0]["via"], "system");
    assert_eq!(linked[0]["payload"]["title"], "Fix the login redirect");

    // an older closed payload arriving late: ignored, no entry
    let entries = activity(&h, &format!("entity_key={key}")).await.len();
    let stale = payload(
        "pr_closed_merged",
        &format!("{key}-x"),
        REPO,
        5,
        "2026-10-26T09:00:00Z",
    );
    let (st, v) = post_signed(&h, "pull_request", &stale).await;
    assert_eq!(
        (st, v["result"].as_str()),
        (202, Some("ignored: stale or repeated"))
    );
    assert_eq!(get_story(&h, &key).await["prs"][0]["status"], "open");
    assert_eq!(
        activity(&h, &format!("entity_key={key}")).await.len(),
        entries
    );

    // a pre-import branch of the delivery project (the harness's legacy prefix) resolves to the story the import
    // created from that Jira key, through its external_ref
    let (st, imported) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/stories",
        Some(json!({
            "external_ref": "SCRUM-11", "title": "Legacy", "kind": "story", "epic": null, "description": "",
            "criteria": [], "owner": null, "labels": ["legacy-import"], "delivery": "in_progress", "resolution": null
        })),
    )
    .await;
    assert_eq!(st, 200, "{imported}");
    let legacy = imported["value"]["key"].as_str().unwrap().to_string();
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload("pr_opened", "SCRUM-11-old", REPO, 6, "2026-10-26T10:00:00Z"),
    )
    .await;
    assert_eq!((st, v["result"].as_str()), (202, Some("linked")), "{v}");
    let s = get_story(&h, &legacy).await;
    assert_eq!(s["prs"][0]["number"], 6);
    let entry = activity(&h, &format!("entity_key={legacy}&action=pr_linked")).await;
    assert_eq!(entry.len(), 1);
    assert_eq!(
        (&entry[0]["actor"], &entry[0]["via"]),
        (&json!("github"), &json!("system"))
    );
}

#[tokio::test]
async fn webhook_maps_closed_without_merge_and_reopened() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let branch = format!("{key}-x");
    let status = |fixture: &'static str, at: &'static str| payload(fixture, &branch, REPO, 9, at);
    let (_, v) = post_signed(
        &h,
        "pull_request",
        &status("pr_opened", "2026-10-26T10:00:00Z"),
    )
    .await;
    assert_eq!(v["result"], "linked", "{v}");
    let (_, v) = post_signed(
        &h,
        "pull_request",
        &status("pr_closed_unmerged", "2026-10-26T11:00:00Z"),
    )
    .await;
    assert_eq!(v["result"], "updated", "{v}");
    assert_eq!(get_story(&h, &key).await["prs"][0]["status"], "closed");
    let (_, v) = post_signed(
        &h,
        "pull_request",
        &status("pr_reopened", "2026-10-26T12:00:00Z"),
    )
    .await;
    assert_eq!(v["result"], "updated", "{v}");
    assert_eq!(get_story(&h, &key).await["prs"][0]["status"], "open");
}

#[tokio::test]
async fn webhook_ignores_a_newer_event_with_nothing_visibly_changed() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let branch = format!("{key}-x");
    let opened = |at: &str| payload("pr_opened", &branch, REPO, 11, at);
    assert_eq!(
        post_signed(&h, "pull_request", &opened("2026-10-26T10:00:00Z"))
            .await
            .1["result"],
        "linked"
    );
    let entries = activity(&h, &format!("entity_key={key}")).await.len();
    // a push: same status, title and url, later updated_at
    let (st, v) = post_signed(&h, "pull_request", &opened("2026-10-26T10:05:00Z")).await;
    assert_eq!(
        (st, v["result"].as_str()),
        (202, Some("ignored: no visible change")),
        "{v}"
    );
    assert!(
        activity(&h, &format!("entity_key={key}&action=pr_updated"))
            .await
            .is_empty()
    );
    assert_eq!(
        activity(&h, &format!("entity_key={key}")).await.len(),
        entries
    );
    // the time still advanced: a late closed event that predates the push is stale
    let (_, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_closed_merged",
            &branch,
            REPO,
            11,
            "2026-10-26T10:02:00Z",
        ),
    )
    .await;
    assert_eq!(v["result"], "ignored: stale or repeated", "{v}");
    assert_eq!(get_story(&h, &key).await["prs"][0]["status"], "open");
}

#[tokio::test]
async fn webhook_ignores_pull_requests_from_forks() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let before = activity(&h, &format!("entity_key={key}")).await.len();
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_from_fork",
            &format!("{key}-x"),
            REPO,
            12,
            "2026-10-26T10:00:00Z",
        ),
    )
    .await;
    assert_eq!(
        (st, v["result"].as_str()),
        (202, Some("ignored: pull request from a fork")),
        "{v}"
    );
    assert_eq!(pr_rows(&h).await, 0);
    assert_eq!(get_story(&h, &key).await["prs"], json!([]));
    assert_eq!(
        activity(&h, &format!("entity_key={key}")).await.len(),
        before
    );
    // the same fixture from the base repository itself (head repo == repository) links
    let same = payload(
        "pr_opened",
        &format!("{key}-x"),
        REPO,
        13,
        "2026-10-26T10:00:00Z",
    );
    assert_eq!(
        post_signed(&h, "pull_request", &same).await.1["result"],
        "linked"
    );
}

/// A deleted fork's pull request arrives with `head.repo: null` (an `edited` event after the fork is gone): it must not
/// pass the fork check by having no head repository to compare.
#[tokio::test]
async fn webhook_ignores_pull_requests_whose_head_repository_is_unknown() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let before = activity(&h, &format!("entity_key={key}")).await.len();
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_from_deleted_fork",
            &format!("{key}-x"),
            REPO,
            14,
            "2026-10-26T10:00:00Z",
        ),
    )
    .await;
    assert_eq!(
        (st, v["result"].as_str()),
        (202, Some("ignored: head repository unknown")),
        "{v}"
    );
    // a head object without its repo key at all, and a repo without full_name: the same answer
    for head in [
        json!({"ref": format!("{key}-x")}),
        json!({"ref": format!("{key}-x"), "repo": {}}),
    ] {
        let mut body: Value = serde_json::from_str(&payload(
            "pr_opened",
            &format!("{key}-x"),
            REPO,
            15,
            "2026-10-26T10:00:00Z",
        ))
        .unwrap();
        body["pull_request"]["head"] = head;
        let (st, v) = post_signed(&h, "pull_request", &body.to_string()).await;
        assert_eq!(
            (st, v["result"].as_str()),
            (202, Some("ignored: head repository unknown")),
            "{v}"
        );
    }
    assert_eq!(pr_rows(&h).await, 0);
    assert_eq!(get_story(&h, &key).await["prs"], json!([]));
    assert_eq!(
        activity(&h, &format!("entity_key={key}")).await.len(),
        before
    );
}

#[test]
fn the_webhook_has_no_path_to_moving_a_story() {
    let src = include_str!("../src/routes/webhooks.rs");
    assert!(!src.contains("move_story"));
}

/// The webhook never touches an urgent mark (Urgent mark and the war room design, 2026-11-09): a linked pull request
/// opening and merging leaves the story's mark (at, by, reason) as it was and writes no `urgent_*` activity entry.
#[tokio::test]
async fn webhook_never_touches_an_urgent_mark() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let (st, v) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{key}/urgent"),
        Some(json!({"reason": "customer outage"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let mark = get_story(&h, &key).await["urgent_mark"].clone();
    assert_eq!(mark["reason"], "customer outage", "{mark}");
    let urgent_entries = |entries: Vec<Value>| {
        entries
            .into_iter()
            .filter(|e| {
                e["action"]
                    .as_str()
                    .is_some_and(|a| a.starts_with("urgent"))
            })
            .collect::<Vec<_>>()
    };
    let before = urgent_entries(activity(&h, &format!("entity_key={key}")).await);
    assert_eq!(before.len(), 1, "{before:?}");

    let branch = format!("{key}-fix-login");
    for (fixture, at, result) in [
        ("pr_opened", "2026-10-26T10:00:00Z", "linked"),
        ("pr_closed_merged", "2026-10-26T11:00:00Z", "updated"),
    ] {
        let (st, v) =
            post_signed(&h, "pull_request", &payload(fixture, &branch, REPO, 42, at)).await;
        assert_eq!((st, v["result"].as_str()), (202, Some(result)), "{v}");
    }
    let s = get_story(&h, &key).await;
    assert_eq!(s["prs"][0]["status"], "merged");
    assert_eq!(s["urgent_mark"], mark);
    assert_eq!(s["urgent_sources"], json!([{"item": "story", "key": key}]));
    assert_eq!(
        urgent_entries(activity(&h, &format!("entity_key={key}")).await),
        before
    );
}

/// The webhook never touches a promise (kb/ADR-0025 R2): a linked pull request opening and merging leaves the story's
/// promise as it was and writes no promise activity entry.
#[tokio::test]
async fn webhook_never_touches_a_promise() {
    let h = harness().await;
    let key = seed_story(&h, "s1").await;
    let on = (chrono::Utc::now().date_naive() + chrono::Duration::days(30)).to_string();
    let (st, v) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{key}/promise"),
        Some(json!({"on": on, "to": "Acme"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let promise = get_story(&h, &key).await["promise"].clone();
    assert_eq!(promise["own"]["on"], on.as_str(), "{promise}");
    let promise_entries = |entries: Vec<Value>| {
        entries
            .into_iter()
            .filter(|e| e["action"].as_str().is_some_and(|a| a.contains("promised")))
            .collect::<Vec<_>>()
    };
    let before = promise_entries(activity(&h, &format!("entity_key={key}")).await);
    assert_eq!(before.len(), 1, "{before:?}");

    let branch = format!("{key}-fix-login");
    for (fixture, at, result) in [
        ("pr_opened", "2026-10-26T10:00:00Z", "linked"),
        ("pr_closed_merged", "2026-10-26T11:00:00Z", "updated"),
    ] {
        let (st, v) =
            post_signed(&h, "pull_request", &payload(fixture, &branch, REPO, 42, at)).await;
        assert_eq!((st, v["result"].as_str()), (202, Some(result)), "{v}");
    }
    let s = get_story(&h, &key).await;
    assert_eq!(s["prs"][0]["status"], "merged");
    assert_eq!(s["promise"], promise);
    assert_eq!(
        promise_entries(activity(&h, &format!("entity_key={key}")).await),
        before
    );
}

#[tokio::test]
async fn webhook_resolves_a_story_key_of_an_earlier_prefix_before_external_refs() {
    // Stories and an epic issued under OLD, then the instance switches to TST (with OLD also a legacy prefix, so the
    // native key must win over an imported story's external reference).
    let mut h =
        harness_with(|cfg| cfg.replace("key_prefix = \"TST\"", "key_prefix = \"OLD\"")).await;
    let old = seed_story(&h, "s-old").await;
    assert!(old.starts_with("OLD-"), "{old}");
    let old_epic = create(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"e-old","title":"Old epic","description":""}),
    )
    .await;
    let mut cfg = (*h.state.config).clone();
    cfg.key_prefix = "TST".into();
    cfg.github
        .as_mut()
        .unwrap()
        .legacy_prefixes
        .push("OLD".into());
    let state = reeve_server::build_state(cfg, h.state.pool.clone())
        .await
        .unwrap();
    h.app = reeve_server::app(state.clone());
    h.state = state;
    let current = seed_story(&h, "s-new").await;
    assert!(current.starts_with("TST-"), "{current}");
    sqlx::query("UPDATE stories SET external_ref = $1 WHERE key = $2")
        .bind(&old)
        .bind(&current)
        .execute(&h.state.pool)
        .await
        .unwrap();

    let branch = format!("{}-fix", old.to_lowercase());
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload("pr_opened", &branch, REPO, 70, "2026-10-26T10:00:00Z"),
    )
    .await;
    assert_eq!((st, v["result"].as_str()), (202, Some("linked")), "{v}");
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_closed_merged",
            &branch,
            REPO,
            70,
            "2026-10-26T11:00:00Z",
        ),
    )
    .await;
    assert_eq!((st, v["result"].as_str()), (202, Some("updated")), "{v}");
    let s = get_story(&h, &old).await;
    assert_eq!(
        (&s["prs"][0]["number"], &s["prs"][0]["status"]),
        (&json!(70), &json!("merged"))
    );
    assert_eq!(get_story(&h, &current).await["prs"], json!([]));
    assert_eq!(pr_rows(&h).await, 1);

    // an epic's key under the old prefix names no story
    let (st, v) = post_signed(
        &h,
        "pull_request",
        &payload(
            "pr_opened",
            &format!("{old_epic}-x"),
            REPO,
            71,
            "2026-10-26T10:00:00Z",
        ),
    )
    .await;
    assert_eq!(st, 202, "{v}");
    assert_ne!(v["result"], "linked", "{v}");
    assert_eq!(pr_rows(&h).await, 1);
}
