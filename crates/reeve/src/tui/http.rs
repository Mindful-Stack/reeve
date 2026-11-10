//! The TUI's HTTP: requests through the CLI's `Client` with the TUI's timeout, and the event stream over a client with
//! a connect timeout but no total timeout, which would end every stream after it.
use crate::client::{Client, Outcome};
use crate::tui::Options;
use crate::tui::effect::{Completion, Ended, Failure, Reply, Request};
use reeve_protocol::{
    EpicDto, ErrorCode, InstanceConfigDto, MarkUrgent, Me, StoryDto, StoryWithReadiness,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct Http {
    api: Arc<Client>,
    stream: reqwest::Client,
    open_timeout: std::time::Duration,
    base: String,
    token: String,
}

pub(crate) fn conv<T>(r: anyhow::Result<Outcome<T>>, wrap: fn(T) -> Reply) -> Completion {
    match r {
        Err(e) => Completion::Failed(Failure::Transport(format!("{e:#}"))),
        Ok(Outcome::Ok(v)) => Completion::Ok(wrap(v)),
        Ok(Outcome::Refused {
            reasons,
            overridable,
        }) => Completion::Refused {
            reasons,
            overridable,
        },
        // Only the server's own envelope means what its code says (`client::Outcome`): a proxy's page, a bare 401
        // or 404 included, is a server failure the owners retry or refetch, never a final sign-out.
        Ok(Outcome::Error {
            message,
            envelope: false,
            ..
        }) => Completion::Failed(Failure::Server(message)),
        Ok(Outcome::Error { code, message, .. }) => Completion::Failed(match code {
            ErrorCode::Unauthenticated => Failure::Unauthenticated,
            ErrorCode::Forbidden => Failure::Forbidden(message),
            ErrorCode::Conflict => Failure::Conflict,
            ErrorCode::Validation => Failure::Validation(message),
            ErrorCode::NotFound => Failure::NotFound(message),
            ErrorCode::Internal | ErrorCode::RateLimited => Failure::Server(message),
        }),
    }
}

impl Http {
    pub fn new(base: String, token: String, o: &Options) -> Self {
        Self {
            api: Arc::new(Client::with_timeout(
                base.clone(),
                token.clone(),
                o.request_timeout,
            )),
            stream: reqwest::Client::builder()
                .connect_timeout(o.connect_timeout)
                .build()
                .expect("http client"),
            open_timeout: o.connect_timeout,
            base,
            token,
        }
    }

    pub async fn request(&self, r: Request) -> Completion {
        match r {
            Request::Me => conv(self.api.get::<Me>("/me").await, Reply::Me),
            Request::Config => conv(
                self.api.get::<InstanceConfigDto>("/config").await,
                Reply::Config,
            ),
            Request::Stories => conv(
                self.api
                    .get_query::<Vec<StoryWithReadiness>>(
                        "/stories",
                        &[("done", "recent".to_string())],
                    )
                    .await,
                Reply::Stories,
            ),
            Request::Epics => conv(self.api.get::<Vec<EpicDto>>("/epics").await, Reply::Epics),
            Request::Move { key, body } => {
                let body = serde_json::to_value(body).expect("request bodies serialise");
                conv(
                    self.api
                        .post::<StoryDto>(&format!("/stories/{key}/move"), body)
                        .await,
                    Reply::Story,
                )
            }
            Request::Edit { key, body } => {
                let body = serde_json::to_value(body).expect("request bodies serialise");
                conv(
                    self.api
                        .patch::<StoryDto>(&format!("/stories/{key}"), body)
                        .await,
                    Reply::Story,
                )
            }
            Request::MarkUrgent { key, reason } => {
                let body =
                    serde_json::to_value(MarkUrgent { reason }).expect("request bodies serialise");
                conv(
                    self.api
                        .post::<StoryDto>(&format!("/stories/{key}/urgent"), body)
                        .await,
                    Reply::Story,
                )
            }
            Request::ClearUrgent { key } => conv(
                self.api
                    .delete::<StoryDto>(&format!("/stories/{key}/urgent"))
                    .await,
                Reply::Story,
            ),
        }
    }

    /// Opens `GET /api/v1/events`; the server's own 401 envelope is `Unauthorized`, any other refusal or transport error `Failed`.
    pub async fn open_events(&self, after: Option<i64>) -> Result<reqwest::Response, Ended> {
        let mut req = self
            .stream
            .get(format!("{}/api/v1/events", self.base))
            .bearer_auth(&self.token)
            .header("accept", "text/event-stream");
        if let Some(id) = after {
            req = req.header("last-event-id", id.to_string());
        }
        headers(self.open_timeout, req.send()).await
    }
}

/// The most of a refusal's body the stream open reads: an error envelope is a few lines, a proxy's page may be large.
const REFUSAL_BODY_MAX: usize = 64 * 1024;

/// Waits for the stream's response headers. Bounded: a server that accepts and never answers must not leave the
/// stream "connecting" for ever. The body has no total timeout; the idle watchdog guards it. A refusal's small body is
/// read inside the same bound: only the server's own `unauthenticated` envelope is `Unauthorized` (the token is dead),
/// as in `client::Outcome`; a proxy's bare 401, any other status or a transport error is `Failed` (the live slice
/// reconnects with backoff).
async fn headers(
    within: std::time::Duration,
    send: impl std::future::Future<Output = reqwest::Result<reqwest::Response>>,
) -> Result<reqwest::Response, Ended> {
    let open = async {
        let mut r = send.await?;
        if r.status().is_success() {
            return Ok(Ok(r));
        }
        let status = r.status();
        let mut body = Vec::new();
        while body.len() < REFUSAL_BODY_MAX {
            match r.chunk().await? {
                Some(c) => body.extend_from_slice(&c),
                None => break,
            }
        }
        body.truncate(REFUSAL_BODY_MAX);
        Ok::<_, reqwest::Error>(Err((status, body)))
    };
    match tokio::time::timeout(within, open).await {
        Err(_) => Err(Ended::Failed("no answer".into())),
        // `{:#}` keeps the cause chain ("error sending request: … connection refused").
        Ok(Err(e)) => Err(Ended::Failed(format!("{:#}", anyhow::Error::from(e)))),
        Ok(Ok(Ok(r))) => Ok(r),
        Ok(Ok(Err((status, body)))) => {
            let own_401 = status == reqwest::StatusCode::UNAUTHORIZED
                && serde_json::from_slice::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        Outcome::<serde_json::Value>::from_envelope(status.as_u16(), v).ok()
                    })
                    .is_some_and(|o| {
                        matches!(
                            o,
                            Outcome::Error {
                                code: ErrorCode::Unauthenticated,
                                envelope: true,
                                ..
                            }
                        )
                    });
            if own_401 {
                Err(Ended::Unauthorized)
            } else {
                Err(Ended::Failed(format!("HTTP {status}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_maps_to_its_failure_and_transport_errors_to_transport() {
        let err = |code| {
            Ok(Outcome::<StoryDto>::Error {
                code,
                message: "m".into(),
                envelope: true,
                retry_after: None,
                current: None,
            })
        };
        assert!(matches!(
            conv(err(ErrorCode::Unauthenticated), Reply::Story),
            Completion::Failed(Failure::Unauthenticated)
        ));
        assert!(
            matches!(conv(err(ErrorCode::Forbidden), Reply::Story), Completion::Failed(Failure::Forbidden(m)) if m == "m")
        );
        assert!(matches!(
            conv(err(ErrorCode::Conflict), Reply::Story),
            Completion::Failed(Failure::Conflict)
        ));
        assert!(matches!(
            conv(err(ErrorCode::Validation), Reply::Story),
            Completion::Failed(Failure::Validation(_))
        ));
        assert!(matches!(
            conv(err(ErrorCode::NotFound), Reply::Story),
            Completion::Failed(Failure::NotFound(_))
        ));
        assert!(matches!(
            conv(err(ErrorCode::Internal), Reply::Story),
            Completion::Failed(Failure::Server(_))
        ));
        for flag in [true, false] {
            let refused: anyhow::Result<Outcome<StoryDto>> = Ok(Outcome::Refused {
                reasons: vec![reeve_protocol::Reason::NoCriteria],
                overridable: flag,
            });
            assert!(
                matches!(conv(refused, Reply::Story), Completion::Refused { reasons, overridable } if reasons.len() == 1 && overridable == flag),
                "the server's flag is carried as sent: {flag}"
            );
        }
        let lost: anyhow::Result<Outcome<StoryDto>> = Err(anyhow::anyhow!("operation timed out"));
        assert!(
            matches!(conv(lost, Reply::Story), Completion::Failed(Failure::Transport(m)) if m.contains("timed out"))
        );
    }

    #[test]
    fn an_error_that_is_not_the_servers_envelope_is_a_server_failure_whatever_its_code() {
        for code in [
            ErrorCode::Unauthenticated,
            ErrorCode::Forbidden,
            ErrorCode::Conflict,
            ErrorCode::Validation,
            ErrorCode::NotFound,
            ErrorCode::Internal,
        ] {
            let proxy: anyhow::Result<Outcome<StoryDto>> = Ok(Outcome::Error {
                code,
                message: "HTTP 401 Unauthorized".into(),
                envelope: false,
                retry_after: None,
                current: None,
            });
            assert!(
                matches!(conv(proxy, Reply::Story), Completion::Failed(Failure::Server(m)) if m == "HTTP 401 Unauthorized"),
                "{code:?}"
            );
        }
    }

    fn answer_with(status: u16, body: &'static str) -> reqwest::Result<reqwest::Response> {
        Ok(axum::http::Response::builder()
            .status(status)
            .body(body)
            .unwrap()
            .into())
    }

    fn answer(status: u16) -> reqwest::Result<reqwest::Response> {
        answer_with(status, "")
    }

    const ENVELOPE_401: &str = r#"{"outcome":"error","code":"unauthenticated","message":"no"}"#;
    const ENVELOPE_403: &str = r#"{"outcome":"error","code":"forbidden","message":"no"}"#;

    #[tokio::test]
    async fn the_servers_own_401_envelope_ends_the_stream_as_unauthorized() {
        let t = std::time::Duration::from_secs(10);
        assert_eq!(
            headers(t, async { answer_with(401, ENVELOPE_401) })
                .await
                .err(),
            Some(Ended::Unauthorized)
        );
    }

    #[tokio::test]
    async fn a_proxys_401_is_a_failure_that_reconnects() {
        let t = std::time::Duration::from_secs(10);
        for body in [
            "",
            "Unauthorized",
            "<html>login</html>",
            ENVELOPE_403,
            r#"{"a":1}"#,
        ] {
            assert_eq!(
                headers(t, async { answer_with(401, body) }).await.err(),
                Some(Ended::Failed("HTTP 401 Unauthorized".into())),
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn other_refusals_are_failures_even_with_an_unauthenticated_envelope() {
        let t = std::time::Duration::from_secs(10);
        assert_eq!(
            headers(t, async { answer_with(403, ENVELOPE_403) })
                .await
                .err(),
            Some(Ended::Failed("HTTP 403 Forbidden".into()))
        );
        assert_eq!(
            headers(t, async { answer_with(403, ENVELOPE_401) })
                .await
                .err(),
            Some(Ended::Failed("HTTP 403 Forbidden".into()))
        );
        assert_eq!(
            headers(t, async { answer(500) }).await.err(),
            Some(Ended::Failed("HTTP 500 Internal Server Error".into()))
        );
        assert!(headers(t, async { answer(200) }).await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_whose_headers_never_arrive_fails_at_the_connect_timeout() {
        let t = std::time::Duration::from_secs(10);
        let start = tokio::time::Instant::now();
        let pending = std::future::pending::<reqwest::Result<reqwest::Response>>();
        // The outer bound is longer: without the inner one the open would wait until it fires.
        let r = tokio::time::timeout(t * 2, headers(t, pending))
            .await
            .expect("the open is bounded");
        assert_eq!(r.err(), Some(Ended::Failed("no answer".into())));
        assert_eq!(start.elapsed(), t);
    }
}
