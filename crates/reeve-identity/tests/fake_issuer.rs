//! The fake OpenID issuer that the Google adapter's tests stand on: what it serves, what it signs, what it refuses.
//! Needs no Docker. Compiled only with `static-oidc`.
#![cfg(feature = "static-oidc")]
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use openidconnect::core::{
    CoreIdTokenVerifier, CoreJsonWebKeySet, CoreProviderMetadata, CoreTokenResponse,
};
use openidconnect::{
    ClaimsVerificationError, ClientId, IssuerUrl, JsonWebKey, Nonce, SignatureVerificationError,
    TokenResponse,
};
use reeve_identity::{FakeIssuer, Grant, Signer};
use sha2::{Digest, Sha256};

const ID: &str = "fake-client";
const SECRET: &str = "fake-secret";
const REDIRECT: &str = "http://127.0.0.1:7474/auth/callback";
/// 64 characters, inside PKCE's 43..=128.
const VERIFIER: &str = "verifier-0123456789-abcdefghijklmnopqrstuvwxyz-ABCDEFGHIJKLMNOP";
const OTHER_VERIFIER: &str = "verifier-9876543210-zyxwvutsrqponmlkjihgfedcba-PONMLKJIHGFEDCBA";

/// The S256 challenge, computed here with sha2 and base64, independently of the fake and of openidconnect.
fn s256(v: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes()))
}

fn grant(fake: &FakeIssuer, signer: Signer) -> Grant {
    Grant {
        challenge: s256(VERIFIER),
        challenge_method: "S256".into(),
        redirect_uri: REDIRECT.into(),
        claims: fake.claims("n-1", "p@example.com"),
        signer,
    }
}

enum Auth<'a> {
    Missing,
    Basic(&'a str, &'a str),
    Body(&'a str, &'a str),
    /// Basic credentials and `client_secret` in the body: two methods in one request.
    Both(&'a str, &'a str),
}

/// Every client a test builds ignores the system proxy settings, so a loopback request cannot leave the machine.
fn http() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

async fn post_token(fake: &FakeIssuer, auth: Auth<'_>, form: &[(&str, &str)]) -> reqwest::Response {
    let mut form = form.to_vec();
    let mut req = http().post(format!("{}/token", fake.issuer_url()));
    match auth {
        Auth::Missing => {}
        Auth::Basic(id, secret) => {
            req = req.header(
                "authorization",
                format!("Basic {}", STANDARD.encode(format!("{id}:{secret}"))),
            );
        }
        Auth::Body(id, secret) => {
            form.push(("client_id", id));
            form.push(("client_secret", secret));
        }
        Auth::Both(id, secret) => {
            req = req.header(
                "authorization",
                format!("Basic {}", STANDARD.encode(format!("{id}:{secret}"))),
            );
            form.push(("client_secret", secret));
        }
    }
    req.form(&form).send().await.unwrap()
}

fn exchange<'a>(code: &'a str, verifier: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", REDIRECT),
        ("code_verifier", verifier),
    ]
}

async fn refused(res: reqwest::Response, status: u16, error: &str, says: &str) {
    assert_eq!(res.status().as_u16(), status);
    let b: serde_json::Value = res.json().await.unwrap();
    assert_eq!(b["error"], error, "{b}");
    assert!(
        b["error_description"].as_str().unwrap().contains(says),
        "{b}"
    );
}

