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

/// How many entries with `action` the activity log has on `key`.
async fn entries(h: &Harness, key: &str, action: &str) -> usize {
    get(h, &format!("/api/v1/activity?entity_key={key}"))
        .await
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == action)
        .count()
}

/// How many rows `table` has, read from the database itself.
async fn rows(h: &Harness, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
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

/// A plan story may carry the delivery state of the ticket it migrates. A state other than todo is
/// grandfathered like the importer's: created in that state and logged `imported`, never `override`.
#[tokio::test]
async fn a_plan_story_carries_its_delivery_state_and_is_logged_imported() {
    let h = harness().await;
    let plan = json!({"id": "scrum", "stories": [
        {"name": "a", "title": "A", "external_ref": "SCRUM-1", "delivery": "in_progress"},
        {"name": "b", "title": "B", "external_ref": "SCRUM-2", "delivery": "in_review"},
        {"name": "c", "title": "C", "external_ref": "SCRUM-3", "delivery": "testing"},
        {"name": "d", "title": "D", "external_ref": "SCRUM-4", "delivery": "todo"},
        {"name": "e", "title": "E"}
    ]});
    let (st, b) = post(&h, "/api/v1/plans", plan.clone()).await;
    assert_eq!(st, 200, "{b}");
    let items = b["value"]["items"].as_array().unwrap().clone();
    let want = [
        ("a", "in_progress", "imported"),
        ("b", "in_review", "imported"),
        ("c", "testing", "imported"),
        ("d", "todo", "created"),
        ("e", "todo", "created"),
    ];
    for (name, delivery, action) in want {
        let key = items.iter().find(|i| i["name"] == name).unwrap()["key"]
            .as_str()
            .unwrap()
            .to_string();
        let s = get(&h, &format!("/api/v1/stories/{key}")).await;
        assert_eq!(s["delivery"], delivery, "{name}: {s}");
        let act = get(&h, &format!("/api/v1/activity?entity_key={key}")).await;
        let actions: Vec<&str> = act
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["action"].as_str())
            .collect();
        assert_eq!(actions, vec![action], "{name}: {act}");
    }

    // applying it again creates nothing and leaves the states as they are
    let (st, again) = post(&h, "/api/v1/plans", plan).await;
    assert_eq!(st, 200, "{again}");
    assert!(
        again["value"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["created"] == false),
        "{again}"
    );
    let a = &items[0]["key"].as_str().unwrap().to_string();
    assert_eq!(
        get(&h, &format!("/api/v1/stories/{a}")).await["delivery"],
        "in_progress"
    );
}

/// Only migrated work can skip the gate, and done (which needs a resolution) is the importer's alone.
#[tokio::test]
async fn a_plan_delivery_state_needs_an_external_ref_and_is_never_done() {
    let h = harness().await;
    let (st, b) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "bad-delivery", "stories": [
            {"name": "noref", "title": "No ref", "delivery": "in_progress"},
            {"name": "done", "title": "Done", "external_ref": "SCRUM-5", "delivery": "done"},
            {"name": "fine", "title": "Fine", "external_ref": "SCRUM-6", "delivery": "testing"}
        ]}),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    let m = b["message"].as_str().unwrap();
    assert!(
        m.contains("story \"noref\": a delivery state requires an external_ref"),
        "{m}"
    );
    assert!(m.contains("story \"done\": delivery done"), "{m}");
    assert!(!m.contains("\"fine\""), "{m}");
    assert_eq!(get(&h, "/api/v1/stories").await, json!([]));
    // an unknown state is refused too
    let (st, _) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "typo", "stories": [{"name": "s", "title": "S", "external_ref": "SCRUM-7", "delivery": "doing"}]}),
    )
    .await;
    assert_eq!(st, 400);
}

