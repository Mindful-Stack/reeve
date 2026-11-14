//! The server's clock (`AppState.clock`): the callers read it, so a test moves time instead of waiting.
mod common;
use chrono::Duration;
use common::*;

async fn me(h: &Harness, token: &str) -> u16 {
    call(&h.app, Some(token), "GET", "/api/v1/me", None).await.0
}

async fn discover(h: &Harness, token: &str) -> u16 {
    send(
        &h.app,
        mcp_modern(token, 1, "server/discover", serde_json::json!({})),
    )
    .await
    .0
}

/// Catches a `Caller` or an `McpCaller` that reads the system time, and an expiry boundary on the wrong side:
/// a one-hour token works one second before its hour is up and is refused exactly at it. Nothing here depends on
/// how long the test takes: the clock is fixed and both tokens are issued at its instant.
#[tokio::test]
async fn both_callers_read_the_servers_clock() {
    let h = mcp_harness().await.with_fixed_clock();
    assert_eq!(h.state.clock.now(), h.started);
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = h.state.activity.clone();
    let policy = h.state.access.load(&h.state.identity).await.unwrap();
    let api = reeve_identity::issue_token(
        &h.state.pool,
        &*h.state.tokens,
        &*log,
        &policy,
        "test",
        reeve_protocol::Via::Interactive,
        "dan@example.com",
        reeve_protocol::TokenKind::Interactive,
        "one hour",
        Some(h.started + Duration::hours(1)),
        h.started,
    )
    .await
    .unwrap()
    .secret;
    let mcp = issue_mcp_token(&h, "dan@example.com").await.secret;
    h.state
        .clock
        .advance(Duration::hours(1) - Duration::seconds(1));
    assert_eq!((me(&h, &api).await, discover(&h, &mcp).await), (200, 200));
    h.state.clock.advance(Duration::seconds(1));
    assert_eq!((me(&h, &api).await, discover(&h, &mcp).await), (401, 401));
}
