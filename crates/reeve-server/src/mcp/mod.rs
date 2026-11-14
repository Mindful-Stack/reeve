//! The MCP endpoint, `/mcp` (kb/ADR-0029 R1): Reeve's own handler for MCP's Streamable HTTP transport. It serves
//! protocol revisions 2026-07-28 and 2025-11-25 without a session and answers every request with one JSON object;
//! `Accept` is ignored. The handler takes the raw request and runs its checks itself, in the numbered order below:
//! an extractor argument would run before the switch and the origin check. Its errors are `McpError`, never
//! `AppError` (which renders the API's envelope).
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

/// A 2026-07-28 result: `resultType`, the server's identity, and for the cacheable ones the caching hints.
/// `cacheScope` is `public` while every caller sees the same list; it becomes `private` once scopes filter it
/// (GRA-658).
fn modern(mut result: Value, cacheable: bool) -> Value {
    result["resultType"] = json!("complete");
    result["_meta"] = json!({"io.modelcontextprotocol/serverInfo": server_info()});
    if cacheable {
        result["ttlMs"] = json!(TTL_MS);
        result["cacheScope"] = json!("public");
    }
    result
}

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
    /// Steps 6 to 11, and a tool call's protocol errors.
    Rpc(RpcError),
    /// Logged at `error`; answered -32603 with no detail.
    Internal(anyhow::Error),
}

impl McpError {
    fn status(&self) -> StatusCode {
        match self {
            McpError::Disabled => StatusCode::NOT_FOUND,
            McpError::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            McpError::OriginNotAllowed => StatusCode::FORBIDDEN,
            McpError::Unauthenticated => StatusCode::UNAUTHORIZED,
            McpError::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            McpError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            McpError::BadBody => StatusCode::BAD_REQUEST,
            McpError::Rpc(e) => e.status,
            McpError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The response. `id` is the request's id when one was read; `with_body` is false for a message that step 6
    /// parsed and found to be a notification, which
    /// gets the status alone.
    fn respond(self, id: Option<&Value>, with_body: bool) -> Response {
        let status = self.status();
        if let McpError::Internal(e) = &self {
            tracing::error!(error = ?e, "mcp: internal error");
        }
        if !with_body {
            return status.into_response();
        }
        match self {
            // Plain text, never a JSON-RPC body: a client must not take a switched-off endpoint for an MCP server.
            McpError::Disabled => (
                status,
                "MCP is not enabled on this instance ([mcp] enabled)",
            )
                .into_response(),
            McpError::MethodNotAllowed => {
                (status, [(header::ALLOW, "POST")], "/mcp accepts POST only").into_response()
            }
            McpError::OriginNotAllowed => (status, "origin not allowed").into_response(),
            // GRA-655 adds `resource_metadata="<public_url>/.well-known/oauth-protected-resource"` to this header
            // (kb/ADR-0029 R2); Claude starts sign-in only on this 401.
            McpError::Unauthenticated => (
                status,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                "unauthenticated",
            )
                .into_response(),
            McpError::UnsupportedMediaType => {
                (status, "Content-Type must be application/json").into_response()
            }
            McpError::TooLarge => (status, "the request body is larger than 1 MiB").into_response(),
            McpError::BadBody => (status, "the request body could not be read").into_response(),
            McpError::Rpc(e) => (status, Json(e.body(id))).into_response(),
            McpError::Internal(_) => {
                let e = RpcError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    protocol::INTERNAL_ERROR,
                    "Internal error",
                );
                (status, Json(e.body(id))).into_response()
            }
        }
    }
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
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
        Err(e) => return e.respond(None, true),
    };
    // 6. One JSON-RPC request or notification.
    let msg = match protocol::parse(&bytes) {
        Ok(msg) => msg,
        Err(e) => return McpError::Rpc(e).respond(None, true),
    };
    let outcome = answer(&parts.headers, &msg);
    match (&msg.id, outcome) {
        // 12.
        (Some(id), Ok(result)) => {
            Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
        }
        (Some(id), Err(e)) => e.respond(Some(id), true),
        // Step 6 parsed the message, so it is known to be a notification, and from here on it never gets a
        // JSON-RPC response: 202 when it passed the checks, else the failing check's status alone. A failure at
        // steps 1 to 6 (above) is answered as for any request: the server cannot know yet what the message is.
        (None, Ok(_)) => StatusCode::ACCEPTED.into_response(),
        (None, Err(e)) => e.respond(None, false),
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
    // 3. An Origin header, when present, must be allowed; an absent one passes.
    if let Some(origin) = headers.get(header::ORIGIN)
        && !origin
            .to_str()
            .is_ok_and(|o| s.config.mcp_origin_allowed(o))
    {
        tracing::warn!(path = "/mcp", reason = "origin not allowed", origin = ?origin, "request refused");
        return Err(McpError::OriginNotAllowed);
    }
    // 4. The caller, before every protocol check: an unauthenticated call gets the 401 whatever else is wrong.
    let caller = McpCaller::from_headers(s, headers)
        .await
        .map_err(|e| match e {
            AuthError::Db(e) => McpError::Internal(e.into()),
            // `McpCaller` logged the refusal, with the request's protocol headers.
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

/// Steps 7 to 11 for one parsed message: a request's result, or `Null` for an accepted notification.
fn answer(headers: &HeaderMap, msg: &Message) -> Result<Value, McpError> {
    // 7 to 10, or the legacy classification in their place.
    let revision = protocol::classify(
        msg,
        Headers {
            protocol_version: header_text(headers, "mcp-protocol-version"),
            method: header_text(headers, "mcp-method"),
            name: header_text(headers, "mcp-name"),
        },
    )
    .map_err(McpError::Rpc)?;
    // A notification that passed the checks is accepted; this endpoint acts on none.
    if msg.id.is_none() {
        return Ok(Value::Null);
    }
    // 11. The methods of each revision; anything else is not implemented.
    match (revision, msg.method.as_str()) {
        (Revision::Modern, "server/discover") => Ok(modern(
            json!({"supportedVersions": protocol::SUPPORTED, "capabilities": capabilities()}),
            true,
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
                Revision::Modern => modern(result, true),
                Revision::Legacy => result,
            })
        }
        _ => Err(McpError::Rpc(RpcError::new(
            StatusCode::NOT_FOUND,
            protocol::METHOD_NOT_FOUND,
            format!("Method not found: {}", msg.method),
        ))),
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