/// kb/ADR-0031 R1, R4, R7 through a plan. A plan links items of any level: two it creates, one it creates to one
/// that exists, and two that exist, one of them named by its external reference. Every link is logged on both of
/// its items, a link that was there before the plan answers `created` false, a `relates_to` is answered as stored,
/// and a second apply creates nothing.
#[tokio::test]
async fn a_plan_links_items_of_any_level_and_reapplying_it_creates_nothing() {
    let h = harness().await;
    let (_, old) = post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"old","title":"Old epic"}),
    )
    .await;
    let old = old["value"]["key"].as_str().unwrap().to_string();
    let (_, held) = post(
        &h,
        "/api/v1/stories",
        json!({"idempotency_key":"h","title":"Held","kind":"chore","criteria":[],"labels":[],"external_ref":"JPD-9"}),
    )
    .await;
    let held = held["value"]["key"].as_str().unwrap().to_string();
    // A link that exists before the plan names it.
    let (st, b) = post(
        &h,
        &format!("/api/v1/items/{old}/links"),
        json!({"type": "relates_to", "to": held}),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let old_version = get(&h, &format!("/api/v1/epics/{old}")).await["version"].clone();

    let plan = json!({
        "id": "sequence",
        "initiatives": [{"name": "ini", "title": "Ini", "horizon": "Now"}],
        "epics": [{"name": "e", "title": "E", "initiative": "ini"}],
        "stories": [
            {"name": "first", "title": "First", "epic": "e"},
            {"name": "second", "title": "Second", "epic": "e"}
        ],
        "links": [
            {"from": "second", "type": "follows", "to": "first"},
            {"from": "e", "type": "duplicates", "to": old},
            {"from": "second", "type": "relates_to", "to": "ini"},
            {"from": "JPD-9", "type": "relates_to", "to": old}
        ]
    });
    let (st, b) = post(&h, "/api/v1/plans", plan.clone()).await;
    assert_eq!(st, 200, "{b}");
    let v = b["value"].clone();
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
    // The control for the two `relates_to` rows below: the plan names the higher key first in both.
    assert!(
        key("ini").as_bytes() < key("second").as_bytes() && old.as_bytes() < held.as_bytes(),
        "the control: {} < {} and {old} < {held}",
        key("ini"),
        key("second")
    );
    let want = |created: [bool; 4]| {
        json!([
            {"from": key("second"), "from_level": "story", "type": "follows",
             "to": key("first"), "to_level": "story", "created": created[0]},
            {"from": key("e"), "from_level": "epic", "type": "duplicates",
             "to": old, "to_level": "epic", "created": created[1]},
            {"from": key("ini"), "from_level": "initiative", "type": "relates_to",
             "to": key("second"), "to_level": "story", "created": created[2]},
            {"from": old, "from_level": "epic", "type": "relates_to",
             "to": held, "to_level": "story", "created": created[3]}
        ])
    };
    assert_eq!(v["links"], want([true, true, true, false]), "{v}");

    // Each end reads its link, with the other item's level and title.
    let on_first = get(&h, &format!("/api/v1/items/{}/links", key("first"))).await;
    assert_eq!(
        on_first["links"],
        json!([{"type": "follows", "direction": "in", "label": "is followed by",
                "item": {"key": key("second"), "level": "story", "title": "Second"}}])
    );
    let on_ini = get(&h, &format!("/api/v1/items/{}/links", key("ini"))).await;
    assert_eq!(
        on_ini["links"],
        json!([{"type": "relates_to", "direction": "out", "label": "relates to",
                "item": {"key": key("second"), "level": "story", "title": "Second"}}])
    );
    // One entry on every item a link names; the link that was there writes none.
    let logged = [
        (key("second"), 2),
        (key("first"), 1),
        (key("e"), 1),
        (key("ini"), 1),
        (old.clone(), 2),
        (held.clone(), 1),
    ];
    for (item, n) in &logged {
        assert_eq!(entries(&h, item, "item_link_added").await, *n, "{item}");
    }
    assert_eq!(rows(&h, "item_links").await, 4);
    // kb/ADR-0031 R3: a link is no field of its item.
    assert_eq!(
        get(&h, &format!("/api/v1/epics/{old}")).await["version"],
        old_version,
        "a plan's link leaves the version of an item it names alone"
    );

    // The same plan again: the same links, none created, nothing logged.
    let (st, again) = post(&h, "/api/v1/plans", plan).await;
    assert_eq!(st, 200, "{again}");
    assert_eq!(again["value"]["links"], want([false; 4]), "{again}");
    for (item, n) in &logged {
        assert_eq!(
            entries(&h, item, "item_link_added").await,
            *n,
            "{item} after the second apply"
        );
    }
    assert_eq!(rows(&h, "item_links").await, 4);
}

