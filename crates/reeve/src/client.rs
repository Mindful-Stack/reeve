use reeve_protocol::{ErrorCode, Reason};
use serde::de::DeserializeOwned;

#[derive(Debug)]
pub enum Outcome<T> {
    Ok(T),
    /// `overridable` is the server's word on whether an interactive override can get past `reasons`.
    Refused {
        reasons: Vec<Reason>,
        overridable: bool,
    },
    /// `envelope` is false for an error body the server did not write (a proxy's page): only the server's own
    /// refusals mean what their code says.
    Error {
        code: ErrorCode,
        message: String,
        envelope: bool,
        /// Seconds to wait before retrying, when a rate limit (`rate_limited`) came with a Retry-After.
        retry_after: Option<u64>,
        /// The envelope's `current`, as the server sent it: on a `conflict`, the row's version now (`-1` for a concurrent
        /// update with no version to give: re-read and retry); `None` when the envelope has none.
        current: Option<i64>,
    },
}

impl<T: DeserializeOwned> Outcome<T> {
    pub fn from_envelope(_status: u16, v: serde_json::Value) -> anyhow::Result<Self> {
        match v.get("outcome").and_then(|o| o.as_str()) {
            Some("ok") => Ok(Outcome::Ok(serde_json::from_value(v["value"].clone())?)),
            Some("refused") => Ok(Outcome::Refused {
                reasons: serde_json::from_value(v["reasons"].clone())?,
                // Absent from an older server's body: no override is offered.
                overridable: v["overridable"].as_bool().unwrap_or(false),
            }),
            Some("error") => Ok(Outcome::Error {
                code: serde_json::from_value(v["code"].clone())?,
                message: v["message"].as_str().unwrap_or("").to_string(),
                envelope: true,
                retry_after: None,
                current: v.get("current").and_then(|c| c.as_i64()),
            }),
            _ => anyhow::bail!("unexpected response: {v}"),
        }
    }
}
impl<T> Outcome<T> {
    /// The one place that decides a command's exit status: 0 success; 2 the server refused, either with reasons or by
    /// refusing the caller (`forbidden`, `unauthenticated`); 1 every other error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Outcome::Ok(_) => 0,
            Outcome::Refused { .. } => 2,
            Outcome::Error {
                code: ErrorCode::Forbidden | ErrorCode::Unauthenticated,
                ..
            } => 2,
            Outcome::Error { .. } => 1,
        }
    }
}

/// An error body that is not an envelope (a proxy's page, say) is described by its status.
fn non_json_outcome<T>(status: u16) -> Outcome<T> {
    let code = match status {
        401 => ErrorCode::Unauthenticated,
        403 => ErrorCode::Forbidden,
        404 => ErrorCode::NotFound,
        _ => ErrorCode::Internal,
    };
    Outcome::Error {
        code,
        message: format!("HTTP {status}"),
        envelope: false,
        retry_after: None,
        current: None,
    }
}

