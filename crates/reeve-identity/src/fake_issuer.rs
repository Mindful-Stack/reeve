//! A fake OpenID provider for tests, compiled only under `static-oidc` (which the Dockerfile never enables).
//!
//! It serves discovery, a JWKS and a token endpoint on 127.0.0.1 and signs RS256 ID tokens with openidconnect's own
//! encoder (`CoreIdToken::new` over a `CoreRsaPrivateSigningKey`), so the adapter under test parses and verifies what a
//! real provider sends. It checks `client_secret_basic` (RFC 6749 §2.3.1: form-urldecoded, one method per request),
//! PKCE S256 (the only method it accepts), the redirect and single use, and counts every request it serves.
//!
//! Not reproduced: TLS; the browser leg (`/authorize` is advertised, not served: tests read the authorization request
//! from the URL `begin` returns and register the grant directly); HTTP caching headers; Google's real time scales
//! (the adapter's tests advance a `TestClock` instead).
use axum::{
    Form, Json, Router,
    extract::{State, rejection::FormRejection},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use openidconnect::core::{
    CoreIdToken, CoreIdTokenClaims, CoreIdTokenFields, CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
    CoreRsaPrivateSigningKey, CoreTokenResponse, CoreTokenType,
};
use openidconnect::{
    AccessToken, Audience, EmptyAdditionalClaims, EmptyExtraTokenFields, EndUserEmail, EndUserName,
    IssuerUrl, JsonWebKeyId, LocalizedClaim, Nonce, PrivateSigningKey, StandardClaims,
    SubjectIdentifier,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The kid of the key no `publish` can name: tokens it signs never verify against the fake's JWKS.
const UNPUBLISHED: &str = "fake-key-never-published";

/// PKCS#1 PEMs of the three keys, 2048-bit RSA generated once per process and never written anywhere.
fn pems() -> &'static HashMap<&'static str, String> {
    static PEMS: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    PEMS.get_or_init(|| {
        use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
        [FakeIssuer::KEY_A, FakeIssuer::KEY_B, UNPUBLISHED]
            .into_iter()
            .map(|kid| {
                let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048)
                    .expect("rsa key generation");
                let pem = key.to_pkcs1_pem(LineEnding::LF).expect("pkcs1 pem");
                (kid, pem.to_string())
            })
            .collect()
    })
}

/// The private half of key `pem_kid`, labelled `kid` (which `CoreIdToken::new` copies into the JWS header). The two
/// differ only for `Signer::ForgedAs`.
fn signing_key(pem_kid: &'static str, kid: &'static str) -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(&pems()[pem_kid], Some(JsonWebKeyId::new(kid.into())))
        .expect("the fake's key parses")
}

