//! MCP clients (kb/ADR-0029 R3). A client is the software that connects (claude.ai). Its id is the https address
//! of a metadata document it publishes (OAuth Client ID Metadata Documents, draft 03). This file holds what such a
//! document must say, the port that fetches one, and the one function that decides whether a client may connect.
use async_trait::async_trait;
use serde_json::Value;

/// The longest `client_name` accepted: it is shown to a person and stored with a connection.
const MAX_CLIENT_NAME_CHARS: usize = 200;

/// Why a client is refused. The text is for the operator's log and never holds a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientRefusal {
    #[error("the client id is not acceptable: {0}")]
    BadClientId(&'static str),
    #[error("the client's host is not in [mcp] client_hosts")]
    HostNotListed,
    #[error("the redirect address is not in [mcp] redirect_uris")]
    RedirectNotEnabled,
    #[error("too many client documents are being fetched")]
    Busy,
    #[error("the client document could not be fetched: {0}")]
    Unreachable(String),
    #[error("the client document's address answered HTTP {0}, not 200")]
    Status(u16),
    #[error("the client document is larger than 64 KiB")]
    TooLarge,
    #[error("the client document is not acceptable: {0}")]
    BadDocument(&'static str),
    #[error("the redirect address is not one of the client's own")]
    RedirectNotRegistered,
}

impl ClientRefusal {
    /// Whether the instance has simply not enabled this app (its host or its redirect address is not listed),
    /// which an admin can change. Every other refusal means the request was not valid.
    pub fn not_enabled(&self) -> bool {
        matches!(self, Self::HostNotListed | Self::RedirectNotEnabled)
    }
}

/// A client's metadata document whose own rules hold. `parse` is its only constructor. It says nothing about how
/// the bytes were obtained: that is the fetcher's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientDocument {
    client_id: String,
    client_name: String,
    redirect_uris: Vec<String>,
}

impl ClientDocument {
    /// Reads the body published at `client_id`, or says with `BadDocument` which rule it breaks.
    pub fn parse(client_id: &str, body: &[u8]) -> Result<Self, ClientRefusal> {
        let bad = ClientRefusal::BadDocument;
        let value: Value = serde_json::from_slice(body).map_err(|_| bad("it is not JSON"))?;
        let Some(object) = value.as_object() else {
            return Err(bad("it is not a JSON object"));
        };
        // A string comparison, never a URL comparison (draft 03, section 3).
        if object.get("client_id").and_then(Value::as_str) != Some(client_id) {
            return Err(bad("its client_id is not the address it was fetched from"));
        }
        let Some(client_name) = object
            .get("client_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            return Err(bad("it has no client_name"));
        };
        if client_name.chars().count() > MAX_CLIENT_NAME_CHARS {
            return Err(bad("its client_name is longer than 200 characters"));
        }
        let redirect_uris: Option<Vec<String>> = object
            .get("redirect_uris")
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .map(|uri| uri.as_str().map(str::to_string))
                    .collect()
            });
        let Some(redirect_uris) = redirect_uris else {
            return Err(bad("it has no redirect_uris list of strings"));
        };
        if object.contains_key("client_secret") || object.contains_key("client_secret_expires_at") {
            return Err(bad("it carries a client secret"));
        }
        // Reeve's clients are public: the only method is `none`. Absent means `none` (draft 03, never RFC 7591's
        // default); the list form must include it.
        let method_is_none = object
            .get("token_endpoint_auth_method")
            .is_none_or(|method| method.as_str() == Some("none"));
        let methods_include_none = object
            .get("token_endpoint_auth_methods_supported")
            .is_none_or(|methods| {
                methods
                    .as_array()
                    .is_some_and(|list| list.iter().any(|method| method.as_str() == Some("none")))
            });
        if !method_is_none || !methods_include_none {
            return Err(bad("its token endpoint authentication is not none"));
        }
        Ok(ClientDocument {
            client_id: client_id.to_string(),
            client_name: client_name.to_string(),
            redirect_uris,
        })
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The name the client gives itself: a secondary label, shown as text.
    pub fn client_name(&self) -> &str {
        &self.client_name
    }

    pub fn redirect_uris(&self) -> &[String] {
        &self.redirect_uris
    }
}

