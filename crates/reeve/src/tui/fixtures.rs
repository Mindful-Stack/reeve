//! Protocol values for the TUI's unit tests.
use crate::tui::app::{Msg, Start, State};
use crate::tui::board;
use crate::tui::effect::{Completion, Effect, Reply, Request, Route, Seq, Stamp, StreamEvent};
use crate::tui::live;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use reeve_protocol::{
    DeliveryState, Key, Kind, Me, Readiness, Reason, Role, StoryDto, StoryWithReadiness, TokenKind,
};
use std::time::Instant;

/// The wall clock `Sim` draws with.
pub const WALL: &str = "2026-11-10T12:00:00Z";

pub fn key(s: &str) -> Key {
    s.parse().unwrap()
}

/// A ready story of kind story, unowned, in epic ABC-100 with one criterion (a ready story of kind story always has
/// an epic and criteria, kb/ADR-0027 R4), in `state`, at `version`. Tests that need it not ready say so with `not_ready`.
pub fn story(k: &str, state: DeliveryState, version: i64) -> StoryWithReadiness {
    StoryWithReadiness {
        story: StoryDto {
            key: key(k),
            title: format!("Title of {k}"),
            kind: Kind::Story,
            epic: Some(key("ABC-100")),
            description: String::new(),
            criteria: vec![reeve_protocol::CriterionDto {
                text: "it works".into(),
                done: false,
            }],
            owner: None,
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
            promise: None,
        },
        readiness: Readiness {
            ready: true,
            reasons: vec![],
        },
    }
}

pub fn owned(mut s: StoryWithReadiness, email: &str) -> StoryWithReadiness {
    s.story.owner = Some(email.into());
    s
}

/// A mark made by dana two days before `WALL`, without a reason, expiring ten days after `WALL`.
pub fn own_mark() -> reeve_protocol::UrgentMarkDto {
    reeve_protocol::UrgentMarkDto {
        at: "2026-11-08T12:00:00Z".parse().unwrap(),
        by: "dana@example.com".into(),
        reason: None,
        expires_at: "2026-11-20T12:00:00Z".parse().unwrap(),
        active: true,
        done: false,
    }
}

/// The story as the server sends an urgent one: its own active mark, which makes it its own source.
pub fn urgent(mut s: StoryWithReadiness) -> StoryWithReadiness {
    s.story.urgent_mark = Some(own_mark());
    s.story.urgent_sources = vec![reeve_protocol::ItemRefDto {
        key: s.story.key.clone(),
        item: reeve_protocol::ItemType::Story,
    }];
    s
}

/// A story that is urgent only through its epic: the server lists the epic as the one source and the story has no mark.
pub fn urgent_via_epic(mut s: StoryWithReadiness) -> StoryWithReadiness {
    s.story.urgent_sources = vec![reeve_protocol::ItemRefDto {
        key: key("ABC-100"),
        item: reeve_protocol::ItemType::Epic,
    }];
    s
}

/// A story that is not done, with an active own mark and no sources. The server never sends this shape for a live
/// story (an active mark makes it its own source), so a client that reads the mark would wrongly show it as urgent.
pub fn marked_without_sources(mut s: StoryWithReadiness) -> StoryWithReadiness {
    s.story.urgent_mark = Some(own_mark());
    s
}

/// A done story with an active own mark and no sources: the server's shape, since a done story's mark makes nothing urgent.
pub fn done_marked(mut s: StoryWithReadiness) -> StoryWithReadiness {
    s.story.delivery = DeliveryState::Done;
    s.story.urgent_mark = Some(reeve_protocol::UrgentMarkDto {
        done: true,
        ..own_mark()
    });
    s
}

/// The story as the server sends one promised through its epic ABC-100 (the epic's date counts), overdue by 3 days: the
/// state, the days and the effective date are the server's, never the TUI's.
pub fn promised(mut s: StoryWithReadiness) -> StoryWithReadiness {
    let fact = |on: &str, to: &str| reeve_protocol::PromiseFactDto {
        on: on.parse().unwrap(),
        by: "dana@example.com".into(),
        at: "2026-11-01T09:00:00Z".parse().unwrap(),
        to: Some(to.into()),
    };
    s.story.promise = Some(reeve_protocol::StoryPromiseDto {
        own: Some(fact("2026-11-30", "Beta")),
        epic: Some(reeve_protocol::EpicPromiseRefDto {
            key: key("ABC-100"),
            fact: fact("2026-11-07", "Acme"),
        }),
        on: "2026-11-07".parse().unwrap(),
        holder: reeve_protocol::PromiseHolder::Epic,
        state: Some(reeve_protocol::PromiseState::Overdue),
        due: None,
        days: Some(3),
    });
    s
}

pub fn not_ready(mut s: StoryWithReadiness, reasons: Vec<Reason>) -> StoryWithReadiness {
    s.readiness = Readiness {
        ready: false,
        reasons,
    };
    s
}

/// A refusal of `reasons` as the server sends it: the flag comes from the server's own constructor
/// (`Response::refused`), so no test hand-writes which reasons an override can get past.
pub fn refused(reasons: Vec<Reason>) -> Completion {
    match reeve_protocol::Response::<reeve_protocol::Empty>::refused(reasons) {
        reeve_protocol::Response::Refused {
            reasons,
            overridable,
        } => Completion::Refused {
            reasons,
            overridable,
        },
        _ => unreachable!("Response::refused builds a refusal"),
    }
}

pub fn stamp(sent: u64, at_done: u64) -> Stamp {
    Stamp { sent, at_done }
}

pub fn stories(list: Vec<StoryWithReadiness>) -> Completion {
    Completion::Ok(Reply::Stories(list))
}

