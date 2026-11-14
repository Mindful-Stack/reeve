//! The MCP endpoint, `/mcp` (kb/ADR-0029 R1): Reeve's own handler for MCP's Streamable HTTP transport. It serves
//! protocol revisions 2026-07-28 and 2025-11-25 without a session and answers every request with one JSON object;
//! `Accept` is ignored. The handler takes the raw request and runs its checks itself, in the numbered order below:
//! an extractor argument would run before the switch and the origin check. A failure before the message is parsed
//! is an `McpError`, one after it an `RpcError`; neither is ever an `AppError` (which renders the API's envelope).
mod protocol;
mod tools;

use crate::{auth::McpCaller, state::AppState};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use protocol::{Headers, Message, Revision, RpcError};
use reeve_identity::AuthError;
use serde_json::{Value, json};

/// The largest request body read (step 5). A raw `Request` gets no default body limit, so this is the only one.
const MAX_BODY: usize = 1024 * 1024;

/// How long a client may keep `server/discover` and `tools/list` answers: neither changes while the process runs.
const TTL_MS: u64 = 300_000;

fn capabilities() -> Value {
    // No `listChanged`: no list ever changes at run time, and `subscriptions/listen` is not implemented.
    json!({"tools": {}})
}

fn server_info() -> Value {
    json!({"name": "reeve", "version": env!("CARGO_PKG_VERSION")})
}

/// A 2026-07-28 result that a client may cache: `resultType`, the server's identity and the caching hints.
/// `cacheScope` is `public` while every caller sees the same list; it becomes `private` once scopes filter it
/// (GRA-658).
fn modern(mut result: Value) -> Value {
    result["resultType"] = json!("complete");
    result["_meta"] = json!({"io.modelcontextprotocol/serverInfo": server_info()});
    result["ttlMs"] = json!(TTL_MS);
    result["cacheScope"] = json!("public");
    result
}

/// A failure at steps 1 to 5, before the message is parsed: nothing is known of it yet, so there is no id to
/// answer with and no telling a notification from a request.
pub(crate) enum McpError {
    /// Step 1.
    Disabled,
    /// Step 2.
    MethodNotAllowed,
    /// Step 3.
    OriginNotAllowed,
    /// Step 4.
    Unauthenticated,
    /// Step 5.
    UnsupportedMediaType,
    TooLarge,
    BadBody,
    /// Logged at `error`; answered -32603 with no detail.
    Internal(anyhow::Error),
}

impl IntoResponse for McpError {
    fn into_response(self) -> Response {
        match self {
            // Plain text, never a JSON-RPC body: a client must not take a switched-off endpoint for an MCP server.
            McpError::Disabled => (
                StatusCode::NOT_FOUND,
                "MCP is not enabled on this instance ([mcp] enabled)",
            )
                .into_response(),
            McpError::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                [(header::ALLOW, "POST")],
                "/mcp accepts POST only",
            )
                .into_response(),
            McpError::OriginNotAllowed => {
                (StatusCode::FORBIDDEN, "origin not allowed").into_response()
            }
            // GRA-655 adds `resource_metadata="<public_url>/.well-known/oauth-protected-resource"` to this header
            // (kb/ADR-0029 R2); Claude starts sign-in only on this 401.
            McpError::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                "unauthenticated",
            )
                .into_response(),
            McpError::UnsupportedMediaType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json",
            )
                .into_response(),
            McpError::TooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "the request body is larger than 1 MiB",
            )
                .into_response(),
            McpError::BadBody => (
                StatusCode::BAD_REQUEST,
                "the request body could not be read",
            )
                .into_response(),
            McpError::Internal(e) => {
                tracing::error!(error = ?e, "mcp: internal error");
                rpc_error(
                    &RpcError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        protocol::INTERNAL_ERROR,
                        "Internal error",
                    ),
                    None,
                )
            }
        }
    }
}

/// A JSON-RPC error response, with the request's id when one was read.
fn rpc_error(e: &RpcError, id: Option<&Value>) -> Response {
    (e.status, Json(e.body(id))).into_response()
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// A protocol header's value: `None` only when the header is absent. One sent twice, or one that is not text, is
/// refused (-32020): read as absent or as its first value, it could move the request onto the other revision's
/// path.
fn protocol_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, RpcError> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(protocol::mismatch(format!(
            "the {name} header must be sent once"
        )));
    }
    match value.to_str() {
        Ok(text) => Ok(Some(text)),
        Err(_) => Err(protocol::mismatch(format!("the {name} header is not text"))),
    }
}

/// The one line `serve` logs at startup about this endpoint.
pub fn mcp_startup_line(config: &crate::ServerConfig) -> String {
    if !config.mcp.enabled {
        return "mcp endpoint disabled".into();
    }
    format!(
        "mcp endpoint enabled at {}mcp (protocol {} and {}, {} tools)",
        config.public_url(),
        protocol::MODERN,
        protocol::LEGACY,
        tools::list().len()
    )
}

