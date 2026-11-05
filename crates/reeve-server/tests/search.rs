//! Search items by text across titles, briefs, descriptions and Jira references, newest first.
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
    // case-insensitive, across levels, newest first, saying where it matched
    let hits = search("northcloud").await;
    assert_eq!(
        hits,
        json!([
            {"key": story, "item": "story", "kind": "bug", "title": "Move the database", "matched": ["description"]},
            {"key": epic, "item": "epic", "kind": null, "title": "Hosting on NorthCloud", "matched": ["title"]},
            {"key": ini, "item": "initiative", "kind": null, "title": "Leave the old host", "matched": ["brief"]}
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

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