async fn jwks_json(fake: &FakeIssuer) -> serde_json::Value {
    http()
        .get(format!("{}/jwks", fake.issuer_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn jwks_set(fake: &FakeIssuer) -> CoreJsonWebKeySet {
    http()
        .get(format!("{}/jwks", fake.issuer_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn kids(v: &serde_json::Value) -> Vec<String> {
    v["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kid"].as_str().unwrap().to_string())
        .collect()
}

fn modulus(v: &serde_json::Value, kid: &str) -> String {
    v["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["kid"] == kid)
        .unwrap()["n"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn discovery_and_jwks_satisfy_openidconnect_and_each_fetch_is_counted() {
    let fake = FakeIssuer::start(ID, SECRET).await;
    let url = fake.issuer_url();
    assert!(
        url.starts_with("http://127.0.0.1:") && !url.ends_with(":0"),
        "{url}"
    );
    assert_eq!(
        (fake.discovery_hits(), fake.jwks_hits(), fake.token_hits()),
        (0, 0, 0)
    );
    let http = http();
    let meta = CoreProviderMetadata::discover_async(IssuerUrl::new(url.clone()).unwrap(), &http)
        .await
        .unwrap();
    let kids: Vec<String> = meta
        .jwks()
        .keys()
        .iter()
        .map(|k| k.key_id().unwrap().as_str().to_string())
        .collect();
    assert_eq!(kids, [FakeIssuer::KEY_A]);
    assert_eq!(
        meta.token_endpoint().unwrap().as_str(),
        format!("{url}/token")
    );
    assert_eq!((fake.discovery_hits(), fake.jwks_hits()), (1, 1));
    CoreProviderMetadata::discover_async(IssuerUrl::new(url).unwrap(), &http)
        .await
        .unwrap();
    assert_eq!(
        (fake.discovery_hits(), fake.jwks_hits(), fake.token_hits()),
        (2, 2, 0)
    );
}

#[tokio::test]
async fn the_jwks_serves_the_public_halves_of_exactly_the_published_keys() {
    let fake = FakeIssuer::start(ID, SECRET).await;
    let v = jwks_json(&fake).await;
    assert_eq!(kids(&v), [FakeIssuer::KEY_A]);
    for k in v["keys"].as_array().unwrap() {
        assert_eq!(k["kty"], "RSA");
        assert!(k["n"].is_string() && k["e"].is_string(), "{k}");
        for private in ["d", "p", "q", "dp", "dq", "qi"] {
            assert!(k.get(private).is_none(), "{private} leaked: {k}");
        }
    }
    let a = modulus(&v, FakeIssuer::KEY_A);
    fake.publish(&[FakeIssuer::KEY_A, FakeIssuer::KEY_B]);
    let v = jwks_json(&fake).await;
    assert_eq!(kids(&v), [FakeIssuer::KEY_A, FakeIssuer::KEY_B]);
    let b = modulus(&v, FakeIssuer::KEY_B);
    assert_ne!(a, b, "A and B are different keys");
    assert_eq!(
        modulus(&v, FakeIssuer::KEY_A),
        a,
        "a key is stable for the process"
    );
    fake.publish(&[FakeIssuer::KEY_B]);
    let v = jwks_json(&fake).await;
    assert_eq!(kids(&v), [FakeIssuer::KEY_B]);
    assert_eq!(modulus(&v, FakeIssuer::KEY_B), b);
}

#[tokio::test]
async fn a_correct_exchange_returns_an_rs256_token_verifiable_only_with_its_signers_public_half() {
    let fake = FakeIssuer::start(ID, SECRET).await;
    // B signs while only A is published.
    fake.register("c-b", grant(&fake, Signer::Published(FakeIssuer::KEY_B)));
    let res = post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c-b", VERIFIER)).await;
    assert_eq!(res.status().as_u16(), 200);
    let tr: CoreTokenResponse = res.json().await.unwrap();
    let idt = tr.id_token().unwrap().clone();
    let header = idt.to_string().split('.').next().unwrap().to_string();
    let header: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).unwrap()).unwrap();
    assert_eq!(header["alg"], "RS256");
    assert_eq!(header["kid"], FakeIssuer::KEY_B);
    let verifier = |set| {
        CoreIdTokenVerifier::new_public_client(
            ClientId::new(ID.into()),
            IssuerUrl::new(fake.issuer_url()).unwrap(),
            set,
        )
    };
    let nonce = Nonce::new("n-1".into());
    let only_a = jwks_set(&fake).await;
    assert!(matches!(
        idt.claims(&verifier(only_a), &nonce),
        Err(ClaimsVerificationError::SignatureVerification(
            SignatureVerificationError::NoMatchingKey
        ))
    ));
    fake.publish(&[FakeIssuer::KEY_A, FakeIssuer::KEY_B]);
    let both = jwks_set(&fake).await;
    let claims = idt.claims(&verifier(both.clone()), &nonce).unwrap();
    assert_eq!(claims.email().map(|e| e.as_str()), Some("p@example.com"));
    assert_eq!(claims.email_verified(), Some(true));
    // A token from the never-published key verifies against no set the fake can serve.
    fake.register("c-u", grant(&fake, Signer::Unpublished));
    let res = post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c-u", VERIFIER)).await;
    let tr: CoreTokenResponse = res.json().await.unwrap();
    assert!(matches!(
        tr.id_token()
            .unwrap()
            .claims(&verifier(both.clone()), &nonce),
        Err(ClaimsVerificationError::SignatureVerification(
            SignatureVerificationError::NoMatchingKey
        ))
    ));
    // A header naming A over another key's signature: the lookup finds A, the RSA check fails.
    fake.register("c-f", grant(&fake, Signer::ForgedAs(FakeIssuer::KEY_A)));
    let res = post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c-f", VERIFIER)).await;
    let tr: CoreTokenResponse = res.json().await.unwrap();
    let forged = tr.id_token().unwrap().clone();
    let header = forged.to_string().split('.').next().unwrap().to_string();
    let header: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).unwrap()).unwrap();
    assert_eq!(header["kid"], FakeIssuer::KEY_A);
    assert!(matches!(
        forged.claims(&verifier(both), &nonce),
        Err(ClaimsVerificationError::SignatureVerification(
            SignatureVerificationError::CryptoError(_)
        ))
    ));
}

#[tokio::test]
async fn the_token_endpoint_refuses_each_bad_request_with_the_oauth_error_a_provider_sends() {
    let fake = FakeIssuer::start(ID, SECRET).await;
    let a = || grant(&fake, Signer::Published(FakeIssuer::KEY_A));
    fake.register("c1", a());
    refused(
        post_token(&fake, Auth::Missing, &exchange("c1", VERIFIER)).await,
        401,
        "invalid_client",
        "client authentication",
    )
    .await;
    fake.register("c2", a());
    refused(
        post_token(&fake, Auth::Basic(ID, "wrong"), &exchange("c2", VERIFIER)).await,
        401,
        "invalid_client",
        "client authentication",
    )
    .await;
    fake.register("c3", a());
    refused(
        post_token(&fake, Auth::Body(ID, SECRET), &exchange("c3", VERIFIER)).await,
        401,
        "invalid_client",
        "client authentication",
    )
    .await;
    fake.register("c4", a());
    let mut refresh = exchange("c4", VERIFIER);
    refresh[0] = ("grant_type", "refresh_token");
    refused(
        post_token(&fake, Auth::Basic(ID, SECRET), &refresh).await,
        400,
        "unsupported_grant_type",
        "authorization_code",
    )
    .await;
    // consumed by the authenticated request, although its grant type was refused
    refused(
        post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c4", VERIFIER)).await,
        400,
        "invalid_grant",
        "unknown or already used",
    )
    .await;
    // an unauthenticated request consumes nothing
    assert_eq!(
        post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c1", VERIFIER))
            .await
            .status()
            .as_u16(),
        200
    );
    refused(
        post_token(
            &fake,
            Auth::Basic(ID, SECRET),
            &exchange("never-registered", VERIFIER),
        )
        .await,
        400,
        "invalid_grant",
        "unknown or already used",
    )
    .await;
    fake.register("c5", a());
    let mut elsewhere = exchange("c5", VERIFIER);
    elsewhere[2] = ("redirect_uri", "http://127.0.0.1:7474/elsewhere");
    refused(
        post_token(&fake, Auth::Basic(ID, SECRET), &elsewhere).await,
        400,
        "invalid_grant",
        "redirect_uri",
    )
    .await;
    fake.register("c6", a());
    let mut no_verifier = exchange("c6", VERIFIER);
    no_verifier.pop(); // the last field is code_verifier
    refused(
        post_token(&fake, Auth::Basic(ID, SECRET), &no_verifier).await,
        400,
        "invalid_grant",
        "code_verifier missing",
    )
    .await;
    fake.register("c7", a());
    refused(
        post_token(
            &fake,
            Auth::Basic(ID, SECRET),
            &exchange("c7", OTHER_VERIFIER),
        )
        .await,
        400,
        "invalid_grant",
        "does not match",
    )
    .await;
    // consumed by the refused attempt
    refused(
        post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c7", VERIFIER)).await,
        400,
        "invalid_grant",
        "unknown or already used",
    )
    .await;
    fake.register("c8", a());
    let challenge = s256(VERIFIER);
    refused(
        post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c8", &challenge)).await,
        400,
        "invalid_grant",
        "does not match",
    )
    .await;
    fake.register("c9", a());
    assert_eq!(
        post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c9", VERIFIER))
            .await
            .status()
            .as_u16(),
        200
    );
    refused(
        post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c9", VERIFIER)).await,
        400,
        "invalid_grant",
        "unknown or already used",
    )
    .await;
    // RFC 6749 §2.3: one authentication method per request; refused before authentication, so nothing is consumed
    fake.register("c10", a());
    refused(
        post_token(&fake, Auth::Both(ID, SECRET), &exchange("c10", VERIFIER)).await,
        400,
        "invalid_request",
        "one client authentication method",
    )
    .await;
    assert_eq!(
        post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c10", VERIFIER))
            .await
            .status()
            .as_u16(),
        200
    );
    // an authorization request that asked for `plain`: the verifier equals the challenge, and still it is refused
    fake.register(
        "c11",
        Grant {
            challenge: VERIFIER.into(),
            challenge_method: "plain".into(),
            ..a()
        },
    );
    refused(
        post_token(&fake, Auth::Basic(ID, SECRET), &exchange("c11", VERIFIER)).await,
        400,
        "invalid_grant",
        "S256",
    )
    .await;
    // RFC 7636 Appendix B: the test's s256 and the fake's check both agree with the RFC's own vector
    const RFC_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const RFC_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    assert_eq!(s256(RFC_VERIFIER), RFC_CHALLENGE);
    fake.register(
        "c12",
        Grant {
            challenge: RFC_CHALLENGE.into(),
            ..a()
        },
    );
    assert_eq!(
        post_token(
            &fake,
            Auth::Basic(ID, SECRET),
            &exchange("c12", RFC_VERIFIER)
        )
        .await
        .status()
        .as_u16(),
        200
    );
    // every request above was served and counted, authenticated or not
    assert_eq!(fake.token_hits(), 18);
}

/// A secret with every character RFC 6749 §2.3.1's form-urlencoding changes: `:`, `+`, a space, `%`, `/`.
const RESERVED_SECRET: &str = "s:e+c r%t/";

#[tokio::test]
async fn basic_credentials_are_form_urldecoded_before_they_are_compared() {
    let fake = FakeIssuer::start(ID, RESERVED_SECRET).await;
    let a = || grant(&fake, Signer::Published(FakeIssuer::KEY_A));
    // encoded by hand, a space as `+` (what a form encoder writes) and as `%20` (equally valid)
    for (code, encoded) in [("c1", "s%3Ae%2Bc+r%25t%2F"), ("c2", "s%3Ae%2Bc%20r%25t%2F")] {
        fake.register(code, a());
        assert_eq!(
            post_token(&fake, Auth::Basic(ID, encoded), &exchange(code, VERIFIER))
                .await
                .status()
                .as_u16(),
            200,
            "{encoded}"
        );
    }
    fake.register("c3", a());
    refused(
        post_token(
            &fake,
            Auth::Basic(ID, RESERVED_SECRET),
            &exchange("c3", VERIFIER),
        )
        .await,
        401,
        "invalid_client",
        "client authentication",
    )
    .await;
    assert_eq!(fake.token_hits(), 3);
}

/// `u8::from_str_radix` accepts a leading `+`, so `%+2` would decode to byte 2; a form escape is exactly two hex
/// digits, so the fake refuses it. `%02`, the valid spelling of the same byte, is accepted.
#[tokio::test]
async fn a_signed_percent_escape_in_basic_credentials_is_malformed() {
    let fake = FakeIssuer::start(ID, "s\u{2}t").await;
    fake.register("c1", grant(&fake, Signer::Published(FakeIssuer::KEY_A)));
    refused(
        post_token(&fake, Auth::Basic(ID, "s%+2t"), &exchange("c1", VERIFIER)).await,
        401,
        "invalid_client",
        "client authentication",
    )
    .await;
    fake.register("c2", grant(&fake, Signer::Published(FakeIssuer::KEY_A)));
    let ok = post_token(&fake, Auth::Basic(ID, "s%02t"), &exchange("c2", VERIFIER)).await;
    assert_eq!(ok.status().as_u16(), 200);
    assert_eq!(fake.token_hits(), 2);
}

/// A request whose body is not a form is still a request to `/token`: it is counted, then refused by the form
/// extractor.
#[tokio::test]
async fn a_token_request_that_is_not_a_form_is_counted_and_refused() {
    let fake = FakeIssuer::start(ID, SECRET).await;
    let res = http()
        .post(format!("{}/token", fake.issuer_url()))
        .json(&serde_json::json!({ "grant_type": "authorization_code" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 415);
    assert_eq!(fake.token_hits(), 1);
}

#[tokio::test]
#[should_panic(expected = "iss must be a URL")]
async fn a_grant_that_cannot_become_an_id_token_panics_at_register_on_the_test_thread() {
    let fake = FakeIssuer::start(ID, SECRET).await;
    let mut g = grant(&fake, Signer::Published(FakeIssuer::KEY_A));
    g.claims.iss = "not a url".into();
    fake.register("c", g);
}

#[tokio::test]
async fn dropping_the_handle_stops_the_server() {
    let fake = FakeIssuer::start(ID, SECRET).await;
    let url = format!("{}/jwks", fake.issuer_url());
    let client = || {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_millis(200))
            .build()
            .unwrap()
    };
    // Its connection stays in the pool (keep-alive): the shutdown must close it, not only stop accepting.
    let pooled = client();
    assert!(pooled.get(&url).send().await.unwrap().status().is_success());
    drop(fake);
    // A failure bound only: the loop ends as soon as neither the kept-alive connection nor a fresh one is answered.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if pooled.get(&url).send().await.is_err() && client().get(&url).send().await.is_err() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the fake still answers after its handle dropped"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
