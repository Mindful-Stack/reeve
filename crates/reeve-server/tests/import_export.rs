//! The import endpoints are a switch, off by default, and only an interactive admin may use them.
mod common;
use common::*;
use serde_json::json;

const PATHS: [&str; 4] = [
    "/api/v1/import/initiatives",
    "/api/v1/import/epics",
    "/api/v1/import/stories",
    "/api/v1/import/comments",
];

fn story_body() -> serde_json::Value {
    json!({
        "external_ref": "SCRUM-11", "title": "Legacy match", "kind": "story", "epic": null, "description": "from Jira",
        "criteria": [{"text": "works", "done": true}], "owner": "Bob@Example.com", "labels": ["legacy-import"],
        "delivery": "in_review", "resolution": null
    })
}

#[tokio::test]
async fn import_is_404_while_the_switch_is_off() {
    let h = harness().await;
    let (st, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/config", None).await;
    assert_eq!(st, 200);
    assert_eq!(b["value"]["import_enabled"], false);
    for p in PATHS {
        // an interactive admin with a valid body, and a member with a malformed one: both 404, nothing revealed
        let (st, b) = call(&h.app, Some(&h.admin_token), "POST", p, Some(story_body())).await;
        assert_eq!(st, 404, "{p}");
        assert_eq!(b["code"], "not_found", "{p}");
        let (st, _) = call(&h.app, Some(&h.victim_token), "POST", p, Some(json!({}))).await;
        assert_eq!(st, 404, "{p}");
        // unauthenticated: 401, so the route is mounted and the gate runs before the asset fallback
        let (st, b) = call(&h.app, None, "POST", p, Some(story_body())).await;
        assert_eq!(st, 401, "{p}");
        assert_eq!(b["code"], "unauthenticated", "{p}");
    }
    let (_, list) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/stories", None).await;
    assert_eq!(list["value"], json!([]));
}

#[tokio::test]
async fn import_on_admits_only_an_interactive_admin_and_creates_readable_items() {
    let h = harness_with(|cfg| format!("{cfg}\n[import]\nenabled = true")).await;
    let (_, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/config", None).await;
    assert_eq!(b["value"]["import_enabled"], true);

    for p in PATHS {
        let (st, b) = call(&h.app, Some(&h.victim_token), "POST", p, Some(story_body())).await;
        assert_eq!(st, 403, "member {p}");
        assert_eq!(b["code"], "forbidden");
        let (st, _) = call(&h.app, Some(&h.bot_token), "POST", p, Some(story_body())).await;
        assert_eq!(st, 403, "automation admin {p}");
    }

    let (st, ini) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/initiatives",
        Some(
            json!({"external_ref": "JPD-1", "title": "Booking", "brief": null, "horizon": "Now",
                    "target_month": null, "owner": null, "discovery_complete": true}),
        ),
    )
    .await;
    assert_eq!(st, 200, "{ini}");
    assert_eq!(ini["value"]["created"], true);
    let ini_key = ini["value"]["key"].as_str().unwrap().to_string();
    let (st, epic) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/epics",
        Some(
            json!({"external_ref": "JPD-2", "title": "v2", "initiative": ini_key, "brief": null,
                    "discovery_complete": false}),
        ),
    )
    .await;
    assert_eq!(st, 200, "{epic}");
    let epic_key = epic["value"]["key"].as_str().unwrap().to_string();

    let mut body = story_body();
    body["epic"] = json!(epic_key);
    let (st, s) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/stories",
        Some(body.clone()),
    )
    .await;
    assert_eq!(st, 200, "{s}");
    assert_eq!(s["value"]["created"], true);
    assert_eq!(s["value"]["id"], serde_json::Value::Null);
    let key = s["value"]["key"].as_str().unwrap().to_string();
    // a re-run creates nothing and answers with the same key
    let (st, again) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/stories",
        Some(body),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(
        again["value"],
        json!({"key": key, "id": null, "created": false})
    );

    // readable through the normal endpoints, by key and by external reference
    for r in [key.as_str(), "SCRUM-11"] {
        let (st, got) = call(
            &h.app,
            Some(&h.victim_token),
            "GET",
            &format!("/api/v1/stories/{r}"),
            None,
        )
        .await;
        assert_eq!(st, 200, "{r}: {got}");
        assert_eq!(got["value"]["key"], key.as_str(), "{got}");
        assert_eq!(got["value"]["delivery"], "in_review");
        assert_eq!(got["value"]["owner"], "bob@example.com");
        assert_eq!(got["value"]["external_ref"], "SCRUM-11");
        assert_eq!(got["value"]["epic"], epic_key.as_str());
    }
    let (st, got) = call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        "/api/v1/initiatives/JPD-1",
        None,
    )
    .await;
    assert_eq!(st, 200, "{got}");
    assert_eq!(got["value"]["discovery_complete"]["by"], "dan@example.com");

    let (st, cm) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/comments",
        Some(json!({"external_ref": "jira-comment:1", "story": key, "author": "Alice (alice@example.com)",
                    "body": "looks good", "at": "2025-03-01T10:00:00Z"})),
    )
    .await;
    assert_eq!(st, 200, "{cm}");
    assert_eq!(cm["value"]["created"], true);
    assert!(cm["value"]["id"].is_i64());
    let (_, comments) = call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        &format!("/api/v1/stories/{key}/comments"),
        None,
    )
    .await;
    assert_eq!(comments["value"][0]["author"], "Alice (alice@example.com)");
    assert_eq!(comments["value"][0]["at"], "2025-03-01T10:00:00Z");

    // the activity log holds `imported` entries and no override
    let (_, act) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/activity?entity_key={key}"),
        None,
    )
    .await;
    let actions: Vec<&str> = act["value"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["action"].as_str()).collect())
        .unwrap_or_default();
    assert!(actions.contains(&"imported"), "{act}");
    assert!(!actions.contains(&"override"), "{act}");

    // validation reaches the client as 400
    let mut bad = story_body();
    bad["external_ref"] = json!("SCRUM-12");
    bad["delivery"] = json!("done");
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/stories",
        Some(bad),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert_eq!(b["code"], "validation");
    // a reference in this instance's own key space would be shadowed by the real key
    let mut own = story_body();
    own["external_ref"] = json!("TST-3");
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/stories",
        Some(own),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert_eq!(b["code"], "validation");
    // a reference already imported as a story (SCRUM-11 above) is refused as an epic, naming the story
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/import/epics",
        Some(
            json!({"external_ref": "SCRUM-11", "title": "dup", "initiative": null, "brief": null,
                    "discovery_complete": false}),
        ),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert_eq!(b["code"], "validation");
    assert!(
        b["message"]
            .as_str()
            .unwrap()
            .contains(&format!("SCRUM-11 is already imported as story {key}")),
        "{b}"
    );
}

