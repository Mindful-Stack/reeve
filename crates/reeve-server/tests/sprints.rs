//! Sprints and goals over the wire (kb/ADR-0014): the interactive-only writes, the goal filter on `GET /stories`, and
//! the counts at close read back through `GET /activity` (planning never reads them).
mod common;
use common::*;
use serde_json::json;

async fn req(
    h: &Harness,
    token: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
    call(&h.app, Some(token), method, path, body).await
}

async fn ok_value(
    h: &Harness,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> serde_json::Value {
    let (st, b) = req(h, &h.admin_token, method, path, body).await;
    assert_eq!(st, 200, "{method} {path}: {b}");
    b["value"].clone()
}

fn keys(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s["story"]["key"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn opening_with_leave_out_moves_that_goal_to_the_next_draft() {
    let h = harness().await;
    let mut ids = vec![];
    for idem in ["keep", "leave"] {
        let g = ok_value(
            &h,
            "POST",
            "/api/v1/goals",
            Some(json!({"idempotency_key": idem, "outcome": idem, "check": "c"})),
        )
        .await;
        ids.push(g["goal"]["id"].as_i64().unwrap());
    }
    let draft = ok_value(&h, "GET", "/api/v1/sprints", None).await["draft"].clone();
    let sid = draft["id"].as_i64().unwrap();
    ok_value(
        &h,
        "POST",
        &format!("/api/v1/sprints/{sid}/open"),
        Some(json!({"version": draft["version"], "leave_out": [ids[1]]})),
    )
    .await;

    let all = ok_value(&h, "GET", "/api/v1/sprints", None).await;
    let goal_ids = |s: &serde_json::Value| -> Vec<i64> {
        s["goals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["id"].as_i64().unwrap())
            .collect()
    };
    assert_eq!(goal_ids(&all["active"]["sprint"]), vec![ids[0]]);
    assert_eq!(
        goal_ids(&all["draft"]),
        vec![ids[1]],
        "the left-out goal is in the next draft"
    );
}

#[tokio::test]
async fn a_sprint_cycle_over_the_wire() {
    let h = harness().await;
    let epic = ready_epic(&h, "ready-epic").await;
    let story = |idem: &'static str| json!({"idempotency_key": idem, "title": idem, "kind": "chore", "epic": epic, "criteria": ["x"], "labels": []});
    let s1 = ok_value(&h, "POST", "/api/v1/stories", Some(story("s1"))).await;
    let s1_key = s1["key"].as_str().unwrap().to_string();
    let s2 = ok_value(&h, "POST", "/api/v1/stories", Some(story("s2"))).await;
    let s2_key = s2["key"].as_str().unwrap().to_string();

    // The first goal creates the draft.
    let created = ok_value(
        &h,
        "POST",
        "/api/v1/goals",
        Some(json!({"idempotency_key": "g1", "outcome": "Customers can book", "check": "a customer booked one"})),
    )
    .await;
    assert_eq!(created["warnings"], json!([]));
    assert_eq!(created["goal"]["owner"], "dan@example.com");
    let gid = created["goal"]["id"].as_i64().unwrap();
    let linked = ok_value(
        &h,
        "POST",
        &format!("/api/v1/goals/{gid}/links"),
        Some(json!({"items": [s1_key]})),
    )
    .await;
    assert_eq!(linked["links"], json!([s1_key]));

    // Automation may not create a goal (kb/ADR-0014 R5).
    let (st, body) = req(
        &h,
        &h.bot_token,
        "POST",
        "/api/v1/goals",
        Some(json!({"idempotency_key": "g2", "outcome": "o", "check": "c"})),
    )
    .await;
    assert_eq!(st, 422, "{body}");
    assert_eq!(body["outcome"], "refused");
    assert_eq!(body["reasons"], json!([{"reason": "interactive_only"}]));
    assert_eq!(body["overridable"], false);

    let all = ok_value(&h, "GET", "/api/v1/sprints", None).await;
    assert_eq!(all["active"], serde_json::Value::Null);
    let draft = all["draft"].clone();
    assert_eq!(draft["state"], "draft");
    assert_eq!(draft["goals"].as_array().unwrap().len(), 1);
    let sid = draft["id"].as_i64().unwrap();
    let draft_view = ok_value(&h, "GET", &format!("/api/v1/sprints/{sid}"), None).await;
    assert_eq!(
        draft_view["counts"],
        json!({"goals": [], "completed": 0, "upkeep_completed": 0}),
        "a draft has no window"
    );

    // Only an interactive token opens it.
    let (st, body) = req(
        &h,
        &h.bot_token,
        "POST",
        &format!("/api/v1/sprints/{sid}/open"),
        Some(json!({"version": draft["version"]})),
    )
    .await;
    assert_eq!(st, 422, "{body}");
    assert_eq!(body["reasons"], json!([{"reason": "interactive_only"}]));
    let opened = ok_value(
        &h,
        "POST",
        &format!("/api/v1/sprints/{sid}/open"),
        Some(json!({"version": draft["version"]})),
    )
    .await;
    assert_eq!(opened["state"], "active");
    assert_eq!(opened["warnings"], json!([]));

    let all = ok_value(&h, "GET", "/api/v1/sprints", None).await;
    assert_eq!(all["active"]["sprint"]["id"], sid);
    assert_eq!(all["active"]["sprint"]["state"], "active");
    assert_eq!(
        all["active"]["sprint"]["goals"].as_array().unwrap().len(),
        1
    );
    assert_eq!(all["active"]["upkeep_line"], 20);
    assert_eq!(
        all["draft"]["state"], "draft",
        "opening made the next draft"
    );

    // Automation may suggest a confidence.
    let (st, body) = req(
        &h,
        &h.bot_token,
        "POST",
        &format!("/api/v1/goals/{gid}/suggestion"),
        Some(json!({"confidence": "at_risk", "note": "1 of 1 in flight"})),
    )
    .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["value"]["suggested_confidence"], "at_risk");
    assert_eq!(body["value"]["confidence"], serde_json::Value::Null);

    // The goal filter: any, none, an id; anything else is a validation error.
    let any = ok_value(&h, "GET", "/api/v1/stories?goal=any", None).await;
    assert_eq!(keys(&any), vec![s1_key.clone()]);
    assert_eq!(any[0]["story"]["goals"][0]["id"], gid);
    assert_eq!(any[0]["story"]["goals"][0]["via"], "story");
    let none = ok_value(&h, "GET", "/api/v1/stories?goal=none", None).await;
    assert_eq!(keys(&none), vec![s2_key.clone()]);
    let by_id = ok_value(&h, "GET", &format!("/api/v1/stories?goal={gid}"), None).await;
    assert_eq!(keys(&by_id), vec![s1_key.clone()]);
    let (st, body) = req(&h, &h.admin_token, "GET", "/api/v1/stories?goal=x", None).await;
    assert_eq!(st, 400, "{body}");
    assert_eq!(body["code"], "validation");
    let context = ok_value(
        &h,
        "GET",
        &format!("/api/v1/stories/{s1_key}/context"),
        None,
    )
    .await;
    assert_eq!(context["goals"][0]["goal"]["id"], gid);
    assert_eq!(context["goals"][0]["via"], "story");

    // Complete s1 inside the window, then close.
    let moved = ok_value(
        &h,
        "POST",
        &format!("/api/v1/stories/{s1_key}/move"),
        Some(json!({"version": s1["version"], "to": "in_progress"})),
    )
    .await;
    ok_value(
        &h,
        "POST",
        &format!("/api/v1/stories/{s1_key}/move"),
        Some(json!({"version": moved["version"], "to": "done", "resolution": "completed"})),
    )
    .await;
    let active = ok_value(&h, "GET", &format!("/api/v1/sprints/{sid}"), None).await;
    let (st, body) = req(
        &h,
        &h.admin_token,
        "POST",
        &format!("/api/v1/sprints/{sid}/close"),
        Some(json!({"version": active["sprint"]["version"], "verdicts": []})),
    )
    .await;
    assert_eq!(st, 422, "{body}");
    assert_eq!(
        body["reasons"],
        json!([{"reason": "verdict_missing", "goals": [gid]}])
    );
    let closed = ok_value(
        &h,
        "POST",
        &format!("/api/v1/sprints/{sid}/close"),
        Some(json!({"version": active["sprint"]["version"], "verdicts": [{"goal_id": gid, "verdict": "met"}]})),
    )
    .await;
    assert_eq!(closed["state"], "closed");
    assert_eq!(closed["warnings"], json!([]));
    assert_eq!(closed["goals"][0]["verdict"], "met");

    // The counts at close live only in the `closed` entry.
    let feed = ok_value(
        &h,
        "GET",
        &format!("/api/v1/activity?entity_type=sprint&entity_key={sid}&action=closed"),
        None,
    )
    .await;
    let entries = feed.as_array().unwrap();
    assert_eq!(entries.len(), 1, "{feed}");
    let at_close: reeve_protocol::SprintCountsDto =
        serde_json::from_value(entries[0]["payload"]["counts"].clone()).unwrap();
    assert_eq!(
        at_close,
        reeve_protocol::SprintCountsDto {
            goals: vec![reeve_protocol::GoalProgressDto {
                goal_id: gid,
                completed: 1,
                ..Default::default()
            }],
            completed: 1,
            upkeep_completed: 0,
        }
    );
    let view = ok_value(&h, "GET", &format!("/api/v1/sprints/{sid}"), None).await;
    let now: reeve_protocol::SprintCountsDto =
        serde_json::from_value(view["counts"].clone()).unwrap();
    assert_eq!(now, at_close, "nothing changed since the close");
    assert_eq!(view["demo"][0]["key"], s1_key);
    let all = ok_value(&h, "GET", "/api/v1/sprints", None).await;
    assert_eq!(all["active"], serde_json::Value::Null);
    assert_eq!(all["closed"][0]["id"], sid);

    // A closed sprint's goals are read-only, for every caller.
    let (st, body) = req(
        &h,
        &h.bot_token,
        "DELETE",
        &format!("/api/v1/goals/{gid}/links/{s1_key}"),
        None,
    )
    .await;
    assert_eq!(st, 422, "{body}");
    assert_eq!(body["reasons"], json!([{"reason": "sprint_closed"}]));
}

/// Goal edits, removal by version, unlinking and a link to no item, on the draft.
#[tokio::test]
async fn goal_edits_and_links_on_the_draft() {
    let h = harness().await;
    let g = ok_value(
        &h,
        "POST",
        "/api/v1/goals",
        Some(json!({"idempotency_key": "g1", "outcome": "o", "check": "c", "notes": "why"})),
    )
    .await["goal"]
        .clone();
    let gid = g["id"].as_i64().unwrap();
    assert_eq!(g["notes"], "why");
    let edited = ok_value(
        &h,
        "PATCH",
        &format!("/api/v1/goals/{gid}"),
        Some(json!({"version": g["version"], "outcome": "o2"})),
    )
    .await;
    assert_eq!(edited["outcome"], "o2");
    assert_eq!(edited["check"], "c");
    let (st, _) = req(
        &h,
        &h.admin_token,
        "PATCH",
        &format!("/api/v1/goals/{gid}"),
        Some(json!({"version": g["version"], "outcome": "o3"})),
    )
    .await;
    assert_eq!(st, 409);
    let e = ok_value(
        &h,
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key": "e1", "title": "E"})),
    )
    .await;
    let e_key = e["key"].as_str().unwrap().to_string();
    let (st, _) = req(
        &h,
        &h.bot_token,
        "POST",
        &format!("/api/v1/goals/{gid}/links"),
        Some(json!({"items": [e_key, "ABC-999"]})),
    )
    .await;
    assert_eq!(st, 404, "a key that names no item links nothing");
    let linked = ok_value(
        &h,
        "POST",
        &format!("/api/v1/goals/{gid}/links"),
        Some(json!({"items": [e_key]})),
    )
    .await;
    assert_eq!(linked["links"], json!([e_key]));
    let unlinked = ok_value(
        &h,
        "DELETE",
        &format!("/api/v1/goals/{gid}/links/{e_key}"),
        None,
    )
    .await;
    assert_eq!(unlinked["links"], json!([]));
    ok_value(
        &h,
        "DELETE",
        &format!("/api/v1/goals/{gid}?version={}", edited["version"]),
        None,
    )
    .await;
    let all = ok_value(&h, "GET", "/api/v1/sprints", None).await;
    assert_eq!(all["draft"]["goals"], json!([]));
    assert_eq!(all["draft"]["warnings"], json!(["no_goals"]));
}

/// `GET /sprints` carries the server's UTC date as `today`, so clients never read their own clock for promises
/// (kb/ADR-0025). Either side of the call is accepted, to tolerate midnight.
#[tokio::test]
async fn sprints_carry_the_request_utc_date() {
    let h = harness().await;
    let before = chrono::Utc::now().date_naive();
    let (st, b) = req(&h, &h.admin_token, "GET", "/api/v1/sprints", None).await;
    let after = chrono::Utc::now().date_naive();
    assert_eq!(st, 200, "{b}");
    let today: chrono::NaiveDate = b["value"]["today"].as_str().unwrap().parse().unwrap();
    assert!(today == before || today == after, "{today}");
}

/// `GET /sprints/planning` answers each active and draft goal's gates; `POST /goals` with `links` creates the goal
/// linked, and an unknown key creates nothing (404).
#[tokio::test]
async fn planning_lists_each_goals_gates_and_a_goal_is_created_linked() {
    let h = harness().await;
    let (st, e) = req(
        &h,
        &h.admin_token,
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key": "e", "title": "Unmarked"})),
    )
    .await;
    assert_eq!(st, 200, "{e}");
    let epic = e["value"]["key"].as_str().unwrap().to_string();
    let (st, b) = req(
        &h,
        &h.admin_token,
        "POST",
        "/api/v1/goals",
        // The harness's key prefix is TST: a well-formed key that names no item.
        Some(json!({"idempotency_key": "bad", "outcome": "o", "check": "c", "links": ["TST-999"]})),
    )
    .await;
    assert_eq!(st, 404, "{b}");
    // One goal in the active sprint, one in the draft: the route answers both.
    let a = ok_value(
        &h,
        "POST",
        "/api/v1/goals",
        Some(json!({"idempotency_key": "a", "outcome": "a", "check": "c", "links": [epic]})),
    )
    .await;
    let aid = a["goal"]["id"].as_i64().unwrap();
    let draft = ok_value(&h, "GET", "/api/v1/sprints", None).await["draft"].clone();
    let sid = draft["id"].as_i64().unwrap();
    ok_value(
        &h,
        "POST",
        &format!("/api/v1/sprints/{sid}/open"),
        Some(json!({"version": draft["version"]})),
    )
    .await;
    let g = ok_value(
        &h,
        "POST",
        "/api/v1/goals",
        Some(json!({"idempotency_key": "g", "outcome": "o", "check": "c", "links": [epic]})),
    )
    .await;
    assert_eq!(g["goal"]["links"], json!([epic]));
    assert!(g["goal"]["created_at"].is_string(), "{g}");
    let gid = g["goal"]["id"].as_i64().unwrap();
    let planned = ok_value(&h, "GET", "/api/v1/sprints/planning", None).await;
    assert_eq!(
        planned["goals"],
        json!([
            {
                "goal_id": aid, "initiatives": [],
                "discoveries": [{"key": epic, "item": "epic", "title": "Unmarked"}],
                "decisions": [], "discovery_total": 1, "decisions_total": 0
            },
            {
                "goal_id": gid, "initiatives": [],
                "discoveries": [{"key": epic, "item": "epic", "title": "Unmarked"}],
                "decisions": [], "discovery_total": 1, "decisions_total": 0
            }
        ])
    );
}

async fn new_initiative(h: &Harness, idem: &str, title: &str, horizon: &str) -> String {
    ok_value(
        h,
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key": idem, "title": title, "horizon": horizon})),
    )
    .await["key"]
        .as_str()
        .unwrap()
        .to_string()
}

/// `GET /sprints/planning` names each goal's initiatives with their horizon and whether it is the first configured
/// bucket, and lists that bucket's initiatives with the goals that reach them (display only, kb/ADR-0026).
#[tokio::test]
async fn planning_compares_goals_with_the_first_horizon_bucket() {
    let h = harness().await;
    // The harness keeps the default buckets, which start "Now", "Next" (`default_buckets` in config.rs).
    // Created in the reverse of their title order, so a list in creation or key order would fail.
    let empty = new_initiative(&h, "i-empty", "No goal reaches it", "Now").await;
    let outside = new_initiative(&h, "i-out", "Outside it", "Next").await;
    let inside = new_initiative(&h, "i-in", "In the bucket", "Now").await;
    let goal = |idem: &'static str, links: serde_json::Value| {
        let h = &h;
        async move {
            ok_value(
                h,
                "POST",
                "/api/v1/goals",
                Some(
                    json!({"idempotency_key": idem, "outcome": idem, "check": "c", "links": links}),
                ),
            )
            .await["goal"]["id"]
                .as_i64()
                .unwrap()
        }
    };
    let both = goal("both", json!([inside, outside])).await;
    let none = goal("none", json!([])).await;
    let p = ok_value(&h, "GET", "/api/v1/sprints/planning", None).await;
    assert_eq!(p["first_bucket"], "Now", "{p}");
    assert_eq!(
        p["goals"][0]["initiatives"],
        json!([
            {"key": outside, "title": "Outside it", "horizon": "Next", "in_first_bucket": false},
            {"key": inside, "title": "In the bucket", "horizon": "Now", "in_first_bucket": true}
        ]),
        "{p}"
    );
    assert_eq!(p["goals"][0]["goal_id"], both);
    assert_eq!(p["goals"][1]["goal_id"], none);
    assert_eq!(p["goals"][1]["initiatives"], json!([]));
    assert_eq!(
        p["first_bucket_initiatives"],
        json!([
            {"key": inside, "title": "In the bucket", "target_month": null, "discovery_complete": false, "goals": [both]},
            {"key": empty, "title": "No goal reaches it", "target_month": null, "discovery_complete": false, "goals": []}
        ]),
        "{p}"
    );
}

/// Without a configured bucket the answer names none and lists none, and no initiative counts as in it. A server
/// without buckets refuses to create an initiative, so one is seeded through a server that has them and read through a
/// second server, on the same database, that has none.
#[tokio::test]
async fn planning_names_no_bucket_when_none_is_configured() {
    let seed = harness().await;
    let ini = new_initiative(&seed, "i-seed", "Seeded", "Now").await;
    ok_value(
        &seed,
        "POST",
        "/api/v1/goals",
        Some(json!({"idempotency_key": "g", "outcome": "o", "check": "c", "links": [ini]})),
    )
    .await;
    let h = harness_beside(&seed, |c| format!("horizon_buckets = []\n{c}")).await;
    let p = ok_value(&h, "GET", "/api/v1/sprints/planning", None).await;
    assert!(p.as_object().unwrap().contains_key("first_bucket"), "{p}");
    assert_eq!(p["first_bucket"], json!(null), "{p}");
    assert_eq!(p["first_bucket_initiatives"], json!([]), "{p}");
    assert_eq!(
        p["goals"][0]["initiatives"],
        json!([{"key": ini, "title": "Seeded", "horizon": "Now", "in_first_bucket": false}]),
        "{p}"
    );
}