/// What a response means. A 429 is `rate_limited` whatever the body (the proxy in front of the server sends it, not the
/// server), with its Retry-After in seconds or as an HTTP date; a body that is not JSON is reported by its status.
fn outcome_of<T: DeserializeOwned>(
    status: u16,
    retry_after: Option<&str>,
    body: &[u8],
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Outcome<T>> {
    if status == 429 {
        let retry_after = retry_after.and_then(|h| {
            let h = h.trim();
            h.parse::<u64>().ok().or_else(|| {
                let at = chrono::DateTime::parse_from_rfc2822(h).ok()?;
                Some((at.with_timezone(&chrono::Utc) - now).num_seconds().max(0) as u64)
            })
        });
        return Ok(Outcome::Error {
            code: ErrorCode::RateLimited,
            message: match retry_after {
                Some(s) => {
                    format!("rate limited (HTTP 429): retry the same command after {s} seconds")
                }
                None => "rate limited (HTTP 429): wait and retry the same command".into(),
            },
            envelope: false,
            retry_after,
            current: None,
        });
    }
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(v) => Outcome::from_envelope(status, v),
        Err(_) => Ok(non_json_outcome(status)),
    }
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    /// The last `X-Reeve-Cli` a response carried (kb/ADR-0030): the build the server offers.
    server_cli: std::sync::Mutex<Option<String>>,
}
impl Client {
    pub fn new(base: String, token: String) -> Self {
        Self::with_timeout(base, token, std::time::Duration::from_secs(30))
    }
    /// A client whose every request has this total timeout.
    pub fn with_timeout(base: String, token: String, timeout: std::time::Duration) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("http client"),
            base,
            token,
            server_cli: std::sync::Mutex::new(None),
        }
    }
    /// The build the server offers, from the last response that carried one; `None` before any did.
    pub fn server_cli(&self) -> Option<String> {
        self.server_cli.lock().unwrap().clone()
    }
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<Outcome<T>> {
        self.send(method, path, query, body, None).await
    }
    /// One request; `timeout` replaces the client's default 30-second total timeout for this request alone.
    async fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
        timeout: Option<std::time::Duration>,
    ) -> anyhow::Result<Outcome<T>> {
        let (status, headers, bytes) = self.exchange(method, path, query, body, timeout).await?;
        outcome_of(
            status,
            header(&headers, reqwest::header::RETRY_AFTER).as_deref(),
            &bytes,
            chrono::Utc::now(),
        )
    }
    /// Sends one authenticated request and reads the whole answer: status, headers and body.
    async fn exchange(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
        timeout: Option<std::time::Duration>,
    ) -> anyhow::Result<(u16, reqwest::header::HeaderMap, Vec<u8>)> {
        let mut req = self
            .http
            .request(method, format!("{}/api/v1{}", self.base, path))
            .query(query)
            .bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let res = req.send().await?;
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        if let Some(v) = headers
            .get(reeve_protocol::CLI_VERSION_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|v| reeve_protocol::is_cli_version(v))
        {
            crate::cli_version::observe(v);
            *self.server_cli.lock().unwrap() = Some(v.to_string());
        }
        Ok((status, headers, res.bytes().await?.to_vec()))
    }
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<Outcome<T>> {
        self.call(reqwest::Method::GET, path, &[], None).await
    }
    /// GET with its own total timeout, for a request that may legitimately outlast the default (the export).
    pub async fn get_with_timeout<T: DeserializeOwned>(
        &self,
        path: &str,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Outcome<T>> {
        self.send(reqwest::Method::GET, path, &[], None, Some(timeout))
            .await
    }
    /// GET an image route: on 200 its content type and bytes; any other answer is the server's outcome. An image is at
    /// most the 10 MiB upload cap (kb/ADR-0023), so it is held in memory.
    pub async fn get_image(
        &self,
        path: &str,
    ) -> anyhow::Result<Result<(String, Vec<u8>), Outcome<()>>> {
        let (status, headers, body) = self
            .exchange(reqwest::Method::GET, path, &[], None, None)
            .await?;
        if status == 200 {
            let content_type = header(&headers, reqwest::header::CONTENT_TYPE).unwrap_or_default();
            return Ok(Ok((content_type, body)));
        }
        Ok(Err(outcome_of(
            status,
            header(&headers, reqwest::header::RETRY_AFTER).as_deref(),
            &body,
            chrono::Utc::now(),
        )?))
    }
    /// GET with query parameters, encoded by reqwest.
    pub async fn get_query<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> anyhow::Result<Outcome<T>> {
        self.call(reqwest::Method::GET, path, query, None).await
    }
    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> anyhow::Result<Outcome<T>> {
        self.call(reqwest::Method::POST, path, &[], Some(body))
            .await
    }
    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<Outcome<T>> {
        self.call(reqwest::Method::DELETE, path, &[], None).await
    }
    pub async fn patch<T: DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> anyhow::Result<Outcome<T>> {
        self.call(reqwest::Method::PATCH, path, &[], Some(body))
            .await
    }
}

