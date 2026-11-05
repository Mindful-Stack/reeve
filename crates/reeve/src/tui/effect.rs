//! What update functions ask the runtime to do (kb/ADR-0011 R4), and what comes back.
use crate::tui::sse::Frame;
use reeve_protocol::{
    EditItem, EpicDto, InstanceConfigDto, Key, Me, MoveStory, Reason, StoryDto, StoryWithReadiness,
};
use std::{fmt, sync::Arc, time::Duration};

/// The core numbers every request when it is sent (`State::next_seq`).
pub type Seq = u64;

/// `sent`: the request's own number. `at_done`: the highest number sent when its completion was processed. A board
/// load with `sent <= at_done` of a later write completion started before that write committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub sent: Seq,
    pub at_done: Seq,
}

#[expect(
    clippy::large_enum_variant,
    reason = "built once per request and moved through the channel; boxing buys nothing measurable"
)]
#[derive(Debug, Clone)]
pub enum Request {
    Me,
    Config,
    /// `GET /stories?done=recent`.
    Stories,
    Epics,
    Move {
        key: Key,
        body: MoveStory,
    },
    Edit {
        key: Key,
        body: EditItem,
    },
}

impl Request {
    /// The story a write is for; `None` for a read. The core words a write's lost answer and conflict by it.
    pub fn write_key(&self) -> Option<&Key> {
        match self {
            Request::Move { key, .. } | Request::Edit { key, .. } => Some(key),
            Request::Me | Request::Config | Request::Stories | Request::Epics => None,
        }
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "built once per request and moved through the channel; boxing buys nothing measurable"
)]
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Me(Me),
    Config(InstanceConfigDto),
    Stories(Vec<StoryWithReadiness>),
    Epics(Vec<EpicDto>),
    Story(StoryDto),
}

#[expect(
    clippy::large_enum_variant,
    reason = "built once per request and moved through the channel; boxing buys nothing measurable"
)]
#[derive(Debug, Clone, PartialEq)]
pub enum Completion {
    Ok(Reply),
    /// `overridable`: the server's word on whether an interactive override can get past `reasons`.
    Refused {
        reasons: Vec<Reason>,
        overridable: bool,
    },
    Failed(Failure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Unauthenticated,
    Forbidden(String),
    /// A stale version. The envelope carries only the current version, not the current state.
    Conflict,
    Validation(String),
    NotFound(String),
    /// The server failed (`internal`), or a body it did not write (a proxy's page, whatever its status) answered.
    Server(String),
    /// No answer: unreachable, timed out, or an unreadable body. A write may or may not have committed.
    Transport(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The server closed the stream (a dead token or a failed read; the 401 comes on reconnect).
    Closed,
    /// The reconnect itself was refused with 401.
    Unauthorized,
    /// No bytes for the idle timeout.
    Idle,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Opened,
    Frame(Frame),
    Ended(Ended),
}

pub type Route<M> = Box<dyn FnOnce(Stamp, Completion) -> M + Send>;
pub type StreamRoute<M> = Arc<dyn Fn(StreamEvent) -> M + Send + Sync>;

#[expect(
    clippy::large_enum_variant,
    reason = "built once per request and moved through the channel; boxing buys nothing measurable"
)]
pub enum Effect<M> {
    Http {
        req: Request,
        route: Route<M>,
    },
    After {
        delay: Duration,
        msg: M,
    },
    /// Opens the event stream, replacing any open one; `after_id` is sent as `Last-Event-ID`.
    Connect {
        after_id: Option<i64>,
        route: StreamRoute<M>,
    },
    Disconnect,
    Open(String),
    Copy(String),
}

impl<M: 'static> Effect<M> {
    pub fn http(req: Request, route: impl FnOnce(Stamp, Completion) -> M + Send + 'static) -> Self {
        Effect::Http {
            req,
            route: Box::new(route),
        }
    }
    pub fn after(delay: Duration, msg: M) -> Self {
        Effect::After { delay, msg }
    }
    /// Lifts a slice's effect into the core's message type.
    pub fn map<N: 'static>(self, f: impl Fn(M) -> N + Send + Sync + 'static) -> Effect<N> {
        match self {
            Effect::Http { req, route } => Effect::Http {
                req,
                route: Box::new(move |s, c| f(route(s, c))),
            },
            Effect::After { delay, msg } => Effect::After { delay, msg: f(msg) },
            Effect::Connect { after_id, route } => Effect::Connect {
                after_id,
                route: Arc::new(move |e| f(route(e))),
            },
            Effect::Disconnect => Effect::Disconnect,
            Effect::Open(u) => Effect::Open(u),
            Effect::Copy(t) => Effect::Copy(t),
        }
    }
}

impl<M: fmt::Debug> fmt::Debug for Effect<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::Http { req, .. } => write!(f, "Http({req:?})"),
            Effect::After { delay, msg } => write!(f, "After({delay:?}, {msg:?})"),
            Effect::Connect { after_id, .. } => write!(f, "Connect({after_id:?})"),
            Effect::Disconnect => write!(f, "Disconnect"),
            Effect::Open(u) => write!(f, "Open({u:?})"),
            Effect::Copy(t) => write!(f, "Copy({t:?})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    #[allow(clippy::large_enum_variant)]
    enum Inner {
        A(u8),
        Done(Seq, Completion),
        Ev(StreamEvent),
    }
    #[derive(Debug, PartialEq)]
    enum Outer {
        Wrapped(Inner),
    }

    #[test]
    fn map_keeps_the_delay_and_maps_the_message() {
        match Effect::after(Duration::from_millis(400), Inner::A(1)).map(Outer::Wrapped) {
            Effect::After { delay, msg } => {
                assert_eq!(delay, Duration::from_millis(400));
                assert_eq!(msg, Outer::Wrapped(Inner::A(1)));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn map_keeps_the_request_and_composes_the_route() {
        let e = Effect::http(Request::Stories, |s, c| Inner::Done(s.sent, c)).map(Outer::Wrapped);
        let Effect::Http { req, route } = e else {
            panic!()
        };
        assert!(matches!(req, Request::Stories));
        let m = route(
            Stamp {
                sent: 9,
                at_done: 9,
            },
            Completion::Failed(Failure::Conflict),
        );
        assert_eq!(
            m,
            Outer::Wrapped(Inner::Done(9, Completion::Failed(Failure::Conflict)))
        );
    }

    #[test]
    fn map_composes_the_stream_route() {
        let route: StreamRoute<Inner> = Arc::new(Inner::Ev);
        let e = Effect::Connect {
            after_id: Some(4),
            route,
        }
        .map(Outer::Wrapped);
        let Effect::Connect { after_id, route } = e else {
            panic!()
        };
        assert_eq!(after_id, Some(4));
        assert_eq!(
            route(StreamEvent::Opened),
            Outer::Wrapped(Inner::Ev(StreamEvent::Opened))
        );
    }
}
