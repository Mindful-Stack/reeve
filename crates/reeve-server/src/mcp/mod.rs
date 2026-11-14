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
use tools::ToolError;

/// The largest request body read (step 5). A raw `Request` gets no default body limit, so this is the only one.
const MAX_BODY: usize = 1024 * 1024;

/// How long a client may keep `server/discover` and `tools/list` answers: neither changes while the process runs.
const TTL_MS: u64 = 300_000;

/// Every tool, built once with the state (`build_state`): building a tool generates its input schema, which is
/// not work to repeat on every request.
pub(crate) type Tools = std::sync::Arc<Vec<tools::Tool>>;

pub(crate) fn build_tools(config: &crate::ServerConfig) -> Tools {
    std::sync::Arc::new(tools::registry(&config.key_prefix))
}

fn capabilities() -> Value {
    // No `listChanged`: no list ever changes at run time, and `subscriptions/listen` is not implemented.
    json!({"tools": {}})
}

fn server_info() -> Value {
    json!({"name": "reeve", "version": env!("CARGO_PKG_VERSION")})
}

/// A 2026-07-28 result: `resultType` and the server's identity.
fn complete(mut result: Value) -> Value {
    result["resultType"] = json!("complete");
    result["_meta"] = json!({"io.modelcontextprotocol/serverInfo": server_info()});
    result
}

/// A 2026-07-28 result that a client may cache: a complete one with the caching hints. `cacheScope` is `public`
/// while every caller sees the same list; it becomes `private` once scopes filter it (GRA-658).
fn modern(result: Value) -> Value {
    let mut result = complete(result);
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
    /// Step 4. `challenge` is the whole `WWW-Authenticate` value, built by `challenge()`.
    Unauthenticated {
        challenge: String,
    },
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
            // Claude starts sign-in only on this 401 and reads where to go from its header (kb/ADR-0029 R2).
            McpError::Unauthenticated { challenge } => (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, challenge)],
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
            McpError::Internal(e) => rpc_error(&internal(e), None),
        }
    }
}

/// An internal error: logged at `error`, answered 500 and -32603 with no detail, in either revision.
fn internal(e: anyhow::Error) -> RpcError {
    tracing::error!(error = ?e, "mcp: internal error");
    RpcError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        protocol::INTERNAL_ERROR,
        "Internal error",
    )
}

