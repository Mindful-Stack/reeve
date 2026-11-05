//! A scripted planner for the TUI's integration tests: canned answers per endpoint, a queue of event-stream
//! connections, and a record of what the TUI sent. No database; no domain rules.
//!
//! Every body is built from `reeve_protocol` types (the envelope, the story, the configuration, the activity entry),
//! so a protocol change breaks this fixture rather than letting it drift from the server.
#![allow(dead_code)] // each test file uses part of it

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{Stream, StreamExt, stream::BoxStream};
use ratatui::backend::TestBackend;
use reeve::tui::{Driver, Input, Io, Options, Setup};
use reeve_protocol::{
    ActivityEntryDto, DeliveryState, EpicDto, ErrorCode, InstanceConfigDto, Kind, Me, Readiness,
    Response as Envelope, Role, StoryDto, StoryWithReadiness, TokenKind, Via,
};
use serde_json::Value;
use std::{
    collections::VecDeque,
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

pub enum Conn {
    /// Answer 200 and send these frames; then close, or hold the connection open.
    Frames { frames: Vec<String>, close: bool },
    /// Answer with this status and the server's error envelope for it.
    Status(u16),
    /// Accept the request and never send the response headers.
    NoHeaders,
}

pub enum Answer {
    Json(u16, Value),
    /// Never answer.
    Hang,
}

#[derive(Default)]
pub struct Seen {
    /// Each `/events` connection's `Last-Event-ID`, in arrival order.
    pub last_event_ids: Vec<Option<String>>,
    pub stories_calls: usize,
    /// Stream bodies dropped before they ended: the client closed or aborted the connection.
    pub closed_streams: usize,
    pub moves: Vec<Value>,
}

pub struct Script {
    pub me: Me,
    pub stories: Mutex<Vec<StoryWithReadiness>>,
    pub stories_hang: Mutex<bool>,
    pub events: Mutex<VecDeque<Conn>>,
    pub moves: Mutex<VecDeque<Answer>>,
    pub seen: Mutex<Seen>,
}

pub fn me(kind: TokenKind) -> Me {
    Me {
        email: "dan@example.com".into(),
        role: Role::Member,
        token_kind: kind,
    }
}

impl Script {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            me: me(TokenKind::Interactive),
            stories: Mutex::new(vec![]),
            stories_hang: Mutex::new(false),
            events: Mutex::new(VecDeque::new()),
            moves: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Seen::default()),
        })
    }
    pub fn seen<R>(&self, f: impl FnOnce(&Seen) -> R) -> R {
        f(&self.seen.lock().unwrap())
    }
}

fn wire<T: serde::Serialize>(e: Envelope<T>) -> Value {
    serde_json::to_value(e).expect("envelopes serialise")
}

/// `{"outcome":"ok","value":…}`.
pub fn ok<T: serde::Serialize>(value: T) -> Value {
    wire(Envelope::Ok { value })
}

/// `{"outcome":"error","code":…,"message":…}`.
pub fn error(code: ErrorCode) -> Value {
    wire(Envelope::<()>::Error {
        code,
        message: format!("scripted {code:?}"),
        current: None,
    })
}

/// A version conflict as the server answers it (with 409): the row's version now.
pub fn conflict(current: i64) -> Value {
    wire(Envelope::<()>::Error {
        code: ErrorCode::Conflict,
        message: "version conflict".into(),
        current: Some(current),
    })
}

/// The error code the server pairs with an HTTP status.
fn code_for(status: u16) -> ErrorCode {
    match status {
        401 => ErrorCode::Unauthenticated,
        403 => ErrorCode::Forbidden,
        404 => ErrorCode::NotFound,
        409 => ErrorCode::Conflict,
        400 | 422 => ErrorCode::Validation,
        _ => ErrorCode::Internal,
    }
}

pub fn story(
    key: &str,
    state: DeliveryState,
    version: i64,
    owner: Option<&str>,
) -> StoryWithReadiness {
    StoryWithReadiness {
        story: StoryDto {
            key: key.parse().unwrap(),
            title: format!("Title of {key}"),
            kind: Kind::Story,
            epic: None,
            description: String::new(),
            criteria: vec![],
            owner: owner.map(str::to_string),
            labels: vec![],
            rank: 0,
            blocked_by: vec![],
            delivery: state,
            resolution: None,
            external_ref: None,
            version,
            prs: vec![],
            goals: vec![],
        },
        readiness: Readiness {
            ready: true,
            reasons: vec![],
        },
    }
}