/// An epic-less bug (untriaged by kb/ADR-0027 R4) in an active state is created in that state, logged `imported`, and
/// never as an `override`: the importer does not go through the readiness gate.
#[tokio::test]
async fn an_imported_epicless_bug_keeps_its_active_state_with_no_override_entry() {
    let h = harness_with(|cfg| format!("{cfg}\n[import]\nenabled = true")).await;
    let got = post(
        &h,
        "/api/v1/import/stories",
        json!({"external_ref": "SCRUM-90", "title": "Crash on save", "kind": "bug", "epic": null,
                "description": null, "criteria": [], "owner": "bob@example.com", "labels": [],
                "delivery": "in_progress", "resolution": null}),
    )
    .await;
    assert_eq!(got["created"], true, "{got}");
    let key = got["key"].as_str().unwrap().to_string();
    let (st, story) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/stories/{key}"),
        None,
    )
    .await;
    assert_eq!(st, 200, "{story}");
    assert_eq!(story["value"]["kind"], "bug");
    assert_eq!(story["value"]["delivery"], "in_progress");
    assert_eq!(story["value"]["epic"], serde_json::Value::Null);
    let (_, act) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/activity?entity_key={key}"),
        None,
    )
    .await;
    let actions: Vec<&str> = act["value"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["action"].as_str()).collect())
        .unwrap_or_default();
    assert!(actions.contains(&"imported"), "{act}");
    assert!(!actions.contains(&"override"), "{act}");
}

// ---- A hand-migrated item records its Jira origin on create, and a later import sees it as existing.