/// Which key signs a grant's ID token.
#[derive(Debug, Clone, Copy)]
pub enum Signer {
    /// `FakeIssuer::KEY_A` or `FakeIssuer::KEY_B`, whether or not it is published right now.
    Published(&'static str),
    /// A key that is never in the published set.
    Unpublished,
    /// A header naming `FakeIssuer::KEY_A` or `KEY_B` over a signature by the never-published key.
    ForgedAs(&'static str),
}

/// The ID token's claims, as the test wants them. `FakeIssuer::claims` gives valid ones to edit.
#[derive(Debug, Clone)]
pub struct IdClaims {
    pub iss: String,
    pub aud: Vec<String>,
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub nonce: Option<String>,
    pub iat: DateTime<Utc>,
    pub exp: DateTime<Utc>,
}

impl IdClaims {
    fn into_core(self) -> CoreIdTokenClaims {
        let mut standard = StandardClaims::new(SubjectIdentifier::new(self.sub))
            .set_email(self.email.map(EndUserEmail::new))
            .set_email_verified(self.email_verified);
        if let Some(n) = self.name {
            standard = standard.set_name(Some(LocalizedClaim::from(EndUserName::new(n))));
        }
        CoreIdTokenClaims::new(
            IssuerUrl::new(self.iss).expect("iss must be a URL"),
            self.aud.into_iter().map(Audience::new).collect(),
            self.exp,
            self.iat,
            standard,
            EmptyAdditionalClaims {},
        )
        .set_nonce(self.nonce.map(Nonce::new))
    }
}

/// What a code is good for: the challenge, its method and the redirect of its authorization request, the claims and
/// the signer. The token endpoint redeems only `challenge_method == "S256"`.
#[derive(Debug, Clone)]
pub struct Grant {
    pub challenge: String,
    pub challenge_method: String,
    pub redirect_uri: String,
    pub claims: IdClaims,
    pub signer: Signer,
}

struct Shared {
    issuer: String,
    client_id: String,
    client_secret: String,
    grants: Mutex<HashMap<String, Grant>>,
    published: Mutex<Vec<&'static str>>,
    discovery_hits: AtomicUsize,
    jwks_hits: AtomicUsize,
    token_hits: AtomicUsize,
    advertised_jwks_uri: Mutex<Option<String>>,
    advertised_token_endpoint: Mutex<Option<String>>,
}

/// A running fake provider. Dropping the handle drops `_shutdown`, which shuts the server down gracefully: it stops
/// accepting and closes every connection once its in-flight request is answered, kept-alive ones included.
pub struct FakeIssuer {
    shared: Arc<Shared>,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl FakeIssuer {
    pub const KEY_A: &'static str = "fake-key-a";
    pub const KEY_B: &'static str = "fake-key-b";

    /// Serves on 127.0.0.1 with a free port, publishing `[KEY_A]`, for this one client.
    pub async fn start(client_id: &str, client_secret: &str) -> FakeIssuer {
        pems(); // generate before serving, so no request waits on key generation
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind 127.0.0.1:0");
        let issuer = format!("http://{}", listener.local_addr().expect("local addr"));
        let shared = Arc::new(Shared {
            issuer,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            grants: Mutex::new(HashMap::new()),
            published: Mutex::new(vec![Self::KEY_A]),
            discovery_hits: AtomicUsize::new(0),
            jwks_hits: AtomicUsize::new(0),
            token_hits: AtomicUsize::new(0),
            advertised_jwks_uri: Mutex::new(None),
            advertised_token_endpoint: Mutex::new(None),
        });
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/token", post(token))
            .with_state(shared.clone());
        let (shutdown, stop) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    // nothing is ever sent: the sender's drop, with the handle, is the signal
                    let _ = stop.await;
                })
                .await
                .expect("the fake issuer serves")
        });
        FakeIssuer {
            shared,
            _shutdown: shutdown,
        }
    }

    /// `http://127.0.0.1:<port>`, no trailing slash: the exact string discovery and the `iss` claim carry.
    pub fn issuer_url(&self) -> String {
        self.shared.issuer.clone()
    }

    /// Replaces the published set; the next `/jwks` request serves it. Only `KEY_A` and `KEY_B` can be published.
    pub fn publish(&self, kids: &[&'static str]) {
        assert!(
            kids.iter().all(|k| [Self::KEY_A, Self::KEY_B].contains(k)),
            "only FakeIssuer::KEY_A and FakeIssuer::KEY_B can be published: {kids:?}"
        );
        *self.shared.published.lock().unwrap() = kids.to_vec();
    }

    /// The `jwks_uri` the discovery document names from now on, instead of this fake's own `/jwks`.
    pub fn advertise_jwks_uri(&self, url: &str) {
        *self.shared.advertised_jwks_uri.lock().unwrap() = Some(url.into());
    }

    /// The `token_endpoint` the discovery document names from now on, instead of this fake's own `/token`.
    pub fn advertise_token_endpoint(&self, url: &str) {
        *self.shared.advertised_token_endpoint.lock().unwrap() = Some(url.into());
    }

    /// Makes `code` redeemable once for `grant`. Panics here, on the test's thread, for a grant the token endpoint
    /// could not sign (an unknown key, claims that do not form an ID token), rather than inside the handler.
    pub fn register(&self, code: &str, grant: Grant) {
        if let Signer::Published(kid) | Signer::ForgedAs(kid) = grant.signer {
            assert!(
                [Self::KEY_A, Self::KEY_B].contains(&kid),
                "unknown key {kid:?}"
            );
        }
        grant.claims.clone().into_core();
        self.shared
            .grants
            .lock()
            .unwrap()
            .insert(code.into(), grant);
    }

    /// Valid claims for this issuer and client: verified `email`, a name, `nonce`, issued now, expiring in 5 minutes.
    pub fn claims(&self, nonce: &str, email: &str) -> IdClaims {
        let now = Utc::now();
        IdClaims {
            iss: self.issuer_url(),
            aud: vec![self.shared.client_id.clone()],
            sub: format!("sub-{email}"),
            email: Some(email.into()),
            email_verified: Some(true),
            name: Some("Fake Person".into()),
            nonce: Some(nonce.into()),
            iat: now,
            exp: now + chrono::Duration::minutes(5),
        }
    }

    pub fn discovery_hits(&self) -> usize {
        self.shared.discovery_hits.load(Ordering::SeqCst)
    }

    pub fn jwks_hits(&self) -> usize {
        self.shared.jwks_hits.load(Ordering::SeqCst)
    }

    /// Requests to `/token`, refused ones included.
    pub fn token_hits(&self) -> usize {
        self.shared.token_hits.load(Ordering::SeqCst)
    }
}

