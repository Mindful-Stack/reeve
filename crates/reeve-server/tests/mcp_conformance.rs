//! `/mcp` against the official SDK's client (rmcp), over a real loopback port, in both protocol revisions. rmcp is
//! one implementation's reading of the specification, written independently of this server and of the request
//! builders in `common`; claude.ai and ChatGPT themselves are tried by hand (see the README).
mod common;
use common::*;
use rmcp::{
    ClientLifecycleMode, ClientServiceExt, ServiceExt,
    model::{CallToolRequestParams, ProtocolVersion},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::json;

/// The harness's router on a loopback port of its own; returns the endpoint's URL.
async fn serve(h: &Harness) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = h.app.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/mcp")
}

fn config(url: &str, token: &str) -> StreamableHttpClientTransportConfig {
    // The token without the `Bearer ` prefix.
    StreamableHttpClientTransportConfig::with_uri(url.to_string()).auth_header(token)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sdk_client_discovers_and_lists_tools_over_2026_07_28() {
    let h = mcp_harness().await;
    let token = issue_mcp_token(&h, "dan@example.com").await.secret;
    let url = serve(&h).await;
    let client = ()
        .serve_with_lifecycle(
            StreamableHttpClientTransport::from_config(config(&url, &token)),
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .expect("server/discover is answered");
    let info = client.peer_info().expect("the discover result is kept");
    assert_eq!(info.protocol_version, ProtocolVersion::V_2026_07_28);
    assert_eq!(
        info.server_info.as_ref().map(|i| i.name.as_str()),
        Some("reeve")
    );
    let tools = client
        .list_all_tools()
        .await
        .expect("tools/list is answered");
    assert_eq!(
        tools.iter().map(|t| &*t.name).collect::<Vec<&str>>(),
        vec!["get_story"]
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sdk_client_shakes_hands_and_lists_tools_over_2025_11_25() {
    let h = mcp_harness().await;
    let token = issue_mcp_token(&h, "dan@example.com").await.secret;
    let url = serve(&h).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_config(config(
            &url, &token,
        )))
        .await
        .expect("initialize is answered");
    let info = client.peer_info().expect("the initialize result is kept");
    assert_eq!(info.protocol_version, ProtocolVersion::V_2025_11_25);
    let tools = client
        .list_all_tools()
        .await
        .expect("tools/list is answered");
    assert_eq!(
        tools.iter().map(|t| &*t.name).collect::<Vec<&str>>(),
        vec!["get_story"]
    );
    client.cancel().await.unwrap();
}

/// A story created through the API as the admin; returns its key.
async fn a_story(h: &Harness) -> String {
    let epic = ready_epic(h, "conf-e1").await;
    let (st, story) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key": "conf-s1", "title": "Booking", "kind": "story", "epic": epic, "criteria": ["works"], "labels": []})),
    )
    .await;
    assert_eq!(st, 200, "{story}");
    story["value"]["key"].as_str().unwrap().to_string()
}

fn get_story_call(key: &str) -> CallToolRequestParams {
    let mut arguments = serde_json::Map::new();
    arguments.insert("key".into(), json!(key));
    CallToolRequestParams::new("get_story").with_arguments(arguments)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sdk_client_calls_get_story_in_both_revisions_and_gets_the_same_answer() {
    let h = mcp_harness().await;
    let token = issue_mcp_token(&h, "dan@example.com").await.secret;
    let key = a_story(&h).await;
    let url = serve(&h).await;

    let modern = ()
        .serve_with_lifecycle(
            StreamableHttpClientTransport::from_config(config(&url, &token)),
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .expect("server/discover is answered");
    let over_modern = modern
        .call_tool(get_story_call(&key))
        .await
        .expect("tools/call is answered");
    modern.cancel().await.unwrap();

    let legacy = ()
        .serve(StreamableHttpClientTransport::from_config(config(
            &url, &token,
        )))
        .await
        .expect("initialize is answered");
    let over_legacy = legacy
        .call_tool(get_story_call(&key))
        .await
        .expect("tools/call is answered");
    legacy.cancel().await.unwrap();

    for result in [&over_modern, &over_legacy] {
        assert_eq!(result.is_error, Some(false));
        let structured = result
            .structured_content
            .as_ref()
            .expect("structuredContent");
        assert_eq!(structured["story"]["key"], json!(key));
        assert_eq!(structured["comments_omitted"], 0);
    }
    assert_eq!(
        over_modern.structured_content,
        over_legacy.structured_content
    );
}
