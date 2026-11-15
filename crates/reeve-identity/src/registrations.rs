//! MCP clients that register themselves (kb/ADR-0029 R3; RFC 7591, OAuth 2.0 Dynamic Client Registration): an app
//! that publishes no metadata document asks for a client id. This file holds what a registration request must
//! say, how long a registration is kept and how many, the port that stores them, and the one function that makes
//! one. Everything a registered client says of itself is self-asserted: only its redirect addresses are checked,
//! against the instance's list. Nothing here reads the clock (kb/ADR-0010 R4).
use crate::{
    clients::{ClientPolicy, MAX_CLIENT_NAME_CHARS, host_of},
    connections::CONNECTION_DAYS,
    domain::random_secret,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value};
use sqlx::{PgPool, Postgres, Transaction};

/// What every registered client id starts with; 24 random bytes in hex follow. It has no `:`, so such an id is
/// never a URL and is never mistaken for the address of a metadata document.
pub(crate) const REGISTERED_PREFIX: &str = "dyn_";
/// At most this many registrations are stored (kb/ADR-0029 R3: "Stored registrations MUST be capped").
pub const MAX_REGISTERED_CLIENTS: i64 = 1000;
/// A registration nobody approved a connection for expires this long after it was made (R3: "unused ones MUST
/// expire").
const UNUSED_HOURS: i64 = 24;
/// A used registration expires this long after its last approval.
const USED_DAYS: i64 = 90;
/// A connection ends `CONNECTION_DAYS` after its approval and is listed for as long again, so a used registration
/// outlives every listed connection of it.
const _: () = assert!(USED_DAYS > 2 * CONNECTION_DAYS);
const MAX_REDIRECT_URIS: usize = 5;
/// The longest path of a refused redirect address that is logged.
const MAX_LOGGED_PATH_CHARS: usize = 200;
/// The longest host of a refused redirect address that is logged: a DNS name's longest.
const MAX_LOGGED_HOST_CHARS: usize = 253;

/// How long registrations are kept, as the two instants `now` puts the bounds at: a never-used registration made
/// at or before the first has expired, and so has a used one last used at or before the second. The one place
/// the two periods become instants: `RegisteredClient::expired` and the store's delete both ask here.
pub(crate) fn retention(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    (
        now - Duration::hours(UNUSED_HOURS),
        now - Duration::days(USED_DAYS),
    )
}

/// Whether `client_id` has the shape of an id this server made: the prefix and 48 lower-case hex digits. Such
/// text has no `:`, so it is no URL; and no URL has this shape.
pub(crate) fn is_registered_id(client_id: &str) -> bool {
    client_id
        .strip_prefix(REGISTERED_PREFIX)
        .is_some_and(|hex| {
            hex.len() == 48 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}

/// A stored registration.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub created_at: DateTime<Utc>,
    /// When a person last approved a connection for it; `None` until one has.
    pub last_used_at: Option<DateTime<Utc>>,
}

impl RegisteredClient {
    /// Whether it may no longer start a connect. The same bounds as the store's delete (`retention`): a row this
    /// calls expired is one the next registration deletes.
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        let (unused_before, used_before) = retention(now);
        match self.last_used_at {
            None => self.created_at <= unused_before,
            Some(used) => used <= used_before,
        }
    }
}

