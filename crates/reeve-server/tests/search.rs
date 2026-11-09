//! Search items by text across titles, briefs, descriptions and Jira references; a query naming a key finds
//! that item first.
mod common;
use common::*;
use serde_json::json;

#[tokio::test]
async fn search_finds_items_by_title_text_or_reference_newest_first() {
    let h = harness().await;
    let post = |path: &'static str, body: serde_json::Value| {
        let (app, token) = (h.app.clone(), h.admin_token.clone());
        async move {
            let (st, b) = call(&app, Some(&token), "POST", path, Some(body)).await;
            assert_eq!(st, 200, "{b}");
            b["value"]["key"].as_str().unwrap().to_string()
        }
    };
    let ini = post(
        "/api/v1/initiatives",
        json!({"idempotency_key":"i","title":"Leave the old host","horizon":"Now","brief":"Move every environment to NorthCloud.","external_ref":"JPD-30"}),
    )
    .await;
    let epic = post(
        "/api/v1/epics",
        json!({"idempotency_key":"e","title":"Hosting on NorthCloud","initiative":ini}),
    )
    .await;
    let story = post(
        "/api/v1/stories",
        json!({"idempotency_key":"s","title":"Move the database","kind":"bug","epic":epic,"description":"Needs a 50% discount; see northcloud docs","criteria":[],"labels":[]}),
    )
    .await;
    post(
        "/api/v1/stories",
        json!({"idempotency_key":"o","title":"Unrelated","kind":"story","criteria":[],"labels":[]}),
    )
    .await;

    let search = |q: &str| {
        let (app, token) = (h.app.clone(), h.victim_token.clone());
        let path = format!("/api/v1/search?q={}", urlencoding(q));
        async move {
            let (st, b) = call(&app, Some(&token), "GET", &path, None).await;
            assert_eq!(st, 200, "{b}");
            b["value"].clone()
        }
    };
    // case-insensitive, across levels, saying where it matched; title hits before text hits, newest first within each
    let hits = search("northcloud").await;
    assert_eq!(
        hits,
        json!([
            {"key": epic, "item": "epic", "kind": null, "title": "Hosting on NorthCloud", "matched": ["title"], "exact": false},
            {"key": story, "item": "story", "kind": "bug", "title": "Move the database", "matched": ["description"], "exact": false},
            {"key": ini, "item": "initiative", "kind": null, "title": "Leave the old host", "matched": ["brief"], "exact": false}
        ])
    );
    assert_eq!(
        search("jpd-30").await[0]["matched"],
        json!(["external_ref"])
    );
    // LIKE wildcards in the text are literal
    assert_eq!(search("50%").await.as_array().unwrap().len(), 1);
    assert_eq!(search("%").await.as_array().unwrap().len(), 1);
    assert_eq!(search("nothing like this").await, json!([]));
    // a blank query is refused rather than listing everything
    let (st, _) = call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        "/api/v1/search?q=%20",
        None,
    )
    .await;
    assert_eq!(st, 400);
}

