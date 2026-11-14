//! The public OAuth surface of the MCP endpoint (kb/ADR-0029 R2, R11): the routes that answer without a caller.
//! They are always mounted, outside the API router, so they are not in the OpenAPI document and never answer in
//! the API's envelope. The `[mcp]` switch is inside each handler: off, each answers 404. `[mcp] enabled` loads only
//! together with `[oidc]` (`ServerConfig::validate`), so an instance without sign-in answers 404 here as well.
//!
//! This file holds the two metadata documents and what the OAuth routes share. `/oauth/authorize` is in
//! `authorize.rs`; `/oauth/token` is in `token.rs`.
use crate::{config::ServerConfig, state::AppState};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};

/// RFC 9728 section 3.1: the well-known part goes between the host and the resource's path (`/mcp`).
pub(crate) const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
/// RFC 8414 section 3: the issuer has no path, so nothing follows the well-known part.
pub(crate) const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";
pub(crate) const AUTHORIZE_PATH: &str = "/oauth/authorize";
pub(crate) const TOKEN_PATH: &str = "/oauth/token";
/// The two scopes on the wire (kb/ADR-0029 R7). A refresh token is always issued; `offline_access` is accepted at
/// authorize and ignored, so it is not advertised.
pub(crate) const SCOPES: [&str; 2] = ["read", "write"];

/// The resource an MCP token is issued for: the endpoint's address, `<issuer>/mcp`. Clients send it as the
/// `resource` parameter and compare it with the document as a string.
pub(crate) fn resource(config: &ServerConfig) -> String {
    format!("{}/mcp", config.issuer())
}

/// The address of the protected resource's document: what the 401 from `/mcp` names (RFC 9728 section 5.1).
pub(crate) fn resource_metadata_url(config: &ServerConfig) -> String {
    format!("{}{PROTECTED_RESOURCE_PATH}", config.issuer())
}

/// The parameters of an OAuth request, from a query or a form body, decoded strictly.
pub(crate) struct Params(Vec<(String, String)>);

/// A percent escape that is not two hex digits, or a name or value that is not UTF-8 once decoded.
#[derive(Debug)]
pub(crate) struct Malformed;

/// The value of one hex digit; `None` for any other byte.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// `application/x-www-form-urlencoded` decoding of one name or value: `+` is a space, `%XX` a byte.
fn decode(raw: &str) -> Result<String, Malformed> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let digit = |at: usize| bytes.get(at).copied().and_then(hex_digit).ok_or(Malformed);
                out.push(digit(i + 1)? * 16 + digit(i + 2)?);
                i += 2;
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8(out).map_err(|_| Malformed)
}

impl Params {
    /// Every `name=value` pair of `text`, in the order sent. Fails as a whole on anything malformed: nothing of
    /// such a request is trusted.
    pub(crate) fn parse(text: &str) -> Result<Self, Malformed> {
        text.split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                Ok((decode(name)?, decode(value)?))
            })
            .collect::<Result<Vec<_>, Malformed>>()
            .map(Self)
    }

    /// The first of `recognised` that was sent more than once (RFC 6749 section 3.1: "MUST NOT be included more
    /// than once"). A route asks this for every parameter it knows before it reads any value. A parameter it
    /// does not know may repeat: it is ignored.
    pub(crate) fn repeated(&self, recognised: &[&'static str]) -> Option<&'static str> {
        recognised
            .iter()
            .copied()
            .find(|name| self.0.iter().filter(|(k, _)| k == name).count() > 1)
    }

    /// The value of `name`; `None` when it is absent or sent without a value (RFC 6749 section 3.1:
    /// "Parameters sent without a value MUST be treated as if they were omitted").
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }
}

/// An authorization response: the client's validated redirect address with `answer`, then `state` when the
/// client sent one, then `iss` (RFC 9207 section 2: on every authorization response, errors included). A query
/// the address already has is kept (RFC 6749 section 3.1).
pub(crate) fn answer_address(
    redirect_uri: &str,
    answer: &[(&str, &str)],
    state: Option<&str>,
    issuer: &str,
) -> String {
    let mut url = url::Url::parse(redirect_uri)
        .expect("a validated redirect address is a URL ([mcp] redirect_uris is validated at load)");
    {
        let mut query = url.query_pairs_mut();
        for (name, value) in answer {
            query.append_pair(name, value);
        }
        if let Some(state) = state {
            query.append_pair("state", state);
        }
        query.append_pair("iss", issuer);
    }
    url.into()
}

