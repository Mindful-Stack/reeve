//! A scripted planner for the TUI's integration tests: canned answers per endpoint, a queue of event-stream
//! connections plus a live channel into the open one, held replies for urgency writes, and a record of what the TUI
//! sent. No database; no domain rules.
//!
//! Every body is built from `reeve_protocol` types (the envelope, the story, the configuration, the activity entry),
//! so a protocol change breaks this fixture rather than letting it drift from the server.
#![allow(dead_code)] // each test file uses part of it

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{Stream, StreamExt, stream::BoxStream};
use ratatui::backend::TestBackend;
use reeve::tui::{Driver, Input, Io, Options, Setup};
use reeve_protocol::{
    ActivityEntryDto, DeliveryState, EpicDto, ErrorCode, InstanceConfigDto, ItemRefDto, ItemType,
    Kind, Me, Readiness, Response as Envelope, Role, StoryDto, StoryWithReadiness, TokenKind,
    UrgentMarkDto, Via,
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
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    oneshot,
};

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
    /// Each urgency write (`POST`/`DELETE /stories/{key}/urgent`), in arrival order.
    pub urgent: Vec<Sent>,
}

/// A request as the TUI sent it: method, path and raw body (empty for a body-less `DELETE`).
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub method: String,
    pub path: String,
    pub body: String,
}

/// The release of one held urgency reply: until [`Held::release`] runs, the TUI's request waits for its answer.
pub struct Held(oneshot::Sender<(u16, Value)>);
impl Held {
    /// Answers the held request. Released before any urgency write arrived, the reply is buffered and answers the next
    /// one at once.
    pub fn release(self, status: u16, body: Value) {
        self.0
            .send((status, body))
            .expect("the held reply's receiver is gone (the request was dropped)");
    }
}

pub struct Script {
    pub me: Me,
    pub stories: Mutex<Vec<StoryWithReadiness>>,
    pub stories_hang: Mutex<bool>,
    pub events: Mutex<VecDeque<Conn>>,
    pub moves: Mutex<VecDeque<Answer>>,
    /// Held replies for urgency writes, taken in order; a write with none queued is answered 500.
    pub urgent: Mutex<VecDeque<oneshot::Receiver<(u16, Value)>>>,
    /// The live channel: frames pushed here reach the open event-stream connection after its scripted frames,
    /// without a reconnect. The first held-open connection takes the receiver.
    live_tx: UnboundedSender<String>,
    live_rx: Mutex<Option<UnboundedReceiver<String>>>,
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
        let (live_tx, live_rx) = unbounded_channel();
        Arc::new(Self {
            me: me(TokenKind::Interactive),
            stories: Mutex::new(vec![]),
            stories_hang: Mutex::new(false),
            events: Mutex::new(VecDeque::new()),
            moves: Mutex::new(VecDeque::new()),
            urgent: Mutex::new(VecDeque::new()),
            live_tx,
            live_rx: Mutex::new(Some(live_rx)),
            seen: Mutex::new(Seen::default()),
        })
    }
    pub fn seen<R>(&self, f: impl FnOnce(&Seen) -> R) -> R {
        f(&self.seen.lock().unwrap())
    }
    /// What `GET /stories` answers from now on. Change it before releasing a write, or the refetch the write's
    /// completion triggers brings back the old board.
    pub fn set_stories(&self, rows: Vec<StoryWithReadiness>) {
        *self.stories.lock().unwrap() = rows;
    }
    /// Queues a held reply for the next urgency write; the request waits until the returned handle is released.
    pub fn hold_urgent(&self) -> Held {
        let (tx, rx) = oneshot::channel();
        self.urgent.lock().unwrap().push_back(rx);
        Held(tx)
    }
    /// Sends `frame` on the open event-stream connection, which is not reconnected. Panics when no connection holds
    /// the live channel (none opened yet, or it was dropped), so a test cannot pass by a reconnect instead. Delivery past
    /// that is best effort: a frame lost in transit shows up as the test's wait timing out.
    pub fn push_live(&self, frame: String) {
        assert!(
            self.live_rx.lock().unwrap().is_none(),
            "no event-stream connection has taken the live channel yet"
        );
        self.live_tx
            .send(frame)
            .expect("the connection holding the live channel is gone");
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
            urgent_mark: None,
            urgent_sources: vec![],
        },
        readiness: Readiness {
            ready: true,
            reasons: vec![],
        },
    }
}

