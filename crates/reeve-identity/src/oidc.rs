//! The OpenID Connect port: the server drives the redirect dance, identity verifies what comes back.
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub use crate::handler::VerifiedIdentity;

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("provider refused the code: {0}")]
    Exchange(String),
    #[error("id token invalid: {0}")]
    IdToken(String),
    #[error("id token has no email claim")]
    NoEmail,
    #[error("pkce verifier must be 43 to 128 characters")]
    BadVerifier,
}

#[async_trait]
pub trait OidcClient: Send + Sync {
    /// The URL to send the browser to. `state` and `nonce` are the server's (stored against the pending login);
    /// `pkce_verifier` is the server's too (43..=128 characters, else `BadVerifier`), the adapter derives the S256
    /// challenge from it.
    async fn begin(
        &self,
        state: &str,
        nonce: &str,
        pkce_verifier: &str,
    ) -> Result<String, OidcError>;
    /// Exchanges the code, verifies the ID token's signature, issuer, audience, expiry and nonce, and returns the claims.
    async fn finish(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
    ) -> Result<VerifiedIdentity, OidcError>;
}

fn check_verifier(v: &str) -> Result<(), OidcError> {
    if (43..=128).contains(&v.len()) {
        Ok(())
    } else {
        Err(OidcError::BadVerifier)
    }
}

/// An error and its whole cause chain, joined with ": ". The oauth2 and openidconnect errors never print secrets,
/// codes, nonces or verifiers, so the chain is safe to carry in `OidcError` and to log.
fn chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(s) = cur {
        out.push_str(": ");
        out.push_str(&s.to_string());
        cur = s.source();
    }
    out
}

#[derive(Clone)]
pub struct GoogleOidcConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
}

impl std::fmt::Debug for GoogleOidcConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleOidcConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("redirect_url", &self.redirect_url)
            .finish()
    }
}

type GoogleClient = openidconnect::core::CoreClient<
    openidconnect::EndpointSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointMaybeSet,
    openidconnect::EndpointMaybeSet,
>;

const ISSUER: &str = "https://accounts.google.com";
/// Discovered metadata (and with it the JWKS) older than this is refreshed before the next verification.
const MAX_METADATA_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// A forced refresh (after a key error) is skipped if the metadata is younger than this, so forged tokens with
/// unknown key ids cannot make the server hammer Google.
const MIN_REFRESH_GAP: Duration = Duration::from_secs(60);

/// Where the adapter discovers from, how often it refreshes and which clock ages its metadata. Production passes
/// `Options::google()`; only the test factory, compiled under `static-oidc`, passes anything else.
struct Options {
    issuer: String,
    min_refresh_gap: Duration,
    max_metadata_age: Duration,
    clock: Clock,
}

impl Options {
    fn google() -> Self {
        Options {
            issuer: ISSUER.into(),
            min_refresh_gap: MIN_REFRESH_GAP,
            max_metadata_age: MAX_METADATA_AGE,
            clock: Clock::System,
        }
    }
}

/// The monotonic clock the adapter ages its metadata by. `System` is `Instant::now`, what `elapsed()` read before.
enum Clock {
    System,
    #[cfg(feature = "static-oidc")]
    Test(TestClock),
}

impl Clock {
    fn now(&self) -> Instant {
        match self {
            Clock::System => Instant::now(),
            #[cfg(feature = "static-oidc")]
            Clock::Test(c) => c.now(),
        }
    }
}

/// A monotonic clock for tests that moves only when `advance` is called; clones share one time. It starts at the
/// real `Instant::now()` of its creation.
#[cfg(feature = "static-oidc")]
#[derive(Clone)]
pub struct TestClock(Arc<std::sync::Mutex<Instant>>);

#[cfg(feature = "static-oidc")]
impl TestClock {
    pub fn new() -> Self {
        TestClock(Arc::new(std::sync::Mutex::new(Instant::now())))
    }
    pub fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap();
        *now = now
            .checked_add(by)
            .expect("the test clock was advanced past the range of Instant");
    }
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

#[cfg(feature = "static-oidc")]
impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

/// What a refresh needs to rebuild the client from fresh discovery.
struct Refresh {
    client_id: String,
    client_secret: String,
    redirect: openidconnect::RedirectUrl,
}

struct Live {
    client: GoogleClient,
    fetched_at: Instant,
}

