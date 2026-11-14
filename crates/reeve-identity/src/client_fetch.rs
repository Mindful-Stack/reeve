//! The HTTP adapter of `ClientDocumentFetcher` (kb/ADR-0029 R3): a client's metadata document is fetched only
//! over https from a public address, without redirects or a proxy, within 64 KiB and 5 seconds, at most four at
//! once. Two factories build it: the production one and, under `static-oidc`, one that fetches only http from a
//! loopback IP literal, as `oidc.rs` has `ProviderHttp::production` and `loopback_only`.
use crate::clients::{ClientDocument, ClientDocumentFetcher, ClientRefusal};
use async_trait::async_trait;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

/// The largest document read.
const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
/// The limit on one fetch, from the start of connecting (resolving included) to the end of the body.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// How many fetches run at once in the process; one more is refused, not queued.
const MAX_CONCURRENT_FETCHES: usize = 4;

/// RFC 6890's IPv4 special-purpose blocks, and multicast, as (network, prefix length). 240.0.0.0/4 holds the
/// broadcast address.
const V4_NOT_PUBLIC: &[(Ipv4Addr, u32)] = &[
    (Ipv4Addr::new(0, 0, 0, 0), 8),       // this host on this network
    (Ipv4Addr::new(10, 0, 0, 0), 8),      // private
    (Ipv4Addr::new(100, 64, 0, 0), 10),   // shared address space
    (Ipv4Addr::new(127, 0, 0, 0), 8),     // loopback
    (Ipv4Addr::new(169, 254, 0, 0), 16),  // link-local
    (Ipv4Addr::new(172, 16, 0, 0), 12),   // private
    (Ipv4Addr::new(192, 0, 0, 0), 24),    // IETF protocol assignments
    (Ipv4Addr::new(192, 0, 2, 0), 24),    // documentation
    (Ipv4Addr::new(192, 88, 99, 0), 24),  // 6to4 relay anycast
    (Ipv4Addr::new(192, 168, 0, 0), 16),  // private
    (Ipv4Addr::new(198, 18, 0, 0), 15),   // benchmarking
    (Ipv4Addr::new(198, 51, 100, 0), 24), // documentation
    (Ipv4Addr::new(203, 0, 113, 0), 24),  // documentation
    (Ipv4Addr::new(224, 0, 0, 0), 4),     // multicast
    (Ipv4Addr::new(240, 0, 0, 0), 4),     // reserved
];

/// The only IPv6 space that can be public: global unicast.
const V6_GLOBAL_UNICAST: (Ipv6Addr, u32) = (Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3);

/// The parts of global unicast that are not public.
const V6_NOT_PUBLIC: &[(Ipv6Addr, u32)] = &[
    (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23), // IETF protocol assignments
    (Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32), // documentation
    (Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16), // 6to4
    (Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20), // documentation
];

fn in_v4_block(ip: Ipv4Addr, (network, length): (Ipv4Addr, u32)) -> bool {
    let mask = u32::MAX << (32 - length);
    u32::from(ip) & mask == u32::from(network) & mask
}

fn in_v6_block(ip: Ipv6Addr, (network, length): (Ipv6Addr, u32)) -> bool {
    let mask = u128::MAX << (128 - length);
    u128::from(ip) & mask == u128::from(network) & mask
}

/// Whether a connection to `ip` leaves for the public internet. An IPv4-mapped IPv6 address is judged as the
/// IPv4 address it carries; IPv4 by a deny-list, IPv6 by an allow-list.
fn is_public(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => !V4_NOT_PUBLIC.iter().any(|block| in_v4_block(ip, *block)),
        IpAddr::V6(ip) => {
            in_v6_block(ip, V6_GLOBAL_UNICAST)
                && !V6_NOT_PUBLIC.iter().any(|block| in_v6_block(ip, *block))
        }
    }
}

