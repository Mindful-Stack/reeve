//! The Google adapter against the fake issuer: every verification step of `finish`, the key-rotation retry and its
//! rate limit, and the metadata refresh, on the production windows and a clock the test moves. Needs no Docker and
//! no network beyond 127.0.0.1. Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
use reeve_identity::{
    FakeIssuer, GoogleOidcConfig, GoogleOidcTestOptions, Grant, IdClaims, OidcClient, OidcError,
    Signer, TestClock, VerifiedIdentity, make_google_oidc_for_tests,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const CLIENT_ID: &str = "reeve-test-client";
const CLIENT_SECRET: &str = "reeve-test-secret";
const REDIRECT: &str = "http://127.0.0.1:7474/auth/callback";
/// 64 characters, inside PKCE's 43..=128.
const VERIFIER: &str = "verifier-0123456789-abcdefghijklmnopqrstuvwxyz-ABCDEFGHIJKLMNOP";
const NONCE: &str = "nonce-from-the-server";
const OTHER_VERIFIER: &str = "verifier-9876543210-zyxwvutsrqponmlkjihgfedcba-PONMLKJIHGFEDCBA";
const EMAIL: &str = "ada@example.com";
/// Just past the production minimum refresh gap.
const PAST_THE_GAP: Duration = Duration::from_secs(61);

/// The production windows (60 s minimum refresh gap, 24 h maximum metadata age) on a fresh clock that only the test
/// moves.
fn opts(issuer: String) -> GoogleOidcTestOptions {
    GoogleOidcTestOptions {
        issuer,
        min_refresh_gap: Duration::from_secs(60),
        max_metadata_age: Duration::from_secs(24 * 60 * 60),
        clock: TestClock::new(),
    }
}

fn config(secret: &str, redirect: &str) -> GoogleOidcConfig {
    GoogleOidcConfig {
        client_id: CLIENT_ID.into(),
        client_secret: secret.into(),
        redirect_url: redirect.into(),
    }
}

async fn adapter(o: GoogleOidcTestOptions) -> Arc<dyn OidcClient> {
    make_google_oidc_for_tests(config(CLIENT_SECRET, REDIRECT), o)
        .await
        .expect("the adapter discovers the fake")
}

fn query(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[tokio::test]
async fn the_test_factory_discovers_from_the_given_issuer_and_begins_there() {
    let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    let a = adapter(opts(fake.issuer_url())).await;
    assert_eq!((fake.discovery_hits(), fake.jwks_hits()), (1, 1));
    let url = a.begin("state-1", NONCE, VERIFIER).await.unwrap();
    assert!(
        url.starts_with(&format!("{}/authorize?", fake.issuer_url())),
        "{url}"
    );
    assert_eq!(query(&url)["redirect_uri"], REDIRECT);
    assert_eq!(fake.discovery_hits(), 1, "begin uses the cached metadata");
}

#[tokio::test]
async fn the_test_factory_still_refuses_a_non_loopback_http_redirect_before_any_request() {
    let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    let err = make_google_oidc_for_tests(
        config(CLIENT_SECRET, "http://reeve.example.com/auth/callback"),
        opts(fake.issuer_url()),
    )
    .await
    .err()
    .expect("refused");
    assert!(err.to_string().contains("https"), "{err}");
    assert_eq!(
        (fake.discovery_hits(), fake.jwks_hits(), fake.token_hits()),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn the_test_factory_refuses_an_issuer_that_is_not_http_on_a_loopback_ip() {
    for issuer in [
        "https://accounts.google.com",
        "http://localhost:9",
        "http://10.0.0.1:9",
        "https://127.0.0.1:9",
    ] {
        let err = make_google_oidc_for_tests(config(CLIENT_SECRET, REDIRECT), opts(issuer.into()))
            .await
            .err()
            .unwrap_or_else(|| panic!("{issuer} accepted"));
        assert!(
            err.to_string()
                .contains("a test issuer must be http on a loopback IP address"),
            "{issuer}: {err}"
        );
    }
}

/// TEST-NET-1 (RFC 5737): a public-range address reserved for documentation, which no host owns.
const PUBLIC_IP: &str = "http://192.0.2.1:9";
const REFUSED: &str = "the test HTTP client sends only to http on a loopback IP address; refused";

#[tokio::test]
async fn the_test_adapter_refuses_a_discovered_jwks_uri_off_loopback_without_requesting_it() {
    let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    fake.advertise_jwks_uri(&format!("{PUBLIC_IP}/jwks"));
    let err = make_google_oidc_for_tests(config(CLIENT_SECRET, REDIRECT), opts(fake.issuer_url()))
        .await
        .err()
        .expect("refused");
    assert!(
        err.to_string()
            .contains(&format!("{REFUSED} {PUBLIC_IP}/jwks")),
        "{err}"
    );
    assert_eq!((fake.discovery_hits(), fake.jwks_hits()), (1, 0));
}

#[tokio::test]
async fn the_test_adapter_refuses_a_discovered_token_endpoint_off_loopback_without_requesting_it() {
    let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    fake.advertise_token_endpoint(&format!("{PUBLIC_IP}/token"));
    let a = adapter(opts(fake.issuer_url())).await;
    match a.finish("c", VERIFIER, NONCE).await {
        Err(OidcError::Exchange(m)) => {
            assert!(m.contains(&format!("{REFUSED} {PUBLIC_IP}/token")), "{m}")
        }
        other => panic!("expected OidcError::Exchange, got {other:?}"),
    }
    assert_eq!(fake.token_hits(), 0, "refused before any request");
}

/// An IPv4-mapped IPv6 address: the guard counts only 127.0.0.0/8 and ::1 as loopback, but the kernel delivers
/// this one to 127.0.0.1. Under mutation G1 (no guard) the request therefore stays on this machine and reaches the
/// fake, which answers `invalid_grant`, so the test goes red without sending anything off the machine.
#[tokio::test]
async fn the_guard_and_not_the_network_refuses_a_non_loopback_endpoint() {
    let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    let port = url::Url::parse(&fake.issuer_url()).unwrap().port().unwrap();
    fake.advertise_token_endpoint(&format!("http://[::ffff:127.0.0.1]:{port}/token"));
    let a = adapter(opts(fake.issuer_url())).await;
    match a.finish("c", VERIFIER, NONCE).await {
        Err(OidcError::Exchange(m)) => assert!(m.contains(REFUSED), "{m}"),
        other => panic!("expected OidcError::Exchange, got {other:?}"),
    }
    // the mapped address reaches this fake when nothing refuses it, so this counter is what G1 would move
    assert_eq!(fake.token_hits(), 0, "refused before any request");
}

/// `begin`, then `code` registered with the challenge, its method and the redirect the adapter actually sent (never
/// recomputed here, so a wrong challenge in `begin` cannot be masked) and valid claims for `EMAIL`, edited by `edit`.
async fn begin_and_register(
    fake: &FakeIssuer,
    a: &dyn OidcClient,
    code: &str,
    signer: Signer,
    edit: impl FnOnce(&mut IdClaims),
) {
    let q = query(&a.begin("state-1", NONCE, VERIFIER).await.unwrap());
    let mut claims = fake.claims(&q["nonce"], EMAIL);
    edit(&mut claims);
    fake.register(
        code,
        Grant {
            challenge: q["code_challenge"].clone(),
            challenge_method: q["code_challenge_method"].clone(),
            redirect_uri: q["redirect_uri"].clone(),
            claims,
            signer,
        },
    );
}

#[track_caller]
fn id_token_error(r: Result<VerifiedIdentity, OidcError>) -> String {
    match r {
        Err(OidcError::IdToken(m)) => m,
        other => panic!("expected OidcError::IdToken, got {other:?}"),
    }
}

#[track_caller]
fn exchange_error(r: Result<VerifiedIdentity, OidcError>) -> String {
    match r {
        Err(OidcError::Exchange(m)) => m,
        other => panic!("expected OidcError::Exchange, got {other:?}"),
    }
}

/// A fake, an adapter on it with the production windows, and the adapter's clock.
async fn started() -> (FakeIssuer, Arc<dyn OidcClient>, TestClock) {
    let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    let clock = TestClock::new();
    let a = adapter(GoogleOidcTestOptions {
        clock: clock.clone(),
        ..opts(fake.issuer_url())
    })
    .await;
    (fake, a, clock)
}

const A: Signer = Signer::Published(FakeIssuer::KEY_A);

#[tokio::test]
async fn a_valid_token_maps_the_email_its_verification_and_the_name() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |c| c.name = Some("Ada Lovelace".into())).await;
    let who = a.finish("c", VERIFIER, NONCE).await.unwrap();
    assert_eq!(who.email, EMAIL);
    assert!(who.email_verified);
    assert_eq!(who.name.as_deref(), Some("Ada Lovelace"));
}

#[tokio::test]
async fn an_unverified_email_is_reported_not_refused() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |c| c.email_verified = Some(false)).await;
    let who = a.finish("c", VERIFIER, NONCE).await.unwrap();
    assert!(!who.email_verified);
    assert_eq!(who.email, EMAIL);
}