/// The Google adapter. Google rotates its signing keys, so the discovered metadata (and JWKS) is not fixed for the
/// life of the process: it sits behind a lock, is refreshed when older than 24 hours, and is refreshed once and the
/// verification retried once when an ID token fails with a signature or key error (`NoMatchingKey`, `CryptoError`,
/// `InvalidKey`). Every discovery and refresh goes through the same no-redirect, time-limited HTTP client.
pub(crate) struct GoogleOidc {
    live: RwLock<Live>,
    http: ProviderHttp,
    refresh: Option<Refresh>,
    /// The issuer every discovery and refresh goes to, the two refresh windows and the clock that measures them.
    options: Options,
}

fn http_builder() -> reqwest::ClientBuilder {
    // SSRF guard the openidconnect docs require: never follow redirects on provider calls. The timeouts keep a
    // stalled provider from hanging startup or a sign-in callback.
    reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
}

/// The HTTP client every provider call goes through.
#[derive(Clone)]
struct ProviderHttp {
    client: reqwest::Client,
    /// Under `static-oidc`, set only by the test factory: refuse every request that is not http to a loopback IP.
    #[cfg(feature = "static-oidc")]
    loopback_only: bool,
}

impl ProviderHttp {
    /// The client production has always used.
    fn production() -> anyhow::Result<Self> {
        Ok(ProviderHttp {
            client: http_builder().build()?,
            #[cfg(feature = "static-oidc")]
            loopback_only: false,
        })
    }

    /// The production settings, no proxy, no name resolution, and the per-request loopback guard.
    #[cfg(feature = "static-oidc")]
    fn loopback_only() -> anyhow::Result<Self> {
        Ok(ProviderHttp {
            client: http_builder()
                .no_proxy()
                .dns_resolver(Arc::new(NoNames))
                .build()?,
            loopback_only: true,
        })
    }
}

impl<'c> openidconnect::AsyncHttpClient<'c> for ProviderHttp {
    type Error = openidconnect::HttpClientError<reqwest::Error>;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<openidconnect::HttpResponse, Self::Error>>
                + Send
                + Sync
                + 'c,
        >,
    >;

    fn call(&'c self, request: openidconnect::HttpRequest) -> Self::Future {
        #[cfg(feature = "static-oidc")]
        {
            if self.loopback_only && !is_loopback_http(&request.uri().to_string()) {
                let refused = format!(
                    "the test HTTP client sends only to http on a loopback IP address; refused {}",
                    request.uri()
                );
                return Box::pin(
                    async move { Err(openidconnect::HttpClientError::Other(refused)) },
                );
            }
        }
        openidconnect::AsyncHttpClient::call(&self.client, request)
    }
}

/// `http` with a host that is a loopback IP literal (127.0.0.0/8 or ::1). Hostnames, `localhost` included, and
/// IPv4-mapped IPv6 addresses are not.
#[cfg(feature = "static-oidc")]
fn is_loopback_http(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    u.scheme() == "http"
        && match u.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        }
}

/// Defence in depth behind the guard: the test client resolves no hostnames at all.
#[cfg(feature = "static-oidc")]
struct NoNames;

#[cfg(feature = "static-oidc")]
impl reqwest::dns::Resolve for NoNames {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let name = name.as_str().to_string();
        Box::pin(async move {
            Err(format!("the test HTTP client resolves no hostnames (asked for {name})").into())
        })
    }
}

async fn discover(
    http: &ProviderHttp,
    issuer: &str,
) -> anyhow::Result<openidconnect::core::CoreProviderMetadata> {
    use openidconnect::IssuerUrl;
    use openidconnect::core::CoreProviderMetadata;
    CoreProviderMetadata::discover_async(IssuerUrl::new(issuer.into())?, http)
        .await
        .map_err(|e| anyhow::anyhow!("oidc discovery failed: {}", chain(&e)))
}

fn build_client(meta: openidconnect::core::CoreProviderMetadata, r: &Refresh) -> GoogleClient {
    use openidconnect::core::CoreClient;
    use openidconnect::{ClientId, ClientSecret};
    CoreClient::from_provider_metadata(
        meta,
        ClientId::new(r.client_id.clone()),
        Some(ClientSecret::new(r.client_secret.clone())),
    )
    .set_redirect_uri(r.redirect.clone())
}