async fn discovery(State(s): State<Arc<Shared>>) -> Json<serde_json::Value> {
    s.discovery_hits.fetch_add(1, Ordering::SeqCst);
    let i = &s.issuer;
    let jwks_uri = s
        .advertised_jwks_uri
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| format!("{i}/jwks"));
    let token_endpoint = s
        .advertised_token_endpoint
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| format!("{i}/token"));
    Json(serde_json::json!({
        "issuer": i,
        "authorization_endpoint": format!("{i}/authorize"),
        "token_endpoint": token_endpoint,
        "jwks_uri": jwks_uri,
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic"],
    }))
}

async fn jwks(State(s): State<Arc<Shared>>) -> Json<CoreJsonWebKeySet> {
    s.jwks_hits.fetch_add(1, Ordering::SeqCst);
    let kids = s.published.lock().unwrap().clone();
    Json(CoreJsonWebKeySet::new(
        kids.into_iter()
            .map(|kid| signing_key(kid, kid).as_verification_key())
            .collect(),
    ))
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

fn invalid_grant(description: &str) -> Response {
    oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", description)
}

/// `application/x-www-form-urlencoded` decoding of one value: `+` is a space, `%XX` a byte. `None` for a malformed
/// escape or bytes that are not UTF-8.
fn form_urldecode(v: &str) -> Option<String> {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                // exactly two hex digits: `from_str_radix` alone would also take a sign, as in `%+2`
                let hex = b.get(i + 1..i + 3)?;
                if !hex.iter().all(u8::is_ascii_hexdigit) {
                    return None;
                }
                out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// RFC 6749 §2.3.1: the Basic credentials are the form-urlencoded client id and secret, so each half is decoded and
/// then compared with the raw id and secret.
fn client_authenticated(headers: &HeaderMap, s: &Shared) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|b| STANDARD.decode(b).ok())
        .and_then(|raw| String::from_utf8(raw).ok())
        .is_some_and(|raw| {
            raw.split_once(':').is_some_and(|(id, secret)| {
                form_urldecode(id).as_deref() == Some(s.client_id.as_str())
                    && form_urldecode(secret).as_deref() == Some(s.client_secret.as_str())
            })
        })
}

async fn token(
    State(s): State<Arc<Shared>>,
    headers: HeaderMap,
    form: Result<Form<HashMap<String, String>>, FormRejection>,
) -> Response {
    // counted before the body is read, so a request that is not a form is counted too
    s.token_hits.fetch_add(1, Ordering::SeqCst);
    let Form(f) = match form {
        Ok(f) => f,
        Err(rejection) => return rejection.into_response(),
    };
    // RFC 6749 §2.3: a client MUST NOT use more than one authentication method in a request.
    if headers.contains_key(AUTHORIZATION) && f.contains_key("client_secret") {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "use only one client authentication method",
        );
    }
    if !client_authenticated(&headers, &s) {
        return oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client authentication failed",
        );
    }
    // Single use: an authenticated request consumes its code before any other check, whatever its outcome.
    let grant = f
        .get("code")
        .and_then(|c| s.grants.lock().unwrap().remove(c));
    if f.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "only authorization_code is supported",
        );
    }
    let Some(grant) = grant else {
        return invalid_grant("unknown or already used code");
    };
    if f.get("redirect_uri") != Some(&grant.redirect_uri) {
        return invalid_grant("redirect_uri does not match the authorization request");
    }
    if grant.challenge_method != "S256" {
        return invalid_grant(&format!(
            "code_challenge_method {:?} is not supported; only S256",
            grant.challenge_method
        ));
    }
    let Some(verifier) = f.get("code_verifier") else {
        return invalid_grant("code_verifier missing");
    };
    if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != grant.challenge {
        return invalid_grant("code_verifier does not match the challenge");
    }
    let key = match grant.signer {
        Signer::Published(kid) => signing_key(kid, kid),
        Signer::Unpublished => signing_key(UNPUBLISHED, UNPUBLISHED),
        Signer::ForgedAs(kid) => signing_key(UNPUBLISHED, kid),
    };
    let id_token = CoreIdToken::new(
        grant.claims.into_core(),
        &key,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        None,
        None,
    )
    .expect("the fake signs");
    let mut body = CoreTokenResponse::new(
        AccessToken::new(format!("fake-access-{}", rand::random::<u64>())),
        CoreTokenType::Bearer,
        CoreIdTokenFields::new(Some(id_token), EmptyExtraTokenFields {}),
    );
    body.set_expires_in(Some(&std::time::Duration::from_secs(3600)));
    Json(body).into_response()
}