/// A header's value, when present and printable.
fn header(
    headers: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Serves `answers` (raw HTTP responses) on loopback, one per connection, and returns the base url.
    async fn raw(answers: Vec<String>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move {
            for a in answers {
                let (mut s, _) = l.accept().await.unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                s.write_all(a.as_bytes()).await.unwrap();
                let _ = s.shutdown().await;
            }
        });
        url
    }

    fn answer(cli: Option<&str>) -> String {
        let body = r#"{"outcome":"ok","value":[]}"#;
        let h = cli
            .map(|v| format!("x-reeve-cli: {v}\r\n"))
            .unwrap_or_default();
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n{h}content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn the_client_keeps_the_last_offered_build_it_was_sent() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let url = raw(vec![
            answer(Some(&a)),
            answer(Some(&b)),
            answer(None),
            answer(Some("not-a-version")),
        ])
        .await;
        let c = Client::new(url, "t".into());
        assert_eq!(c.server_cli(), None, "nothing before the first answer");
        let _ = c.get::<Vec<serde_json::Value>>("/x").await.unwrap();
        assert_eq!(c.server_cli().as_deref(), Some(a.as_str()));
        let _ = c.get::<Vec<serde_json::Value>>("/x").await.unwrap();
        assert_eq!(
            c.server_cli().as_deref(),
            Some(b.as_str()),
            "the last one wins"
        );
        let _ = c.get::<Vec<serde_json::Value>>("/x").await.unwrap();
        assert_eq!(
            c.server_cli().as_deref(),
            Some(b.as_str()),
            "an answer without the header keeps it"
        );
        let _ = c.get::<Vec<serde_json::Value>>("/x").await.unwrap();
        assert_eq!(
            c.server_cli().as_deref(),
            Some(b.as_str()),
            "a garbled header is ignored"
        );
    }
    #[test]
    fn envelope_maps_to_outcome_and_exit_code() {
        let ok: Outcome<u8> =
            Outcome::from_envelope(200, serde_json::json!({"outcome":"ok","value":7})).unwrap();
        assert!(matches!(ok, Outcome::Ok(7)));
        assert_eq!(ok.exit_code(), 0);
        let refused: Outcome<u8> = Outcome::from_envelope(
            422,
            serde_json::json!({"outcome":"refused","reasons":[{"reason":"no_criteria"}]}),
        )
        .unwrap();
        assert_eq!(refused.exit_code(), 2);
        assert!(
            matches!(
                &refused,
                Outcome::Refused {
                    overridable: false,
                    ..
                }
            ),
            "a body without the flag (an older server) is not overridable: {refused:?}"
        );
        let over: Outcome<u8> = Outcome::from_envelope(
            422,
            serde_json::json!({"outcome":"refused","reasons":[{"reason":"no_criteria"}],"overridable":true}),
        )
        .unwrap();
        assert!(
            matches!(&over, Outcome::Refused { reasons, overridable: true } if reasons == &[Reason::NoCriteria]),
            "{over:?}"
        );
        let err: Outcome<u8> = Outcome::from_envelope(
            409,
            serde_json::json!({"outcome":"error","code":"conflict","message":"version conflict: current is 2"}),
        )
        .unwrap();
        assert_eq!(err.exit_code(), 1);
        // a conflict's current version is kept; any other error has none
        assert!(matches!(
            Outcome::<u8>::from_envelope(
                409,
                serde_json::json!({"outcome":"error","code":"conflict","message":"m","current":4}),
            )
            .unwrap(),
            Outcome::Error {
                current: Some(4),
                ..
            }
        ));
        assert!(matches!(err, Outcome::Error { current: None, .. }));
        // the server refusing the caller exits 2; every other error exits 1
        for (code, want) in [
            ("forbidden", 2),
            ("unauthenticated", 2),
            ("conflict", 1),
            ("validation", 1),
            ("not_found", 1),
            ("internal", 1),
        ] {
            let out: Outcome<u8> = Outcome::from_envelope(
                400,
                serde_json::json!({"outcome":"error","code":code,"message":"m"}),
            )
            .unwrap();
            assert_eq!(out.exit_code(), want, "{code}");
        }
    }
    /// A 429 is its own code, whatever body the proxy sent, with the Retry-After it gave in seconds or as a date.
    #[test]
    fn a_rate_limit_is_a_retryable_code_with_its_retry_after() {
        let now = chrono::Utc::now();
        let date = (now + chrono::Duration::seconds(90)).to_rfc2822();
        for (header, want) in [
            (Some("30"), Some(30)),
            (Some(date.as_str()), Some(90)),
            (Some("soon"), None),
            (None, None),
        ] {
            for body in [&b"<html>slow down</html>"[..], br#"{"message":"too many"}"#] {
                match outcome_of::<u8>(429, header, body, now).unwrap() {
                    Outcome::Error {
                        code: ErrorCode::RateLimited,
                        envelope: false,
                        retry_after,
                        ..
                    } => assert!(
                        retry_after.is_some_and(|s| want.is_some_and(|w: u64| s.abs_diff(w) <= 1))
                            || (retry_after.is_none() && want.is_none()),
                        "{header:?}: {retry_after:?}"
                    ),
                    o => panic!("{header:?}: {o:?}"),
                }
            }
        }
    }

    #[test]
    fn non_json_status_maps_to_a_code() {
        for (status, want) in [
            (401, ErrorCode::Unauthenticated),
            (403, ErrorCode::Forbidden),
            (404, ErrorCode::NotFound),
            (502, ErrorCode::Internal),
        ] {
            match non_json_outcome::<u8>(status) {
                Outcome::Error {
                    code,
                    message,
                    envelope,
                    current,
                    ..
                } => {
                    assert!(!envelope);
                    assert_eq!(current, None);
                    assert_eq!(code, want);
                    assert_eq!(message, format!("HTTP {status}"));
                }
                _ => panic!(),
            }
        }
    }
}