/// Why a name is not connected to. Its text reaches the operator's log through the request's error.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
struct LookupRefused(String);

/// The addresses `name` resolved to, in canonical form; refused when there are none or any one of them is not
/// public. Never a part of the set.
fn check_public(name: &str, found: Vec<IpAddr>) -> Result<Vec<IpAddr>, LookupRefused> {
    let found: Vec<IpAddr> = found.into_iter().map(|ip| ip.to_canonical()).collect();
    if found.is_empty() {
        return Err(LookupRefused(format!("{name} resolves to no address")));
    }
    if let Some(bad) = found.iter().find(|ip| !is_public(**ip)) {
        return Err(LookupRefused(format!(
            "{name} resolves to {bad}, which is not a public address"
        )));
    }
    Ok(found)
}

/// The resolver the production client connects through: the connector is handed only addresses `check_public`
/// returned, so the address connected to is one that was checked.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let name = name.as_str().to_string();
        Box::pin(async move {
            let found: Vec<IpAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .map(|address| address.ip())
                .collect();
            let checked = check_public(&name, found)?;
            // Port 0: reqwest replaces it with the scheme's port.
            let addresses: reqwest::dns::Addrs =
                Box::new(checked.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addresses)
        })
    }
}

/// Which client ids a fetcher admits.
#[derive(Debug, Clone, Copy)]
enum IdRules {
    /// https, a host that is a name, no port.
    Production,
    /// http on a loopback IP literal: the test factory's.
    #[cfg(feature = "static-oidc")]
    LoopbackOnly,
}

/// The client id as a URL, or why it is refused. Nothing has been sent when this returns.
fn check_client_id(raw: &str, rules: IdRules) -> Result<url::Url, ClientRefusal> {
    let bad = ClientRefusal::BadClientId;
    let url = url::Url::parse(raw).map_err(|_| bad("it is not a URL"))?;
    match rules {
        IdRules::Production => {
            if url.scheme() != "https" {
                return Err(bad("it must be https"));
            }
            // An address literal never reaches the resolver, so it is refused here.
            if !matches!(url.host(), Some(url::Host::Domain(_))) {
                return Err(bad("its host must be a name, not an address"));
            }
            if url.port().is_some() {
                return Err(bad("it names a port"));
            }
        }
        #[cfg(feature = "static-oidc")]
        IdRules::LoopbackOnly => {
            if !crate::oidc::is_loopback_http(raw) {
                return Err(bad(
                    "the test fetcher fetches only http from a loopback IP address",
                ));
            }
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad("it carries user information"));
    }
    if url.path() == "/" {
        return Err(bad("it has no path"));
    }
    if url.query().is_some() {
        return Err(bad("it has a query"));
    }
    if url.fragment().is_some() {
        return Err(bad("it has a fragment"));
    }
    // The parser removes dot segments, lowercases the host and drops a default port. An id is compared as a
    // string and fetched as written, so one that changes when parsed is refused.
    if url.as_str() != raw {
        return Err(bad(
            "it is not in its normal form (dot segments, upper case or a default port)",
        ));
    }
    Ok(url)
}

/// The one adapter. Production and the tests differ only in the client the factory builds and in `rules`.
pub(crate) struct HttpClientFetcher {
    http: reqwest::Client,
    rules: IdRules,
    permits: tokio::sync::Semaphore,
}

/// What both factories share: no redirect is followed, no proxy is used (the system's included), and the whole
/// request, from connecting to the end of the body, ends at `timeout`.
fn builder(timeout: Duration) -> reqwest::ClientBuilder {
    reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(timeout)
        .user_agent("reeve")
}

impl HttpClientFetcher {
    fn new(http: reqwest::Client, rules: IdRules) -> Self {
        HttpClientFetcher {
            http,
            rules,
            permits: tokio::sync::Semaphore::new(MAX_CONCURRENT_FETCHES),
        }
    }

