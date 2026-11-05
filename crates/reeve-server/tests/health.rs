//! `GET /health` reports the database, not just the process: the container healthcheck and a deploy's gate rely on
//! it.
mod common;
use common::*;
use std::time::{Duration, Instant};

#[tokio::test]
async fn health_answers_503_when_the_database_is_gone() {
    let (c, h) = harness_in_own_container().await;
    assert_eq!(
        call(&h.app, None, "GET", "/health", None).await,
        (200, serde_json::json!({"ok": true}))
    );
    // The database goes away; the pool still holds connections to it.
    c.stop().await.expect("stop the PostgreSQL container");
    let started = Instant::now();
    assert_eq!(
        call(&h.app, None, "GET", "/health", None).await,
        (503, serde_json::json!({"ok": false}))
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "health must answer within its own timeout, took {:?}",
        started.elapsed()
    );
}