#[tokio::test]
async fn a_missing_email_verified_claim_maps_to_false() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |c| c.email_verified = None).await;
    assert!(!a.finish("c", VERIFIER, NONCE).await.unwrap().email_verified);
}

#[tokio::test]
async fn a_token_without_an_email_is_no_email() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |c| c.email = None).await;
    assert!(matches!(
        a.finish("c", VERIFIER, NONCE).await,
        Err(OidcError::NoEmail)
    ));
}

#[tokio::test]
async fn a_token_from_another_issuer_is_refused() {
    let (fake, a, clock) = started().await;
    // Past the gap, so a refresh would be allowed if the adapter took an issuer error for a key error.
    clock.advance(PAST_THE_GAP);
    // Google's own issuer: the adapter is pointed at the fake, so even this name is foreign here.
    begin_and_register(&fake, &*a, "c", A, |c| {
        c.iss = "https://accounts.google.com".into()
    })
    .await;
    let m = id_token_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(m.contains("Invalid issuer"), "{m}");
    assert_eq!(
        fake.discovery_hits(),
        1,
        "an issuer error is not a key error: no refresh"
    );
}

#[tokio::test]
async fn a_token_for_another_audience_is_refused() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |c| {
        c.aud = vec!["another-client".into()]
    })
    .await;
    let m = id_token_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(m.contains("Invalid audiences"), "{m}");
}