/// The server's first frame: `event: cursor`, the start id as both id and data.
pub fn cursor(id: i64) -> String {
    format!("event: cursor\nid: {id}\ndata: {id}\n\n")
}

/// An `event: activity` frame whose id is the entry id and whose data is the entry's JSON.
pub fn activity(id: i64, action: &str, key: &str, payload: Value) -> String {
    let dto = ActivityEntryDto {
        id,
        at: "2026-10-28T10:00:00Z".parse().unwrap(),
        actor: "anna@example.com".into(),
        via: Via::Interactive,
        entity_type: "story".into(),
        entity_key: key.into(),
        action: action.into(),
        payload,
    };
    let data = serde_json::to_string(&dto).unwrap();
    format!("event: activity\nid: {id}\ndata: {data}\n\n")
}

/// A stream body that counts itself in `closed_streams` when it is dropped before it ended (the client closed or
/// aborted); a body that ran to its end (`close: true`) is not counted.
struct Counted {
    inner: BoxStream<'static, Result<Bytes, Infallible>>,
    script: Arc<Script>,
    ended: bool,
}

impl Stream for Counted {
    type Item = Result<Bytes, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let p = self.inner.poll_next_unpin(cx);
        if matches!(p, Poll::Ready(None)) {
            self.ended = true;
        }
        p
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        if !self.ended {
            self.script.seen.lock().unwrap().closed_streams += 1;
        }
    }
}

async fn stories(State(s): State<Arc<Script>>) -> Response {
    s.seen.lock().unwrap().stories_calls += 1;
    let hang = *s.stories_hang.lock().unwrap();
    if hang {
        std::future::pending::<()>().await;
    }
    let rows = s.stories.lock().unwrap().clone();
    Json(ok(rows)).into_response()
}

async fn mv(
    State(s): State<Arc<Script>>,
    Path(_key): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    s.seen.lock().unwrap().moves.push(body);
    let next = s.moves.lock().unwrap().pop_front();
    match next {
        Some(Answer::Json(code, v)) => {
            (StatusCode::from_u16(code).unwrap(), Json(v)).into_response()
        }
        Some(Answer::Hang) => std::future::pending().await,
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error(ErrorCode::Internal)),
        )
            .into_response(),
    }
}

async fn events(State(s): State<Arc<Script>>, headers: HeaderMap) -> Response {
    let id = headers
        .get("last-event-id")
        .map(|v| v.to_str().unwrap().to_string());
    s.seen.lock().unwrap().last_event_ids.push(id);
    let next = s.events.lock().unwrap().pop_front();
    // Unscripted connections stay open after the cursor frame, as the real server's do.
    match next.unwrap_or(Conn::Frames {
        frames: vec![cursor(0)],
        close: false,
    }) {
        Conn::Status(code) => (
            StatusCode::from_u16(code).unwrap(),
            Json(error(code_for(code))),
        )
            .into_response(),
        Conn::NoHeaders => std::future::pending().await,
        Conn::Frames { frames, close } => {
            let head = futures_util::stream::iter(
                frames
                    .into_iter()
                    .map(|f| Ok::<_, Infallible>(Bytes::from(f))),
            );
            let tail: BoxStream<'static, Result<Bytes, Infallible>> = if close {
                futures_util::stream::empty().boxed()
            } else {
                futures_util::stream::pending().boxed()
            };
            let body = Counted {
                inner: head.chain(tail).boxed(),
                script: s.clone(),
                ended: false,
            };
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(body))
                .unwrap()
        }
    }
}