/// The all-or-nothing check covers item links: an end that names nothing, a link from an item to itself (by one
/// name, and by two spellings of one existing item) and a key that no item has are each reported, and nothing of
/// the plan is written.
#[tokio::test]
async fn a_plan_with_a_bad_link_writes_nothing_and_names_every_problem() {
    let h = harness().await;
    let (_, held) = post(
        &h,
        "/api/v1/stories",
        json!({"idempotency_key":"h","title":"Held","kind":"chore","criteria":[],"labels":[],"external_ref":"JPD-9"}),
    )
    .await;
    let held = held["value"]["key"].as_str().unwrap().to_string();
    let (st, b) = post(
        &h,
        "/api/v1/plans",
        json!({
            "id": "bad-links",
            "stories": [{"name": "a", "title": "A"}, {"name": "b", "title": "B"}],
            "links": [
                {"from": "a", "type": "follows", "to": "nowhere"},
                {"from": "a", "type": "duplicates", "to": "a"},
                {"from": held, "type": "duplicates", "to": "JPD-9"},
                {"from": "TST-999", "type": "relates_to", "to": "b"}
            ]
        }),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert_eq!(b["code"], "validation");
    let m = b["message"].as_str().unwrap();
    for want in [
        "item link \"a\" follows \"nowhere\": to \"nowhere\" is neither a name in the plan nor an existing item".to_string(),
        "item link \"a\" duplicates \"a\": an item cannot have an item link to itself".to_string(),
        format!("item link \"{held}\" duplicates \"JPD-9\": an item cannot have an item link to itself"),
        "item link \"TST-999\" relates_to \"b\": from \"TST-999\" is neither a name in the plan nor an existing item".to_string(),
    ] {
        assert!(m.contains(&want), "missing {want:?} in {m}");
    }
    assert_eq!(
        m.matches("an item cannot have an item link to itself")
            .count(),
        2,
        "the two self links and no other: {m}"
    );
    assert_eq!(
        get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
        1,
        "only the story made before the plan"
    );
    assert_eq!(rows(&h, "item_links").await, 0);

    // A name outside the three, and a misspelt field, are refused as the body is read; nothing is written either.
    for bad in [
        json!({"id": "t", "stories": [{"name": "a", "title": "A"}, {"name": "b", "title": "B"}],
               "links": [{"from": "a", "type": "blocks", "to": "b"}]}),
        json!({"id": "t", "stories": [{"name": "a", "title": "A"}, {"name": "b", "title": "B"}],
               "links": [{"from": "a", "kind": "follows", "to": "b"}]}),
    ] {
        let (st, b) = post(&h, "/api/v1/plans", bad).await;
        assert_eq!(st, 400, "{b}");
        assert_eq!(b["code"], "validation");
    }
    assert_eq!(
        get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
        1
    );
}

/// A plan name and the key (or the external reference) of the item an earlier apply of this plan made for that
/// name are one item. A plan applied again with a link between two such spellings is refused by the check, before
/// anything is written: the story the edited plan adds does not exist afterwards. The same two spellings are also
/// one end when they are written: a link named both ways is one link.
#[tokio::test]
async fn a_reapplied_plan_that_links_an_item_to_itself_under_two_spellings_writes_nothing() {
    let h = harness().await;
    let plan = |more: bool, links: serde_json::Value| {
        let mut stories = vec![json!({"name": "s", "title": "S", "external_ref": "JPD-9"})];
        if more {
            stories.push(json!({"name": "t", "title": "T"}));
        }
        json!({"id": "alias", "stories": stories, "links": links})
    };
    let (st, b) = post(&h, "/api/v1/plans", plan(false, json!([]))).await;
    assert_eq!(st, 200, "{b}");
    let s = b["value"]["items"][0]["key"].as_str().unwrap().to_string();

    for (to, name) in [(s.as_str(), "follows"), ("JPD-9", "duplicates")] {
        let (st, b) = post(
            &h,
            "/api/v1/plans",
            plan(true, json!([{"from": "s", "type": name, "to": to}])),
        )
        .await;
        assert_eq!(st, 400, "{to}: {b}");
        assert_eq!(b["code"], "validation");
        let want =
            format!("item link \"s\" {name} \"{to}\": an item cannot have an item link to itself");
        assert!(
            b["message"].as_str().unwrap().contains(&want),
            "missing {want:?} in {b}"
        );
        assert_eq!(
            get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
            1,
            "{to}: the story the edited plan adds was not created"
        );
        assert_eq!(rows(&h, "item_links").await, 0, "{to}");
    }

    // The control: the edited plan is applied when its link joins two items, and the two spellings of `s` are one
    // end there too: the second entry is the first one's link.
    let (st, b) = post(
        &h,
        "/api/v1/plans",
        plan(
            true,
            json!([
                {"from": "t", "type": "follows", "to": s},
                {"from": "t", "type": "follows", "to": "s"}
            ]),
        ),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let links = b["value"]["links"].as_array().unwrap();
    assert_eq!(
        (&links[0]["to"], &links[1]["to"]),
        (&json!(s), &json!(s)),
        "{b}"
    );
    assert_eq!(
        (&links[0]["created"], &links[1]["created"]),
        (&json!(true), &json!(false)),
        "{b}"
    );
    assert_eq!(rows(&h, "item_links").await, 1);
}

/// kb/ADR-0031 R5, R7 through a plan. A plan attaches web links to an item it creates and to one that exists; each
/// is stored trimmed and in the URL parser's form and logged on its item; the same URL may be on two items; a web
/// link that was there, and one the plan lists twice, answer `created` false; a second apply creates nothing.
#[tokio::test]
async fn a_plan_attaches_web_links_and_reapplying_it_creates_nothing() {
    let h = harness().await;
    let (_, held) = post(
        &h,
        "/api/v1/stories",
        json!({"idempotency_key":"h","title":"Held","kind":"chore","criteria":[],"labels":[]}),
    )
    .await;
    let held = held["value"]["key"].as_str().unwrap().to_string();
    let (st, b) = post(
        &h,
        &format!("/api/v1/items/{held}/web-links"),
        json!({"title": "Board", "url": "https://example.com/board"}),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let held_version = get(&h, &format!("/api/v1/stories/{held}")).await["version"].clone();
    assert!(
        held_version.is_number(),
        "the control: a story read carries its version"
    );

    let scenario = "https://example.com/wiki/scenario";
    let plan = json!({
        "id": "pages",
        "epics": [{"name": "e", "title": "E"}],
        "stories": [{"name": "s", "title": "S", "epic": "e"}],
        "web_links": [
            {"on": "s", "title": "  Scenario page ", "url": "HTTPS://Example.com/wiki/scenario"},
            {"on": "e", "title": "Scenario page", "url": scenario},
            {"on": held, "title": "Board", "url": "https://EXAMPLE.com/board"},
            {"on": "s", "title": "Scenario page", "url": scenario}
        ]
    });
    let (st, b) = post(&h, "/api/v1/plans", plan.clone()).await;
    assert_eq!(st, 200, "{b}");
    let v = b["value"].clone();
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
    let want = |created: [bool; 4]| {
        json!([
            {"on": key("s"), "level": "story", "title": "Scenario page", "url": scenario, "created": created[0]},
            {"on": key("e"), "level": "epic", "title": "Scenario page", "url": scenario, "created": created[1]},
            {"on": held, "level": "story", "title": "Board", "url": "https://example.com/board", "created": created[2]},
            {"on": key("s"), "level": "story", "title": "Scenario page", "url": scenario, "created": created[3]}
        ])
    };
    assert_eq!(v["web_links"], want([true, true, false, false]), "{v}");

    let on_s = get(&h, &format!("/api/v1/items/{}/links", key("s"))).await;
    let listed = on_s["web_links"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{on_s}");
    assert_eq!(
        (&listed[0]["title"], &listed[0]["url"]),
        (&json!("Scenario page"), &json!(scenario))
    );
    let logged = [(key("s"), 1), (key("e"), 1), (held.clone(), 1)];
    for (item, n) in &logged {
        assert_eq!(entries(&h, item, "web_link_added").await, *n, "{item}");
    }
    assert_eq!(rows(&h, "web_links").await, 3);

    let (st, again) = post(&h, "/api/v1/plans", plan).await;
    assert_eq!(st, 200, "{again}");
    assert_eq!(again["value"]["web_links"], want([false; 4]), "{again}");
    for (item, n) in &logged {
        assert_eq!(
            entries(&h, item, "web_link_added").await,
            *n,
            "{item} after the second apply"
        );
    }
    assert_eq!(rows(&h, "web_links").await, 3);

    // kb/ADR-0031 R3: a web link is no field of its item. The plan above only repeated what `held` had, so another
    // plan attaches a web link to it that is new, and the story's version is where it was.
    let (st, more) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "pages-2", "web_links": [{"on": held, "title": "Notes", "url": "https://example.com/notes"}]}),
    )
    .await;
    assert_eq!(st, 200, "{more}");
    assert_eq!(
        more["value"]["web_links"][0]["created"], true,
        "the control: this one was written: {more}"
    );
    assert_eq!(
        get(&h, &format!("/api/v1/stories/{held}")).await["version"],
        held_version,
        "a plan's web link leaves the version of the item it is on alone"
    );
}

/// The all-or-nothing check covers web links: an `on` that names nothing, a bad title, a URL with a user name and
/// a password, a scheme that is not http or https, one URL under two titles on one item within the plan, and a URL
/// an existing item already has under another title are each reported, and nothing of the plan is written. The
/// same URL under another title on a *different* item is no problem, and no message repeats a refused URL.
#[tokio::test]
async fn a_plan_with_a_bad_web_link_writes_nothing_and_names_every_problem() {
    let h = harness().await;
    let mut made = vec![];
    for (idem, title, page, url) in [
        ("h", "Held", "Board", "https://example.com/board"),
        ("o", "Other", "Roadmap", "https://example.com/roadmap"),
    ] {
        let (_, s) = post(
            &h,
            "/api/v1/stories",
            json!({"idempotency_key": idem, "title": title, "kind": "chore", "criteria": [], "labels": []}),
        )
        .await;
        let k = s["value"]["key"].as_str().unwrap().to_string();
        let (st, b) = post(
            &h,
            &format!("/api/v1/items/{k}/web-links"),
            json!({"title": page, "url": url}),
        )
        .await;
        assert_eq!(st, 200, "{b}");
        made.push(k);
    }
    let (held, other) = (made[0].clone(), made[1].clone());
    let (st, b) = post(
        &h,
        "/api/v1/plans",
        json!({
            "id": "bad-pages",
            "stories": [{"name": "a", "title": "A"}, {"name": "b", "title": "B"}],
            "web_links": [
                {"on": "ghost", "title": "Lost", "url": "https://example.com/"},
                {"on": "a", "title": "  ", "url": "https://example.com/"},
                {"on": "a", "title": "Creds", "url": "https://mia:secret@example.com/wiki"},
                {"on": "a", "title": "Ftp", "url": "ftp://example.com/x"},
                {"on": "b", "title": "One", "url": "https://example.com/page"},
                {"on": "b", "title": "Two", "url": "HTTPS://EXAMPLE.com/page"},
                {"on": held, "title": "Kanban", "url": "https://example.com/board"},
                // Not problems: a repeat of what `other` has, and `other`'s URL on `held` under another title.
                // The repeat is here so that `other`'s stored rows are among those the check reads.
                {"on": other, "title": "Roadmap", "url": "https://example.com/roadmap"},
                {"on": held, "title": "Roadmap 2", "url": "https://example.com/roadmap"}
            ]
        }),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert_eq!(b["code"], "validation");
    let m = b["message"].as_str().unwrap();
    for want in [
        "web link \"Lost\" on \"ghost\": on \"ghost\" is neither a name in the plan nor an existing item".to_string(),
        "web link \"\" on \"a\": a web link's title".to_string(),
        "web link \"Creds\" on \"a\": a web link's URL".to_string(),
        "web link \"Ftp\" on \"a\": a web link's URL".to_string(),
        "web link \"Two\" on \"b\": the plan gives \"b\" that URL under two titles (\"One\" and \"Two\")".to_string(),
        format!("web link \"Kanban\" on \"{held}\": this item already has a web link to that URL, titled \"Board\""),
    ] {
        assert!(m.contains(&want), "missing {want:?} in {m}");
    }
    assert!(
        !m.contains("Roadmap 2"),
        "another item's URL is no problem: {m}"
    );
    assert!(
        !m.contains("\"One\" on"),
        "the first title a URL gets is not the problem: {m}"
    );
    assert!(!m.contains("secret"), "a refused URL is not repeated: {m}");
    assert_eq!(
        get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
        2,
        "only the two stories made before the plan"
    );
    assert_eq!(rows(&h, "web_links").await, 2);

    // A misspelt field in an entry is refused as the body is read.
    let (st, _) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "t", "stories": [{"name": "a", "title": "A"}],
               "web_links": [{"on": "a", "title": "T", "link": "https://example.com/"}]}),
    )
    .await;
    assert_eq!(st, 400);
    assert_eq!(
        get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
        2
    );
}