/// Fetches the document a client id names. Defined here, beside `resolve_client`, its one consumer; the HTTP
/// adapter is in `client_fetch.rs`.
#[async_trait]
pub trait ClientDocumentFetcher: Send + Sync {
    /// The client id as a URL when it keeps this fetcher's rules for client ids. Nothing is sent.
    fn admit(&self, client_id: &str) -> Result<url::Url, ClientRefusal>;
    /// The validated document at `client_id`. Applies `admit` itself before anything is sent.
    async fn fetch(&self, client_id: &str) -> Result<ClientDocument, ClientRefusal>;
}

/// The instance's rules for clients, from `[mcp]`. Both lists are compared exactly; an empty list refuses every
/// client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientPolicy {
    pub client_hosts: Vec<String>,
    pub redirect_uris: Vec<String>,
}

/// A client that passed every rule, with the redirect address it may be sent back to. Only `resolve_client`
/// makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedClient {
    client_id: String,
    client_host: String,
    client_name: String,
    redirect_uri: String,
}

impl ValidatedClient {
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The host of the client id: what a person is shown as the app's identity.
    pub fn client_host(&self) -> &str {
        &self.client_host
    }

    /// The client's own name for itself: a secondary label, shown as text, at most 200 characters.
    pub fn client_name(&self) -> &str {
        &self.client_name
    }

    /// The redirect address of this request: listed by the instance and by the client.
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }
}

/// The client `client_id` names, allowed to be sent back to `redirect_uri`, or why not
/// (`ClientRefusal::not_enabled` tells the two kinds of refusal apart). Each refusal is logged here, once.
pub async fn resolve_client(
    fetcher: &dyn ClientDocumentFetcher,
    policy: &ClientPolicy,
    client_id: &str,
    redirect_uri: &str,
) -> Result<ValidatedClient, ClientRefusal> {
    let outcome = resolve(fetcher, policy, client_id, redirect_uri).await;
    if let Err(refusal) = &outcome {
        // The one line per refusal: the host and the reason, never the whole request.
        tracing::warn!(client_host = ?host_of(client_id), reason = %refusal, "mcp client refused");
    }
    outcome
}

fn host_of(client_id: &str) -> Option<String> {
    url::Url::parse(client_id)
        .ok()?
        .host_str()
        .map(str::to_string)
}

