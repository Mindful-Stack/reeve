//! JSON-RPC and MCP messages for `/mcp`: parsing one message, classifying it as 2026-07-28 (modern) or 2025-11-25
//! (legacy), the header and `_meta` checks of 2026-07-28, and the error codes. Knows nothing about Reeve; the only
//! types it takes from outside are serde's, base64's and the HTTP status code.
use axum::http::StatusCode;
use base64::Engine;
use serde_json::{Map, Value, json};

pub(crate) const MODERN: &str = "2026-07-28";
pub(crate) const LEGACY: &str = "2025-11-25";
/// Every answer that lists the supported revisions lists these, in this order.
pub(crate) const SUPPORTED: [&str; 2] = [MODERN, LEGACY];

pub(crate) const PARSE_ERROR: i64 = -32700;
pub(crate) const INVALID_REQUEST: i64 = -32600;
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
pub(crate) const INVALID_PARAMS: i64 = -32602;
pub(crate) const INTERNAL_ERROR: i64 = -32603;
pub(crate) const HEADER_MISMATCH: i64 = -32020;
pub(crate) const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// A JSON-RPC error and the HTTP status it is answered with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    pub status: StatusCode,
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub(crate) fn new(status: StatusCode, code: i64, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            data: None,
        }
    }

    /// The JSON-RPC error response, with the request's id when one was read.
    pub(crate) fn body(&self, id: Option<&Value>) -> Value {
        let mut error = json!({"code": self.code, "message": self.message});
        if let Some(data) = &self.data {
            error["data"] = data.clone();
        }
        let mut body = json!({"jsonrpc": "2.0", "error": error});
        if let Some(id) = id {
            body["id"] = id.clone();
        }
        body
    }
}

/// One JSON-RPC request (`id` set) or notification (`id` absent).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Message {
    pub id: Option<Value>,
    pub method: String,
    pub params: Map<String, Value>,
}

/// Step 6: the body is one JSON-RPC request or notification. Not a batch, not a response.
pub(crate) fn parse(body: &[u8]) -> Result<Message, RpcError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| {
        RpcError::new(
            StatusCode::BAD_REQUEST,
            PARSE_ERROR,
            "Parse error: the body is not JSON",
        )
    })?;
    let invalid = |why: &str| {
        RpcError::new(
            StatusCode::BAD_REQUEST,
            INVALID_REQUEST,
            format!("Invalid request: {why}"),
        )
    };
    let mut obj = match value {
        Value::Object(obj) => obj,
        Value::Array(_) => {
            return Err(invalid(
                "a batch is not accepted; send one message per POST",
            ));
        }
        _ => return Err(invalid("the body must be one JSON object")),
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(invalid("jsonrpc must be \"2.0\""));
    }
    let Some(Value::String(method)) = obj.remove("method") else {
        return Err(invalid("method must be a string"));
    };
    let id = match obj.remove("id") {
        None => None,
        Some(Value::String(s)) => Some(Value::String(s)),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Some(Value::Number(n)),
        Some(_) => return Err(invalid("id must be a string or an integer")),
    };
    let params = match obj.remove("params") {
        None => Map::new(),
        Some(Value::Object(params)) => params,
        Some(_) => return Err(invalid("params must be an object")),
    };
    Ok(Message { id, method, params })
}

/// The request-metadata headers of one POST, as text (`None`: absent or not text).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Headers<'a> {
    pub protocol_version: Option<&'a str>,
    pub method: Option<&'a str>,
    pub name: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Revision {
    Modern,
    Legacy,
}

fn mismatch(why: String) -> RpcError {
    RpcError::new(
        StatusCode::BAD_REQUEST,
        HEADER_MISMATCH,
        format!("Header mismatch: {why}"),
    )
}

fn unsupported(requested: &str) -> RpcError {
    RpcError {
        status: StatusCode::BAD_REQUEST,
        code: UNSUPPORTED_PROTOCOL_VERSION,
        message: format!("Unsupported protocol version; supported: {MODERN}, {LEGACY}"),
        data: Some(json!({"supported": SUPPORTED, "requested": requested})),
    }
}

