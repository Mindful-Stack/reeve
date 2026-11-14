//! `/mcp` against the official SDK's client (rmcp), over a real loopback port, in both protocol revisions. rmcp is
//! one implementation's reading of the specification, written independently of this server and of the request
//! builders in `common`; claude.ai and ChatGPT themselves are tried by hand (see the README).
mod common;
use common::*;
use rmcp::{
    ClientLifecycleMode, ClientServiceExt, ServiceExt,
    model::ProtocolVersion,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

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
    assert!(tools.is_empty());
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
    assert!(tools.is_empty());
    client.cancel().await.unwrap();
}