#[tokio::test]
async fn a_hand_created_external_ref_is_shown_and_a_later_import_creates_nothing() {
    let h = harness_with(|cfg| format!("{cfg}\n[import]\nenabled = true")).await;
    let ini = post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"i1","title":"Leave the old host","horizon":"Now","external_ref":" JPD-30 "}),
    )
    .await;
    assert_eq!(ini["external_ref"], "JPD-30", "trimmed: {ini}");
    let ini_key = ini["key"].as_str().unwrap().to_string();
    let epic = post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"e1","title":"Hosting","initiative":ini_key,"external_ref":"SCRUM-480"}),
    )
    .await;
    assert_eq!(epic["external_ref"], "SCRUM-480");
    let epic_key = epic["key"].as_str().unwrap().to_string();
    let story = post(
        &h,
        "/api/v1/stories",
        json!({"idempotency_key":"s1","title":"Move db","kind":"story","epic":epic_key,"criteria":[],"labels":[],
               "external_ref":"SCRUM-481"}),
    )
    .await;
    assert_eq!(story["external_ref"], "SCRUM-481");
    let story_key = story["key"].as_str().unwrap().to_string();
    // readable by the reference, and the created entry records it
    let (st, got) = call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        "/api/v1/initiatives/JPD-30",
        None,
    )
    .await;
    assert_eq!(st, 200, "{got}");
    assert_eq!(got["value"]["key"], ini_key.as_str());
    let (_, act) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/activity?entity_type=story&entity_key={story_key}"),
        None,
    )
    .await;
    assert_eq!(act["value"][0]["action"], "created", "{act}");
    assert_eq!(
        act["value"][0]["payload"]["external_ref"], "SCRUM-481",
        "{act}"
    );

    // the importer finds each one by its reference and creates nothing
    let imports = [
        (
            "/api/v1/import/initiatives",
            json!({"external_ref": "JPD-30", "title": "Leave the old host", "brief": null, "horizon": "Now",
                    "target_month": null, "owner": null, "discovery_complete": true}),
            &ini_key,
        ),
        (
            "/api/v1/import/epics",
            json!({"external_ref": "SCRUM-480", "title": "Hosting", "initiative": ini_key, "brief": null,
                    "discovery_complete": false}),
            &epic_key,
        ),
        (
            "/api/v1/import/stories",
            json!({"external_ref": "SCRUM-481", "title": "Move db", "kind": "story", "epic": epic_key,
                    "description": null, "criteria": [], "owner": null, "labels": [], "delivery": "todo",
                    "resolution": null}),
            &story_key,
        ),
    ];
    for (path, body, key) in imports {
        let got = post(&h, path, body).await;
        assert_eq!(
            got,
            json!({"key": key, "id": null, "created": false}),
            "{path}"
        );
    }
    for (path, n) in [
        ("/api/v1/initiatives", 1),
        ("/api/v1/epics", 1),
        ("/api/v1/stories", 1),
    ] {
        let (_, list) = call(&h.app, Some(&h.admin_token), "GET", path, None).await;
        assert_eq!(list["value"].as_array().unwrap().len(), n, "{path}: {list}");
    }
}

#[tokio::test]
async fn a_hand_created_external_ref_names_one_item() {
    let h = harness().await;
    let ini = post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"i1","title":"Leave the old host","horizon":"Now","external_ref":"JPD-30"}),
    )
    .await;
    let ini_key = ini["key"].as_str().unwrap();
    // a retried create answers the same item
    let again = post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"i1","title":"Leave the old host","horizon":"Now","external_ref":"JPD-30"}),
    )
    .await;
    assert_eq!(again["key"], ini_key);
    // the same reference on a second item, of the same type or another, is refused naming the holder
    for (path, body) in [
        (
            "/api/v1/initiatives",
            json!({"idempotency_key":"i2","title":"Dup","horizon":"Now","external_ref":"JPD-30"}),
        ),
        (
            "/api/v1/epics",
            json!({"idempotency_key":"e2","title":"Dup","external_ref":"JPD-30"}),
        ),
        (
            "/api/v1/stories",
            json!({"idempotency_key":"s2","title":"Dup","kind":"chore","criteria":[],"labels":[],"external_ref":"JPD-30"}),
        ),
    ] {
        let (st, b) = call(&h.app, Some(&h.admin_token), "POST", path, Some(body)).await;
        assert_eq!(st, 400, "{path}: {b}");
        assert_eq!(b["code"], "validation");
        assert!(
            b["message"].as_str().unwrap().contains(&format!(
                "JPD-30 is already the external reference of initiative {ini_key}"
            )),
            "{path}: {b}"
        );
    }
    // a reference must look like a key and lie outside this instance's own key space
    for bad in ["not a key", "", "TST-5"] {
        let (st, b) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/stories",
            Some(json!({"idempotency_key":format!("bad {bad}"),"title":"Bad","kind":"chore","criteria":[],"labels":[],
                        "external_ref":bad})),
        )
        .await;
        assert_eq!(st, 400, "{bad:?}: {b}");
    }
    let (_, list) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/stories", None).await;
    assert_eq!(list["value"], json!([]));
}

