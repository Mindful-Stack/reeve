//! Client documents in the server (GRA-656): the state `build_state` makes resolves a client through the lists
//! of `[mcp]`, exactly as the authorize route will call it. Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
mod common;
use common::clients::client_harness;
use reeve_identity::{ClientRefusal, resolve_client};

const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";
const ELSEWHERE: &str = "https://elsewhere.example/cb";

/// Catches `client_policy` handing the lists over swapped or empty (the listed client would be refused as an
/// unlisted host), a state without a fetcher that can reach the harness's client, and an unlisted redirect
/// address that causes a fetch.
#[tokio::test]
async fn the_state_resolves_a_client_through_the_configured_lists() {
    let (h, host) = client_harness(&[CALLBACK]).await;
    let s = &h.state;
    assert_eq!(s.config.mcp.redirect_uris, [CALLBACK]);
    assert_eq!(s.config.mcp.client_hosts, ["127.0.0.1"]);
    let id = host.publish("/client.json", "Example App", &[CALLBACK, ELSEWHERE]);
    // The client lists it, `[mcp] redirect_uris` does not: the instance decides, before any fetch.
    let refused = resolve_client(
        &*s.client_fetcher,
        &s.config.client_policy(),
        &id,
        ELSEWHERE,
    )
    .await;
    assert_eq!(refused, Err(ClientRefusal::RedirectNotEnabled));
    assert_eq!(host.hits("/client.json"), 0);
    let client = resolve_client(&*s.client_fetcher, &s.config.client_policy(), &id, CALLBACK)
        .await
        .unwrap();
    assert_eq!(
        (
            client.client_id(),
            client.client_host(),
            client.client_name(),
            client.redirect_uri()
        ),
        (id.as_str(), "127.0.0.1", "Example App", CALLBACK)
    );
    assert_eq!(host.hits("/client.json"), 1);
}