/// Which revision a message is served under: the classification table, then steps 7 to 10 for 2026-07-28. Every
/// combination has one answer; nothing falls back from one revision to the other.
pub(crate) fn classify(msg: &Message, h: Headers<'_>) -> Result<Revision, RpcError> {
    let meta = msg.params.get("_meta").and_then(Value::as_object);
    let body_version = match meta.and_then(|m| m.get(META_VERSION)) {
        None => None,
        Some(Value::String(v)) => Some(v.as_str()),
        Some(_) => {
            return Err(RpcError::new(
                StatusCode::BAD_REQUEST,
                INVALID_PARAMS,
                format!("Invalid params: _meta.{META_VERSION} must be a string"),
            ));
        }
    };
    match (body_version, h.protocol_version) {
        // Step 7: the header is present and equals the body's version.
        (Some(body), header) if header != Some(body) => Err(mismatch(format!(
            "the MCP-Protocol-Version header must equal _meta.{META_VERSION} ({body})"
        ))),
        (Some(MODERN), _) => modern_checks(msg, meta, h).map(|()| Revision::Modern),
        // A legacy version never travels in `_meta`; such a request is refused, not served as legacy.
        (Some(LEGACY), _) => Err(RpcError::new(
            StatusCode::BAD_REQUEST,
            INVALID_REQUEST,
            format!(
                "Invalid request: {LEGACY} carries its version in the MCP-Protocol-Version header only; a version in _meta means {MODERN}"
            ),
        )),
        // Step 8.
        (Some(other), _) => Err(unsupported(other)),
        (None, Some(MODERN)) => Err(mismatch(format!(
            "{MODERN} requires _meta.{META_VERSION} in the request body"
        ))),
        (None, Some(LEGACY)) => Ok(Revision::Legacy),
        (None, Some(other)) => Err(unsupported(other)),
        // The handshake that opens 2025-11-25 carries no version header.
        (None, None) if msg.method == "initialize" => Ok(Revision::Legacy),
        (None, None) => Err(mismatch(format!(
            "the MCP-Protocol-Version header is missing; supported: {MODERN}, {LEGACY}"
        ))),
    }
}

/// Steps 9 and 10.
fn modern_checks(
    msg: &Message,
    meta: Option<&Map<String, Value>>,
    h: Headers<'_>,
) -> Result<(), RpcError> {
    if !meta
        .and_then(|m| m.get(META_CAPABILITIES))
        .is_some_and(Value::is_object)
    {
        return Err(RpcError::new(
            StatusCode::BAD_REQUEST,
            INVALID_PARAMS,
            format!("Invalid params: _meta.{META_CAPABILITIES} is required"),
        ));
    }
    if h.method != Some(msg.method.as_str()) {
        return Err(mismatch(format!(
            "the Mcp-Method header must equal the body's method ({})",
            msg.method
        )));
    }
    if msg.method == "tools/call" {
        let Some(name) = msg.params.get("name").and_then(Value::as_str) else {
            return Err(RpcError::new(
                StatusCode::BAD_REQUEST,
                INVALID_PARAMS,
                "Invalid params: tools/call requires params.name",
            ));
        };
        if h.name.and_then(decode_header_value).as_deref() != Some(name) {
            return Err(mismatch(
                "the Mcp-Name header must equal params.name".to_string(),
            ));
        }
    }
    Ok(())
}

