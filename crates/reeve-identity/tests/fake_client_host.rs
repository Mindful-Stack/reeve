//! The fake client host itself, through a plain HTTP client: the fetch tests here and the authorize tests in the
//! server stand on these answers. Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
use reeve_identity::{FakeAnswer, FakeClientHost};
use serde_json::json;

/// A plain client: no proxy, no redirects followed, nothing of the code under test.
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

/// Catches a JSON body that is not the value's own text, a published document that is not what its arguments
/// describe, a hit not counted, and an unknown path that answers.
#[tokio::test]
async fn it_serves_what_was_published_and_counts_requests() {
    let host = FakeClientHost::start().await;
    assert_eq!(host.host(), "127.0.0.1");
    let id = host.publish("/app.json", "Example App", &["https://example.test/cb"]);
    assert_eq!(id, host.client_id("/app.json"));
    assert!(id.starts_with("http://127.0.0.1:"), "{id}");
    let expected = json!({
        "client_id": id,
        "client_name": "Example App",
        "redirect_uris": ["https://example.test/cb"],
        "token_endpoint_auth_method": "none"
    });
    let answer = http().get(&id).send().await.unwrap();
    assert_eq!(answer.status(), 200);
    assert_eq!(answer.text().await.unwrap(), expected.to_string());
    assert_eq!((host.hits("/app.json"), host.hits("/other")), (1, 0));
    let missing = http().get(host.client_id("/other")).send().await.unwrap();
    assert_eq!(missing.status(), 404);
    assert_eq!(host.hits("/other"), 1);
}

/// Catches a "redirect" with nothing to follow and a status not sent as given.
#[tokio::test]
async fn a_status_answer_carries_its_code_and_location() {
    let host = FakeClientHost::start().await;
    host.answer(
        "/moved",
        FakeAnswer::Status {
            code: 302,
            location: Some(host.client_id("/real")),
        },
    );
    host.answer(
        "/broken",
        FakeAnswer::Status {
            code: 500,
            location: None,
        },
    );
    let moved = http().get(host.client_id("/moved")).send().await.unwrap();
    assert_eq!(moved.status(), 302);
    assert_eq!(
        moved.headers()["location"].to_str().unwrap(),
        host.client_id("/real")
    );
    let broken = http().get(host.client_id("/broken")).send().await.unwrap();
    assert_eq!(broken.status(), 500);
    assert!(!broken.headers().contains_key("location"));
}