/// "Against what is stored" includes what an earlier apply of this plan stored. A plan edited to give a URL a new
/// title is refused by the check, before anything is written: the message says where in the plan, and the story the
/// edited plan adds is not created.
#[tokio::test]
async fn a_plan_edited_to_retitle_a_web_link_is_refused_before_anything_is_written() {
    let h = harness().await;
    let plan = |title: &str, more: bool| {
        let mut stories = vec![json!({"name": "s", "title": "S"})];
        if more {
            stories.push(json!({"name": "t", "title": "T"}));
        }
        json!({
            "id": "edited",
            "stories": stories,
            "web_links": [{"on": "s", "title": title, "url": "https://example.com/page"}]
        })
    };
    let (st, b) = post(&h, "/api/v1/plans", plan("One", false)).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"]["web_links"][0]["created"], true);

    let (st, b) = post(&h, "/api/v1/plans", plan("Two", true)).await;
    assert_eq!(st, 400, "{b}");
    let m = b["message"].as_str().unwrap();
    assert!(
        m.contains("web link \"Two\" on \"s\": this item already has a web link to that URL, titled \"One\""),
        "{m}"
    );
    assert_eq!(
        get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
        1,
        "the story the edited plan adds was not created"
    );
    assert_eq!(rows(&h, "web_links").await, 1);
}