/// A JSON-RPC error response, with the request's id when one was read.
fn rpc_error(e: &RpcError, id: Option<&Value>) -> Response {
    (e.status, Json(e.body(id))).into_response()
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The `WWW-Authenticate` value of a 401: where the protected resource's metadata is (RFC 9728 section 5.1), and
/// `error="invalid_token"` when a bearer token was presented and refused. A request without one carries no error
/// (RFC 6750 section 3.1). No `scope`: a client then asks for what the metadata lists.
fn challenge(config: &crate::ServerConfig, token_refused: bool) -> String {
    let metadata = crate::auth::oauth::resource_metadata_url(config);
    if token_refused {
        format!(r#"Bearer error="invalid_token", resource_metadata="{metadata}""#)
    } else {
        format!(r#"Bearer resource_metadata="{metadata}""#)
    }
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

/// The one line `serve` logs at startup about this endpoint, from the state as built.
pub fn mcp_startup_line(state: &AppState) -> String {
    startup_line(&state.config, state.mcp_tools.len())
}

fn startup_line(config: &crate::ServerConfig, tools: usize) -> String {
    if !config.mcp.enabled {
        return "mcp endpoint disabled".into();
    }
    format!(
        "mcp endpoint enabled at {} (protocol {} and {}, {tools} tools)",
        crate::auth::oauth::resource(config),
        protocol::MODERN,
        protocol::LEGACY
    )
}

/// `ANY /mcp`. The first failing step answers and stops.
pub(crate) async fn handle(State(s): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let (caller, bytes) = match admit(&s, &parts.method, &parts.headers, body).await {
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
    let outcome = match revision {
        Ok(revision) => answer(&s, &caller, revision, &msg).await,
        Err(e) => Err(e),
    };
    match outcome {
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
    // Whether a bearer token was presented decides the 401's `error`; whether it is accepted is identity's. A bearer
    // header with nothing after the scheme presents no token.
    let presented = matches!(crate::auth::bearer_secret(headers), Ok(Some(secret)) if !secret.trim().is_empty());
    let caller = McpCaller::from_headers(s, headers)
        .await
        .map_err(|e| match e {
            AuthError::Db(e) => McpError::Internal(e.into()),
            // Already logged, once: a missing or malformed bearer header by `McpCaller`, a refused token by
            // identity.
            _ => McpError::Unauthenticated {
                challenge: challenge(&s.config, presented),
            },
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
async fn answer(
    s: &AppState,
    caller: &McpCaller,
    revision: Revision,
    msg: &Message,
) -> Result<Value, RpcError> {
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
            let result = json!({"tools": tools::list(&s.mcp_tools)});
            Ok(match revision {
                Revision::Modern => modern(result),
                Revision::Legacy => result,
            })
        }
        (_, "tools/call") => call_tool(s, caller, revision, msg).await,
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

/// `tools/call`: look the tool up, run it, and shape its outcome. An unknown tool and a call without a name are
/// protocol errors (-32602); a failure the model can act on is a result with `isError: true`.
async fn call_tool(
    s: &AppState,
    caller: &McpCaller,
    revision: Revision,
    msg: &Message,
) -> Result<Value, RpcError> {
    // 400 in 2026-07-28; 2025-11-25 carries a JSON-RPC error in an ordinary answer, as for an unknown method.
    let invalid = |message: String| {
        RpcError::new(
            match revision {
                Revision::Modern => StatusCode::BAD_REQUEST,
                Revision::Legacy => StatusCode::OK,
            },
            protocol::INVALID_PARAMS,
            message,
        )
    };
    let Some(name) = msg.params.get("name").and_then(Value::as_str) else {
        return Err(invalid(
            "Invalid params: tools/call requires params.name".into(),
        ));
    };
    let Some(tool) = s.mcp_tools.iter().find(|t| t.name == name) else {
        return Err(invalid(format!("Unknown tool: {name}")));
    };
    // `arguments` may be left out (no arguments); when present it must be a JSON object.
    let args = msg
        .params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let who = caller.acting();
    let outcome = if args.is_object() {
        (tool.run)(s, &who, args).await
    } else {
        Err(ToolError::Arguments(
            "the arguments must be a JSON object".into(),
        ))
    };
    let failure =
        |message: String| json!({"content": [{"type": "text", "text": message}], "isError": true});
    let result = match outcome {
        Ok(value) => json!({
            "content": [{"type": "text", "text": value.to_string()}],
            "structuredContent": value,
            "isError": false
        }),
        // The schema is the one `tools/list` shows, with this instance's example, so the model can correct itself.
        Err(ToolError::Arguments(message)) => failure(format!(
            "Invalid arguments: {message}. The arguments must match this schema: {}",
            tool.input_schema
        )),
        Err(ToolError::Told(message)) => failure(message),
        Err(ToolError::Internal(e)) => return Err(internal(e)),
    };
    // Not cacheable: no `ttlMs`, no `cacheScope`.
    Ok(match revision {
        Revision::Modern => complete(result),
        Revision::Legacy => result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "database_url = \"postgres://x\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"admin@example.com\"]\npublic_url = \"https://reeve.example.com\"\n[oidc]\nprovider = \"google\"\nclient_id = \"i\"\nclient_secret = \"s\"\n";

    #[test]
    fn the_startup_line_says_whether_and_where_mcp_is_served() {
        let off = crate::ServerConfig::parse(BASE).unwrap();
        assert_eq!(startup_line(&off, 3), "mcp endpoint disabled");
        let on = crate::ServerConfig::parse(&format!("{BASE}[mcp]\nenabled = true\n")).unwrap();
        let line = startup_line(&on, 3);
        assert!(
            line.starts_with(
                "mcp endpoint enabled at https://reeve.example.com/mcp (protocol 2026-07-28 and 2025-11-25, "
            ),
            "{line}"
        );
    }
}
