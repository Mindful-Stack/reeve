mod common;
use axum::body::{Body, BodyDataStream};
use axum::http::Request;
use common::*;
use futures_util::StreamExt;
use serde_json::json;
use std::time::Duration;

/// Reads chunks until a blank line ends a frame, skipping `:` comment lines (keep-alives); `None` when the body
/// ends first. axum writes each event as one chunk, so no bytes may follow the blank line (asserted, not dropped).
async fn next_frame(body: &mut BodyDataStream) -> Option<String> {
    let mut buf = String::new();
    loop {
        let chunk = body.next().await?.ok()?;
        buf.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
        while let Some(end) = buf.find("\n\n") {
            let raw: String = buf.drain(..end + 2).collect();
            let frame = raw
                .lines()
                .filter(|l| !l.is_empty() && !l.starts_with(':'))
                .collect::<Vec<_>>()
                .join("\n");
            if !frame.is_empty() {
                assert!(buf.is_empty(), "bytes after a frame: {buf:?}");
                return Some(frame);
            }
        }
    }
}

#[tokio::test]
async fn events_stream_replays_after_last_event_id_and_delivers_new_entries() {
    let h = harness().await;
    // the harness already logged token_issued entries; resume from the newest of those so the replay starts at our seed
    let (_, latest) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/activity?limit=1",
        None,
    )
    .await;
    let after = latest["value"][0]["id"].as_i64().unwrap();
    let (_, ini) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"i1","title":"A","horizon":"Now"})),
    )
    .await;
    // open the stream with Last-Event-ID = after → the seed arrives as `event: activity`, `id: <id>`, data = ActivityEntryDto JSON
    let res = call_raw(
        &h.app,
        Request::get("/api/v1/events")
            .header("authorization", format!("Bearer {}", h.admin_token))
            .header("last-event-id", after.to_string())
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["content-type"], "text/event-stream");
    assert_eq!(res.headers()["x-accel-buffering"], "no");
    let mut body = res.into_body().into_data_stream();
    let cursor = tokio::time::timeout(Duration::from_secs(5), next_frame(&mut body))
        .await
        .expect("cursor frame")
        .unwrap();
    assert!(
        cursor.contains("event: cursor") && cursor.contains(&format!("id: {after}")),
        "{cursor}"
    );
    let first = tokio::time::timeout(Duration::from_secs(5), next_frame(&mut body))
        .await
        .expect("first frame")
        .unwrap();
    assert!(
        !first.contains("token_issued"),
        "entries at or before the cursor must not replay: {first}"
    );
    assert!(
        first.contains("event: activity")
            && first.contains("\"entity_key\":\"")
            && first.contains(ini["value"]["key"].as_str().unwrap()),
        "{first}"
    );
    // a new mutation shows up within the poll interval
    call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key":"e1","title":"E"})),
    )
    .await;
    let second = tokio::time::timeout(Duration::from_secs(5), next_frame(&mut body))
        .await
        .expect("second frame")
        .unwrap();
    assert!(second.contains("\"entity_type\":\"epic\""), "{second}");
    // unauthenticated → 401 JSON, not a stream
    assert_eq!(
        call_raw(
            &h.app,
            Request::get("/api/v1/events").body(Body::empty()).unwrap()
        )
        .await
        .status(),
        401
    );
}

#[tokio::test]
async fn a_revoked_session_stops_receiving_within_one_poll() {
    let h = harness().await;
    // a fresh interactive token for bob, used only by the stream
    let (_, t) = call(
        &h.app,
        Some(&h.victim_token),
        "POST",
        "/api/v1/tokens",
        Some(json!({"kind":"interactive","label":"stream"})),
    )
    .await;
    let secret = t["value"]["secret"].as_str().unwrap().to_string();
    let id = t["value"]["token"]["id"].as_i64().unwrap();
    let res = call_raw(
        &h.app,
        Request::get("/api/v1/events")
            .header("authorization", format!("Bearer {secret}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    let mut body = res.into_body().into_data_stream();
    // establish a running stream first: the cursor frame, then one real activity frame
    let _cursor = tokio::time::timeout(Duration::from_secs(5), next_frame(&mut body))
        .await
        .expect("cursor")
        .unwrap();
    call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"i-rev","title":"Before","horizon":"Now"})),
    )
    .await;
    let before = tokio::time::timeout(Duration::from_secs(5), next_frame(&mut body))
        .await
        .expect("frame before revoke")
        .unwrap();
    assert!(before.contains("event: activity"));
    call(
        &h.app,
        Some(&h.victim_token),
        "DELETE",
        &format!("/api/v1/tokens/{id}"),
        None,
    )
    .await;
    call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"i-after","title":"After","horizon":"Now"})),
    )
    .await;
    // the body must end (None) within ~2 polls; a frame after the revoke is a failure
    let end = tokio::time::timeout(Duration::from_secs(6), async {
        while let Some(chunk) = body.next().await {
            let c = chunk.unwrap();
            assert!(
                !String::from_utf8_lossy(&c).contains("\"After\""),
                "frame after revoke"
            );
        }
    })
    .await;
    assert!(
        end.is_ok(),
        "stream did not end after the token was revoked"
    );
}

#[tokio::test]
async fn a_resume_point_beyond_the_newest_entry_still_delivers_the_next_write() {
    let h = harness().await;
    let res = call_raw(
        &h.app,
        Request::get("/api/v1/events")
            .header("authorization", format!("Bearer {}", h.admin_token))
            .header("last-event-id", (i64::MAX - 1).to_string())
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    let mut body = res.into_body().into_data_stream();
    let cursor = tokio::time::timeout(Duration::from_secs(5), next_frame(&mut body))
        .await
        .expect("cursor frame")
        .unwrap();
    assert!(
        cursor.contains("event: cursor") && !cursor.contains(&(i64::MAX - 1).to_string()),
        "{cursor}"
    );
    call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"i-clamp","title":"Clamped","horizon":"Now"})),
    )
    .await;
    let frame = tokio::time::timeout(Duration::from_secs(5), next_frame(&mut body))
        .await
        .expect("frame after clamped resume")
        .unwrap();
    assert!(
        frame.contains("event: activity") && frame.contains("Clamped"),
        "{frame}"
    );
}

/// With a 50 ms poll, three consecutive mutations each reach an open stream within 1 s. Mutation 0 is answered by the
/// stream's immediate first read, but mutations 1 and 2 each start a fresh poll sleep after their POST, so under a
/// fixed two-second poll they would wait at least 2 s, beyond the 1 s bound.
#[tokio::test]
async fn events_poll_interval_is_configuration() {
    let h = harness_patched(|c| c.events.poll_ms = 50).await;
    let res = call_raw(
        &h.app,
        Request::get("/api/v1/events")
            .header("authorization", format!("Bearer {}", h.admin_token))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), 200);
    let mut body = res.into_body().into_data_stream();
    let cursor = tokio::time::timeout(Duration::from_secs(5), next_frame(&mut body))
        .await
        .expect("cursor")
        .unwrap();
    assert!(cursor.contains("event: cursor"), "{cursor}");
    for i in 0..3 {
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/epics",
            Some(json!({"idempotency_key": format!("poll-{i}"), "title": format!("E{i}")})),
        )
        .await;
        let frame = tokio::time::timeout(Duration::from_millis(1000), next_frame(&mut body))
            .await
            .unwrap_or_else(|_| panic!("mutation {i} not delivered within 1 s"))
            .unwrap();
        assert!(frame.contains("\"entity_type\":\"epic\""), "{frame}");
        assert!(frame.contains(&format!("E{i}")), "{frame}");
    }
}