async fn resolve(
    fetcher: &dyn ClientDocumentFetcher,
    policy: &ClientPolicy,
    client_id: &str,
    redirect_uri: &str,
) -> Result<ValidatedClient, ClientRefusal> {
    // 1. The client id keeps the fetcher's rules.
    let url = fetcher.admit(client_id)?;
    // 2. Its host is one the instance lists.
    let client_host = url.host_str().unwrap_or_default().to_string();
    if !policy.client_hosts.contains(&client_host) {
        return Err(ClientRefusal::HostNotListed);
    }
    // 3. The redirect address is one the instance lists, exactly. Until here nothing was sent.
    if !policy
        .redirect_uris
        .iter()
        .any(|listed| listed == redirect_uri)
    {
        return Err(ClientRefusal::RedirectNotEnabled);
    }
    // 4. The document.
    let document = fetcher.fetch(client_id).await?;
    // 5. The redirect address is one of the client's own, exactly.
    if !document
        .redirect_uris()
        .iter()
        .any(|own| own == redirect_uri)
    {
        return Err(ClientRefusal::RedirectNotRegistered);
    }
    Ok(ValidatedClient {
        client_id: document.client_id().to_string(),
        client_host,
        client_name: document.client_name().to_string(),
        redirect_uri: redirect_uri.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ID: &str = "https://claude.ai/oauth/client.json";
    const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";
    const OWN_ONLY: &str = "http://localhost/callback";

    /// A document shaped like the one Claude Code publishes, changed by `edit`.
    fn body(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
        let mut value = json!({
            "client_id": ID,
            "client_name": "Claude",
            "client_uri": "https://claude.ai",
            "logo_uri": "https://claude.ai/logo.png",
            "redirect_uris": [CALLBACK, OWN_ONLY],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        });
        edit(&mut value);
        value.to_string().into_bytes()
    }

    fn remove(value: &mut Value, key: &str) {
        value.as_object_mut().unwrap().remove(key);
    }

    fn refused(edit: impl FnOnce(&mut Value)) -> ClientRefusal {
        ClientDocument::parse(ID, &body(edit)).unwrap_err()
    }

    /// Catches a parser that drops or reorders the redirect addresses, keeps the name's padding, refuses a name
    /// of exactly 200 characters, or needs a field the rules do not name.
    #[test]
    fn a_document_that_keeps_every_rule_is_read() {
        let document =
            ClientDocument::parse(ID, &body(|v| v["client_name"] = json!("  Claude  "))).unwrap();
        assert_eq!(document.client_id(), ID);
        assert_eq!(document.client_name(), "Claude");
        assert_eq!(document.redirect_uris(), [CALLBACK, OWN_ONLY]);
        let longest = "é".repeat(200);
        let document =
            ClientDocument::parse(ID, &body(|v| v["client_name"] = json!(longest))).unwrap();
        assert_eq!(document.client_name().chars().count(), 200);
        // Absent means none; the list form may name more than none.
        assert!(
            ClientDocument::parse(ID, &body(|v| remove(v, "token_endpoint_auth_method"))).is_ok()
        );
        assert!(
            ClientDocument::parse(
                ID,
                &body(|v| {
                    remove(v, "token_endpoint_auth_method");
                    v["token_endpoint_auth_methods_supported"] = json!(["private_key_jwt", "none"]);
                })
            )
            .is_ok()
        );
    }

    /// One case per rule; each names the rule that failed. Catches a rule that is not checked at all and two
    /// rules reported as one another.
    #[test]
    fn a_document_is_refused_for_each_rule_it_breaks() {
        let bad = ClientRefusal::BadDocument;
        assert_eq!(
            ClientDocument::parse(ID, b"<html>").unwrap_err(),
            bad("it is not JSON")
        );
        assert_eq!(
            ClientDocument::parse(ID, b"[]").unwrap_err(),
            bad("it is not a JSON object")
        );
        let differs = bad("its client_id is not the address it was fetched from");
        assert_eq!(
            refused(|v| v["client_id"] = json!("https://claude.ai/oauth/other.json")),
            differs
        );
        // Compared as strings, never as URLs.
        assert_eq!(
            refused(|v| v["client_id"] = json!("https://CLAUDE.ai/oauth/client.json")),
            differs
        );
        assert_eq!(refused(|v| remove(v, "client_id")), differs);
        let no_name = bad("it has no client_name");
        assert_eq!(refused(|v| remove(v, "client_name")), no_name);
        assert_eq!(refused(|v| v["client_name"] = json!("   ")), no_name);
        assert_eq!(refused(|v| v["client_name"] = json!(7)), no_name);
        assert_eq!(
            refused(|v| v["client_name"] = json!("x".repeat(201))),
            bad("its client_name is longer than 200 characters")
        );
        let no_list = bad("it has no redirect_uris list of strings");
        assert_eq!(refused(|v| remove(v, "redirect_uris")), no_list);
        assert_eq!(
            refused(|v| v["redirect_uris"] = json!([CALLBACK, 7])),
            no_list
        );
        let secret = bad("it carries a client secret");
        assert_eq!(refused(|v| v["client_secret"] = json!("s3cr3t")), secret);
        assert_eq!(
            refused(|v| v["client_secret_expires_at"] = json!(0)),
            secret
        );
        let not_none = bad("its token endpoint authentication is not none");
        assert_eq!(
            refused(|v| v["token_endpoint_auth_method"] = json!("client_secret_basic")),
            not_none
        );
        assert_eq!(
            refused(|v| v["token_endpoint_auth_methods_supported"] = json!(["private_key_jwt"])),
            not_none
        );
    }

    /// A fetcher that admits any URL with a host, answers with the document above, and counts its fetches.
    struct Counting {
        fetches: AtomicUsize,
    }

    #[async_trait]
    impl ClientDocumentFetcher for Counting {
        fn admit(&self, client_id: &str) -> Result<url::Url, ClientRefusal> {
            url::Url::parse(client_id).map_err(|_| ClientRefusal::BadClientId("it is not a URL"))
        }

        async fn fetch(&self, client_id: &str) -> Result<ClientDocument, ClientRefusal> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            ClientDocument::parse(client_id, &body(|_| {}))
        }
    }

    fn policy(client_hosts: &[&str], redirect_uris: &[&str]) -> ClientPolicy {
        ClientPolicy {
            client_hosts: client_hosts.iter().map(|h| h.to_string()).collect(),
            redirect_uris: redirect_uris.iter().map(|r| r.to_string()).collect(),
        }
    }

    /// The order, read from what was fetched: a client id the fetcher does not admit, a host that is not listed
    /// and a redirect address the instance has not enabled are each refused with no fetch at all; only then is
    /// the document fetched and the client's own list judged. Catches a fetch before the instance's two lists
    /// (an outbound request an anonymous caller could cause for any listed host and any address), a loose host
    /// match, and a client let through on one list alone.
    #[tokio::test]
    async fn nothing_is_fetched_until_the_instance_has_enabled_the_host_and_the_address() {
        let fetcher = Counting {
            fetches: AtomicUsize::new(0),
        };
        let fetches = || fetcher.fetches.load(Ordering::SeqCst);
        let enabled = policy(&["claude.ai"], &[CALLBACK, "https://evil.example/cb"]);

        let refused = resolve_client(&fetcher, &enabled, "not a url", CALLBACK).await;
        assert_eq!(refused, Err(ClientRefusal::BadClientId("it is not a URL")));
        let lists: [&[&str]; 4] = [
            &[],
            &["chatgpt.com"],
            &["www.claude.ai"],
            &["claude.ai.evil.example"],
        ];
        for hosts in lists {
            let refused = resolve_client(&fetcher, &policy(hosts, &[CALLBACK]), ID, CALLBACK).await;
            assert_eq!(refused, Err(ClientRefusal::HostNotListed), "{hosts:?}");
        }
        // The client lists it, the instance does not; and a longer address is another address.
        for asked in [OWN_ONLY, "https://claude.ai/api/mcp/auth_callback/x"] {
            let refused = resolve_client(&fetcher, &enabled, ID, asked).await;
            assert_eq!(refused, Err(ClientRefusal::RedirectNotEnabled), "{asked}");
        }
        assert_eq!(fetches(), 0, "nothing was fetched for any refusal above");

        // The instance lists it, the client does not: known only after the fetch.
        let refused = resolve_client(&fetcher, &enabled, ID, "https://evil.example/cb").await;
        assert_eq!(refused, Err(ClientRefusal::RedirectNotRegistered));
        assert_eq!(fetches(), 1);

        let client = resolve_client(&fetcher, &enabled, ID, CALLBACK)
            .await
            .unwrap();
        assert_eq!(
            (
                client.client_id(),
                client.client_host(),
                client.client_name(),
                client.redirect_uri()
            ),
            (ID, "claude.ai", "Claude", CALLBACK)
        );
        assert_eq!(fetches(), 2, "no cache: each resolve fetches");
    }

    /// Catches a page message that drifts: exactly the two refusals an admin can lift are "not enabled".
    #[test]
    fn only_an_unlisted_host_and_an_unlisted_redirect_address_are_not_enabled() {
        assert!(ClientRefusal::HostNotListed.not_enabled());
        assert!(ClientRefusal::RedirectNotEnabled.not_enabled());
        for other in [
            ClientRefusal::BadClientId("x"),
            ClientRefusal::Busy,
            ClientRefusal::Unreachable("x".into()),
            ClientRefusal::Status(404),
            ClientRefusal::TooLarge,
            ClientRefusal::BadDocument("x"),
            ClientRefusal::RedirectNotRegistered,
        ] {
            assert!(!other.not_enabled(), "{other}");
        }
    }
}
