//! `POST /plans` creates a plan of items, required decisions and blockers in one request: validated whole
//! before anything is written, and idempotent per plan id.
mod common;
use common::*;
use serde_json::json;

async fn post(h: &Harness, path: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
    let (st, b) = call(&h.app, Some(&h.admin_token), "POST", path, Some(body)).await;
    (st, b)
}

async fn get(h: &Harness, path: &str) -> serde_json::Value {
    let (st, b) = call(&h.app, Some(&h.admin_token), "GET", path, None).await;
    assert_eq!(st, 200, "{path}: {b}");
    b["value"].clone()
}

#[tokio::test]
async fn a_plan_is_created_in_one_request_and_reapplying_it_creates_nothing() {
    let h = harness().await;
    // an existing epic the plan refers to by key
    let (_, old) = post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"old","title":"Old epic"}),
    )
    .await;
    let old_key = old["value"]["key"].as_str().unwrap().to_string();
    let plan = json!({
        "id": "refine-hosting",
        "initiatives": [{"name": "leave-old-host", "title": "Leave the old host", "horizon": "Now", "brief": "Why."}],
        "epics": [{"name": "hosting", "title": "Hosting", "initiative": "leave-old-host"}],
        "stories": [
            {"name": "db", "title": "Move the database", "epic": "hosting", "criteria": ["moved"]},
            {"name": "dns", "title": "Point DNS", "kind": "chore", "epic": "hosting", "labels": ["infra"]},
            {"name": "fix", "title": "Fix the old thing", "kind": "bug", "epic": old_key}
        ],
        "decisions": [
            {"name": "host", "on": "hosting", "title": "Pick a host", "decider": "dan@example.com"},
            {"name": "price", "on": "hosting", "title": "Pricing", "decider": "dan@example.com", "reversibility": "one_way"}
        ],
        "blocks": [{"story": "dns", "blocked_by": "db"}, {"story": "fix", "blocked_by": "db"}]
    });
    let (st, b) = post(&h, "/api/v1/plans", plan.clone()).await;
    assert_eq!(st, 200, "{b}");
    let v = &b["value"];
    let key = |name: &str| {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == name)
            .unwrap()["key"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["created"] == true),
        "{v}"
    );
    assert_eq!(v["items"].as_array().unwrap().len(), 5);
    assert_eq!(v["decisions"][0]["created"], true);
    assert!(
        v["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["created"] == true),
        "{v}"
    );

    let epic = get(&h, &format!("/api/v1/epics/{}", key("hosting"))).await;
    assert_eq!(epic["initiative"], key("leave-old-host"));
    let dns = get(&h, &format!("/api/v1/stories/{}", key("dns"))).await;
    assert_eq!(dns["kind"], "chore");
    assert_eq!(dns["epic"], key("hosting"));
    assert_eq!(dns["blocked_by"], json!([key("db")]));
    let fix = get(&h, &format!("/api/v1/stories/{}", key("fix"))).await;
    assert_eq!(fix["epic"], old_key.as_str());
    let decisions = get(&h, &format!("/api/v1/decisions?on={}", key("hosting"))).await;
    assert_eq!(decisions.as_array().unwrap().len(), 2);
    // kb/ADR-0013 R1: a plan may classify a decision; one without the field stays unset.
    let decision_id = |name: &str| {
        v["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == name)
            .unwrap()["id"]
            .clone()
    };
    let price = get(&h, &format!("/api/v1/decisions/{}", decision_id("price"))).await;
    assert_eq!(price["reversibility"], "one_way", "{price}");
    let host = get(&h, &format!("/api/v1/decisions/{}", decision_id("host"))).await;
    assert!(host["reversibility"].is_null(), "{host}");

    // the same plan again: the same keys, nothing created
    let (st, again) = post(&h, "/api/v1/plans", plan).await;
    assert_eq!(st, 200, "{again}");
    let a = &again["value"];
    assert!(
        a["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["created"] == false),
        "{a}"
    );
    assert_eq!(a["decisions"][0]["created"], false);
    assert!(
        a["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["created"] == false),
        "{a}"
    );
    for i in a["items"].as_array().unwrap() {
        assert_eq!(i["key"], key(i["name"].as_str().unwrap()).as_str());
    }
    assert_eq!(
        get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
        3
    );
}

#[tokio::test]
async fn an_invalid_plan_writes_nothing_and_names_every_problem() {
    let h = harness().await;
    let plan = json!({
        "id": "bad",
        "initiatives": [{"name": "ini", "title": "Ini", "horizon": "Someday"}],
        "epics": [{"name": "e", "title": "E", "initiative": "ini"}],
        "stories": [
            {"name": "s", "title": "S", "epic": "nowhere"},
            {"name": "s", "title": "Twice"},
            {"name": "TST-9", "title": "Looks like a key"}
        ],
        "blocks": [{"story": "s", "blocked_by": "s"}]
    });
    let (st, b) = post(&h, "/api/v1/plans", plan).await;
    assert_eq!(st, 400, "{b}");
    assert_eq!(b["code"], "validation");
    let m = b["message"].as_str().unwrap();
    for want in [
        "initiative \"ini\"",
        "Someday",
        "story \"s\": epic \"nowhere\"",
        "\"s\" is used twice",
        "\"TST-9\"",
        "cannot block itself",
    ] {
        assert!(m.contains(want), "missing {want:?} in {m}");
    }
    for path in ["/api/v1/initiatives", "/api/v1/epics", "/api/v1/stories"] {
        assert_eq!(get(&h, path).await, json!([]), "{path}");
    }
    // a misspelt field is refused, not dropped
    let (st, _) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "typo", "stories": [{"name": "s", "title": "S", "criterion": ["x"]}]}),
    )
    .await;
    assert_eq!(st, 400);
}

