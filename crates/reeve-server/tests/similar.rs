//! `GET /stories/similar`: the duplicate check behind the Report dialog.
mod common;
use common::*;
use serde_json::json;

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tokio::test]
async fn similar_finds_a_reworded_title_and_validates_its_query() {
    let h = harness().await;
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"s","title":"The export PDF is blank","kind":"bug","criteria":[],"labels":[]})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let key = b["value"]["key"].as_str().unwrap().to_string();
    let get = |q: String| {
        let (app, t) = (h.app.clone(), h.victim_token.clone());
        async move {
            call(
                &app,
                Some(&t),
                "GET",
                &format!("/api/v1/stories/similar?{q}"),
                None,
            )
            .await
        }
    };

    let (st, b) = get(format!(
        "q={}",
        urlencoding("PDF export shows a blank page")
    ))
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(
        b["value"],
        json!([{"key": key, "kind": "bug", "title": "The export PDF is blank", "epic": null, "delivery": "todo", "resolution": null}])
    );

    for bad in [
        "q=short".to_string(),
        format!("q={}", "x".repeat(201)),
        "q=long+enough+text&limit=0".to_string(),
        "q=long+enough+text&limit=11".to_string(),
        "q=long+enough+text&limit=-1".to_string(),
        "q=long+enough+text&limit=two".to_string(),
        "limit=5".to_string(),
    ] {
        let (st, b) = get(bad.clone()).await;
        assert_eq!(st, 400, "{bad}: {st} {b}");
    }
    let (st, _) = call(
        &h.app,
        None,
        "GET",
        "/api/v1/stories/similar?q=long+enough+text",
        None,
    )
    .await;
    assert_eq!(st, 401);
}

#[tokio::test]
async fn similar_is_not_shadowed_by_the_story_key_route() {
    let h = harness().await;
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/stories/similar?q=nothing+like+it+here",
        None,
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"], json!([]));
}
