//! The terminal board (`reeve tui`); kb/ADR-0011. `Driver` is the whole application with an injectable backend,
//! input stream, server and side-effect sink; `main` wires it to the real terminal.
mod app;
mod board;
mod drawer;
mod effect;
#[cfg(test)]
mod fixtures;
mod generation;
mod http;
mod keys;
mod line;
mod live;
mod login;
mod moves;
mod note;
mod runtime;
#[cfg(test)]
mod scenarios;
mod set_epic;
mod sse;
mod term;
mod urgent;
mod view;
pub(crate) mod words;

pub use runtime::Io;

use crate::client::Outcome;
use crate::tui::effect::Effect;
use futures_util::{Stream, StreamExt, stream::BoxStream};
use ratatui::{Terminal, backend::Backend};
use reeve_protocol::{ErrorCode, Me};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// Timing the I/O layer uses; production takes `Default`, the scripted-server tests shorten it.
#[derive(Debug, Clone)]
pub struct Options {
    /// Total time for one request; a write that exceeds it is reported as "no answer" and the board refetches.
    pub request_timeout: std::time::Duration,
    /// Time to establish the event stream's connection and receive its response headers (the server reads the
    /// newest activity id before it answers); the idle watchdog covers the body after that.
    pub connect_timeout: std::time::Duration,
    /// The stream is dropped after this long without a byte (three server keep-alives).
    pub idle_timeout: std::time::Duration,
    /// Reconnect and `/me` retry delays; the last repeats.
    pub backoff: Vec<std::time::Duration>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            request_timeout: std::time::Duration::from_secs(15),
            connect_timeout: std::time::Duration::from_secs(10),
            idle_timeout: std::time::Duration::from_secs(45),
            backoff: live::BACKOFF.to_vec(),
        }
    }
}

/// What the input stream yields.
pub enum Input {
    Event(crossterm::event::Event),
    /// The terminal failed: exit 1. (Signals do not pass through here: `term::watch_signals` handles them, and a
    /// stream that ends exits cleanly.)
    Error(std::io::Error),
}

/// How many runtime messages `step` folds in before it draws.
const DRAIN: usize = 64;

/// What the driver starts from: the resolved login, the first `/me` (if it answered), and the timing.
pub struct Setup {
    pub url: String,
    pub token: String,
    /// `None`: the first `/me` could not be reached; the TUI starts `connecting`.
    pub me: Option<Me>,
    pub reeve_token_set: bool,
    pub color: bool,
    pub options: Options,
}

/// The whole application: the core state, the runtime that runs its effects, a terminal and an input stream.
pub struct Driver<B: Backend> {
    terminal: Terminal<B>,
    state: app::State,
    rx: UnboundedReceiver<app::Msg>,
    runtime: runtime::Runtime,
    input: BoxStream<'static, Input>,
    tick: tokio::time::Interval,
    /// Why the driver stopped with 1: the terminal failed (read or draw).
    failure: Option<std::io::Error>,
}