    fn production() -> anyhow::Result<Self> {
        let http = builder(FETCH_TIMEOUT)
            .https_only(true)
            .dns_resolver(Arc::new(PublicOnly))
            .build()?;
        Ok(Self::new(http, IdRules::Production))
    }

    /// `timeout` is a parameter so that one test can see the fetcher give up without waiting five seconds.
    #[cfg(feature = "static-oidc")]
    fn loopback(timeout: Duration) -> anyhow::Result<Self> {
        let http = builder(timeout)
            .dns_resolver(Arc::new(crate::oidc::NoNames))
            .build()?;
        Ok(Self::new(http, IdRules::LoopbackOnly))
    }

    /// The body at `url`: status 200, at most `MAX_DOCUMENT_BYTES`, counted as it arrives (reqwest's `stream`
    /// feature is off, so the body is read chunk by chunk).
    async fn body(&self, url: &url::Url) -> Result<Vec<u8>, ClientRefusal> {
        // Without the URL: the log line names the host and the reason, never the whole client id.
        let unreachable =
            |e: reqwest::Error| ClientRefusal::Unreachable(crate::oidc::chain(&e.without_url()));
        let mut answer = self
            .http
            .get(url.as_str())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(unreachable)?;
        if answer.status() != reqwest::StatusCode::OK {
            return Err(ClientRefusal::Status(answer.status().as_u16()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = answer.chunk().await.map_err(unreachable)? {
            if body.len() + chunk.len() > MAX_DOCUMENT_BYTES {
                return Err(ClientRefusal::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

#[async_trait]
impl ClientDocumentFetcher for HttpClientFetcher {
    fn admit(&self, client_id: &str) -> Result<url::Url, ClientRefusal> {
        check_client_id(client_id, self.rules)
    }

    async fn fetch(&self, client_id: &str) -> Result<ClientDocument, ClientRefusal> {
        let url = self.admit(client_id)?;
        // Not queued: a request beyond the limit is refused, and the permit is given back when this returns.
        let _permit = self
            .permits
            .try_acquire()
            .map_err(|_| ClientRefusal::Busy)?;
        let body = self.body(&url).await?;
        ClientDocument::parse(client_id, &body)
    }
}

/// The production fetcher: https only, names resolved through `PublicOnly`, client ids by the production rules.
pub fn make_client_fetcher() -> anyhow::Result<Arc<dyn ClientDocumentFetcher>> {
    Ok(Arc::new(HttpClientFetcher::production()?))
}

/// The same adapter for the static provider: it fetches only http from a loopback IP literal and resolves no
/// name at all. Exists only under `static-oidc`, which the Dockerfile never enables.
#[cfg(feature = "static-oidc")]
pub fn make_loopback_client_fetcher() -> anyhow::Result<Arc<dyn ClientDocumentFetcher>> {
    Ok(Arc::new(HttpClientFetcher::loopback(FETCH_TIMEOUT)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    /// The edges of every block, both sides. Catches a missing block, a wrong prefix length, an IPv4-mapped
    /// address judged as IPv6, and any IPv6 address outside global unicast let through.
    #[test]
    fn only_addresses_outside_every_special_block_are_public() {
        for public in [
            "1.1.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.0",
            "160.79.104.10",
            "172.15.255.255",
            "172.32.0.0",
            "192.0.1.0",
            "192.167.255.255",
            "198.17.255.255",
            "198.20.0.0",
            "223.255.255.255",
            "2001:200::1",
            "2001:4860:4860::8888",
            "2606:4700::1111",
            "2a00:1450::1",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public(ip(public)), "{public} was refused");
        }
        for special in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "127.255.255.254",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "239.255.255.255",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::2",
            "::7f00:1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:0:7f00:1",
            "1::1",
            "64:ff9b::a00:1",
            "64:ff9b:1::1",
            "100::1",
            "2001::1",
            "2001:1ff:ffff::1",
            "2001:db8::1",
            "2002:a00:1::1",
            "3fff::1",
            "4000::1",
            "5f00::1",
            "fc00::1",
            "fdff::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
        ] {
            assert!(!is_public(ip(special)), "{special} was allowed");
        }
    }

    /// Catches a check that filters (returns the public address and drops the private one) where it must
    /// refuse, one that returns anything but the addresses it checked, and an empty answer let through. The
    /// `localhost` row is what the system resolver gives for that name.
    #[test]
    fn one_address_that_is_not_public_refuses_the_whole_set() {
        let refused = |name: &str, found: &[&str]| {
            check_public(name, found.iter().map(|text| ip(text)).collect())
                .unwrap_err()
                .0
        };
        assert_eq!(
            refused("client.example", &["160.79.104.10", "10.0.0.5"]),
            "client.example resolves to 10.0.0.5, which is not a public address"
        );
        assert_eq!(
            refused("client.example", &["169.254.169.254", "160.79.104.10"]),
            "client.example resolves to 169.254.169.254, which is not a public address"
        );
        assert_eq!(
            refused("localhost", &["::1", "127.0.0.1"]),
            "localhost resolves to ::1, which is not a public address"
        );
        assert_eq!(
            refused("client.example", &["::ffff:192.168.1.1"]),
            "client.example resolves to 192.168.1.1, which is not a public address"
        );
        assert_eq!(
            refused("client.example", &[]),
            "client.example resolves to no address"
        );
        let found = vec![
            ip("160.79.104.10"),
            ip("2606:4700::1111"),
            ip("::ffff:8.8.8.8"),
        ];
        assert_eq!(
            check_public("client.example", found),
            Ok(vec![
                ip("160.79.104.10"),
                ip("2606:4700::1111"),
                ip("8.8.8.8")
            ])
        );
    }

    /// Each row is refused and none is fetched. The reasons may be reworded; no row may be accepted. Catches a
    /// rule that is missing, above all the address literal (which would bypass the resolver) and the dot segments
    /// (which the URL parser hides).
    #[test]
    fn the_production_rules_refuse_every_client_id_that_breaks_one() {
        let rules = IdRules::Production;
        for accepted in [
            "https://claude.ai/oauth/claude-code-client-metadata",
            "https://chatgpt.com/a/b.json",
        ] {
            let url = check_client_id(accepted, rules).unwrap();
            assert_eq!(url.as_str(), accepted);
        }
        let normal_form =
            "it is not in its normal form (dot segments, upper case or a default port)";
        let address = "its host must be a name, not an address";
        for (refused, why) in [
            ("", "it is not a URL"),
            ("claude.ai/client.json", "it is not a URL"),
            ("http://claude.ai/client.json", "it must be https"),
            ("ftp://claude.ai/client.json", "it must be https"),
            ("https://160.79.104.10/client.json", address),
            ("https://127.0.0.1/client.json", address),
            ("https://[2606:4700::1111]/client.json", address),
            ("https://claude.ai:8443/client.json", "it names a port"),
            (
                "https://user@claude.ai/client.json",
                "it carries user information",
            ),
            (
                "https://user:pw@claude.ai/client.json",
                "it carries user information",
            ),
            ("https://claude.ai", "it has no path"),
            ("https://claude.ai/", "it has no path"),
            ("https://claude.ai/client.json?v=1", "it has a query"),
            ("https://claude.ai/client.json?", "it has a query"),
            ("https://claude.ai/client.json#x", "it has a fragment"),
            ("https://claude.ai:443/client.json", normal_form),
            ("https://Claude.ai/client.json", normal_form),
            ("https://claude.ai/a/../client.json", normal_form),
            ("https://claude.ai/a/./client.json", normal_form),
            ("https://claude.ai/a/%2e%2e/client.json", normal_form),
            ("https://claude.ai/a/%2E/client.json", normal_form),
            ("https://claude.ai/a/client.json/..", normal_form),
        ] {
            assert_eq!(
                check_client_id(refused, rules).unwrap_err(),
                ClientRefusal::BadClientId(why),
                "{refused}"
            );
        }
        // The production factory uses these rules.
        let fetcher = make_client_fetcher().unwrap();
        assert!(
            fetcher
                .admit("https://claude.ai/oauth/claude-code-client-metadata")
                .is_ok()
        );
        assert_eq!(
            fetcher.admit("http://127.0.0.1:4000/client.json"),
            Err(ClientRefusal::BadClientId("it must be https"))
        );
    }

    #[cfg(feature = "static-oidc")]
    mod against_the_fake_host {
        use super::*;
        use crate::fake_client_host::{FakeAnswer, FakeClientHost};

        /// The test factory's rules reach nothing but loopback. Catches a test fetcher that could be pointed at
        /// the internet.
        #[test]
        fn the_loopback_rules_admit_only_http_on_a_loopback_ip_literal() {
            let rules = IdRules::LoopbackOnly;
            assert!(check_client_id("http://127.0.0.1:4000/client.json", rules).is_ok());
            for refused in [
                "https://claude.ai/client.json",
                "http://claude.ai/client.json",
                "http://localhost:4000/client.json",
                "https://127.0.0.1:4000/client.json",
                "http://192.168.1.1/client.json",
                "http://127.0.0.1:4000/",
                "http://127.0.0.1:4000/client.json?v=1",
                "http://127.0.0.1:4000/a/../client.json",
            ] {
                assert!(check_client_id(refused, rules).is_err(), "{refused}");
            }
        }

        /// With all four permits held, a fetch is refused at once and nothing reaches the host; with them free,
        /// five fetches one after another all succeed, so each gave its permit back. Catches a missing limit, a
        /// limit that waits, and a permit that leaks.
        #[tokio::test]
        async fn a_fetch_beyond_four_is_refused_and_permits_come_back() {
            let host = FakeClientHost::start().await;
            let id = host.publish("/client.json", "Example App", &[]);
            let fetcher = HttpClientFetcher::loopback(FETCH_TIMEOUT).unwrap();
            let held = fetcher.permits.try_acquire_many(4).unwrap();
            assert_eq!(fetcher.fetch(&id).await, Err(ClientRefusal::Busy));
            assert_eq!(host.hits("/client.json"), 0);
            drop(held);
            for _ in 0..5 {
                fetcher.fetch(&id).await.unwrap();
            }
            assert_eq!(host.hits("/client.json"), 5);
            assert_eq!(fetcher.permits.available_permits(), 4);
        }

        /// The standing test of the timeout. The host receives the request and never answers; a fetcher built
        /// with a 1000 ms limit gives up by itself, as `Unreachable`, and frees its permit. Catches the timeout
        /// missing from the client: the fetch would then never return, and the ten-second guard (reached only in
        /// that case) fails the test. Both factories build through `builder(timeout)`, and the production value
        /// is pinned below.
        #[tokio::test]
        async fn a_fetch_that_gets_no_answer_ends_at_the_fetchers_timeout() {
            let host = FakeClientHost::start().await;
            host.answer("/never.json", FakeAnswer::Never);
            let fetcher = HttpClientFetcher::loopback(Duration::from_millis(1000)).unwrap();
            let id = host.client_id("/never.json");
            let outcome = tokio::time::timeout(Duration::from_secs(10), fetcher.fetch(&id))
                .await
                .expect("the fetcher gives up by itself");
            assert!(
                matches!(outcome, Err(ClientRefusal::Unreachable(_))),
                "{outcome:?}"
            );
            assert_eq!(host.hits("/never.json"), 1);
            assert_eq!(fetcher.permits.available_permits(), 4);
            assert_eq!(FETCH_TIMEOUT, Duration::from_secs(5));
        }
    }
}