pub fn epic(k: &str, title: &str) -> reeve_protocol::EpicDto {
    reeve_protocol::EpicDto {
        key: key(k),
        title: title.into(),
        initiative: None,
        brief: String::new(),
        discovery_complete: None,
        external_ref: None,
        version: 1,
        urgent_mark: None,
        urgent_sources: vec![],
        promise: None,
    }
}

/// The instance configuration as the server sends it.
pub fn config() -> Completion {
    Completion::Ok(Reply::Config(reeve_protocol::InstanceConfigDto {
        key_prefix: "ABC".into(),
        horizon_buckets: vec![],
        repos: vec![],
        sign_in_configured: false,
        import_enabled: false,
        done_window_days: 14,
    }))
}

/// The requests among `effects`, in order.
pub fn requests<M>(effects: &[Effect<M>]) -> Vec<&Request> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Http { req, .. } => Some(req),
            _ => None,
        })
        .collect()
}

/// A board showing everyone's stories (`all`), loaded once with `list`, nothing locked.
pub fn board_with(list: Vec<StoryWithReadiness>) -> board::State {
    let mut b = board::State::test_showing_all();
    let cx = board::Ctx {
        me: Some("dan@example.com"),
        wide: true,
        seq: 0,
    };
    let _ = b.on_note(&crate::tui::note::Note::Refetch, &cx);
    let _ = b.update(
        board::Msg::Loaded {
            stamp: stamp(1, 1),
            result: stories(list),
        },
        &cx,
    );
    b
}

/// The root reducer driven the way the runtime drives it: requests are held until a test answers them, timers
/// until it fires them.
pub(crate) struct Sim {
    pub s: State,
    pub pending: Vec<(Seq, Request, Route<Msg>)>,
    pub timers: Vec<Msg>,
    pub other: Vec<Effect<Msg>>,
    pub now: Instant,
}

pub fn me(kind: TokenKind) -> Me {
    Me {
        email: "dan@example.com".into(),
        role: Role::Member,
        token_kind: kind,
    }
}

impl Sim {
    pub fn start(me: Option<Me>) -> Sim {
        let now = Instant::now();
        let (s, fx) = State::new(
            Start {
                url: "https://reeve.example.com".into(),
                me,
                reeve_token_set: false,
                width: 140,
                height: 40,
                color: false,
                backoff: live::BACKOFF.to_vec(),
            },
            now,
        );
        let mut sim = Sim {
            s,
            pending: vec![],
            timers: vec![],
            other: vec![],
            now,
        };
        sim.s.set_wall(WALL.parse().unwrap());
        sim.absorb(fx);
        sim
    }
    /// A started TUI whose first load answered `list` (and the config).
    pub fn loaded(kind: TokenKind, list: Vec<StoryWithReadiness>) -> Sim {
        let mut sim = Sim::start(Some(me(kind)));
        sim.reply(|r| matches!(r, Request::Config), config());
        sim.reply(|r| matches!(r, Request::Stories), stories(list));
        sim
    }
    pub fn absorb(&mut self, fx: Vec<Effect<Msg>>) {
        for e in fx {
            match e {
                Effect::Http { req, route } => {
                    let seq = self.s.next_seq();
                    self.pending.push((seq, req, route));
                }
                Effect::After { msg, .. } => self.timers.push(msg),
                other => self.other.push(other),
            }
        }
    }
    pub fn send(&mut self, m: Msg) {
        let fx = self.s.update(m, self.now);
        self.absorb(fx);
    }
    pub fn press(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.send(Msg::Input(Event::Key(KeyEvent::new(code, mods))));
    }
    pub fn char(&mut self, c: char) {
        let m = if c.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        self.press(KeyCode::Char(c), m);
    }
    /// Completes the oldest pending request matching `pick`.
    pub fn reply(&mut self, pick: impl Fn(&Request) -> bool, c: Completion) {
        let i = self
            .pending
            .iter()
            .position(|(_, r, _)| pick(r))
            .expect("no such pending request");
        let (sent, req, route) = self.pending.remove(i);
        self.send(Msg::Done {
            sent,
            completion: c,
            route,
            write: req.write_key().cloned(),
        });
    }
    /// Fires every pending timer in the order it was scheduled, ignoring the delays; timers they schedule wait for
    /// the next call.
    pub fn fire_timers(&mut self) {
        for m in std::mem::take(&mut self.timers) {
            self.send(m);
        }
    }
    pub fn has_pending(&self, pick: impl Fn(&Request) -> bool) -> bool {
        self.pending.iter().any(|(_, r, _)| pick(r))
    }
    /// Delivers a stream event through the latest Connect's route.
    pub fn stream(&mut self, ev: StreamEvent) {
        let m = self
            .other
            .iter()
            .rev()
            .find_map(|e| match e {
                Effect::Connect { route, .. } => Some(route(ev.clone())),
                _ => None,
            })
            .expect("no stream");
        self.send(m);
    }
    pub fn status(&self) -> String {
        self.s
            .ui
            .status
            .as_ref()
            .map(|s| s.text.clone())
            .unwrap_or_default()
    }
    /// Draws the state into a `w`×`h` test terminal: the rows as text (`view::rows`, which skips the continuation cell
    /// after a wide character) and the buffer, for style assertions at chosen cells. A row's character count is its
    /// column count only while no cell holds a combining mark or a ZWJ sequence.
    pub fn render(&mut self, w: u16, h: u16) -> (Vec<String>, ratatui::buffer::Buffer) {
        self.s.ui.width = w;
        self.s.ui.height = h;
        self.s.set_wall(WALL.parse().unwrap());
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        term.draw(|f| crate::tui::view::draw(f, &self.s, self.now))
            .unwrap();
        let buf = term.backend().buffer().clone();
        (crate::tui::view::rows(&buf), buf)
    }
}