impl GoogleOidc {
    /// A redirect must be https; plain http is allowed only on loopback (local development), and Google itself
    /// rejects it elsewhere. Any other scheme is refused.
    pub(crate) fn check_redirect(url: &str) -> anyhow::Result<()> {
        let u = url::Url::parse(url)?;
        let loopback = matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        let ok = u.scheme() == "https" || (u.scheme() == "http" && loopback);
        if !ok {
            anyhow::bail!("oidc redirect_url must be https unless it is http on loopback");
        }
        Ok(())
    }

    /// An adapter around an already built client; it never refreshes (there is no discovery to repeat).
    #[cfg(test)]
    pub(crate) fn from_client(client: GoogleClient, http: reqwest::Client) -> Self {
        GoogleOidc {
            live: RwLock::new(Live {
                client,
                fetched_at: Instant::now(),
            }),
            http: ProviderHttp {
                client: http,
                #[cfg(feature = "static-oidc")]
                loopback_only: false,
            },
            refresh: None,
            options: Options::google(),
        }
    }

    /// How old the cached metadata is, by the adapter's clock.
    async fn age(&self) -> Duration {
        self.options
            .clock
            .now()
            .saturating_duration_since(self.live.read().await.fetched_at)
    }

    /// The current client, refreshed first if the metadata has gone stale.
    async fn current(&self) -> GoogleClient {
        if self.age().await > self.options.max_metadata_age {
            self.refresh_keys(Duration::ZERO).await;
        }
        self.live.read().await.client.clone()
    }

    /// Re-runs discovery and swaps the client in. Skipped when the cached metadata is younger than `min_age`, when
    /// this adapter cannot refresh, or when discovery fails (the old client stays). Returns whether it swapped.
    async fn refresh_keys(&self, min_age: Duration) -> bool {
        let Some(r) = &self.refresh else { return false };
        if self.age().await < min_age {
            return false;
        }
        let Ok(meta) = discover(&self.http, &self.options.issuer).await else {
            return false;
        };
        let client = build_client(meta, r);
        *self.live.write().await = Live {
            client,
            fetched_at: self.options.clock.now(),
        };
        true
    }
}

/// Signature failures that a rotated key set can cause.
fn is_key_error(e: &openidconnect::ClaimsVerificationError) -> bool {
    use openidconnect::{ClaimsVerificationError as C, SignatureVerificationError as S};
    matches!(
        e,
        C::SignatureVerification(S::NoMatchingKey | S::CryptoError(_) | S::InvalidKey(_))
    )
}

/// Verifies the ID token with the library and extracts the identity from the verified claims.
fn verify(
    client: &GoogleClient,
    id_token: &openidconnect::core::CoreIdToken,
    nonce: &str,
) -> Result<VerifiedIdentity, openidconnect::ClaimsVerificationError> {
    let claims = id_token.claims(
        &client.id_token_verifier(),
        &openidconnect::Nonce::new(nonce.to_string()),
    )?;
    Ok(VerifiedIdentity {
        // Filled in below; an empty address is mapped to NoEmail by the caller.
        email: claims.email().map(|e| e.to_string()).unwrap_or_default(),
        email_verified: claims.email_verified().unwrap_or(false),
        name: claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.to_string()),
    })
}

/// Discovers Google's metadata (network) and builds the client; call at startup, fail loudly.
pub async fn make_google_oidc(cfg: GoogleOidcConfig) -> anyhow::Result<Arc<dyn OidcClient>> {
    // The redirect is checked before the HTTP client is built, as today, so a bad redirect is the error reported.
    GoogleOidc::check_redirect(&cfg.redirect_url)?;
    build_google_oidc(cfg, Options::google(), ProviderHttp::production()?).await
}

async fn build_google_oidc(
    cfg: GoogleOidcConfig,
    options: Options,
    http: ProviderHttp,
) -> anyhow::Result<Arc<dyn OidcClient>> {
    GoogleOidc::check_redirect(&cfg.redirect_url)?;
    let refresh = Refresh {
        client_id: cfg.client_id,
        client_secret: cfg.client_secret,
        redirect: openidconnect::RedirectUrl::new(cfg.redirect_url)?,
    };
    let client = build_client(discover(&http, &options.issuer).await?, &refresh);
    let fetched_at = options.clock.now();
    Ok(Arc::new(GoogleOidc {
        live: RwLock::new(Live { client, fetched_at }),
        http,
        refresh: Some(refresh),
        options,
    }))
}

