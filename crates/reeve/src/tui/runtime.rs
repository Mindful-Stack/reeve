//! Runs effects (kb/ADR-0011 R3): each becomes a task whose result returns as a message on the one channel. Tasks hold
//! the HTTP handle and the sender, never state.
use crate::tui::app::Msg;
use crate::tui::effect::{Effect, Ended, Seq, StreamEvent, StreamRoute};
use crate::tui::http::Http;
use crate::tui::sse::Parser;
use futures_util::StreamExt;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc::UnboundedSender, task::AbortHandle};

/// The side effects that leave the process: the browser and the clipboard.
pub trait Io: Send + Sync + 'static {
    /// Opens `url` and waits for the opener to finish; may block (the runtime calls it off the loop).
    fn open(&self, url: &str) -> Result<(), String>;
    fn copy(&self, text: &str) -> Result<(), String>;
}

pub struct Runtime {
    tx: UnboundedSender<Msg>,
    http: Http,
    io: Arc<dyn Io>,
    idle: Duration,
    stream: Option<AbortHandle>,
    tasks: Vec<AbortHandle>,
}

impl Runtime {
    pub fn new(
        tx: UnboundedSender<Msg>,
        http: Http,
        io: Arc<dyn Io>,
        o: &crate::tui::Options,
    ) -> Self {
        Self {
            tx,
            http,
            io,
            idle: o.idle_timeout,
            stream: None,
            tasks: Vec::new(),
        }
    }

    fn spawn(&mut self, fut: impl std::future::Future<Output = ()> + Send + 'static) {
        self.tasks.retain(|h| !h.is_finished());
        self.tasks.push(tokio::spawn(fut).abort_handle());
    }

    /// `sent` is the request's number for an `Http` effect (the driver numbers it), ignored otherwise.
    pub fn run(&mut self, e: Effect<Msg>, sent: Seq) {
        let tx = self.tx.clone();
        match e {
            Effect::Http { req, route } => {
                let http = self.http.clone();
                let write = req.write_key().cloned();
                self.spawn(async move {
                    let completion = http.request(req).await;
                    let _ = tx.send(Msg::Done {
                        sent,
                        completion,
                        route,
                        write,
                    });
                });
            }
            Effect::After { delay, msg } => {
                // The deadline is fixed now, at dispatch, not when the task is first polled.
                let deadline = tokio::time::Instant::now() + delay;
                self.spawn(async move {
                    tokio::time::sleep_until(deadline).await;
                    let _ = tx.send(msg);
                });
            }
            Effect::Connect { after_id, route } => {
                if let Some(h) = self.stream.take() {
                    h.abort();
                }
                let (http, idle) = (self.http.clone(), self.idle);
                self.stream =
                    Some(tokio::spawn(stream_task(http, after_id, route, tx, idle)).abort_handle());
            }
            Effect::Disconnect => {
                // With no stream open this is a no-op.
                if let Some(h) = self.stream.take() {
                    h.abort();
                }
            }
            Effect::Open(url) => {
                // Opening waits for the opener's exit status (an installed xdg-open with no browser fails), so it runs
                // off the loop.
                let io = self.io.clone();
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = io.open(&url) {
                        let _ = tx.send(Msg::Status(format!("cannot open {url}: {e}")));
                    }
                });
            }
            Effect::Copy(text) => {
                if let Err(e) = self.io.copy(&text) {
                    let _ = tx.send(Msg::Status(format!("cannot copy: {e}")));
                }
            }
        }
    }

    /// Aborts every task and the stream (on quit).
    pub fn shutdown(&mut self) {
        for h in self.tasks.drain(..) {
            h.abort();
        }
        if let Some(h) = self.stream.take() {
            h.abort();
        }
    }
}