/// `ANY /mcp`. The first failing step answers and stops.
pub(crate) async fn handle(State(s): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let (_caller, bytes) = match admit(&s, &parts.method, &parts.headers, body).await {
        Ok(admitted) => admitted,
        Err(e) => return e.into_response(),
    };
    // 6. One JSON-RPC request or notification.
    let msg = match protocol::parse(&bytes) {
        Ok(msg) => msg,
        Err(e) => return rpc_error(&e, None),
    };
    // 7 to 10, or the legacy classification in their place.
    let revision = protocol_headers(&parts.headers).and_then(|h| protocol::classify(&msg, h));
    // Step 6 parsed the message, so a notification is known to be one, and from here on it never gets a JSON-RPC
    // response: 202 when it passed the checks, else the failing check's status alone. This endpoint acts on none.
    // (A failure at steps 1 to 6, above, is answered as for any request: the server cannot know yet what the
    // message is.)
    let Some(id) = &msg.id else {
        return match revision {
            Ok(_) => StatusCode::ACCEPTED,
            Err(e) => e.status,
        }
        .into_response();
    };
    match revision.and_then(|revision| answer(revision, &msg)) {
        // 12.
        Ok(result) => Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response(),
        Err(e) => rpc_error(&e, Some(id)),
    }
}

/// Steps 1 to 5: the switch, the method, the origin, the caller, then the body.
async fn admit(
    s: &AppState,
    method: &Method,
    headers: &HeaderMap,
    body: Body,
) -> Result<(McpCaller, Bytes), McpError> {
    // 1. The switch.
    if !s.config.mcp.enabled {
        return Err(McpError::Disabled);
    }
    // 2. POST only: no stream is offered on GET and there is no session for DELETE to end.
    if *method != Method::POST {
        return Err(McpError::MethodNotAllowed);
    }
    // 3. An Origin header, when present, must be one allowed value; an absent one passes. Two are refused whatever
    // they say: which of them a browser or a proxy meant is not this server's to guess.
    let mut origins = headers.get_all(header::ORIGIN).iter();
    if let Some(origin) = origins.next()
        && (origins.next().is_some()
            || !origin
                .to_str()
                .is_ok_and(|o| s.config.mcp_origin_allowed(o)))
    {
        tracing::warn!(path = "/mcp", reason = "origin not allowed", origin = ?origin, "request refused");
        return Err(McpError::OriginNotAllowed);
    }
    // 4. The caller, before every protocol check: an unauthenticated call gets the 401 whatever else is wrong.
    let caller = McpCaller::from_headers(s, headers)
        .await
        .map_err(|e| match e {
            AuthError::Db(e) => McpError::Internal(e.into()),
            // Already logged, once: a missing or malformed bearer header by `McpCaller`, a refused token by
            // identity.
            _ => McpError::Unauthenticated,
        })?;
    // 5. A JSON body of at most 1 MiB.
    let is_json = header_text(headers, "content-type").is_some_and(|v| {
        v.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
    });
    if !is_json {
        return Err(McpError::UnsupportedMediaType);
    }
    let bytes = axum::body::to_bytes(body, MAX_BODY).await.map_err(|e| {
        let too_large = std::error::Error::source(&e)
            .is_some_and(|source| source.is::<http_body_util::LengthLimitError>());
        if too_large {
            McpError::TooLarge
        } else {
            McpError::BadBody
        }
    })?;
    Ok((caller, bytes))
}

/// The three protocol headers of one request.
fn protocol_headers(headers: &HeaderMap) -> Result<Headers<'_>, RpcError> {
    Ok(Headers {
        protocol_version: protocol_header(headers, "mcp-protocol-version")?,
        method: protocol_header(headers, "mcp-method")?,
        name: protocol_header(headers, "mcp-name")?,
    })
}

/// Step 11 for one classified request: its result.
fn answer(revision: Revision, msg: &Message) -> Result<Value, RpcError> {
    // 11. The methods of each revision; anything else is not implemented.
    match (revision, msg.method.as_str()) {
        (Revision::Modern, "server/discover") => Ok(modern(
            json!({"supportedVersions": protocol::SUPPORTED, "capabilities": capabilities()}),
        )),
        // The answer is 2025-11-25 whatever the client asked for; the client decides whether to go on.
        (Revision::Legacy, "initialize") => Ok(json!({
            "protocolVersion": protocol::LEGACY,
            "capabilities": capabilities(),
            "serverInfo": server_info()
        })),
        (Revision::Legacy, "ping") => Ok(json!({})),
        (_, "tools/list") => {
            let result = json!({"tools": tools::list()});
            Ok(match revision {
                Revision::Modern => modern(result),
                Revision::Legacy => result,
            })
        }
        // 2026-07-28 answers an unknown method with 404. 2025-11-25's transport keeps 404 for an expired session
        // and carries a JSON-RPC error in an ordinary answer.
        _ => Err(RpcError::new(
            match revision {
                Revision::Modern => StatusCode::NOT_FOUND,
                Revision::Legacy => StatusCode::OK,
            },
            protocol::METHOD_NOT_FOUND,
            format!("Method not found: {}", msg.method),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "database_url = \"postgres://x\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"admin@example.com\"]\npublic_url = \"https://reeve.example.com\"\n[oidc]\nprovider = \"google\"\nclient_id = \"i\"\nclient_secret = \"s\"\n";

    #[test]
    fn the_startup_line_says_whether_and_where_mcp_is_served() {
        let off = crate::ServerConfig::parse(BASE).unwrap();
        assert_eq!(mcp_startup_line(&off), "mcp endpoint disabled");
        let on = crate::ServerConfig::parse(&format!("{BASE}[mcp]\nenabled = true\n")).unwrap();
        assert_eq!(
            mcp_startup_line(&on),
            "mcp endpoint enabled at https://reeve.example.com/mcp (protocol 2026-07-28 and 2025-11-25, 0 tools)"
        );
    }
}