#[tokio::test]
async fn an_external_ref_is_set_once_on_edit_and_a_later_import_creates_nothing() {
    let h = harness_with(|cfg| format!("{cfg}\n[import]\nenabled = true")).await;
    let ini = post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"i1","title":"Leave the old host","horizon":"Now"}),
    )
    .await;
    let ini_key = ini["key"].as_str().unwrap().to_string();
    let epic = post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"e1","title":"Hosting","initiative":ini_key}),
    )
    .await;
    let epic_key = epic["key"].as_str().unwrap().to_string();
    let story = post(
        &h,
        "/api/v1/stories",
        json!({"idempotency_key":"s1","title":"Move db","kind":"chore","criteria":[],"labels":[]}),
    )
    .await;
    let story_key = story["key"].as_str().unwrap().to_string();
    let patch = |path: String, body: serde_json::Value| {
        let app = h.app.clone();
        let token = h.admin_token.clone();
        async move { call(&app, Some(&token), "PATCH", &path, Some(body)).await }
    };

    // set on an item that has none; the edit is logged with the reference
    let ini_path = format!("/api/v1/initiatives/{ini_key}");
    let (st, b) = patch(
        ini_path.clone(),
        json!({"version": 1, "external_ref": " JPD-30 "}),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"]["external_ref"], "JPD-30");
    assert_eq!(b["value"]["version"], 2);
    let (_, act) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/activity?entity_type=initiative&entity_key={ini_key}&action=edited"),
        None,
    )
    .await;
    assert_eq!(
        act["value"][0]["payload"]["external_ref"], "JPD-30",
        "{act}"
    );
    // the same reference again is no change; another one is refused, naming the one it has
    let (st, b) = patch(
        ini_path.clone(),
        json!({"version": 2, "external_ref": "JPD-30"}),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let (st, b) = patch(
        ini_path.clone(),
        json!({"version": 3, "external_ref": "JPD-31"}),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert!(
        b["message"]
            .as_str()
            .unwrap()
            .contains(&format!("{ini_key} already has external reference JPD-30")),
        "{b}"
    );
    // a reference another item holds, or one in this instance's key space, is refused
    let epic_path = format!("/api/v1/epics/{epic_key}");
    let (st, b) = patch(
        epic_path.clone(),
        json!({"version": 1, "external_ref": "JPD-30"}),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert!(
        b["message"].as_str().unwrap().contains(&format!(
            "JPD-30 is already the external reference of initiative {ini_key}"
        )),
        "{b}"
    );
    let (st, b) = patch(
        epic_path.clone(),
        json!({"version": 1, "external_ref": "TST-9"}),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    let (st, b) = patch(
        epic_path,
        json!({"version": 1, "external_ref": "SCRUM-480"}),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let (st, b) = patch(
        format!("/api/v1/stories/{story_key}"),
        json!({"version": 1, "external_ref": "SCRUM-481"}),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"]["external_ref"], "SCRUM-481");

    // the importer finds each backfilled item by its reference and creates nothing
    let got = post(
        &h,
        "/api/v1/import/initiatives",
        json!({"external_ref": "JPD-30", "title": "Leave the old host", "brief": null, "horizon": "Now",
                "target_month": null, "owner": null, "discovery_complete": true}),
    )
    .await;
    assert_eq!(got, json!({"key": ini_key, "id": null, "created": false}));
    let got = post(
        &h,
        "/api/v1/import/epics",
        json!({"external_ref": "SCRUM-480", "title": "Hosting", "initiative": ini_key, "brief": null,
                "discovery_complete": false}),
    )
    .await;
    assert_eq!(got, json!({"key": epic_key, "id": null, "created": false}));
    let got = post(
        &h,
        "/api/v1/import/stories",
        json!({"external_ref": "SCRUM-481", "title": "Move db", "kind": "chore", "epic": null,
                "description": null, "criteria": [], "owner": null, "labels": [], "delivery": "todo",
                "resolution": null}),
    )
    .await;
    assert_eq!(got, json!({"key": story_key, "id": null, "created": false}));
}

// ---- The export is one consistent snapshot, admin + interactive only, and holds no secret.

async fn post(h: &Harness, path: &str, body: serde_json::Value) -> serde_json::Value {
    let (st, b) = call(&h.app, Some(&h.admin_token), "POST", path, Some(body)).await;
    assert_eq!(st, 200, "{path}: {b}");
    b["value"].clone()
}

/// One of each item through the API: an initiative, its epic, two stories (the second blocked by the first, the first
/// with a linked pull request), a decision on the first and a comment on it. Returns the first story's key.
async fn seed(h: &Harness) -> String {
    let ini = post(
        h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"i1","title":"Orion","horizon":"Now"}),
    )
    .await;
    let epic = post(
        h,
        "/api/v1/epics",
        json!({"idempotency_key":"e1","title":"v2","initiative":ini["key"]}),
    )
    .await;
    let story = post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key":"s1","title":"Booking","kind":"story","epic":epic["key"],"criteria":["works"],"labels":[]}),
    )
    .await;
    let key = story["key"].as_str().unwrap().to_string();
    let second = post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key":"s2","title":"Follow","kind":"story","epic":epic["key"],"criteria":["works"],"labels":[]}),
    )
    .await;
    let second = second["key"].as_str().unwrap().to_string();
    post(
        h,
        &format!("/api/v1/stories/{second}/blocks"),
        json!({"blocker": key}),
    )
    .await;
    post(
        h,
        "/api/v1/decisions",
        json!({"idempotency_key":"d1","on":key,"title":"Pick","decider":"dan@example.com"}),
    )
    .await;
    post(
        h,
        &format!("/api/v1/stories/{key}/comments"),
        json!({"body":"first"}),
    )
    .await;
    sqlx::query("INSERT INTO story_prs (story_key, repo, number, url, title, status, updated_at) VALUES ($1, 'Acme/portal', 7, 'https://github.com/Acme/portal/pull/7', 'Booking', 'open', now())")
        .bind(&key)
        .execute(&h.state.pool)
        .await
        .unwrap();
    key
}

#[tokio::test]
async fn export_holds_every_item_created_before_it() {
    let h = harness().await;
    let key = seed(&h).await;
    post(
        &h,
        &format!("/api/v1/stories/{key}/urgent"),
        json!({"reason":"hot"}),
    )
    .await;
    let (st, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/export", None).await;
    assert_eq!(st, 200, "{b}");
    let e = &b["value"];
    assert_eq!(e["key_prefix"], "TST");
    // The export carries the mark on its item and the urgency it spreads, computed at the export time.
    let from_story = json!([{"key": key, "item": "story"}]);
    assert_eq!(e["stories"][0]["urgent_mark"]["reason"], "hot");
    assert_eq!(e["stories"][0]["urgent_sources"], from_story);
    assert_eq!(
        e["stories"][1]["urgent_sources"],
        json!([]),
        "a sibling stays calm"
    );
    assert_eq!(e["epics"][0]["urgent_sources"], from_story);
    assert_eq!(e["initiatives"][0]["urgent_sources"], from_story);
    assert_eq!(e["initiatives"].as_array().unwrap().len(), 1);
    assert_eq!(e["epics"].as_array().unwrap().len(), 1);
    let stories = e["stories"].as_array().unwrap();
    assert_eq!(stories.len(), 2);
    assert_eq!(stories[0]["key"], key.as_str(), "stories by key number");
    assert_eq!(stories[1]["blocked_by"], json!([key]));
    assert_eq!(stories[0]["prs"][0]["number"], 7, "{e}");
    assert_eq!(e["decisions"].as_array().unwrap().len(), 1);
    assert_eq!(e["comments"][0]["body"], "first");
    let emails: Vec<&str> = e["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["email"].as_str().unwrap())
        .collect();
    assert_eq!(emails, vec!["bob@example.com", "dan@example.com"]);
    assert_eq!(e["users"][1]["role"], "admin");
    // What the export builds on its transaction equals what the API serves from the pool.
    let (_, ini) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/initiatives",
        None,
    )
    .await;
    assert_eq!(e["initiatives"][0], ini["value"][0]);
    let (_, story) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/stories/{key}"),
        None,
    )
    .await;
    assert_eq!(stories[0], story["value"]);
}

/// The route, `export_document` on the server's state and `export_document` on offline sources built from the
/// configuration produce one document.
#[tokio::test]
async fn export_document_is_what_the_route_serves() {
    let h = harness().await;
    seed(&h).await;
    let (st, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/export", None).await;
    assert_eq!(st, 200, "{b}");
    let without_at = |mut v: serde_json::Value| {
        v.as_object_mut()
            .unwrap()
            .remove("exported_at")
            .expect("the document has exported_at");
        v
    };
    let route = without_at(b["value"].clone());
    assert!(
        !route["stories"].as_array().unwrap().is_empty(),
        "the seeded document has stories: {route}"
    );
    let now = chrono::Utc::now();
    let direct = reeve_server::export_document(&h.state.export_sources(), now)
        .await
        .unwrap();
    assert_eq!(
        direct.exported_at, now,
        "exported_at is the caller's timestamp"
    );
    assert_eq!(without_at(serde_json::to_value(&direct).unwrap()), route);
    let offline = reeve_server::ExportSources::offline(&h.state.config, h.state.pool.clone());
    let off = reeve_server::export_document(&offline, now).await.unwrap();
    assert_eq!(without_at(serde_json::to_value(&off).unwrap()), route);
}

#[tokio::test]
async fn export_is_one_snapshot_across_readers() {
    use reeve_planning::Ctx;
    use reeve_protocol::{Kind, Via};
    let h = harness().await;
    let p = post(
        &h,
        "/api/v1/stories",
        json!({"idempotency_key":"p","title":"Present","kind":"story","criteria":[],"labels":[]}),
    )
    .await;
    let p = p["key"].as_str().unwrap().to_string();
    // a goal with one link in an opened sprint (opening also creates the next draft)
    let g = post(
        &h,
        "/api/v1/goals",
        json!({"idempotency_key":"g1","outcome":"Old outcome","check":"seen"}),
    )
    .await;
    let gid = g["goal"]["id"].as_i64().unwrap();
    post(
        &h,
        &format!("/api/v1/goals/{gid}/links"),
        json!({"items":[p]}),
    )
    .await;
    let (_, all) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/sprints", None).await;
    let draft = all["value"]["draft"].clone();
    post(
        &h,
        &format!("/api/v1/sprints/{}/open", draft["id"]),
        json!({"version":draft["version"]}),
    )
    .await;

    let (_, all) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/sprints", None).await;
    let gversion = all["value"]["active"]["sprint"]["goals"][0]["version"].clone();

    let mut tx = h
        .state
        .pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .unwrap();
    // the first statement on `tx`, as in the export: the snapshot is taken here
    let urgency = h
        .state
        .handlers
        .urgency_in(&mut tx, chrono::Utc::now())
        .await
        .unwrap();
    let clock = h
        .state
        .handlers
        .promise_clock_in(&mut tx, chrono::Utc::now())
        .await
        .unwrap();
    h.state
        .handlers
        .export_initiatives(&mut tx, &urgency)
        .await
        .unwrap();

    // committed on another pooled connection while `tx` is open
    let late = h
        .state
        .handlers
        .create_story(
            &Ctx {
                client: None,
                actor: "dan@example.com".into(),
                via: Via::Interactive,
                now: chrono::Utc::now(),
            },
            "late",
            "Late",
            Kind::Story,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let l = late.key().to_string();
    // a late urgent mark on P: the exported P carries no mark (the urgency and the rows are one snapshot)
    post(&h, &format!("/api/v1/stories/{p}/urgent"), json!({})).await;

    // more late writes through the normal handlers: an epic, a required decision on P, a comment on P and a new user
    let late_epic = post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"le","title":"Late epic"}),
    )
    .await;
    let late_epic = late_epic["key"].as_str().unwrap().to_string();
    post(
        &h,
        "/api/v1/decisions",
        json!({"idempotency_key":"ld","on":p,"title":"Late pick","decider":"dan@example.com"}),
    )
    .await;
    post(
        &h,
        &format!("/api/v1/stories/{p}/comments"),
        json!({"body":"late comment"}),
    )
    .await;
    let ident_log = reeve_activity::make_activity_store(h.state.pool.clone());
    reeve_identity::issue_token(
        &h.state.pool,
        &*h.state.tokens,
        &*ident_log,
        &reeve_identity::AccessPolicy::floor_only(&h.state.identity),
        "test",
        Via::Interactive,
        "carol@example.com",
        reeve_identity::TokenKind::Interactive,
        "late",
        None,
        chrono::Utc::now(),
    )
    .await
    .unwrap();

    // late sprint writes: a second goal (in the next draft), a link on the first goal, a confidence recording and an
    // edit of the first goal's outcome
    let late_story = post(
        &h,
        "/api/v1/stories",
        json!({"idempotency_key":"ls","title":"Late linked","kind":"chore","criteria":[],"labels":[]}),
    )
    .await;
    let late_story = late_story["key"].as_str().unwrap().to_string();
    post(
        &h,
        "/api/v1/goals",
        json!({"idempotency_key":"g2","outcome":"Late goal","check":"seen"}),
    )
    .await;
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "PATCH",
        &format!("/api/v1/goals/{gid}"),
        Some(json!({"version":gversion,"outcome":"New outcome"})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    post(
        &h,
        &format!("/api/v1/goals/{gid}/links"),
        json!({"items":[late_story]}),
    )
    .await;
    post(
        &h,
        &format!("/api/v1/goals/{gid}/confidence"),
        json!({"confidence":"on_track"}),
    )
    .await;
    // a late close of the active sprint, with a verdict for the goal
    let (_, now_all) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/sprints", None).await;
    let active = now_all["value"]["active"]["sprint"].clone();
    post(
        &h,
        &format!("/api/v1/sprints/{}/close", active["id"]),
        json!({"version":active["version"],"verdicts":[{"goal_id":gid,"verdict":"met","note":"late"}]}),
    )
    .await;

    let sprints = h.state.handlers.export_sprints(&mut tx).await.unwrap();
    let epics = h
        .state
        .handlers
        .export_epics(&mut tx, &urgency, &clock)
        .await
        .unwrap();
    let stories = h
        .state
        .handlers
        .export_stories(&mut tx, &urgency, &clock)
        .await
        .unwrap();
    let decisions = h.state.handlers.export_decisions(&mut tx).await.unwrap();
    let comments = h.state.handlers.export_comments(&mut tx).await.unwrap();
    let users = reeve_identity::list_users_in(&mut tx).await.unwrap();
    let activity = h.state.activity.list_all_in(&mut tx, 0, 500).await.unwrap();
    tx.commit().await.unwrap();

    let keys: Vec<String> = stories.iter().map(|s| s.key.to_string()).collect();
    assert_eq!(keys, vec![p.clone()], "P is in the snapshot and L is not");
    assert!(
        urgency.mark(&stories[0].key).is_none(),
        "the late mark is not in"
    );
    assert!(stories[0].urgent_mark.is_none() && stories[0].urgent_sources.is_empty());
    assert!(epics.is_empty(), "the late epic is after the snapshot");
    let states: Vec<_> = sprints.iter().map(|s| s.state).collect();
    assert_eq!(
        states.len(),
        2,
        "one active and one draft sprint: {sprints:?}"
    );
    assert!(
        states.contains(&reeve_protocol::SprintState::Active),
        "the late close is not in"
    );
    assert!(!states.contains(&reeve_protocol::SprintState::Closed));
    assert!(states.contains(&reeve_protocol::SprintState::Draft));
    let goals: Vec<_> = sprints.iter().flat_map(|s| &s.goals).collect();
    assert_eq!(goals.len(), 1, "the late goal is after the snapshot");
    assert_eq!(goals[0].outcome, "Old outcome", "the late edit is not in");
    assert_eq!(goals[0].confidence, None, "the late confidence is not in");
    assert_eq!(goals[0].verdict, None, "the late verdict is not in");
    assert_eq!(
        goals[0]
            .links
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>(),
        vec![p.clone()],
        "the late link is not in"
    );
    assert!(
        decisions.is_empty(),
        "the late decision is after the snapshot"
    );
    assert!(
        comments.is_empty(),
        "the late comment is after the snapshot"
    );
    assert!(
        !users.iter().any(|u| u.email == "carol@example.com"),
        "the late user is after the snapshot"
    );
    assert!(
        activity
            .iter()
            .any(|e| e.entity_key == p && e.action == "created"),
        "{activity:?}"
    );
    assert!(
        !activity.iter().any(|e| e.entity_key == l),
        "L's entries are after the snapshot: {activity:?}"
    );
    // everything late is committed: a later export holds it
    let (_, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/export", None).await;
    let e = &b["value"];
    assert_eq!(e["stories"].as_array().unwrap().len(), 3);
    let late_p = e["stories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == p.as_str())
        .unwrap();
    assert_eq!(
        late_p["urgent_sources"],
        json!([{"key": p, "item": "story"}])
    );
    let later: Vec<&serde_json::Value> = e["sprints"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["goals"].as_array().unwrap())
        .collect();
    assert_eq!(later.len(), 2, "a later export holds the late goal: {e}");
    let closed: Vec<&serde_json::Value> = e["sprints"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["state"] == "closed")
        .collect();
    assert_eq!(closed.len(), 1, "a later export holds the late close: {e}");
    assert_eq!(closed[0]["goals"][0]["verdict"], "met");
    assert_eq!(e["epics"][0]["key"], late_epic.as_str(), "{e}");
    assert_eq!(e["decisions"].as_array().unwrap().len(), 1);
    assert_eq!(e["comments"][0]["body"], "late comment");
    assert!(
        e["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["email"] == "carol@example.com"),
        "{e}"
    );
}

#[tokio::test]
async fn export_holds_every_activity_entry_in_order() {
    let h = harness().await;
    let key = seed(&h).await;
    for i in 0..520 {
        post(
            &h,
            &format!("/api/v1/stories/{key}/comments"),
            json!({"body": format!("c{i}")}),
        )
        .await;
    }
    let (st, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/export", None).await;
    assert_eq!(st, 200);
    let ids: Vec<i64> = b["value"]["activity"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "strictly ascending");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM activity")
        .fetch_one(&h.state.pool)
        .await
        .unwrap();
    assert!(count > 500, "{count}");
    assert_eq!(ids.len() as i64, count, "every page is read");
    assert_eq!(b["value"]["comments"].as_array().unwrap().len(), 521);
}

#[tokio::test]
async fn export_is_for_an_interactive_admin_only() {
    let h = harness().await;
    let (st, b) = call(&h.app, Some(&h.victim_token), "GET", "/api/v1/export", None).await;
    assert_eq!(st, 403, "member");
    assert_eq!(b["code"], "forbidden");
    let (st, b) = call(&h.app, Some(&h.bot_token), "GET", "/api/v1/export", None).await;
    assert_eq!(st, 403, "automation admin");
    assert_eq!(b["code"], "forbidden");
    let (st, _) = call(&h.app, None, "GET", "/api/v1/export", None).await;
    assert_eq!(st, 401, "anonymous");
    sqlx::query("UPDATE users SET disabled = true WHERE email = 'dan@example.com'")
        .execute(&h.state.pool)
        .await
        .unwrap();
    let (st, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/export", None).await;
    assert_eq!(st, 401, "disabled admin");
    assert_eq!(b["code"], "unauthenticated");
}

const SECRET_KEYS: [&str; 6] = [
    "hash",
    "token_hash",
    "secret",
    "client_secret",
    "webhook_secret",
    "database_url",
];

/// Fails on any string value holding a sentinel and on any key named like a secret.
fn assert_no_secret(v: &serde_json::Value, sentinels: &[&str], path: &str) {
    match v {
        serde_json::Value::String(s) => {
            for x in sentinels {
                assert!(!s.contains(x), "{path} holds a secret");
            }
        }
        serde_json::Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                assert_no_secret(x, sentinels, &format!("{path}[{i}]"));
            }
        }
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                assert!(
                    !SECRET_KEYS.contains(&k.as_str()),
                    "{path}.{k} is a secret's key"
                );
                assert_no_secret(x, sentinels, &format!("{path}.{k}"));
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn export_holds_no_secret() {
    const WEBHOOK: &str = "SENTINEL-webhook-0123456789";
    // the standard harness already has a `[github]` table; its secret is replaced by the sentinel
    let h = harness_with(|cfg| cfg.replace(WEBHOOK_SECRET, WEBHOOK)).await;
    assert_eq!(
        h.state
            .config
            .github
            .as_ref()
            .map(|g| g.webhook_secret.as_str()),
        Some(WEBHOOK),
        "the sentinel is configured"
    );
    seed(&h).await;
    // the stored form of each issued token: prove each is in the database, so the check below is not vacuous
    let hashes: Vec<String> = [&h.admin_token, &h.bot_token, &h.victim_token]
        .iter()
        .map(|t| reeve_identity::hash_secret(t))
        .collect();
    for x in &hashes {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens WHERE hash = $1")
            .bind(x)
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "the token hash sentinel is stored");
    }
    let (st, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/export", None).await;
    assert_eq!(st, 200, "{b}");
    assert!(!b["value"]["users"].as_array().unwrap().is_empty());
    let mut sentinels = vec![h.admin_token.as_str(), WEBHOOK];
    sentinels.extend(hashes.iter().map(String::as_str));
    assert_no_secret(&b, &sentinels, "$");
}

/// The importer has no path to an urgent mark (Urgent mark and the war room design, 2026-11-09): neither its handler
/// nor its route names urgency, so an import never marks, clears or renews one.
#[test]
fn the_importer_has_no_path_to_an_urgent_mark() {
    for (path, src) in [
        (
            "reeve-planning/src/handler/import.rs",
            include_str!("../../reeve-planning/src/handler/import.rs"),
        ),
        (
            "reeve-server/src/routes/import.rs",
            include_str!("../src/routes/import.rs"),
        ),
    ] {
        assert!(
            !src.to_lowercase().contains("urgent"),
            "{path} names urgency"
        );
    }
}

/// kb/ADR-0025 R2: the importer has no path to a promise: neither its handler nor its route names one, so an import never
/// reads, sets or clears it.
#[test]
fn the_importer_has_no_path_to_a_promise() {
    for (path, src) in [
        (
            "reeve-planning/src/handler/import.rs",
            include_str!("../../reeve-planning/src/handler/import.rs"),
        ),
        (
            "reeve-server/src/routes/import.rs",
            include_str!("../src/routes/import.rs"),
        ),
    ] {
        assert!(
            !src.to_lowercase().contains("promis"),
            "{path} names a promise"
        );
    }
}