/// Marks an answer of the sign-in routes as one no cache may store (RFC 6749 section 5.1 for tokens; the same
/// for a code, an approval value and the redirects that carry them).
pub(crate) fn no_store(mut res: Response) -> Response {
    let headers = res.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    res
}

fn protected_resource(config: &ServerConfig) -> Value {
    json!({
        "resource": resource(config),
        "authorization_servers": [config.issuer()],
        "scopes_supported": SCOPES,
        "bearer_methods_supported": ["header"],
    })
}

fn authorization_server(config: &ServerConfig) -> Value {
    let issuer = config.issuer();
    json!({
        "issuer": issuer.as_str(),
        "authorization_endpoint": format!("{issuer}{AUTHORIZE_PATH}"),
        "token_endpoint": format!("{issuer}{TOKEN_PATH}"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
        "scopes_supported": SCOPES,
    })
}

/// One metadata document. The switch comes first, so a switched-off instance says nothing about itself whatever
/// the method; then a read answers and anything else is 405 in plain text.
fn document(s: &AppState, method: &Method, build: fn(&ServerConfig) -> Value) -> Response {
    if !s.config.mcp.enabled {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    if *method != Method::GET && *method != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
            "this address answers GET and HEAD only",
        )
            .into_response();
    }
    Json(build(&s.config)).into_response()
}

async fn protected_resource_metadata(State(s): State<AppState>, method: Method) -> Response {
    document(&s, &method, protected_resource)
}

async fn authorization_server_metadata(State(s): State<AppState>, method: Method) -> Response {
    document(&s, &method, authorization_server)
}

/// Merged into the application outside `api_router()`. `any` keeps every method away from the router's own 405,
/// which answers in the API's envelope.
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(PROTECTED_RESOURCE_PATH, any(protected_resource_metadata))
        .route(AUTHORIZE_PATH, any(super::authorize::authorize))
        .route(TOKEN_PATH, any(super::token::token))
        .route(
            AUTHORIZATION_SERVER_PATH,
            any(authorization_server_metadata),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a repeated parameter read as its first value, an empty one read as present, a value that is not
    /// decoded, and a malformed or non-UTF-8 escape accepted (the standard decoder replaces such bytes silently).
    #[test]
    fn parameters_are_decoded_strictly_and_counted() {
        let p = Params::parse("a=1&b=&a=2&c=x%20y+z&d&e=%C3%A9&f=1&f=").unwrap();
        assert_eq!(p.repeated(&["b", "a", "f"]), Some("a"));
        assert_eq!(
            p.repeated(&["f", "a"]),
            Some("f"),
            "an empty second value still repeats"
        );
        assert_eq!(p.repeated(&["b", "c", "missing"]), None);
        assert_eq!(
            (
                p.get("a"),
                p.get("b"),
                p.get("c"),
                p.get("d"),
                p.get("e"),
                p.get("missing")
            ),
            (Some("1"), None, Some("x y z"), None, Some("é"), None)
        );
        assert!(Params::parse("").unwrap().get("a").is_none());
        for bad in ["a=%", "a=%2", "a=%zz", "a=%+1", "a=%ff", "%ff=1", "a=%C3"] {
            assert!(Params::parse(bad).is_err(), "{bad}");
        }
    }

    /// Read back by parsing, not by comparing text. Catches an answer that drops the address's own query, one
    /// that does not encode a value (a `&` in `state` would add a parameter), and a missing `iss`.
    #[test]
    fn an_answer_keeps_the_address_and_adds_state_and_iss() {
        let to = answer_address(
            "https://client.example/cb?keep=1",
            &[("code", "a b&c=d")],
            Some("s&iss=https://evil.example"),
            "https://reeve.example.com",
        );
        let url = url::Url::parse(&to).unwrap();
        assert_eq!(
            (url.scheme(), url.host_str(), url.path()),
            ("https", Some("client.example"), "/cb")
        );
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let expected = [
            ("keep", "1"),
            ("code", "a b&c=d"),
            ("state", "s&iss=https://evil.example"),
            ("iss", "https://reeve.example.com"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        assert_eq!(pairs, expected);
        let bare = answer_address(
            "https://client.example/cb",
            &[("error", "access_denied")],
            None,
            "https://reeve.example.com",
        );
        let names: Vec<String> = url::Url::parse(&bare)
            .unwrap()
            .query_pairs()
            .map(|(k, _)| k.into_owned())
            .collect();
        assert_eq!(names, ["error", "iss"]);
    }
}