/// What the test factory varies: the issuer to discover (a `FakeIssuer` on 127.0.0.1), the refresh windows and the
/// clock that measures them.
#[cfg(feature = "static-oidc")]
pub struct GoogleOidcTestOptions {
    pub issuer: String,
    pub min_refresh_gap: Duration,
    pub max_metadata_age: Duration,
    pub clock: TestClock,
}

/// The production adapter pointed at a loopback issuer, on a loopback-only HTTP client. Exists only under
/// `static-oidc`, which the Dockerfile never enables, and no configuration reaches it: the server builds
/// `make_google_oidc` from `[oidc]`.
#[cfg(feature = "static-oidc")]
pub async fn make_google_oidc_for_tests(
    cfg: GoogleOidcConfig,
    o: GoogleOidcTestOptions,
) -> anyhow::Result<Arc<dyn OidcClient>> {
    anyhow::ensure!(
        is_loopback_http(&o.issuer),
        "a test issuer must be http on a loopback IP address, got {}",
        o.issuer
    );
    build_google_oidc(
        cfg,
        Options {
            issuer: o.issuer,
            min_refresh_gap: o.min_refresh_gap,
            max_metadata_age: o.max_metadata_age,
            clock: Clock::Test(o.clock),
        },
        ProviderHttp::loopback_only()?,
    )
    .await
}

#[async_trait]
impl OidcClient for GoogleOidc {
    async fn begin(
        &self,
        state: &str,
        nonce: &str,
        pkce_verifier: &str,
    ) -> Result<String, OidcError> {
        use openidconnect::core::CoreAuthenticationFlow;
        use openidconnect::{CsrfToken, Nonce, PkceCodeChallenge, PkceCodeVerifier, Scope};
        check_verifier(pkce_verifier)?;
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(
            pkce_verifier.to_string(),
        ));
        let (state, nonce) = (state.to_string(), nonce.to_string());
        let client = self.current().await;
        let (url, _, _) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(state),
                move || Nonce::new(nonce),
            )
            .add_scope(Scope::new("email".into()))
            .add_scope(Scope::new("profile".into()))
            .set_pkce_challenge(challenge)
            .url();
        Ok(url.to_string())
    }

    async fn finish(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
    ) -> Result<VerifiedIdentity, OidcError> {
        use openidconnect::{AuthorizationCode, PkceCodeVerifier, TokenResponse};
        let client = self.current().await;
        let resp = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| OidcError::Exchange(chain(&e)))?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_string()))
            .request_async(&self.http)
            .await
            .map_err(|e| OidcError::Exchange(chain(&e)))?;
        let id_token = resp
            .id_token()
            .ok_or_else(|| OidcError::IdToken("missing".into()))?;
        let who = match verify(&client, id_token, nonce) {
            Ok(who) => who,
            Err(e) if is_key_error(&e) && self.refresh_keys(self.options.min_refresh_gap).await => {
                // Google rotated its keys since we last looked: one refresh, one retry.
                let client = self.live.read().await.client.clone();
                verify(&client, id_token, nonce).map_err(|e| OidcError::IdToken(chain(&e)))?
            }
            Err(e) => return Err(OidcError::IdToken(chain(&e))),
        };
        if who.email.is_empty() {
            return Err(OidcError::NoEmail);
        }
        Ok(who)
    }
}

/// Test and local-development adapter: any code is "static", the identity is fixed. The server only constructs it
/// for a loopback bind, and it exists only under the `static-oidc` cargo feature, which the
/// Dockerfile never enables, so it cannot become a production backdoor by configuration alone.
#[cfg(feature = "static-oidc")]
pub(crate) struct StaticOidc {
    who: VerifiedIdentity,
    redirect_url: String,
}

#[cfg(feature = "static-oidc")]
pub fn make_static_oidc(email: &str, redirect_url: &str) -> Arc<dyn OidcClient> {
    Arc::new(StaticOidc {
        who: VerifiedIdentity {
            email: email.into(),
            email_verified: true,
            name: Some("Static User".into()),
        },
        redirect_url: redirect_url.into(),
    })
}

