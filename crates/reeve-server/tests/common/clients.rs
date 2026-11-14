//! A harness whose MCP clients publish their metadata documents on a fake client host (loopback, `static-oidc`
//! only).
use super::*;

/// The MCP harness with `redirect_uris` as given and the fake client host as the one entry of `client_hosts`,
/// all by configuration text: the harness's static provider is what admits a loopback client host and what makes
/// `build_state` choose the loopback-only fetcher. Publish a client with `FakeClientHost::publish`; its client id
/// is then `http://127.0.0.1:<port><path>` and its host `127.0.0.1`.
pub async fn client_harness(redirect_uris: &[&str]) -> (Harness, reeve_identity::FakeClientHost) {
    client_harness_with(redirect_uris, "").await
}

/// The same with `mcp_lines` (whole lines, each ending in a newline) added to the `[mcp]` section.
pub async fn client_harness_with(
    redirect_uris: &[&str],
    mcp_lines: &str,
) -> (Harness, reeve_identity::FakeClientHost) {
    let host = reeve_identity::FakeClientHost::start().await;
    let redirects = serde_json::to_string(redirect_uris).unwrap();
    let h = harness_with(|cfg| {
        format!(
            "{cfg}\n[mcp]\nenabled = true\nredirect_uris = {redirects}\nclient_hosts = [\"{}\"]\n{mcp_lines}",
            host.host()
        )
    })
    .await;
    (h, host)
}