/// An `Mcp-Name` header value as the text it stands for: the `=?base64?…?=` form decoded, anything else as it is.
/// `None` when the encoded form is not base64 of UTF-8.
pub(crate) fn decode_header_value(raw: &str) -> Option<String> {
    match raw
        .strip_prefix("=?base64?")
        .and_then(|rest| rest.strip_suffix("?="))
    {
        Some(encoded) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?;
            String::from_utf8(bytes).ok()
        }
        None => Some(raw.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(method: &str, meta_version: Option<&str>) -> Message {
        let mut params = Map::new();
        if let Some(v) = meta_version {
            params.insert(
                "_meta".into(),
                json!({(META_VERSION): v, (META_CAPABILITIES): {}}),
            );
        }
        Message {
            id: Some(json!(1)),
            method: method.into(),
            params,
        }
    }

    fn headers<'a>(version: Option<&'a str>, method: &'a str) -> Headers<'a> {
        Headers {
            protocol_version: version,
            method: Some(method),
            name: None,
        }
    }

    /// Steps 7 and 8 and the two ways into the legacy revision. The whole table is tested once, through the router
    /// (`every_row_of_the_classification_table_through_the_router`); this pins the order of the two steps.
    #[test]
    fn the_header_must_equal_the_body_version_before_the_version_is_judged() {
        let code = |body: Option<&str>, header: Option<&str>| {
            classify(&msg("tools/list", body), headers(header, "tools/list")).map_err(|e| {
                assert_eq!(e.status, StatusCode::BAD_REQUEST);
                e.code
            })
        };
        assert_eq!(code(Some(MODERN), Some(MODERN)), Ok(Revision::Modern));
        assert_eq!(code(None, Some(LEGACY)), Ok(Revision::Legacy));
        // Step 7: absent or different, whether or not the body's version is one this server supports.
        assert_eq!(code(Some(MODERN), None), Err(HEADER_MISMATCH));
        assert_eq!(code(Some(MODERN), Some(LEGACY)), Err(HEADER_MISMATCH));
        assert_eq!(code(Some("1900-01-01"), None), Err(HEADER_MISMATCH));
        assert_eq!(code(None, Some(MODERN)), Err(HEADER_MISMATCH));
        // Step 8: only once the two agree.
        assert_eq!(
            code(Some("1900-01-01"), Some("1900-01-01")),
            Err(UNSUPPORTED_PROTOCOL_VERSION)
        );
        // The handshake opens the legacy revision without a header; nothing else does.
        assert_eq!(
            classify(&msg("initialize", None), headers(None, "initialize")),
            Ok(Revision::Legacy)
        );
        assert_eq!(code(None, None), Err(HEADER_MISMATCH));
    }

    #[test]
    fn an_unsupported_version_names_both_supported_revisions_and_the_requested_one() {
        let e = classify(
            &msg("tools/list", None),
            headers(Some("2025-06-18"), "tools/list"),
        )
        .unwrap_err();
        assert_eq!(
            e.data,
            Some(json!({"supported": ["2026-07-28", "2025-11-25"], "requested": "2025-06-18"}))
        );
        let missing = classify(&msg("tools/list", None), headers(None, "tools/list")).unwrap_err();
        assert!(
            missing.message.contains(MODERN) && missing.message.contains(LEGACY),
            "{}",
            missing.message
        );
    }

    #[test]
    fn a_modern_request_needs_client_capabilities_and_matching_method_and_name_headers() {
        // Step 9: clientCapabilities missing.
        let mut m = msg("tools/list", Some(MODERN));
        m.params["_meta"]
            .as_object_mut()
            .unwrap()
            .remove(META_CAPABILITIES);
        assert_eq!(
            classify(&m, headers(Some(MODERN), "tools/list"))
                .unwrap_err()
                .code,
            INVALID_PARAMS
        );
        // Step 10: Mcp-Method absent, then different.
        let m = msg("tools/list", Some(MODERN));
        let mut h = headers(Some(MODERN), "tools/list");
        h.method = None;
        assert_eq!(classify(&m, h).unwrap_err().code, HEADER_MISMATCH);
        h.method = Some("tools/call");
        assert_eq!(classify(&m, h).unwrap_err().code, HEADER_MISMATCH);
        // Step 10 for tools/call: Mcp-Name absent, different, equal, and equal in the base64 form.
        let mut call = msg("tools/call", Some(MODERN));
        call.params.insert("name".into(), json!("get_story"));
        let mut h = headers(Some(MODERN), "tools/call");
        assert_eq!(classify(&call, h).unwrap_err().code, HEADER_MISMATCH);
        h.name = Some("other");
        assert_eq!(classify(&call, h).unwrap_err().code, HEADER_MISMATCH);
        h.name = Some("get_story");
        assert_eq!(classify(&call, h), Ok(Revision::Modern));
        h.name = Some("=?base64?Z2V0X3N0b3J5?=");
        assert_eq!(classify(&call, h), Ok(Revision::Modern));
        h.name = Some("=?base64?not base64?=");
        assert_eq!(classify(&call, h).unwrap_err().code, HEADER_MISMATCH);
        // A tools/call without a name is malformed, whatever the header says.
        call.params.remove("name");
        assert_eq!(classify(&call, h).unwrap_err().code, INVALID_PARAMS);
    }

    #[test]
    fn header_values_decode_as_the_specification_examples_do() {
        assert_eq!(decode_header_value("us-west1").as_deref(), Some("us-west1"));
        assert_eq!(
            decode_header_value("=?base64?SGVsbG8sIOS4lueVjA==?=").as_deref(),
            Some("Hello, 世界")
        );
        assert_eq!(
            decode_header_value("=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?=").as_deref(),
            Some("=?base64?literal?=")
        );
        assert_eq!(
            decode_header_value("=?BASE64?eA==?=").as_deref(),
            Some("=?BASE64?eA==?=")
        );
    }

    #[test]
    fn parse_accepts_one_request_or_notification_and_nothing_else() {
        let request =
            parse(br#"{"jsonrpc":"2.0","id":"a","method":"tools/list","params":{"x":1}}"#).unwrap();
        assert_eq!(request.id, Some(json!("a")));
        assert_eq!(request.params["x"], 1);
        let notification =
            parse(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert_eq!(notification.id, None);
        assert!(notification.params.is_empty());
        assert_eq!(parse(b"{not json").unwrap_err().code, PARSE_ERROR);
        for bad in [
            r#"[{"jsonrpc":"2.0","id":1,"method":"tools/list"}]"#, // a batch
            r#""text""#,
            r#"{"id":1,"method":"tools/list"}"#, // no jsonrpc
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, // a response
            r#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":1.5,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":[1]}"#,
        ] {
            let e = parse(bad.as_bytes()).unwrap_err();
            assert_eq!(
                (e.status, e.code),
                (StatusCode::BAD_REQUEST, INVALID_REQUEST),
                "{bad}"
            );
        }
    }

    #[test]
    fn an_error_body_carries_the_id_only_when_one_was_read() {
        let e = RpcError::new(
            StatusCode::NOT_FOUND,
            METHOD_NOT_FOUND,
            "Method not found: x",
        );
        assert_eq!(
            e.body(Some(&json!(7))),
            json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -32601, "message": "Method not found: x"}})
        );
        assert_eq!(
            e.body(None),
            json!({"jsonrpc": "2.0", "error": {"code": -32601, "message": "Method not found: x"}})
        );
    }
}