#[cfg(feature = "static-oidc")]
#[async_trait]
impl OidcClient for StaticOidc {
    async fn begin(
        &self,
        state: &str,
        _nonce: &str,
        pkce_verifier: &str,
    ) -> Result<String, OidcError> {
        check_verifier(pkce_verifier)?;
        let state: String = url::form_urlencoded::byte_serialize(state.as_bytes()).collect();
        let sep = if self.redirect_url.contains('?') {
            '&'
        } else {
            '?'
        };
        Ok(format!(
            "{}{sep}code=static&state={state}",
            self.redirect_url
        ))
    }
    async fn finish(&self, code: &str, _v: &str, _n: &str) -> Result<VerifiedIdentity, OidcError> {
        if code != "static" {
            return Err(OidcError::Exchange(
                "static adapter accepts only code=static".into(),
            ));
        }
        Ok(self.who.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "static-oidc")]
    #[test]
    fn is_loopback_http_accepts_only_http_on_a_loopback_ip_literal() {
        for refused in [
            "http://127.0.0.1@192.0.2.1/",
            "http://[::ffff:127.0.0.1]/",
            "http://0.0.0.0/",
            "http://localhost/",
            "https://127.0.0.1/",
            "not a url",
            "http://[2001:db8::1]/",
        ] {
            assert!(!is_loopback_http(refused), "{refused} was allowed");
        }
        for allowed in [
            "http://user@127.0.0.1/",
            "HTTP://127.0.0.1/",
            "http://127.1/",
            "http://[::1]:8080/",
        ] {
            assert!(is_loopback_http(allowed), "{allowed} was refused");
        }
    }

    #[cfg(feature = "static-oidc")]
    const VERIFIER: &str = "verifier-verifier-verifier-verifier-verifier";

    #[cfg(feature = "static-oidc")]
    #[tokio::test]
    async fn static_adapter_returns_its_identity_and_a_local_begin_url() {
        let o = make_static_oidc("dev@example.com", "http://127.0.0.1:7474/auth/callback");
        let url = o.begin("st4te", "n0nce", VERIFIER).await.unwrap();
        assert!(
            url.starts_with("http://127.0.0.1:7474/auth/callback?code=static&state=st4te"),
            "{url}"
        );
        let who = o.finish("static", VERIFIER, "n0nce").await.unwrap();
        assert_eq!(who.email, "dev@example.com");
        assert!(who.email_verified);
        assert!(o.finish("other", "v", "n").await.is_err());
    }

    #[cfg(feature = "static-oidc")]
    #[tokio::test]
    async fn static_adapter_encodes_state_joins_an_existing_query_and_checks_the_verifier() {
        let o = make_static_oidc("dev@example.com", "http://127.0.0.1:7474/cb");
        let url = o.begin("a b&c=d", "n", VERIFIER).await.unwrap();
        assert!(url.ends_with("&state=a+b%26c%3Dd"), "{url}");
        let o = make_static_oidc("dev@example.com", "http://127.0.0.1:7474/cb?x=1");
        let url = o.begin("s", "n", VERIFIER).await.unwrap();
        assert_eq!(url, "http://127.0.0.1:7474/cb?x=1&code=static&state=s");
        assert!(matches!(
            o.begin("s", "n", &"v".repeat(42)).await,
            Err(OidcError::BadVerifier)
        ));
        assert!(o.begin("s", "n", &"v".repeat(43)).await.is_ok());
        assert!(o.begin("s", "n", &"v".repeat(128)).await.is_ok());
        assert!(matches!(
            o.begin("s", "n", &"v".repeat(129)).await,
            Err(OidcError::BadVerifier)
        ));
    }

    #[test]
    fn production_options_name_google_the_documented_windows_and_the_system_clock() {
        let o = Options::google();
        assert_eq!(o.issuer, "https://accounts.google.com");
        assert_eq!(o.min_refresh_gap, Duration::from_secs(60));
        assert_eq!(o.max_metadata_age, Duration::from_secs(24 * 60 * 60));
        assert!(matches!(o.clock, Clock::System));
    }

    #[test]
    fn the_system_clock_is_instant_now() {
        let before = Instant::now();
        let t = Clock::System.now();
        assert!(before <= t && t <= Instant::now());
    }

