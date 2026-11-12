//! `GET /stories` filters on the server (kb/ADR-0021): text, kind, initiative, urgency, not done and relevance
//! order, beside the existing filters, whose meaning is unchanged.
mod common;
use common::*;
use serde_json::{Value, json};

async fn post(h: &Harness, path: &str, body: Value) -> Value {
    let (st, b) = call(&h.app, Some(&h.admin_token), "POST", path, Some(body)).await;
    assert_eq!(st, 200, "{path}: {b}");
    b["value"].clone()
}

async fn new_story(h: &Harness, idem: &str, body: Value) -> String {
    let mut b = json!({"idempotency_key": idem, "kind": "story", "criteria": [], "labels": []});
    for (k, v) in body.as_object().unwrap() {
        b[k] = v.clone();
    }
    post(h, "/api/v1/stories", b).await["key"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The keys `GET /stories?<query>` lists, in order; the status must be 200.
async fn keys(h: &Harness, query: &str) -> Vec<String> {
    let (st, b) = get(h, query).await;
    assert_eq!(st, 200, "{query}: {b}");
    b["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["story"]["key"].as_str().unwrap().to_string())
        .collect()
}

async fn get(h: &Harness, query: &str) -> (u16, Value) {
    call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        &format!("/api/v1/stories?{query}"),
        None,
    )
    .await
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[tokio::test]
async fn every_new_parameter_narrows_the_story_list() {
    let h = harness().await;
    let ini = post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"i","title":"Hosting","horizon":"Now","external_ref":"JPD-30"}),
    )
    .await["key"]
        .as_str()
        .unwrap()
        .to_string();
    let epic = post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"e","title":"Move","initiative":ini}),
    )
    .await["key"]
        .as_str()
        .unwrap()
        .to_string();
    let other_epic = post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"e2","title":"Elsewhere"}),
    )
    .await["key"]
        .as_str()
        .unwrap()
        .to_string();
    let invoice_bug = new_story(
        &h,
        "ib",
        json!({"title":"Invoice totals are wrong","kind":"bug","epic":epic}),
    )
    .await;
    let invoice_story = new_story(
        &h,
        "is",
        json!({"title":"Plain","description":"the invoice pdf","epic":other_epic}),
    )
    .await;
    let urgent = new_story(&h, "u", json!({"title":"On fire"})).await;
    let done = new_story(&h, "d", json!({"title":"Dropped invoice idea"})).await;
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{urgent}/urgent"),
        Some(json!({})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{done}/move"),
        Some(json!({"version":1,"to":"done","resolution":"dropped"})),
    )
    .await;
    assert_eq!(st, 200, "{b}");

    let all = keys(&h, "").await;
    assert_eq!(
        sorted(all.clone()),
        sorted(vec![
            invoice_bug.clone(),
            invoice_story.clone(),
            urgent.clone(),
            done.clone()
        ])
    );
    // q: title, description; the done story matches too unless done=none
    assert_eq!(
        sorted(keys(&h, "q=INVOICE").await),
        sorted(vec![
            invoice_bug.clone(),
            invoice_story.clone(),
            done.clone()
        ])
    );
    assert_eq!(
        sorted(keys(&h, "q=invoice&done=none").await),
        sorted(vec![invoice_bug.clone(), invoice_story.clone()])
    );
    let open = keys(&h, "done=none").await;
    assert!(!open.contains(&done), "{open:?}");
    assert_eq!(open.len(), 3);
    // kind
    assert_eq!(keys(&h, "kind=bug").await, vec![invoice_bug.clone()]);
    // initiative, by key and by external reference
    assert_eq!(
        keys(&h, &format!("initiative={ini}")).await,
        vec![invoice_bug.clone()]
    );
    assert_eq!(
        keys(&h, "initiative=JPD-30").await,
        vec![invoice_bug.clone()]
    );
    assert_eq!(keys(&h, "initiative=JPD-404").await, Vec::<String>::new());
    // epic and initiative combine: an epic outside the initiative keeps nothing, one inside keeps its stories
    assert_eq!(
        keys(&h, &format!("epic={other_epic}&initiative={ini}")).await,
        Vec::<String>::new()
    );
    assert_eq!(
        keys(&h, &format!("epic={epic}&initiative={ini}")).await,
        vec![invoice_bug.clone()]
    );
    // urgent: only the row shown urgent
    let (st, b) = get(&h, "urgent=true").await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"].as_array().unwrap().len(), 1);
    assert_eq!(b["value"][0]["story"]["key"], urgent.as_str());
    assert!(
        !b["value"][0]["story"]["urgent_sources"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // the existing parameters keep their meaning
    assert_eq!(
        sorted(keys(&h, "untriaged=true").await),
        sorted(vec![urgent.clone()]),
        "untriaged: no epic and not done"
    );
    assert_eq!(keys(&h, "delivery=done").await, vec![done.clone()]);
    assert_eq!(
        keys(&h, "delivery=done&done=none").await,
        Vec::<String>::new()
    );
    assert_eq!(
        keys(&h, "done=recent").await,
        all,
        "done just now is recent"
    );
    assert_eq!(
        keys(&h, &format!("epic={other_epic}")).await,
        vec![invoice_story.clone()]
    );
    assert_eq!(keys(&h, "epic=JPD-404").await, Vec::<String>::new());
}

/// Refusals are validation errors in the envelope, and validation runs before the empty result of a reference that
/// names nothing.
#[tokio::test]
async fn the_story_list_refuses_bad_text_sort_and_values() {
    let h = harness().await;
    for query in [
        "q=",
        "q=%20%20",
        &format!("q={}", "a".repeat(201)),
        "sort=relevance",
        "initiative=JPD-404&q=",
        "epic=JPD-404&sort=relevance",
        "kind=epic",
        "sort=x",
        "done=x",
    ] {
        let (st, b) = get(&h, query).await;
        assert_eq!(st, 400, "{query}: {b}");
        assert_eq!(b["code"], "validation", "{query}: {b}");
    }
}

/// Under `sort=relevance` an exact external reference comes first, then title matches, then the rest, each in rank
/// order (state first: a started title match follows the todo one though it was filed first); the default order is
/// today's (rank).
#[tokio::test]
async fn relevance_puts_the_exact_reference_then_titles_first() {
    let h = harness().await;
    let started = new_story(&h, "s", json!({"title":"SCRUM-77 started"})).await;
    post(
        &h,
        &format!("/api/v1/stories/{started}/move"),
        json!({"version":1,"to":"in_progress","override_reason":"the test starts it"}),
    )
    .await;
    let mention = new_story(
        &h,
        "m",
        json!({"title":"Unrelated","description":"see scrum-77"}),
    )
    .await;
    let in_title = new_story(&h, "t", json!({"title":"Follow up SCRUM-77"})).await;
    let exact = new_story(
        &h,
        "x",
        json!({"title":"Imported","external_ref":"SCRUM-77"}),
    )
    .await;
    let longer = new_story(
        &h,
        "l",
        json!({"title":"Imported too","external_ref":"SCRUM-770"}),
    )
    .await;
    let _other = new_story(&h, "o", json!({"title":"Nothing"})).await;
    assert_eq!(
        keys(&h, "q=scrum-77").await,
        vec![
            mention.clone(),
            in_title.clone(),
            exact.clone(),
            longer.clone(),
            started.clone()
        ],
        "rank order by default"
    );
    assert_eq!(
        keys(&h, "q=scrum-77&sort=relevance").await,
        vec![exact, in_title, started, mention, longer]
    );
}

/// `promised` reaches the handler: the request's clock is the server's, so the dates are relative to today (the
/// handler tests pin the exact boundaries). An active sprint ends today + 10; one story is promised today + 3, the
/// other today + 20. An unknown value is refused like every other bad value.
#[tokio::test]
async fn promised_parameter_reaches_the_handler() {
    let h = harness().await;
    post(
        &h,
        "/api/v1/goals",
        json!({"idempotency_key": "g", "outcome": "o", "check": "c"}),
    )
    .await;
    let today = chrono::Utc::now().date_naive();
    let (st, all) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/sprints", None).await;
    assert_eq!(st, 200, "{all}");
    let draft = all["value"]["draft"].clone();
    post(
        &h,
        &format!("/api/v1/sprints/{}/open", draft["id"]),
        json!({"version": draft["version"], "ends_on": today + chrono::Days::new(10)}),
    )
    .await;
    let soon = new_story(&h, "soon", json!({"title": "Soon"})).await;
    let later = new_story(&h, "later", json!({"title": "Later"})).await;
    let _unpromised = new_story(&h, "none", json!({"title": "None"})).await;
    for (key, days) in [(&soon, 3), (&later, 20)] {
        post(
            &h,
            &format!("/api/v1/stories/{key}/promise"),
            json!({"on": today + chrono::Days::new(days)}),
        )
        .await;
    }
    assert_eq!(keys(&h, "promised=this_sprint").await, vec![soon.clone()]);
    assert_eq!(
        sorted(keys(&h, "promised=any").await),
        sorted(vec![soon, later])
    );
    let (st, b) = get(&h, "promised=bogus").await;
    assert_eq!(st, 400, "{b}");
    assert_eq!(b["code"], "validation", "{b}");
}