async fn stream_task(
    http: Http,
    after: Option<i64>,
    route: StreamRoute<Msg>,
    tx: UnboundedSender<Msg>,
    idle: Duration,
) {
    let send = |ev| {
        let _ = tx.send(route(ev));
    };
    let resp = match http.open_events(after).await {
        Ok(r) => r,
        Err(end) => return send(StreamEvent::Ended(end)),
    };
    send(StreamEvent::Opened);
    let mut body = resp.bytes_stream();
    let mut parser = Parser::default();
    loop {
        // The idle watchdog: no byte (not even a keep-alive) for `idle` drops the stream.
        match tokio::time::timeout(idle, body.next()).await {
            Err(_) => return send(StreamEvent::Ended(Ended::Idle)),
            Ok(None) => return send(StreamEvent::Ended(Ended::Closed)),
            Ok(Some(Err(e))) => {
                return send(StreamEvent::Ended(Ended::Failed(format!(
                    "{:#}",
                    anyhow::Error::from(e)
                ))));
            }
            // A too-large frame leaves the parser undefined; it is dropped with the task.
            Ok(Some(Ok(bytes))) => match parser.push(&bytes) {
                Ok(frames) => frames.into_iter().for_each(|f| send(StreamEvent::Frame(f))),
                Err(_) => return send(StreamEvent::Ended(Ended::Failed("frame too large".into()))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::Msg;
    use std::time::Duration;

    struct NoIo;
    impl Io for NoIo {
        fn open(&self, _: &str) -> Result<(), String> {
            Err("no browser here".into())
        }
        fn copy(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn runtime() -> (Runtime, tokio::sync::mpsc::UnboundedReceiver<Msg>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let o = crate::tui::Options::default();
        (
            Runtime::new(
                tx,
                Http::new("http://127.0.0.1:9".into(), "t".into(), &o),
                Arc::new(NoIo),
                &o,
            ),
            rx,
        )
    }
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_timer_delivers_its_message_at_its_delay_and_not_before() {
        let (mut rt, mut rx) = runtime();
        rt.run(
            Effect::after(Duration::from_millis(400), Msg::Status("fired".into())),
            0,
        );
        tokio::time::advance(Duration::from_millis(399)).await;
        settle().await;
        assert!(rx.try_recv().is_err());
        tokio::time::advance(Duration::from_millis(1)).await;
        settle().await;
        assert!(matches!(rx.try_recv(), Ok(Msg::Status(s)) if s == "fired"));
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_aborts_pending_work() {
        let (mut rt, mut rx) = runtime();
        rt.run(
            Effect::after(Duration::from_millis(400), Msg::Status("late".into())),
            0,
        );
        rt.shutdown();
        tokio::time::advance(Duration::from_secs(1)).await;
        settle().await;
        assert!(rx.try_recv().is_err());
    }

    /// Real time (sockets): port 9 on loopback refuses at once, so each stream task ends within milliseconds.
    #[tokio::test]
    async fn a_connect_replaces_the_open_stream() {
        let (mut rt, mut rx) = runtime();
        let tagged = |epoch: u8| -> StreamRoute<Msg> {
            Arc::new(move |ev| Msg::Status(format!("{epoch} {ev:?}")))
        };
        rt.run(
            Effect::Connect {
                after_id: None,
                route: tagged(1),
            },
            0,
        );
        rt.run(
            Effect::Connect {
                after_id: None,
                route: tagged(2),
            },
            0,
        );
        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the second stream ends");
        // The cause chain is kept (reqwest's own message names only the URL).
        assert!(
            matches!(&first, Some(Msg::Status(s)) if s.starts_with("2 Ended(Failed(") && s.contains("Connection refused")),
            "{first:?}"
        );
        // Had the first stream not been aborted, its refusal would arrive in this window.
        let more = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        assert!(more.is_err(), "{more:?}");
    }

    #[tokio::test]
    async fn a_disconnect_with_no_stream_open_leaves_the_runtime_working() {
        let (mut rt, mut rx) = runtime();
        rt.run(Effect::Disconnect, 0);
        rt.run(
            Effect::after(Duration::from_millis(1), Msg::Status("still here".into())),
            0,
        );
        let m = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap();
        assert!(matches!(m, Some(Msg::Status(s)) if s == "still here"));
    }

    #[tokio::test]
    async fn an_open_failure_comes_back_as_a_status() {
        let (mut rt, mut rx) = runtime();
        rt.run(Effect::Open("https://x".into()), 0);
        let m = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap();
        assert!(matches!(m, Some(Msg::Status(s)) if s.contains("no browser here")));
    }
}
