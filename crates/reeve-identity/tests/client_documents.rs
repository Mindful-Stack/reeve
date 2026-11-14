//! The bytes of client metadata documents, fetched through the loopback-only fetcher against the fake client
//! host, and one client resolved over real HTTP. Compiled only with `static-oidc`. The production transport's
//! rules are a table test in `src/client_fetch.rs`; nothing here says anything about them.
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

fn url(text: &str) -> url::Url {
    url::Url::parse(text).unwrap()
}

/// A JSON document whose text is exactly `total` bytes.
fn padded(total: usize) -> serde_json::Value {
    let document = |pad: String| json!({"client_name": "Padded", "pad": pad});
    let unpadded = document(String::new()).to_string().len();
    let value = document("x".repeat(total - unpadded));
    assert_eq!(
        value.to_string().len(),
        total,
        "the fixture's own arithmetic"
    );
    value
}

/// Catches a fetcher that cannot fetch at all (every refusal test below would then pass for the wrong reason),
/// one that returns anything but the bytes the host sent, and a fake whose published document is not what its
/// arguments describe or whose count of requests does not move.
#[tokio::test]
async fn a_published_document_is_fetched_as_its_bytes() {
    let host = FakeClientHost::start().await;
    assert_eq!(host.host(), "127.0.0.1");
    let id = host.publish("/client.json", "Example App", &[CALLBACK]);
    assert_eq!(id, host.client_id("/client.json"));
    assert!(id.starts_with("http://127.0.0.1:"), "{id}");
    let expected = json!({
        "client_id": id,
        "client_name": "Example App",
        "redirect_uris": [CALLBACK],
        "token_endpoint_auth_method": "none"
    });
    assert_eq!(host.hits("/client.json"), 0);
    let body = fetcher().fetch(&url(&id)).await.unwrap();
    assert_eq!(String::from_utf8(body).unwrap(), expected.to_string());
    assert_eq!(host.hits("/client.json"), 1);
}

/// Catches a client that follows the redirect: it would request `/real.json` and return its body.
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
        fetcher().fetch(&url(&host.client_id("/moved.json"))).await,
        Err(ClientRefusal::Status(302))
    );
    assert_eq!((host.hits("/moved.json"), host.hits("/real.json")), (1, 0));
}

/// Catches any status but 200 read as a document.
#[tokio::test]
async fn a_status_other_than_200_is_refused() {
    let host = FakeClientHost::start().await;
    assert_eq!(
        fetcher()
            .fetch(&url(&host.client_id("/missing.json")))
            .await,
        Err(ClientRefusal::Status(404))
    );
    assert_eq!(host.hits("/missing.json"), 1);
    host.answer(
        "/broken.json",
        FakeAnswer::Status {
            code: 500,
            location: None,
        },
    );
    assert_eq!(
        fetcher().fetch(&url(&host.client_id("/broken.json"))).await,
        Err(ClientRefusal::Status(500))
    );
}

/// Catches a missing bound (the larger document would be read) and a bound that is not 64 KiB (the document of
/// exactly 64 KiB is read).
#[tokio::test]
async fn a_document_of_64_kib_is_read_and_one_byte_more_is_refused() {
    let host = FakeClientHost::start().await;
    host.answer("/exact.json", FakeAnswer::Json(padded(LIMIT)));
    let exact = fetcher().fetch(&url(&host.client_id("/exact.json"))).await;
    assert_eq!(exact.unwrap().len(), LIMIT);
    host.answer("/over.json", FakeAnswer::Json(padded(LIMIT + 1)));
    assert_eq!(
        fetcher().fetch(&url(&host.client_id("/over.json"))).await,
        Err(ClientRefusal::TooLarge)
    );
}

/// The loopback fetcher is what the static provider and every test get. Catches one that would fetch from
/// anywhere else: a name (`localhost` included), https, or an address that is not loopback.
#[tokio::test]
async fn the_loopback_fetcher_fetches_from_nowhere_but_loopback() {
    for id in [
        "https://claude.ai/oauth/claude-code-client-metadata",
        "http://claude.ai/client.json",
        "http://localhost:9/client.json",
        "https://127.0.0.1:9/client.json",
        "http://192.168.1.1/client.json",
        "http://[::ffff:127.0.0.1]:9/client.json",
    ] {
        assert_eq!(
            fetcher().fetch(&url(id)).await,
            Err(ClientRefusal::BadClientId(
                "the test fetcher fetches only http from a loopback IP address"
            )),
            "{id}"
        );
    }
}

/// The seam over real HTTP, in this crate: the bytes that were fetched are judged against the address they were
/// fetched from, and a client that keeps every rule resolves. Catches a `resolve_client` that trusts what the
/// document says of itself, and one that hands the fetcher another address than the client id. (The order of the
/// checks is the unit test's in `clients.rs`; the configured lists are the server's `tests/client_documents.rs`.)
#[tokio::test]
async fn a_fetched_document_is_judged_against_the_address_it_came_from() {
    let host = FakeClientHost::start().await;
    let fetcher = fetcher();
    let policy = ClientPolicy {
        client_hosts: vec![host.host().to_string()],
        redirect_uris: vec![CALLBACK.to_string()],
    };
    let other = host.client_id("/other.json");
    host.answer(
        "/differs.json",
        FakeAnswer::Json(
            json!({"client_id": other, "client_name": "X", "redirect_uris": [CALLBACK]}),
        ),
    );
    let differs = host.client_id("/differs.json");
    assert_eq!(
        resolve_client(&*fetcher, &policy, &differs, CALLBACK).await,
        Err(ClientRefusal::BadDocument(
            "its client_id is not the address it was fetched from"
        ))
    );
    assert_eq!(
        (host.hits("/differs.json"), host.hits("/other.json")),
        (1, 0)
    );
    let id = host.publish("/client.json", "Example App", &[CALLBACK]);
    let client = resolve_client(&*fetcher, &policy, &id, CALLBACK)
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