/// Codex review on #33: a plan id with ':' could collide with another plan's keys, and a conflicting external
/// reference was only caught mid-write. Keys no longer collide, conflicts are refused up front, and a plan's own
/// reference re-applies.
#[tokio::test]
async fn plan_ids_and_external_references_are_checked_before_writing() {
    let h = harness().await;
    // ids carry ':' (the skills build them like idempotency keys), and never collide
    let (st, x) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "a:b", "stories": [{"name": "c", "title": "C"}]}),
    )
    .await;
    assert_eq!(st, 200, "{x}");
    let (st, y) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "a", "stories": [{"name": "b:c", "title": "C"}]}),
    )
    .await;
    assert_eq!(st, 200, "{y}");
    assert_ne!(x["value"]["items"][0]["key"], y["value"]["items"][0]["key"]);
    assert_eq!(y["value"]["items"][0]["created"], true);

    let (_, held) = post(&h, "/api/v1/stories", json!({"idempotency_key":"h","title":"Held","kind":"chore","criteria":[],"labels":[],"external_ref":"JPD-9"})).await;
    let held = held["value"]["key"].as_str().unwrap().to_string();
    let (st, b) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "refs", "stories": [
            {"name": "one", "title": "One", "external_ref": "JPD-30"},
            {"name": "two", "title": "Two", "external_ref": "JPD-30"},
            {"name": "three", "title": "Three", "external_ref": "JPD-9"}
        ]}),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    let m = b["message"].as_str().unwrap();
    assert!(
        m.contains("JPD-30 is also the external reference of \"one\""),
        "{m}"
    );
    assert!(
        m.contains(&format!(
            "JPD-9 is already the external reference of story {held}"
        )),
        "{m}"
    );
    assert_eq!(
        get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
        3,
        "nothing written beyond the two plans above and the held story"
    );

    // a plan's own reference is not a conflict on a second apply
    let plan =
        json!({"id": "own", "stories": [{"name": "s", "title": "S", "external_ref": "JPD-31"}]});
    let (st, b) = post(&h, "/api/v1/plans", plan.clone()).await;
    assert_eq!(st, 200, "{b}");
    let (st, b) = post(&h, "/api/v1/plans", plan).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"]["items"][0]["created"], false);
}
