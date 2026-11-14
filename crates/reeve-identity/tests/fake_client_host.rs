//! The fake client host itself, through a plain HTTP client: the fetch tests here and the authorize tests in the
//! server stand on these answers. Compiled only with `static-oidc`. What it publishes and how it counts requests
//! is asserted where it is used, in `tests/client_documents.rs`.
#![cfg(feature = "static-oidc")]
use reeve_identity::{FakeAnswer, FakeClientHost};

/// A plain client: no proxy, no redirects followed, nothing of the code under test.
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
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