#[tokio::test]
async fn an_expired_token_is_refused() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |c| {
        c.iat = chrono::Utc::now() - chrono::Duration::minutes(10);
        c.exp = chrono::Utc::now() - chrono::Duration::minutes(1);
    })
    .await;
    let m = id_token_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(m.contains("Expired"), "{m}");
}

#[tokio::test]
async fn a_token_carrying_another_nonce_is_refused() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |c| {
        c.nonce = Some("another-nonce".into())
    })
    .await;
    let m = id_token_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(m.contains("Invalid nonce"), "{m}");
}

#[tokio::test]
async fn a_wrong_code_verifier_is_an_exchange_error() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |_| {}).await;
    let m = exchange_error(a.finish("c", OTHER_VERIFIER, NONCE).await);
    assert!(
        m.contains("invalid_grant") && m.contains("code_verifier does not match"),
        "{m}"
    );
}

#[tokio::test]
async fn a_wrong_client_secret_is_an_exchange_error() {
    let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    let a = make_google_oidc_for_tests(config("not-the-secret", REDIRECT), opts(fake.issuer_url()))
        .await
        .unwrap();
    begin_and_register(&fake, &*a, "c", A, |_| {}).await;
    let m = exchange_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(m.contains("invalid_client"), "{m}");
}

#[tokio::test]
async fn a_code_is_good_for_one_exchange() {
    let (fake, a, _clock) = started().await;
    begin_and_register(&fake, &*a, "c", A, |_| {}).await;
    a.finish("c", VERIFIER, NONCE).await.unwrap();
    let m = exchange_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(
        m.contains("invalid_grant") && m.contains("unknown or already used"),
        "{m}"
    );
}

#[tokio::test]
async fn a_token_from_an_unpublished_key_fails_after_exactly_one_refresh() {
    let (fake, a, clock) = started().await;
    clock.advance(PAST_THE_GAP);
    begin_and_register(&fake, &*a, "c", Signer::Unpublished, |_| {}).await;
    let m = id_token_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(m.contains("No matching key found"), "{m}");
    assert_eq!((fake.discovery_hits(), fake.jwks_hits()), (2, 2));
}

#[tokio::test]
async fn a_token_naming_a_published_key_but_signed_by_another_fails_after_exactly_one_refresh() {
    let (fake, a, clock) = started().await;
    clock.advance(PAST_THE_GAP);
    begin_and_register(&fake, &*a, "c", Signer::ForgedAs(FakeIssuer::KEY_A), |_| {}).await;
    let m = id_token_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(
        m.contains("Crypto error"),
        "the key was found and the signature failed: {m}"
    );
    assert_eq!((fake.discovery_hits(), fake.jwks_hits()), (2, 2));
}

#[tokio::test]
async fn a_rotated_key_is_picked_up_by_one_refresh_and_kept() {
    let (fake, a, clock) = started().await; // the adapter caches [A]
    fake.publish(&[FakeIssuer::KEY_A, FakeIssuer::KEY_B]);
    assert_eq!(
        fake.discovery_hits(),
        1,
        "publishing reaches no client by itself"
    );
    clock.advance(PAST_THE_GAP);
    begin_and_register(
        &fake,
        &*a,
        "c1",
        Signer::Published(FakeIssuer::KEY_B),
        |_| {},
    )
    .await;
    assert_eq!(a.finish("c1", VERIFIER, NONCE).await.unwrap().email, EMAIL);
    assert_eq!((fake.discovery_hits(), fake.jwks_hits()), (2, 2));
    begin_and_register(
        &fake,
        &*a,
        "c2",
        Signer::Published(FakeIssuer::KEY_B),
        |_| {},
    )
    .await;
    a.finish("c2", VERIFIER, NONCE).await.unwrap();
    assert_eq!(
        (fake.discovery_hits(), fake.jwks_hits()),
        (2, 2),
        "the refreshed set was kept"
    );
}