/// Serves the script on a free loopback port; returns the base url.
pub async fn serve(s: Arc<Script>) -> String {
    let me = s.me.clone();
    let config = InstanceConfigDto {
        key_prefix: "ABC".into(),
        horizon_buckets: vec![],
        repos: vec![],
        sign_in_configured: false,
        import_enabled: false,
        done_window_days: 14,
    };
    let app = Router::new()
        .route("/api/v1/me", get(move || async move { Json(ok(me)) }))
        .route(
            "/api/v1/config",
            get(move || async move { Json(ok(config)) }),
        )
        .route("/api/v1/stories", get(stories))
        .route(
            "/api/v1/epics",
            get(|| async { Json(ok(Vec::<EpicDto>::new())) }),
        )
        .route("/api/v1/stories/{key}/move", post(mv))
        .route("/api/v1/events", get(events))
        .with_state(s);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[derive(Default)]
pub struct Recording {
    pub opened: Mutex<Vec<String>>,
    pub copied: Mutex<Vec<String>>,
}
impl Io for Recording {
    fn open(&self, url: &str) -> Result<(), String> {
        self.opened.lock().unwrap().push(url.into());
        Ok(())
    }
    fn copy(&self, text: &str) -> Result<(), String> {
        self.copied.lock().unwrap().push(text.into());
        Ok(())
    }
}

/// Short timings for scripted tests (real time, bounded waits). The idle timeout is longer than any test's
/// observation period, so a quiet stream never reconnects (each reconnect schedules a load, which would satisfy a "the
/// board refetched" wait on its own); only the watchdog test shortens it.
pub fn quick() -> Options {
    Options {
        request_timeout: Duration::from_secs(1),
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(30),
        backoff: vec![Duration::from_millis(50)],
    }
}

pub struct Tui {
    pub d: Driver<TestBackend>,
    pub keys: UnboundedSender<Input>,
    pub io: Arc<Recording>,
}

pub fn tui(url: &str, token: &str, me: Option<Me>, options: Options) -> Tui {
    let (keys, rx) = unbounded_channel();
    let input =
        futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|i| (i, rx)) });
    let io = Arc::new(Recording::default());
    let setup = Setup {
        url: url.into(),
        token: token.into(),
        me,
        reeve_token_set: false,
        color: false,
        options,
    };
    let d = Driver::new(TestBackend::new(140, 40), input, io.clone(), setup).unwrap();
    Tui { d, keys, io }
}

/// The header (the screen's first row).
pub fn header(d: &Driver<TestBackend>) -> String {
    d.screen().lines().next().unwrap_or_default().to_string()
}

impl Tui {
    pub fn press(&self, code: crossterm::event::KeyCode) {
        use crossterm::event::{Event, KeyEvent, KeyModifiers};
        let mods = match code {
            crossterm::event::KeyCode::Char(c) if c.is_ascii_uppercase() => KeyModifiers::SHIFT,
            _ => KeyModifiers::NONE,
        };
        self.keys
            .send(Input::Event(Event::Key(KeyEvent::new(code, mods))))
            .unwrap();
    }
    pub fn type_text(&self, s: &str) {
        for c in s.chars() {
            self.press(crossterm::event::KeyCode::Char(c));
        }
    }
    /// Steps the driver until `done` holds, failing after `within` with the screen in the message.
    pub async fn until(
        &mut self,
        what: &str,
        within: Duration,
        mut done: impl FnMut(&Driver<TestBackend>) -> bool,
    ) {
        let deadline = tokio::time::Instant::now() + within;
        while !done(&self.d) {
            match tokio::time::timeout_at(deadline, self.d.step()).await {
                Err(_) => panic!(
                    "{what}: not within {within:?}; screen:\n{}",
                    self.d.screen()
                ),
                Ok(Some(code)) => panic!(
                    "{what}: the TUI exited with {code}; screen:\n{}",
                    self.d.screen()
                ),
                Ok(None) => {}
            }
        }
    }
    /// Steps until the stream is live and the loads its opening triggered have landed: a baseline for counting the
    /// loads a later action causes. (Opening the stream schedules one debounced load 500 ms later.)
    pub async fn settled(&mut self, s: &Script) {
        self.until("live", Duration::from_secs(5), |d| {
            header(d).contains("live")
        })
        .await;
        // The first load, then the one the stream's opening schedules 500 ms later.
        self.until("the open-triggered load", Duration::from_secs(5), |_| {
            s.seen(|x| x.stories_calls) >= 2
        })
        .await;
        self.idle_for(Duration::from_millis(300)).await;
        let n = s.seen(|x| x.stories_calls);
        self.idle_for(Duration::from_millis(300)).await;
        assert_eq!(
            s.seen(|x| x.stories_calls),
            n,
            "the board is not settled: loads keep arriving"
        );
    }
    /// Steps for `d` of real time (to show something does *not* happen).
    pub async fn idle_for(&mut self, d: Duration) {
        let deadline = tokio::time::Instant::now() + d;
        while let Ok(r) = tokio::time::timeout_at(deadline, self.d.step()).await {
            if let Some(code) = r {
                panic!(
                    "the TUI exited with {code} while idling; screen:\n{}",
                    self.d.screen()
                );
            }
        }
    }
}