/// The stored registrations. Defined here, beside its consumers; the Postgres adapter is in
/// `registrations_repo.rs`.
#[async_trait]
pub trait RegisteredClients: Send + Sync {
    /// The registration with this id, expired or not: the caller judges with `RegisteredClient::expired`.
    async fn find(&self, client_id: &str) -> Result<Option<RegisteredClient>, sqlx::Error>;
    /// Stores `client`, in the caller's transaction, one registration at a time: deletes the registrations that
    /// have expired at `client.created_at` (`retention`); then, when `cap` rows remain, deletes the oldest
    /// never-used one that no approval is marking at that moment. `false`, with nothing stored, when there is
    /// none to delete (the expired ones stay deleted if the caller commits).
    async fn insert_capped(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        client: &RegisteredClient,
        cap: i64,
    ) -> Result<bool, sqlx::Error>;
    /// Records an approval at `now`, in the caller's transaction; the time of the last use never moves back.
    /// `false` when there is no such registration (it was dropped or deleted): nothing is written, and the row is
    /// never made again here.
    async fn mark_used(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error>;
}

/// Why a registration was refused. The text is for the operator's log and never holds what the caller sent,
/// except the host and the path of a redirect address the instance does not list.
#[derive(Debug, thiserror::Error)]
pub enum RegistrationRefusal {
    /// RFC 7591's `invalid_redirect_uri`: the first address that is not in `[mcp] redirect_uris`.
    #[error("a redirect address is not in [mcp] redirect_uris")]
    InvalidRedirectUri {
        host: Option<String>,
        path: Option<String>,
    },
    /// RFC 7591's `invalid_client_metadata`.
    #[error("the client metadata is not acceptable: {0}")]
    InvalidMetadata(&'static str),
    /// Every stored registration has been used (or is being approved at that moment), so none can be dropped
    /// to make room. While a never-used one remains, a registration is never refused for room.
    #[error("every stored registration has been used")]
    Full,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// The member `key` as a list of strings: `Ok(None)` when it is absent, `Err` when it is anything else.
fn strings<'a>(object: &'a Map<String, Value>, key: &str) -> Result<Option<Vec<&'a str>>, ()> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    value
        .as_array()
        .and_then(|list| list.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
        .map(Some)
        .ok_or(())
}

/// The host and the path of an address, for the log, each cut to a bound: never its query, its user information or its fragment.
fn host_and_path(address: &str) -> (Option<String>, Option<String>) {
    match url::Url::parse(address) {
        Ok(url) => (
            url.host_str()
                .map(|host| host.chars().take(MAX_LOGGED_HOST_CHARS).collect()),
            Some(url.path().chars().take(MAX_LOGGED_PATH_CHARS).collect()),
        ),
        Err(_) => (None, None),
    }
}

/// The name (trimmed) and the redirect addresses (as sent) of a registration request whose rules hold. The
/// redirect addresses are judged last, so `InvalidRedirectUri` means everything else was acceptable.
///
/// What an app asks for beyond these two is not judged (RFC 7591 section 2 lets a server replace requested
/// values, and the answer states what was registered): `grant_types`, `response_types` and
/// `token_endpoint_auth_method` are fixed by this server whatever was sent, and no secret is ever issued.
fn parse(policy: &ClientPolicy, body: &[u8]) -> Result<(String, Vec<String>), RegistrationRefusal> {
    let bad = RegistrationRefusal::InvalidMetadata;
    let value: Value = serde_json::from_slice(body).map_err(|_| bad("the body is not JSON"))?;
    let Some(object) = value.as_object() else {
        return Err(bad("the body is not a JSON object"));
    };
    // The same bounds as a metadata document's name (`ClientDocument::parse`): a person reads it. Absent, null
    // or blank, it gets a default below.
    let name = match object.get("client_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.trim()).filter(|name| !name.is_empty()),
        Some(_) => return Err(bad("client_name is not a string")),
    };
    if name.is_some_and(|name| name.chars().count() > MAX_CLIENT_NAME_CHARS) {
        return Err(bad("client_name is longer than 200 characters"));
    }
    if name.is_some_and(|name| name.chars().any(char::is_control)) {
        return Err(bad("client_name has a control character"));
    }
    let uris = strings(object, "redirect_uris")
        .ok()
        .flatten()
        .filter(|uris| (1..=MAX_REDIRECT_URIS).contains(&uris.len()))
        .ok_or(bad("redirect_uris is not a list of one to five strings"))?;
    // kb/ADR-0029 R3: exactly an entry of the instance's list, compared as text.
    if let Some(unlisted) = uris
        .iter()
        .find(|uri| !policy.redirect_uris.iter().any(|listed| listed == *uri))
    {
        let (host, path) = host_and_path(unlisted);
        return Err(RegistrationRefusal::InvalidRedirectUri { host, path });
    }
    // An app that gives itself no name is called by the host of its first redirect address, which the instance
    // lists (and `[mcp] redirect_uris` holds addresses with a host only).
    let name = match name {
        Some(name) => name.to_string(),
        None => host_of(uris[0])
            .map(|host| host.chars().take(MAX_CLIENT_NAME_CHARS).collect())
            .ok_or(bad("client_name is missing"))?,
    };
    Ok((name, uris.into_iter().map(str::to_string).collect()))
}

/// Registers the client `body` describes (RFC 7591 section 3.1), or says why not, in one transaction of its own.
/// Each outcome is logged here, once; a refusal's line never holds the body, the name, or more of an address
/// than its host and path.
pub async fn register_client(
    pool: &PgPool,
    store: &dyn RegisteredClients,
    policy: &ClientPolicy,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<RegisteredClient, RegistrationRefusal> {
    let outcome = register(pool, store, policy, body, now).await;
    match &outcome {
        Ok(client) => tracing::info!(client_id = %client.client_id, "mcp client registered"),
        // The line an operator reads to learn which address an app wants listed.
        Err(RegistrationRefusal::InvalidRedirectUri { host, path }) => tracing::warn!(
            redirect_host = ?host,
            redirect_path = ?path,
            "mcp client registration refused: a redirect address is not in [mcp] redirect_uris"
        ),
        Err(RegistrationRefusal::Db(e)) => {
            tracing::error!(error = ?e, "mcp client registration failed")
        }
        Err(refusal) => tracing::warn!(reason = %refusal, "mcp client registration refused"),
    }
    outcome
}

async fn register(
    pool: &PgPool,
    store: &dyn RegisteredClients,
    policy: &ClientPolicy,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<RegisteredClient, RegistrationRefusal> {
    let (client_name, redirect_uris) = parse(policy, body)?;
    let client = RegisteredClient {
        client_id: random_secret(REGISTERED_PREFIX),
        client_name,
        redirect_uris,
        created_at: now,
        last_used_at: None,
    };
    let mut tx = pool.begin().await?;
    let stored = store
        .insert_capped(&mut tx, &client, MAX_REGISTERED_CLIENTS)
        .await?;
    // Committed either way: the expired registrations stay deleted when the store is full.
    tx.commit().await?;
    if stored {
        Ok(client)
    } else {
        Err(RegistrationRefusal::Full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CALLBACK: &str = "https://client.example/callback";
    const OTHER: &str = "https://other.example/cb";

    fn request() -> Value {
        json!({
            "client_name": "Example Assistant",
            "redirect_uris": [CALLBACK],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        })
    }

    fn policy() -> ClientPolicy {
        ClientPolicy {
            client_hosts: vec![],
            redirect_uris: vec![CALLBACK.to_string(), OTHER.to_string()],
        }
    }

    /// A request as a hosted app sends one, changed by `edit`, judged against an instance that lists both
    /// addresses.
    fn parsed(edit: impl FnOnce(&mut Value)) -> Result<(String, Vec<String>), RegistrationRefusal> {
        let mut value = request();
        edit(&mut value);
        parse(&policy(), value.to_string().as_bytes())
    }

    fn remove(value: &mut Value, key: &str) {
        value.as_object_mut().unwrap().remove(key);
    }

    fn pair(name: &str, uris: &[&str]) -> (String, Vec<String>) {
        (
            name.to_string(),
            uris.iter().map(|uri| uri.to_string()).collect(),
        )
    }

    /// The text of an `InvalidMetadata`; panics on anything else.
    fn metadata(outcome: Result<(String, Vec<String>), RegistrationRefusal>) -> &'static str {
        match outcome {
            Err(RegistrationRefusal::InvalidMetadata(why)) => why,
            other => panic!("not InvalidMetadata: {other:?}"),
        }
    }

    /// The host and path of an `InvalidRedirectUri`; panics on anything else.
    fn redirect(
        outcome: Result<(String, Vec<String>), RegistrationRefusal>,
    ) -> (Option<String>, Option<String>) {
        match outcome {
            Err(RegistrationRefusal::InvalidRedirectUri { host, path }) => (host, path),
            other => panic!("not InvalidRedirectUri: {other:?}"),
        }
    }

    /// One row per rule. Catches a name whose padding is kept or whose bound is off by one, addresses that are
    /// reordered, anything of the request beyond the name and the addresses reaching the result (a secret, a
    /// software statement), a request refused for what it merely asks for (an authentication method, grant or
    /// response types: the answer states what this server registers), a missing name that is refused instead of
    /// defaulted, an address compared as a URL, not as text, and a refusal that would log a query, a password or
    /// an unbounded path.
    #[test]
    fn a_registration_request_is_read_or_refused_for_the_rule_it_breaks() {
        let whole = pair("Example Assistant", &[CALLBACK]);
        assert_eq!(parsed(|_| {}).unwrap(), whole);
        assert_eq!(
            parsed(|v| v["client_name"] = json!("  Padded  ")).unwrap(),
            pair("Padded", &[CALLBACK])
        );
        let longest = "é".repeat(200);
        assert_eq!(
            parsed(|v| v["client_name"] = json!(longest)).unwrap().0,
            longest
        );
        assert_eq!(
            parsed(|v| v["redirect_uris"] = json!([OTHER, CALLBACK])).unwrap(),
            pair("Example Assistant", &[OTHER, CALLBACK])
        );
        assert_eq!(
            parsed(|v| {
                remove(v, "token_endpoint_auth_method");
                remove(v, "grant_types");
                remove(v, "response_types");
            })
            .unwrap(),
            whole
        );
        // Nothing else of the body is read, so nothing else can be stored or answered.
        assert_eq!(
            parsed(|v| {
                v["client_secret"] = json!("s3cr3t");
                v["scope"] = json!("x");
                v["software_statement"] = json!("x");
                v["unknown"] = json!({});
            })
            .unwrap(),
            whole
        );
        // What an app asks for is replaced, not refused: this server registers public clients with its own
        // grant and response types whatever was sent.
        for (key, asked) in [
            ("token_endpoint_auth_method", json!("client_secret_basic")),
            ("token_endpoint_auth_method", json!("client_secret_post")),
            ("token_endpoint_auth_method", json!(null)),
            ("grant_types", json!(["refresh_token", "implicit"])),
            ("grant_types", json!("authorization_code")),
            ("grant_types", json!([])),
            ("response_types", json!(["token"])),
            ("response_types", json!(7)),
        ] {
            assert_eq!(
                parsed(|v| v[key] = asked.clone()).unwrap(),
                whole,
                "{key} = {asked}"
            );
        }
        // A name that is absent or blank is the host of the first redirect address.
        assert_eq!(
            parsed(|v| remove(v, "client_name")).unwrap(),
            pair("client.example", &[CALLBACK])
        );
        assert_eq!(
            parsed(|v| v["client_name"] = json!("   ")).unwrap(),
            pair("client.example", &[CALLBACK])
        );
        assert_eq!(
            parsed(|v| v["client_name"] = json!(null)).unwrap(),
            pair("client.example", &[CALLBACK])
        );
        assert_eq!(
            parsed(|v| {
                remove(v, "client_name");
                v["redirect_uris"] = json!([OTHER, CALLBACK]);
            })
            .unwrap(),
            pair("other.example", &[OTHER, CALLBACK])
        );

        assert_eq!(
            metadata(parsed(|v| v["client_name"] = json!(7))),
            "client_name is not a string"
        );
        assert_eq!(
            metadata(parsed(|v| v["client_name"] = json!("x".repeat(201)))),
            "client_name is longer than 200 characters"
        );
        assert_eq!(
            metadata(parsed(|v| v["client_name"] = json!("Exam\u{7}ple"))),
            "client_name has a control character"
        );
        let no_list = "redirect_uris is not a list of one to five strings";
        assert_eq!(metadata(parsed(|v| remove(v, "redirect_uris"))), no_list);
        for bad in [
            json!([]),
            json!(CALLBACK),
            json!([CALLBACK, 7]),
            json!(vec![CALLBACK; 6]),
        ] {
            assert_eq!(
                metadata(parsed(|v| v["redirect_uris"] = bad.clone())),
                no_list,
                "{bad}"
            );
        }
        assert!(parsed(|v| v["redirect_uris"] = json!(vec![CALLBACK; 5])).is_ok());
        assert_eq!(
            metadata(parse(&policy(), b"<html>")),
            "the body is not JSON"
        );
        assert_eq!(
            metadata(parse(&policy(), b"[]")),
            "the body is not a JSON object"
        );

        // The first address the instance does not list, by its host and path only.
        assert_eq!(
            redirect(parsed(|v| v["redirect_uris"] =
                json!([CALLBACK, "https://wanted.example/cb/x?k=SENTINEL#f"]))),
            (Some("wanted.example".into()), Some("/cb/x".into()))
        );
        // Compared as text, not as URLs.
        for near in [
            "https://client.example/callback/",
            "https://client.example/callback?x=1",
            "HTTPS://client.example/callback",
            "https://CLIENT.example/callback",
            "https://client.example:443/callback",
            "https://client.example/call%62ack",
        ] {
            assert_eq!(
                redirect(parsed(|v| v["redirect_uris"] = json!([near]))).0,
                Some("client.example".into()),
                "{near}"
            );
        }
        assert_eq!(
            redirect(parsed(|v| v["redirect_uris"] = json!(["not an address"]))),
            (None, None)
        );
        let (host, path) = redirect(parsed(|v| {
            v["redirect_uris"] = json!([format!("https://u:pw@wanted.example/{}", "a".repeat(300))])
        }));
        assert_eq!(host.as_deref(), Some("wanted.example"));
        assert_eq!(path.unwrap().chars().count(), 200);
        let (host, _) = redirect(parsed(|v| {
            v["redirect_uris"] = json!([format!("https://{}example/cb", "abcdefghi.".repeat(30))])
        }));
        assert_eq!(host.unwrap().chars().count(), 253, "the host is cut too");
        // The addresses are judged last: a request with a bad name and an unlisted address is refused for the
        // name.
        assert_eq!(
            metadata(parsed(|v| {
                v["client_name"] = json!(7);
                v["redirect_uris"] = json!(["https://wanted.example/cb"]);
            })),
            "client_name is not a string"
        );
    }

    /// Catches a shape test that is loose (a registered id could then be a URL, or a URL reach the store) and
    /// an id of this server's making that does not pass it.
    #[test]
    fn an_id_of_this_servers_making_is_never_a_url() {
        const DYN: &str = "dyn_0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(is_registered_id(DYN));
        assert!(url::Url::parse(DYN).is_err());
        for other in [
            format!("dyn_{}", "0".repeat(47)),
            format!("dyn_{}", "ABCDEF".repeat(8)),
            format!("{DYN}0"),
            "dyn_x".to_string(),
            String::new(),
            format!("DYN_{}", "0".repeat(48)),
            format!(" {DYN}"),
            "https://claude.ai/oauth/client.json".to_string(),
            format!("https://claude.ai/{DYN}"),
        ] {
            assert!(!is_registered_id(&other), "{other:?}");
        }
        for _ in 0..8 {
            assert!(is_registered_id(&random_secret(REGISTERED_PREFIX)));
        }
    }
}