/// "Within the plan" is about the item, not about how the plan spells it. A plan applied again that gives one
/// item one URL under two titles, naming the item by its plan name in one entry and by its key in the other, is
/// refused by the check: nothing is written, and the story the edited plan adds does not exist afterwards.
#[tokio::test]
async fn a_plan_web_link_on_one_item_under_two_spellings_cannot_have_two_titles() {
    let h = harness().await;
    let (st, b) = post(
        &h,
        "/api/v1/plans",
        json!({"id": "two-spellings", "stories": [{"name": "s", "title": "S"}]}),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let s = b["value"]["items"][0]["key"].as_str().unwrap().to_string();

    let (st, b) = post(
        &h,
        "/api/v1/plans",
        json!({
            "id": "two-spellings",
            "stories": [{"name": "s", "title": "S"}, {"name": "t", "title": "T"}],
            "web_links": [
                {"on": "s", "title": "One", "url": "https://example.com/page"},
                {"on": s, "title": "Two", "url": "https://example.com/page"}
            ]
        }),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert_eq!(b["code"], "validation");
    let want = format!(
        "web link \"Two\" on \"{s}\": the plan gives \"{s}\" that URL under two titles (\"One\" and \"Two\")"
    );
    assert!(
        b["message"].as_str().unwrap().contains(&want),
        "missing {want:?} in {b}"
    );
    assert_eq!(
        get(&h, "/api/v1/stories").await.as_array().unwrap().len(),
        1,
        "the story the edited plan adds was not created"
    );
    assert_eq!(rows(&h, "web_links").await, 0, "neither title was written");
    assert_eq!(entries(&h, &s, "web_link_added").await, 0);

    // The control: the same two spellings with one title are one web link, listed twice.
    let (st, b) = post(
        &h,
        "/api/v1/plans",
        json!({
            "id": "two-spellings",
            "stories": [{"name": "s", "title": "S"}],
            "web_links": [
                {"on": "s", "title": "One", "url": "https://example.com/page"},
                {"on": s, "title": "One", "url": "https://example.com/page"}
            ]
        }),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let created: Vec<_> = b["value"]["web_links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["created"].clone())
        .collect();
    assert_eq!(created, vec![json!(true), json!(false)], "{b}");
    assert_eq!(rows(&h, "web_links").await, 1);
}
