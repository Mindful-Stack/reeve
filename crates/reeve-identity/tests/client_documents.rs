//! Client metadata documents, fetched and resolved through the loopback-only fetcher against the fake client
//! host. Compiled only with `static-oidc`. The production fetcher's own rules are table tests in
//! `src/client_fetch.rs`; nothing here says anything about them.
#![cfg(feature = "static-oidc")]
use reeve_identity::{
    ClientDocumentFetcher, ClientPolicy, ClientRefusal, FakeAnswer, FakeClientHost,
    make_loopback_client_fetcher, resolve_client,
};
use serde_json::json;
use std::sync::Arc;

const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";
const LIMIT: usize = 64 * 1024;

fn fetcher() -> Arc<dyn ClientDocumentFetcher> {
    make_loopback_client_fetcher().unwrap()
}

/// A valid document for `client_id` whose JSON text is exactly `total` bytes.
fn padded(client_id: &str, total: usize) -> serde_json::Value {
    let document = |pad: String| json!({"client_id": client_id, "client_name": "Padded", "redirect_uris": [CALLBACK], "pad": pad});
    let unpadded = document(String::new()).to_string().len();
    let value = document("x".repeat(total - unpadded));
    assert_eq!(
        value.to_string().len(),
        total,
        "the fixture's own arithmetic"
    );
    value
}

/// Catches a fetcher that cannot fetch at all (every refusal test below would then pass for the wrong reason).
#[tokio::test]
async fn a_published_document_is_fetched_and_validated() {
    let host = FakeClientHost::start().await;
    let id = host.publish("/client.json", "Example App", &[CALLBACK]);
    let document = fetcher().fetch(&id).await.unwrap();
    assert_eq!(
        (document.client_id(), document.client_name()),
        (id.as_str(), "Example App")
    );
    assert_eq!(document.redirect_uris(), [CALLBACK]);
    assert_eq!(host.hits("/client.json"), 1);
}

/// Catches a client that follows the redirect: it would request `/real.json`, and fail on the document's id
/// instead of the status.
#[tokio::test]
async fn a_redirect_is_not_followed() {
    let host = FakeClientHost::start().await;
    let real = host.publish("/real.json", "Real", &[CALLBACK]);
    host.answer(
        "/moved.json",
        FakeAnswer::Status {
            code: 302,
            location: Some(real),
        },
    );
    assert_eq!(
        fetcher().fetch(&host.client_id("/moved.json")).await,
        Err(ClientRefusal::Status(302))
    );
    assert_eq!((host.hits("/moved.json"), host.hits("/real.json")), (1, 0));
}

/// Catches any status but 200 read as a document.
#[tokio::test]
async fn a_status_other_than_200_is_refused() {
    let host = FakeClientHost::start().await;
    assert_eq!(
        fetcher().fetch(&host.client_id("/missing.json")).await,
        Err(ClientRefusal::Status(404))
    );
    host.answer(
        "/broken.json",
        FakeAnswer::Status {
            code: 500,
            location: None,
        },
    );
    assert_eq!(
        fetcher().fetch(&host.client_id("/broken.json")).await,
        Err(ClientRefusal::Status(500))
    );
}

/// Catches a missing bound (the larger document would be read) and a bound that is not 64 KiB (the document of
/// exactly 64 KiB is read).
#[tokio::test]
async fn a_document_of_64_kib_is_read_and_one_byte_more_is_refused() {
    let host = FakeClientHost::start().await;
    let exact = host.client_id("/exact.json");
    host.answer("/exact.json", FakeAnswer::Json(padded(&exact, LIMIT)));
    assert_eq!(fetcher().fetch(&exact).await.unwrap().client_id(), exact);
    let over = host.client_id("/over.json");
    host.answer("/over.json", FakeAnswer::Json(padded(&over, LIMIT + 1)));
    assert_eq!(fetcher().fetch(&over).await, Err(ClientRefusal::TooLarge));
}

/// Catches a fetcher that returns the bytes without validating them against the address it fetched.
#[tokio::test]
async fn a_fetched_document_whose_id_differs_is_refused() {
    let host = FakeClientHost::start().await;
    let other = host.client_id("/other.json");
    host.answer(
        "/differs.json",
        FakeAnswer::Json(
            json!({"client_id": other, "client_name": "X", "redirect_uris": [CALLBACK]}),
        ),
    );
    assert_eq!(
        fetcher().fetch(&host.client_id("/differs.json")).await,
        Err(ClientRefusal::BadDocument(
            "its client_id is not the address it was fetched from"
        ))
    );
}

/// The loopback fetcher is what the static provider and every test get. Catches one that would fetch from
/// anywhere else.
#[tokio::test]
async fn the_loopback_fetcher_fetches_from_nowhere_but_loopback() {
    for id in [
        "https://claude.ai/oauth/claude-code-client-metadata",
        "http://claude.ai/client.json",
        "http://localhost:9/client.json",
    ] {
        assert_eq!(
            fetcher().fetch(id).await,
            Err(ClientRefusal::BadClientId(
                "the test fetcher fetches only http from a loopback IP address"
            )),
            "{id}"
        );
    }
}

/// The seam against a real fetch. Catches a host or an address the instance has not enabled causing a request
/// (the host is there and would answer), and a resolved client that is not what the document says.
#[tokio::test]
async fn a_client_resolves_only_once_the_instance_has_enabled_it() {
    let host = FakeClientHost::start().await;
    let fetcher = fetcher();
    let id = host.publish(
        "/client.json",
        "Example App",
        &[CALLBACK, "https://other.example/cb"],
    );
    let enabled = ClientPolicy {
        client_hosts: vec![host.host().to_string()],
        redirect_uris: vec![CALLBACK.to_string()],
    };
    let nobody = ClientPolicy {
        client_hosts: vec![],
        redirect_uris: vec![CALLBACK.to_string()],
    };
    let refused = resolve_client(&*fetcher, &nobody, &id, CALLBACK)
        .await
        .unwrap_err();
    assert!(refused == ClientRefusal::HostNotListed && refused.not_enabled());
    let refused = resolve_client(&*fetcher, &enabled, &id, "https://other.example/cb")
        .await
        .unwrap_err();
    assert!(refused == ClientRefusal::RedirectNotEnabled && refused.not_enabled());
    assert_eq!(host.hits("/client.json"), 0);
    let client = resolve_client(&*fetcher, &enabled, &id, CALLBACK)
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