#[tokio::test]
async fn a_rotated_key_inside_the_refresh_gap_is_refused_without_a_refresh() {
    let (fake, a, clock) = started().await;
    fake.publish(&[FakeIssuer::KEY_A, FakeIssuer::KEY_B]);
    clock.advance(Duration::from_secs(59));
    begin_and_register(
        &fake,
        &*a,
        "c",
        Signer::Published(FakeIssuer::KEY_B),
        |_| {},
    )
    .await;
    let m = id_token_error(a.finish("c", VERIFIER, NONCE).await);
    assert!(m.contains("No matching key found"), "{m}");
    assert_eq!((fake.discovery_hits(), fake.jwks_hits()), (1, 1));
}

#[tokio::test]
async fn forged_tokens_cost_one_refresh_per_gap_measured_from_the_last_refresh() {
    let (fake, a, clock) = started().await;
    clock.advance(PAST_THE_GAP);
    for (code, hits) in [("c1", 2), ("c2", 2)] {
        begin_and_register(&fake, &*a, code, Signer::Unpublished, |_| {}).await;
        id_token_error(a.finish(code, VERIFIER, NONCE).await);
        assert_eq!(fake.discovery_hits(), hits, "after {code}");
    }
    clock.advance(PAST_THE_GAP);
    begin_and_register(&fake, &*a, "c3", Signer::Unpublished, |_| {}).await;
    id_token_error(a.finish("c3", VERIFIER, NONCE).await);
    assert_eq!(fake.discovery_hits(), 3, "a new gap allows one more");
}

#[tokio::test]
async fn metadata_older_than_the_maximum_age_is_refreshed_before_begin_uses_it() {
    let (fake, a, clock) = started().await;
    clock.advance(Duration::from_secs(24 * 60 * 60 - 1));
    a.begin("s", NONCE, VERIFIER).await.unwrap();
    assert_eq!(
        fake.discovery_hits(),
        1,
        "metadata a second short of a day is used as is"
    );
    clock.advance(Duration::from_secs(2));
    a.begin("s", NONCE, VERIFIER).await.unwrap();
    assert_eq!(
        fake.discovery_hits(),
        2,
        "stale metadata is refreshed first"
    );
    a.begin("s", NONCE, VERIFIER).await.unwrap();
    assert_eq!(
        fake.discovery_hits(),
        2,
        "and the refreshed metadata is young again"
    );
}

#[tokio::test]
async fn a_refresh_discovers_from_the_configured_issuer_and_nowhere_else() {
    let fake = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    // A second loopback provider, serving valid metadata and both keys, at the redirect's origin: a refresh that took
    // its issuer from anything but `options.issuer` would succeed against it, and be seen.
    let decoy = FakeIssuer::start(CLIENT_ID, CLIENT_SECRET).await;
    decoy.publish(&[FakeIssuer::KEY_A, FakeIssuer::KEY_B]);
    let clock = TestClock::new();
    let redirect = format!("{}/auth/callback", decoy.issuer_url());
    let a = make_google_oidc_for_tests(
        config(CLIENT_SECRET, &redirect),
        GoogleOidcTestOptions {
            clock: clock.clone(),
            ..opts(fake.issuer_url())
        },
    )
    .await
    .unwrap();
    fake.publish(&[FakeIssuer::KEY_A, FakeIssuer::KEY_B]);
    clock.advance(PAST_THE_GAP);
    begin_and_register(
        &fake,
        &*a,
        "c",
        Signer::Published(FakeIssuer::KEY_B),
        |_| {},
    )
    .await;
    a.finish("c", VERIFIER, NONCE).await.unwrap();
    assert_eq!(fake.discovery_hits(), 2);
    assert_eq!((decoy.discovery_hits(), decoy.jwks_hits()), (0, 0));
}

#[tokio::test]
async fn a_key_error_exactly_at_the_refresh_gap_refreshes() {
    let (fake, a, clock) = started().await;
    clock.advance(Duration::from_secs(60));
    begin_and_register(&fake, &*a, "c", Signer::Unpublished, |_| {}).await;
    id_token_error(a.finish("c", VERIFIER, NONCE).await);
    assert_eq!(
        fake.discovery_hits(),
        2,
        "metadata exactly one gap old is no longer too young to refresh"
    );
    assert_eq!(fake.jwks_hits(), 2, "the refresh fetched the key set again");
}

#[tokio::test]
async fn metadata_exactly_at_the_maximum_age_is_not_yet_stale() {
    let (fake, a, clock) = started().await;
    clock.advance(Duration::from_secs(24 * 60 * 60));
    a.begin("s", NONCE, VERIFIER).await.unwrap();
    assert_eq!(
        fake.discovery_hits(),
        1,
        "metadata exactly a day old is used as is"
    );
}