    #[test]
    fn google_adapter_refuses_a_non_https_redirect_outside_loopback() {
        let err = GoogleOidc::check_redirect("http://reeve.example.com/auth/callback").unwrap_err();
        assert!(err.to_string().contains("https"));
        assert!(GoogleOidc::check_redirect("http://127.0.0.1:7474/auth/callback").is_ok());
        assert!(GoogleOidc::check_redirect("http://[::1]:7474/auth/callback").is_ok());
        assert!(GoogleOidc::check_redirect("http://localhost:7474/auth/callback").is_ok());
        assert!(GoogleOidc::check_redirect("https://reeve.example.com/auth/callback").is_ok());
        for bad in [
            "ftp://localhost/cb",
            "ws://127.0.0.1/cb",
            "http://localhost.evil.com/cb",
            "http://127.0.0.1.evil.com/cb",
            "http://127.0.0.2/cb",
        ] {
            assert!(GoogleOidc::check_redirect(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn config_debug_redacts_the_client_secret() {
        let cfg = GoogleOidcConfig {
            client_id: "the-id".into(),
            client_secret: "s3cr3t-value".into(),
            redirect_url: "https://reeve.example.com/auth/callback".into(),
        };
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("s3cr3t-value"), "{shown}");
        assert!(
            shown.contains("<redacted>") && shown.contains("the-id"),
            "{shown}"
        );
    }

    /// A client built offline: provider metadata assembled by hand, no discovery request.
    fn offline_adapter() -> GoogleOidc {
        use openidconnect::core::{
            CoreClient, CoreJwsSigningAlgorithm, CoreProviderMetadata, CoreResponseType,
        };
        use openidconnect::{
            AuthUrl, ClientId, ClientSecret, EmptyAdditionalProviderMetadata, IssuerUrl,
            JsonWebKeySetUrl, RedirectUrl, ResponseTypes, TokenUrl,
        };
        let meta = CoreProviderMetadata::new(
            IssuerUrl::new("https://idp.example.com".into()).unwrap(),
            AuthUrl::new("https://idp.example.com/authorize".into()).unwrap(),
            JsonWebKeySetUrl::new("https://idp.example.com/jwks".into()).unwrap(),
            vec![ResponseTypes::new(vec![CoreResponseType::Code])],
            vec![],
            vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
            EmptyAdditionalProviderMetadata {},
        )
        .set_token_endpoint(Some(
            TokenUrl::new("https://idp.example.com/token".into()).unwrap(),
        ));
        let client = CoreClient::from_provider_metadata(
            meta,
            ClientId::new("the-client".into()),
            Some(ClientSecret::new("the-secret".into())),
        )
        .set_redirect_uri(
            RedirectUrl::new("https://reeve.example.com/auth/callback".into()).unwrap(),
        );
        GoogleOidc::from_client(client, reqwest::Client::new())
    }

    #[tokio::test]
    async fn google_begin_builds_a_code_flow_url_with_state_nonce_and_s256_pkce() {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let verifier = "A".repeat(43);
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let url = offline_adapter()
            .begin("st", "nonce-value", &verifier)
            .await
            .unwrap();
        let url = url::Url::parse(&url).unwrap();
        assert_eq!(
            url.as_str().split('?').next(),
            Some("https://idp.example.com/authorize")
        );
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "the-client");
        assert_eq!(q["redirect_uri"], "https://reeve.example.com/auth/callback");
        let scopes: Vec<_> = q["scope"].split(' ').collect();
        for s in ["openid", "email", "profile"] {
            assert!(scopes.contains(&s), "{scopes:?}");
        }
        assert_eq!(q["state"], "st");
        assert_eq!(q["nonce"], "nonce-value");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], expected);
        // The secret and the verifier never leave in the URL.
        assert!(!url.as_str().contains("the-secret") && !url.as_str().contains(&verifier));
    }

    #[tokio::test]
    async fn google_begin_rejects_a_verifier_outside_43_to_128_without_panicking() {
        let a = offline_adapter();
        assert!(matches!(
            a.begin("s", "n", &"v".repeat(42)).await,
            Err(OidcError::BadVerifier)
        ));
        assert!(matches!(
            a.begin("s", "n", &"v".repeat(129)).await,
            Err(OidcError::BadVerifier)
        ));
        assert!(a.begin("s", "n", &"v".repeat(43)).await.is_ok());
        assert!(a.begin("s", "n", &"v".repeat(128)).await.is_ok());
    }
}