impl<B: Backend> Driver<B>
where
    B::Error: std::error::Error + Send + Sync + 'static,
{
    /// Builds the core from `setup` and the backend's size, and hands the first effects to the runtime. Needs a tokio
    /// runtime (effects spawn tasks).
    pub fn new(
        backend: B,
        input: impl Stream<Item = Input> + Send + 'static,
        io: Arc<dyn Io>,
        setup: Setup,
    ) -> std::io::Result<Self> {
        let terminal = Terminal::new(backend).map_err(std::io::Error::other)?;
        let size = terminal.size().map_err(std::io::Error::other)?;
        let (tx, rx) = unbounded_channel();
        let http = http::Http::new(setup.url.clone(), setup.token, &setup.options);
        let runtime = runtime::Runtime::new(tx, http, io, &setup.options);
        let (state, fx) = app::State::new(
            app::Start {
                url: setup.url,
                me: setup.me,
                reeve_token_set: setup.reeve_token_set,
                width: size.width,
                height: size.height,
                color: setup.color,
                backoff: setup.options.backoff.clone(),
            },
            Instant::now(),
        );
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut d = Self {
            terminal,
            state,
            rx,
            runtime,
            input: input.boxed(),
            tick,
            failure: None,
        };
        d.run_effects(fx);
        Ok(d)
    }

    /// Hands effects to the runtime, numbering each request as it is taken: `next_seq` once per
    /// `Http` effect, in output order, before any later message is processed.
    fn run_effects(&mut self, fx: Vec<Effect<app::Msg>>) {
        for e in fx {
            let sent = if matches!(e, Effect::Http { .. }) {
                self.state.next_seq()
            } else {
                0
            };
            self.runtime.run(e, sent);
        }
    }

    pub fn draw(&mut self) -> std::io::Result<()> {
        let now = Instant::now();
        self.state.set_wall(chrono::Utc::now());
        let s = &self.state;
        self.terminal
            .draw(|f| view::draw(f, s, now))
            .map_err(std::io::Error::other)?;
        Ok(())
    }

    fn finish(&mut self, code: i32) -> i32 {
        self.runtime.shutdown();
        code
    }

    fn fail(&mut self, e: std::io::Error) -> i32 {
        self.failure = Some(e);
        self.finish(1)
    }

    fn apply(&mut self, msg: app::Msg) {
        let fx = self.state.update(msg, Instant::now());
        self.run_effects(fx);
    }

    /// Processes one input, message or tick, then up to `DRAIN` runtime messages already waiting, and redraws once;
    /// `Some(code)` when the TUI is done (with 1, `failure` says why).
    pub async fn step(&mut self) -> Option<i32> {
        let msg = tokio::select! {
            m = self.rx.recv() => m.expect("the runtime holds a sender"),
            i = self.input.next() => match i {
                Some(Input::Event(e)) => app::Msg::Input(e),
                None => return Some(self.finish(0)),
                Some(Input::Error(e)) => return Some(self.fail(e)),
            },
            _ = self.tick.tick() => app::Msg::Tick,
        };
        self.apply(msg);
        for _ in 0..DRAIN {
            if self.state.quit {
                break;
            }
            match self.rx.try_recv() {
                Ok(m) => self.apply(m),
                Err(_) => break,
            }
        }
        if self.state.quit {
            return Some(self.finish(0));
        }
        if let Err(e) = self.draw() {
            return Some(self.fail(e));
        }
        None
    }

    /// Draws, then steps until the TUI is done: the exit status, or the terminal's error (exit 1).
    pub async fn run(mut self) -> std::io::Result<i32> {
        if let Err(e) = self.draw() {
            self.finish(1);
            return Err(e);
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("REEVE_TUI_PANIC_AFTER_DRAW").is_some() {
            panic!("forced by REEVE_TUI_PANIC_AFTER_DRAW");
        }
        loop {
            if let Some(code) = self.step().await {
                return match self.failure.take() {
                    Some(e) => Err(e),
                    None => Ok(code),
                };
            }
        }
    }
}

impl Driver<ratatui::backend::TestBackend> {
    /// The screen as text, one line per row (tests).
    pub fn screen(&self) -> String {
        view::rows(self.terminal.backend().buffer()).join("\n")
    }
    /// The selected story's key (tests: the screen cannot show which card is selected as text).
    pub fn selected(&self) -> Option<String> {
        self.state.board.selected().map(ToString::to_string)
    }
}

/// What the first `/me` decides: `Ok(Some)` start signed in, `Ok(None)` start connecting (no answer, a server error,
/// or a proxy's page whatever its status, which the running app retries), `Err` exit 1 with this message.
fn first_me(answer: anyhow::Result<Outcome<Me>>) -> Result<Option<Me>, String> {
    match answer {
        Ok(Outcome::Ok(me)) => Ok(Some(me)),
        // Only the server's own envelope means what its code says: a proxy's bare 401 is not a refused login.
        Ok(Outcome::Error {
            envelope: false, ..
        }) => Ok(None),
        Ok(Outcome::Error {
            code: ErrorCode::Unauthenticated | ErrorCode::Forbidden,
            ..
        }) => Err("the stored login was refused: run reeve auth login".into()),
        Ok(Outcome::Error {
            code: ErrorCode::Internal,
            ..
        })
        | Err(_) => Ok(None),
        Ok(Outcome::Error { code, message, .. }) => {
            let code = serde_json::to_value(code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            Err(format!("unexpected answer from /me: {code}: {message}"))
        }
        Ok(Outcome::Refused { reasons, .. }) => Err(format!(
            "unexpected answer from /me: refused:\n{}",
            crate::render::reasons(&reasons)
        )),
    }
}

/// Runs `reeve tui` and returns its exit status.
pub async fn main() -> i32 {
    let login = match login::resolve() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e:#}");
            return 1;
        }
    };
    let options = Options::default();
    // The first `/me`, before the alternate screen; the line keeps a slow host from
    // looking like a hang.
    eprintln!("connecting to {}…", login.url);
    let probe = crate::client::Client::with_timeout(
        login.url.clone(),
        login.token.clone(),
        options.request_timeout,
    );
    let me = match first_me(probe.get::<Me>("/me").await) {
        Ok(me) => me,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    // Before `enter`, so a signal between the two still exits (restoring is a no-op until entered).
    term::watch_signals();
    let guard = match term::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: cannot set up the terminal: {e}");
            return 1;
        }
    };
    let setup = Setup {
        url: login.url,
        token: login.token,
        me,
        reeve_token_set: login.reeve_token_set,
        color,
        options,
    };
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let result = match Driver::new(backend, term::input(), Arc::new(term::TerminalIo), setup) {
        Ok(d) => d.run().await,
        Err(e) => Err(e),
    };
    drop(guard);
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: terminal: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures::me;
    use ratatui::backend::TestBackend;
    use reeve_protocol::TokenKind;

    struct NoIo;
    impl Io for NoIo {
        fn open(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn copy(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn driver(input: impl Stream<Item = Input> + Send + 'static) -> Driver<TestBackend> {
        let setup = Setup {
            url: "http://127.0.0.1:1".into(),
            token: "t".into(),
            me: None,
            reeve_token_set: false,
            color: false,
            options: Options::default(),
        };
        Driver::new(TestBackend::new(140, 40), input, Arc::new(NoIo), setup).unwrap()
    }

    #[tokio::test]
    async fn a_terminal_read_error_ends_the_run_with_that_error() {
        let input = futures_util::stream::iter([Input::Error(std::io::Error::other("tty gone"))]);
        let err = driver(input).run().await.unwrap_err();
        assert_eq!(err.to_string(), "tty gone");
    }

    #[tokio::test]
    async fn an_input_stream_that_ends_exits_cleanly() {
        assert_eq!(
            driver(futures_util::stream::empty()).run().await.unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn one_step_folds_in_every_waiting_message_before_it_draws() {
        let mut d = driver(futures_util::stream::pending());
        let (tx, rx) = unbounded_channel();
        d.rx = rx;
        for t in ["a", "b", "c"] {
            tx.send(app::Msg::Status(t.into())).unwrap();
        }
        assert_eq!(d.step().await, None);
        assert_eq!(
            d.state.ui.status.as_ref().map(|s| s.text.as_str()),
            Some("c")
        );
        assert!(d.rx.try_recv().is_err());
    }

    #[test]
    fn the_first_me_decides_how_the_tui_starts() {
        let err = |code, message: &str| {
            first_me(Ok(Outcome::Error {
                code,
                message: message.into(),
                envelope: true,
                retry_after: None,
                current: None,
            }))
        };
        assert!(matches!(
            first_me(Ok(Outcome::Ok(me(TokenKind::Interactive)))),
            Ok(Some(_))
        ));
        assert!(matches!(err(ErrorCode::Internal, "bad gateway"), Ok(None)));
        assert!(matches!(
            first_me(Err(anyhow::anyhow!("refused"))),
            Ok(None)
        ));
        for code in [ErrorCode::Unauthenticated, ErrorCode::Forbidden] {
            assert_eq!(
                err(code, "no").unwrap_err(),
                "the stored login was refused: run reeve auth login"
            );
        }
        assert_eq!(
            err(ErrorCode::NotFound, "no route").unwrap_err(),
            "unexpected answer from /me: not_found: no route"
        );
        // A proxy's page is not the server's answer: whatever its status, the TUI starts connecting and retries.
        for code in [
            ErrorCode::Unauthenticated,
            ErrorCode::Forbidden,
            ErrorCode::NotFound,
            ErrorCode::Internal,
        ] {
            assert!(
                matches!(
                    first_me(Ok(Outcome::Error {
                        code,
                        message: "HTTP 401 Unauthorized".into(),
                        envelope: false,
                        retry_after: None,
                        current: None,
                    })),
                    Ok(None)
                ),
                "{code:?}"
            );
        }
    }
}