/// The story as the server sends it when urgent through these sources (its own key for its own mark).
pub fn urgent_via(mut s: StoryWithReadiness, sources: &[(&str, ItemType)]) -> StoryWithReadiness {
    s.story.urgent_sources = sources
        .iter()
        .map(|(k, item)| ItemRefDto {
            key: k.parse().unwrap(),
            item: *item,
        })
        .collect();
    s
}

/// The story as the server sends it right after a mark on it: its own active mark, which makes it its own source.
pub fn marked_urgent(s: StoryWithReadiness, reason: Option<&str>) -> StoryWithReadiness {
    let key = s.story.key.to_string();
    let mut s = urgent_via(s, &[(&key, ItemType::Story)]);
    s.story.urgent_mark = Some(UrgentMarkDto {
        at: "2026-11-08T12:00:00Z".parse().unwrap(),
        by: "dan@example.com".into(),
        reason: reason.map(str::to_string),
        expires_at: "2026-11-22T12:00:00Z".parse().unwrap(),
        active: true,
        done: false,
    });
    s
}

/// The server's first frame: `event: cursor`, the start id as both id and data.
pub fn cursor(id: i64) -> String {
    format!("event: cursor\nid: {id}\ndata: {id}\n\n")
}

/// An `event: activity` frame about a story, whose id is the entry id and whose data is the entry's JSON.
pub fn activity(id: i64, action: &str, key: &str, payload: Value) -> String {
    activity_on(id, "story", action, key, payload)
}

/// An `event: activity` frame about any entity type (`"story"`, `"epic"`, `"initiative"`).
pub fn activity_on(id: i64, entity_type: &str, action: &str, key: &str, payload: Value) -> String {
    let dto = ActivityEntryDto {
        id,
        at: "2026-10-28T10:00:00Z".parse().unwrap(),
        actor: "anna@example.com".into(),
        via: Via::Interactive,
        entity_type: entity_type.into(),
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

/// `POST` or `DELETE /stories/{key}/urgent`: records the request, then answers with the next held reply once it is
/// released (500 when none is queued).
async fn urgent(State(s): State<Arc<Script>>, method: Method, uri: Uri, body: Bytes) -> Response {
    s.seen.lock().unwrap().urgent.push(Sent {
        method: method.to_string(),
        path: uri.path().to_string(),
        body: String::from_utf8(body.to_vec()).unwrap(),
    });
    let held = s.urgent.lock().unwrap().pop_front();
    let Some(rx) = held else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error(ErrorCode::Internal)),
        )
            .into_response();
    };
    match rx.await {
        Ok((code, v)) => (StatusCode::from_u16(code).unwrap(), Json(v)).into_response(),
        // The test dropped the handle without releasing it: never answer.
        Err(_) => std::future::pending().await,
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
            // A held-open connection carries the live channel if no earlier connection took it.
            let live = if close {
                None
            } else {
                s.live_rx.lock().unwrap().take()
            };
            let tail: BoxStream<'static, Result<Bytes, Infallible>> = match (close, live) {
                (true, _) => futures_util::stream::empty().boxed(),
                (false, Some(rx)) => futures_util::stream::unfold(rx, |mut rx| async move {
                    rx.recv()
                        .await
                        .map(|f| (Ok::<_, Infallible>(Bytes::from(f)), rx))
                })
                // Defensive: `Script` keeps the sender, so the channel never ends; if it did, the connection stays open.
                .chain(futures_util::stream::pending())
                .boxed(),
                (false, None) => futures_util::stream::pending().boxed(),
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
        .route("/api/v1/stories/{key}/urgent", post(urgent).delete(urgent))
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