/// A number, `#n` or a key in any case finds the item with that key first, above newer items that only mention it; an
/// exact external reference comes next, then titles, then text.
#[tokio::test]
async fn search_puts_the_key_then_the_exact_reference_then_titles_first() {
    let mut h =
        harness_with(|cfg| cfg.replace("key_prefix = \"TST\"", "key_prefix = \"OLD\"")).await;
    let post = |h: &Harness, path: &'static str, body: serde_json::Value| {
        let (app, token) = (h.app.clone(), h.admin_token.clone());
        async move {
            let (st, b) = call(&app, Some(&token), "POST", path, Some(body)).await;
            assert_eq!(st, 200, "{b}");
            b["value"]["key"].as_str().unwrap().to_string()
        }
    };
    let story = |idem: &str, title: &str, description: &str| json!({"idempotency_key":idem,"title":title,"kind":"story","description":description,"criteria":[],"labels":[]});
    let old = post(&h, "/api/v1/stories", story("old", "Issued under OLD", "")).await;
    assert_eq!(old, "OLD-1");
    // the prefix changes; OLD-1 keeps its key
    let mut cfg = (*h.state.config).clone();
    cfg.key_prefix = "TST".into();
    let state = reeve_server::build_state(cfg, h.state.pool.clone())
        .await
        .unwrap();
    h.app = reeve_server::app(state.clone());
    h.state = state;
    // the exact reference is the oldest, so only its tier can put it above the newer competitors
    let exact_ref = post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"q","title":"Exact","horizon":"Now","external_ref":"JPD-30"}),
    )
    .await;
    let target = post(&h, "/api/v1/stories", story("t", "The target", "")).await;
    let n = target.strip_prefix("TST-").unwrap().to_string();
    // newer items whose text, title or reference mentions the number
    let in_text = post(
        &h,
        "/api/v1/stories",
        story("x", "Mentions it", &format!("see {target} and step {n}")),
    )
    .await;
    let in_title = post(
        &h,
        "/api/v1/stories",
        story("y", &format!("Step {n} of the plan"), ""),
    )
    .await;
    let partial_ref = post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key":"p","title":"Partial","horizon":"Now","external_ref":"JPD-300"}),
    )
    .await;
    let ref_in_title = post(&h, "/api/v1/stories", story("z", "Follows up JPD-30", "")).await;
    // a newer item whose external reference is the earlier prefix's key
    let old_ref = post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key":"r","title":"Points at OLD-1","external_ref":"OLD-1"}),
    )
    .await;

    let keys = |hits: serde_json::Value| -> Vec<String> {
        hits.as_array()
            .unwrap()
            .iter()
            .map(|x| x["key"].as_str().unwrap().to_string())
            .collect()
    };
    let search = |q: String| {
        let (app, token) = (h.app.clone(), h.victim_token.clone());
        let path = format!("/api/v1/search?q={}", urlencoding(&q));
        async move {
            let (st, b) = call(&app, Some(&token), "GET", &path, None).await;
            assert_eq!(st, 200, "{b}");
            b["value"].clone()
        }
    };
    for q in [
        n.clone(),
        format!("#{n}"),
        target.clone(),
        target.to_lowercase(),
        format!(" {target} "),
    ] {
        let hits = search(q.clone()).await;
        assert_eq!(hits[0]["key"], target.as_str(), "{q}: {hits}");
        assert_eq!(hits[0]["matched"], json!(["key"]), "{q}");
        assert_eq!(hits[0]["exact"], true, "{q}");
        assert!(
            hits.as_array().unwrap()[1..]
                .iter()
                .all(|x| x["exact"] == false),
            "{q}: {hits}"
        );
    }
    // the key first, then title hits, then text hits
    assert_eq!(
        keys(search(n.clone()).await),
        [target.clone(), in_title.clone(), in_text.clone()],
    );
    // a key issued under an earlier prefix, in any case, above a newer item whose reference is that key
    assert_eq!(
        keys(search("old-1".into()).await),
        ["OLD-1", old_ref.as_str()]
    );
    let hits = search("OLD-1".into()).await;
    assert_eq!(hits[0]["matched"], json!(["key"]));
    assert_eq!(hits[1]["matched"], json!(["title", "external_ref"]));
    assert_eq!(
        (hits[0]["exact"].clone(), hits[1]["exact"].clone()),
        (json!(true), json!(true))
    );
    // an exact external reference first, whatever its age; then titles; then a partial reference
    let hits = search("jpd-30".into()).await;
    assert_eq!(
        keys(hits.clone()),
        [exact_ref.clone(), ref_in_title.clone(), partial_ref.clone()]
    );
    assert_eq!(
        hits.as_array()
            .unwrap()
            .iter()
            .map(|x| x["exact"].clone())
            .collect::<Vec<_>>(),
        [json!(true), json!(false), json!(false)]
    );
    // a key that names no item finds only text
    assert_eq!(keys(search("TST-999".into()).await), Vec::<String>::new());
    assert_eq!(keys(search("999".into()).await), Vec::<String>::new());
    // the full key: the item, then the newer one whose text names it
    let hits = search(target.clone()).await;
    assert_eq!(keys(hits.clone()), [target.clone(), in_text.clone()]);
    assert_eq!(hits[1]["matched"], json!(["description"]));
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
