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
        format!("q={}", urlencoding("  abcdefg  ")),
    ] {
        let (st, b) = get(bad.clone()).await;
        assert_eq!(st, 400, "{bad}: {st} {b}");
        assert_eq!(b["code"], "validation", "{bad}: {b}");
    }
    let (st, b) = call(
        &h.app,
        Some(&h.bot_token),
        "GET",
        "/api/v1/stories/similar?q=long+enough+text",
        None,
    )
    .await;
    assert_eq!(st, 200, "{b}");
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
async fn similar_accepts_the_boundaries_and_counts_code_points() {
    let h = harness().await;
    for q in [
        "q=abcdefgh".to_string(),
        format!("q={}", urlencoding(&"\u{e9}".repeat(200))),
        "q=long+enough+text&limit=1".to_string(),
        "q=long+enough+text&limit=10".to_string(),
    ] {
        let (st, b) = call(
            &h.app,
            Some(&h.victim_token),
            "GET",
            &format!("/api/v1/stories/similar?{q}"),
            None,
        )
        .await;
        assert_eq!(st, 200, "{q}: {b}");
    }
}

/// Done stories count only within `[board] done_window_days`, measured from the handler's clock.
#[tokio::test]
async fn similar_lists_done_stories_only_inside_the_done_window() {
    use chrono::{Duration, Utc};
    let h = harness_with(|cfg| format!("{cfg}\n[board]\ndone_window_days = 7\n")).await;
    let t = Some(h.admin_token.as_str());
    let mut keys = Vec::new();
    for (i, title) in [
        "The export PDF is blank",
        "Export to PDF gives a blank page",
    ]
    .iter()
    .enumerate()
    {
        let (st, s) = call(&h.app, t, "POST", "/api/v1/stories", Some(json!({"idempotency_key":format!("w{i}"),"title":title,"kind":"bug","criteria":["ok"],"labels":[]}))).await;
        assert_eq!(st, 200, "{s}");
        keys.push(s["value"]["key"].as_str().unwrap().to_string());
    }
    for k in &keys {
        let (st, b) = call(
            &h.app,
            t,
            "POST",
            &format!("/api/v1/stories/{k}/move"),
            Some(json!({"version":1,"to":"done","resolution":"dropped"})),
        )
        .await;
        assert_eq!(st, 200, "{b}");
    }
    for (k, age) in [
        (&keys[0], Duration::days(7) + Duration::hours(1)),
        (&keys[1], Duration::days(6) + Duration::hours(23)),
    ] {
        sqlx::query("UPDATE stories SET done_at = $1 WHERE key = $2")
            .bind(Utc::now() - age)
            .bind(k)
            .execute(&h.state.pool)
            .await
            .unwrap();
    }
    let (st, b) = call(
        &h.app,
        t,
        "GET",
        "/api/v1/stories/similar?q=PDF+export+shows+a+blank+page",
        None,
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(
        b["value"],
        json!([{"key": keys[1], "kind": "bug", "title": "Export to PDF gives a blank page", "epic": null, "delivery": "done", "resolution": "dropped"}])
    );
}
