//! `/mcp` against the official SDK's client (rmcp), over a real loopback port, in both protocol revisions. rmcp is
//! one implementation's reading of the specification, written independently of this server and of the request
//! builders in `common`; claude.ai and ChatGPT themselves are tried by hand (see the README).
mod common;
use common::*;
use rmcp::{
    ClientLifecycleMode, ClientServiceExt, ServiceError, ServiceExt,
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

/// The tool names in the committed snapshot, in its order. `tests/mcp.rs` proves that `tools/list` equals that
/// file; here the SDK's client must see the same names in the same order.
fn snapshot_names() -> Vec<String> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/mcp_tools.json");
    let tools: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
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
        tools
            .iter()
            .map(|t| t.name.to_string())
            .collect::<Vec<String>>(),
        snapshot_names()
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
        tools
            .iter()
            .map(|t| t.name.to_string())
            .collect::<Vec<String>>(),
        snapshot_names()
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

fn list_stories_call() -> CallToolRequestParams {
    let mut arguments = serde_json::Map::new();
    arguments.insert("open".into(), json!(true));
    arguments.insert("limit".into(), json!(5));
    CallToolRequestParams::new("list_stories").with_arguments(arguments)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sdk_client_calls_a_read_and_a_list_tool_in_both_revisions_and_gets_the_same_answers() {
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
    let story_over_modern = modern
        .call_tool(get_story_call(&key))
        .await
        .expect("tools/call is answered");
    let list_over_modern = modern
        .call_tool(list_stories_call())
        .await
        .expect("tools/call is answered");
    modern.cancel().await.unwrap();

    let legacy = ()
        .serve(StreamableHttpClientTransport::from_config(config(
            &url, &token,
        )))
        .await
        .expect("initialize is answered");
    let story_over_legacy = legacy
        .call_tool(get_story_call(&key))
        .await
        .expect("tools/call is answered");
    let list_over_legacy = legacy
        .call_tool(list_stories_call())
        .await
        .expect("tools/call is answered");
    legacy.cancel().await.unwrap();

    for result in [&story_over_modern, &story_over_legacy] {
        assert_eq!(result.is_error, Some(false));
        let structured = result
            .structured_content
            .as_ref()
            .expect("structuredContent");
        assert_eq!(structured["story"]["key"], json!(key));
        assert_eq!(structured["comments_omitted"], 0);
    }
    assert_eq!(
        story_over_modern.structured_content,
        story_over_legacy.structured_content
    );

    for result in [&list_over_modern, &list_over_legacy] {
        assert_eq!(result.is_error, Some(false));
        let structured = result
            .structured_content
            .as_ref()
            .expect("structuredContent");
        assert_eq!(structured["stories"].as_array().map(Vec::len), Some(1));
        assert_eq!(structured["stories"][0]["key"], json!(key));
        assert!(structured["next_offset"].is_null());
    }
    assert_eq!(
        list_over_modern.structured_content,
        list_over_legacy.structured_content
    );
}

/// An unknown tool is a protocol error (-32602), and the client must be able to read it as one: not a transport
/// failure it cannot tell from a broken connection, and not a tool result.
#[tokio::test(flavor = "multi_thread")]
async fn the_sdk_client_sees_an_unknown_tool_as_a_json_rpc_error_in_both_revisions() {
    let h = mcp_harness().await;
    let token = issue_mcp_token(&h, "dan@example.com").await.secret;
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
    let over_modern = modern.call_tool(CallToolRequestParams::new("nope")).await;
    modern.cancel().await.unwrap();

    let legacy = ()
        .serve(StreamableHttpClientTransport::from_config(config(
            &url, &token,
        )))
        .await
        .expect("initialize is answered");
    let over_legacy = legacy.call_tool(CallToolRequestParams::new("nope")).await;
    legacy.cancel().await.unwrap();

    for (revision, outcome) in [("2026-07-28", over_modern), ("2025-11-25", over_legacy)] {
        match outcome {
            Err(ServiceError::McpError(e)) => {
                assert_eq!(e.code.0, -32602, "{revision}: {e:?}");
                assert_eq!(e.message, "Unknown tool: nope", "{revision}");
            }
            other => panic!("{revision}: not a JSON-RPC error: {other:?}"),
        }
    }
}

fn create_story_call(idempotency_key: &str) -> CallToolRequestParams {
    let mut arguments = serde_json::Map::new();
    arguments.insert("idempotency_key".into(), json!(idempotency_key));
    arguments.insert("title".into(), json!("Made by the SDK's client"));
    CallToolRequestParams::new("create_story").with_arguments(arguments)
}

/// A successful call's `structuredContent`.
fn receipt(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    assert_eq!(result.is_error, Some(false), "{result:?}");
    result
        .structured_content
        .clone()
        .expect("structuredContent")
}

/// A write through the SDK's client, in both revisions: the call is sent and its receipt is read by an
/// implementation that is not this repository's, and the two revisions write to the same planner (a key used over
/// one is a repeat over the other).
#[tokio::test(flavor = "multi_thread")]
async fn the_sdk_client_calls_a_write_tool_in_both_revisions() {
    let h = mcp_harness().await;
    let token = issue_mcp_token(&h, "dan@example.com").await.secret;
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
        .call_tool(create_story_call("conf-w1"))
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
        .call_tool(create_story_call("conf-w1"))
        .await
        .expect("tools/call is answered");
    let another = legacy
        .call_tool(create_story_call("conf-w2"))
        .await
        .expect("tools/call is answered");
    legacy.cancel().await.unwrap();

    let (first, repeat, second) = (
        receipt(&over_modern),
        receipt(&over_legacy),
        receipt(&another),
    );
    assert_eq!(
        (&first["created"], &repeat["created"], &second["created"]),
        (&json!(true), &json!(false), &json!(true))
    );
    assert_eq!(first["key"], repeat["key"]);
    assert_ne!(first["key"], second["key"]);
    // One story per key, in the planner itself.
    let (status, stories) =
        call(&h.app, Some(&h.admin_token), "GET", "/api/v1/stories", None).await;
    assert_eq!(status, 200);
    assert_eq!(stories["value"].as_array().map(Vec::len), Some(2));
}
