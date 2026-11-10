//! The core (kb/ADR-0011 R2): composition, input mode and key routing, reply classification, the one-shot actions
//! (take, clearing an urgent mark, copy, open), identity, and routing notes between owners. Free of I/O, like every
//! update.
use crate::tui::effect::{Completion, Effect, Failure, Reply, Request, Route, Seq, Stamp};
use crate::tui::generation::Gen;
use crate::tui::keys::{self, Action};
use crate::tui::note::{Note, Out, WriteOutcome};
use crate::tui::{board, drawer, live, moves, set_epic, words};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use reeve_protocol::{DeliveryState, EditItem, Key, Me, TokenKind, normalise_email};
use std::{
    fmt,
    time::{Duration, Instant},
};

/// How long an action's outcome holds the status line before the ticker returns.
pub const STATUS_FOR: Duration = Duration::from_secs(5);
/// From this many terminal columns the board shows five columns.
pub const WIDE: u16 = 120;
pub const SIGNED_OUT: &str = "signed out — run reeve auth login";
/// Why `u` does nothing for a done story: a mark on it would make nothing urgent (kb/ADR-0017 R3).
const NOT_WHEN_DONE: &str = "done stories cannot be urgent";
/// Why `U` does nothing for a story without a mark of its own (one urgent through its lineage included).
const NO_MARK: &str = "no urgent mark";

#[derive(Debug)]
pub struct Session {
    pub url: String,
    pub me: Option<Me>,
    /// Final: no message clears it (kb/ADR-0015 R5).
    pub signed_out: bool,
    pub reeve_token_set: bool,
    me_attempt: usize,
    me_gen: Gen,
}

impl Session {
    /// Why writes are disabled, or `None` when they are allowed.
    pub fn read_only(&self) -> Option<&'static str> {
        if self.signed_out {
            return Some("signed out");
        }
        match &self.me {
            None => Some("connecting"),
            Some(m) if m.token_kind != TokenKind::Interactive => Some("automation token"),
            Some(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub text: String,
    pub until: Instant,
}

#[derive(Debug, Default)]
pub struct Ui {
    /// The drawer is open, scrolled this many lines.
    pub drawer: Option<u16>,
    pub help: bool,
    /// The action menu is open with this row highlighted.
    pub menu: Option<usize>,
    pub status: Option<Status>,
    /// The terminal's width: the layout (columns or list) decides navigation, so the core keeps it.
    pub width: u16,
    /// The terminal's height: with the width, the drawer's size, which caps its scroll.
    pub height: u16,
    /// `NO_COLOR` is unset.
    pub color: bool,
    /// The wall clock of the frame being drawn. The driver reads it once per frame and records it here, so the view and
    /// the drawer's scroll cap word the dates the same way.
    pub wall: chrono::DateTime<chrono::Utc>,
}

/// What draws over the board and receives the keys. At most one is open at a time: opening one closes the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Help,
    ActionMenu,
    MovePrompt,
    MovePicker,
    EpicPicker,
    Search,
    FilterMenu,
}

/// One row of the action menu.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub action: Action,
    /// Why the action would do nothing for this story now; the row is shown dimmed with it.
    pub disabled: Option<String>,
}

pub struct State {
    pub session: Session,
    pub board: board::State,
    pub moves: moves::State,
    pub set_epic: set_epic::State,
    pub live: live::State,
    pub ui: Ui,
    pub quit: bool,
    seq: Seq,
    me_backoff: Vec<Duration>,
    /// The board and the stream have been started (once `/me` is known).
    started: bool,
}

pub struct Start {
    pub url: String,
    /// `None`: the first `/me` could not be reached; the TUI starts `connecting`.
    pub me: Option<Me>,
    pub reeve_token_set: bool,
    pub width: u16,
    pub height: u16,
    pub color: bool,
    /// The reconnect and `/me` retry schedule (1, 2, 5, 10, 30 s).
    pub backoff: Vec<Duration>,
}

pub enum Msg {
    Input(Event),
    Board(board::Msg),
    Move(moves::Msg),
    SetEpic(set_epic::Msg),
    Live(live::Msg),
    /// A finished request, before the core stamps and classifies it. `write`: the story a write was for
    /// (`Request::write_key`), so the core words its lost answer or conflict once, for every owner.
    Done {
        sent: Seq,
        completion: Completion,
        route: Route<Msg>,
        write: Option<Key>,
    },
    MeLoaded(Completion),
    RetryMe(u64),
    TakeDone {
        key: Key,
        stamp: Stamp,
        result: Completion,
    },
    ClearUrgentDone {
        key: Key,
        stamp: Stamp,
        result: Completion,
    },
    /// From the runtime: an open or copy failed.
    Status(String),
    Tick,
}

impl fmt::Debug for Msg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Msg::Input(e) => write!(f, "Input({e:?})"),
            Msg::Board(m) => write!(f, "Board({m:?})"),
            Msg::Move(m) => write!(f, "Move({m:?})"),
            Msg::SetEpic(m) => write!(f, "SetEpic({m:?})"),
            Msg::Live(m) => write!(f, "Live({m:?})"),
            Msg::Done {
                sent,
                completion,
                write,
                ..
            } => write!(f, "Done({sent}, {completion:?}, {write:?})"),
            Msg::MeLoaded(c) => write!(f, "MeLoaded({c:?})"),
            Msg::RetryMe(g) => write!(f, "RetryMe({g})"),
            Msg::TakeDone { key, result, .. } => write!(f, "TakeDone({key}, {result:?})"),
            Msg::ClearUrgentDone { key, result, .. } => {
                write!(f, "ClearUrgentDone({key}, {result:?})")
            }
            Msg::Status(s) => write!(f, "Status({s})"),
            Msg::Tick => write!(f, "Tick"),
        }
    }
}

fn bcx<'a>(session: &'a Session, ui: &Ui, seq: Seq) -> board::Ctx<'a> {
    board::Ctx {
        me: session.me.as_ref().map(|m| m.email.as_str()),
        wide: ui.width >= WIDE,
        seq,
    }
}

impl State {
    pub fn new(start: Start, now: Instant) -> (Self, Vec<Effect<Msg>>) {
        let mut s = State {
            session: Session {
                url: start.url,
                me: start.me,
                signed_out: false,
                reeve_token_set: start.reeve_token_set,
                me_attempt: 0,
                me_gen: Gen::default(),
            },
            board: board::State::default(),
            moves: moves::State::default(),
            set_epic: set_epic::State::default(),
            live: live::State::with_backoff(start.backoff.clone()),
            ui: Ui {
                width: start.width,
                height: start.height,
                color: start.color,
                ..Ui::default()
            },
            quit: false,
            seq: 0,
            me_backoff: start.backoff,
            started: false,
        };
        // Identity first: with `/me` unknown only `/me` is asked; the board, the
        // configuration and the stream start once it answers.
        let fx = if s.session.me.is_some() {
            s.begin_session(now)
        } else {
            vec![Effect::http(Request::Me, |_, c| Msg::MeLoaded(c))]
        };
        (s, fx)
    }

    fn begin_session(&mut self, now: Instant) -> Vec<Effect<Msg>> {
        if std::mem::replace(&mut self.started, true) {
            return Vec::new();
        }
        let o = self.board.start();
        let mut fx = self.absorb(o, Msg::Board, now);
        fx.extend(self.live_msg(live::Msg::Start, now));
        fx
    }

    /// The number of the last request sent.
    pub fn seq(&self) -> Seq {
        self.seq
    }

    /// Records the wall clock the next frame is drawn with. Only the driver (and the test harness) calls this.
    pub fn set_wall(&mut self, wall: chrono::DateTime<chrono::Utc>) {
        self.ui.wall = wall;
    }

    /// Numbers a request as it is sent. Only the driver (and the test harness) calls this.
    pub fn next_seq(&mut self) -> Seq {
        self.seq += 1;
        self.seq
    }

    fn set_status(&mut self, text: impl Into<String>, now: Instant) {
        self.ui.status = Some(Status {
            text: text.into(),
            until: now + STATUS_FOR,
        });
    }

    fn absorb<M: 'static>(
        &mut self,
        o: Out<M>,
        wrap: fn(M) -> Msg,
        now: Instant,
    ) -> Vec<Effect<Msg>> {
        let o = o.map(wrap);
        let mut fx = o.effects;
        fx.extend(self.notes(o.notes, now));
        fx
    }

    fn board_msg(&mut self, m: board::Msg, now: Instant) -> Vec<Effect<Msg>> {
        let cx = bcx(&self.session, &self.ui, self.seq);
        let o = self.board.update(m, &cx);
        let mut fx = self.absorb(o, Msg::Board, now);
        fx.extend(self.board_changed(now));
        fx
    }

    fn moves_msg(&mut self, m: moves::Msg, now: Instant) -> Vec<Effect<Msg>> {
        let cx = moves::Ctx {
            board: self.board.view(),
            read_only: self.session.read_only(),
        };
        let o = self.moves.update(m, &cx);
        self.absorb(o, Msg::Move, now)
    }

    fn set_epic_msg(&mut self, m: set_epic::Msg, now: Instant) -> Vec<Effect<Msg>> {
        let cx = set_epic::Ctx {
            board: self.board.view(),
            read_only: self.session.read_only(),
        };
        let o = self.set_epic.update(m, &cx);
        self.absorb(o, Msg::SetEpic, now)
    }

    fn live_msg(&mut self, m: live::Msg, now: Instant) -> Vec<Effect<Msg>> {
        let o = self.live.update(m, &live::Ctx { now });
        self.absorb(o, Msg::Live, now)
    }

    /// The board's data may have changed: owners holding a version re-check it.
    fn board_changed(&mut self, now: Instant) -> Vec<Effect<Msg>> {
        let mut fx = self.set_epic_msg(set_epic::Msg::BoardChanged, now);
        fx.extend(self.moves_msg(moves::Msg::BoardChanged, now));
        fx
    }

    fn notes(&mut self, notes: Vec<Note>, now: Instant) -> Vec<Effect<Msg>> {
        let mut fx = Vec::new();
        for n in notes {
            match n {
                Note::Status(t) => self.set_status(t, now),
                Note::SignedOut => fx.extend(self.sign_out(now)),
                Note::SelectionLost(k) => {
                    if self.ui.drawer.take().is_some() {
                        self.set_status(format!("{k} is no longer on the board"), now);
                    }
                }
                n @ (Note::WriteStarted(_)
                | Note::WriteFinished { .. }
                | Note::Refetch
                | Note::Invalidate) => {
                    let cx = bcx(&self.session, &self.ui, self.seq);
                    let o = self.board.on_note(&n, &cx);
                    fx.extend(self.absorb(o, Msg::Board, now));
                    fx.extend(self.board_changed(now));
                }
            }
        }
        fx
    }

    fn sign_out(&mut self, now: Instant) -> Vec<Effect<Msg>> {
        if self.session.signed_out {
            return Vec::new();
        }
        self.session.signed_out = true;
        // Every follow-up, picker and prompt goes, so no late reply can start a write (kb/ADR-0015 R5).
        let mut fx = self.moves_msg(moves::Msg::Reset, now);
        fx.extend(self.set_epic_msg(set_epic::Msg::Reset, now));
        fx.extend(self.live_msg(live::Msg::Stop, now));
        self.set_status(SIGNED_OUT, now);
        fx
    }

    /// Classification every completion gets before its owner sees it. A write's lost answer and conflict are worded
    /// here, by the write's story (`write`); owners word only their success and refusal.
    fn classify(&mut self, c: &Completion, write: Option<&Key>, now: Instant) -> Vec<Effect<Msg>> {
        match c {
            Completion::Failed(Failure::Unauthenticated) => self.sign_out(now),
            Completion::Failed(Failure::Conflict) => {
                if let Some(k) = write {
                    self.set_status(
                        format!("{k} changed on the server — refreshing; try again"),
                        now,
                    );
                }
                self.notes(vec![Note::Refetch], now)
            }
            // The owner's WriteFinished (`WriteOutcome::Unknown`) refetches; a read's lost answer is its owner's to show.
            Completion::Failed(Failure::Transport(_)) => {
                if let Some(k) = write {
                    self.set_status(format!("{k}: no answer — refreshing"), now);
                }
                Vec::new()
            }
            Completion::Failed(Failure::Forbidden(m)) => {
                self.set_status(format!("forbidden: {m}"), now);
                Vec::new()
            }
            Completion::Failed(
                Failure::Validation(m) | Failure::NotFound(m) | Failure::Server(m),
            ) => {
                self.set_status(m.clone(), now);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Every open overlay, in the order key routing would pick them.
    fn overlays(&self) -> Vec<Overlay> {
        [
            (self.ui.help, Overlay::Help),
            (self.ui.menu.is_some(), Overlay::ActionMenu),
            (self.moves.prompt().is_some(), Overlay::MovePrompt),
            (self.moves.picker().is_some(), Overlay::MovePicker),
            (self.set_epic.picker().is_some(), Overlay::EpicPicker),
            (self.board.search().is_some(), Overlay::Search),
            (self.board.filter_menu().is_some(), Overlay::FilterMenu),
        ]
        .into_iter()
        .filter_map(|(open, o)| open.then_some(o))
        .collect()
    }

    /// The overlay that receives the keys, if one is open.
    pub fn overlay(&self) -> Option<Overlay> {
        self.overlays().first().copied()
    }

    /// Closes `o` through its owner.
    fn close(&mut self, o: Overlay, now: Instant) -> Vec<Effect<Msg>> {
        match o {
            Overlay::Help => {
                self.ui.help = false;
                Vec::new()
            }
            Overlay::ActionMenu => {
                self.ui.menu = None;
                Vec::new()
            }
            Overlay::MovePrompt | Overlay::MovePicker => self.moves_msg(moves::Msg::Close, now),
            Overlay::EpicPicker => self.set_epic_msg(set_epic::Msg::Close, now),
            Overlay::FilterMenu | Overlay::Search => self.board_msg(board::Msg::CloseOverlays, now),
        }
    }

    /// One overlay at a time across owners: whichever opened during this message (a move prompt opened by a timer or
    /// a late refusal, say) closes every other, which would otherwise be drawn over the one that receives the keys.
    fn one_overlay(&mut self, before: &[Overlay], now: Instant) -> Vec<Effect<Msg>> {
        let open = self.overlays();
        if open.len() <= 1 {
            return Vec::new();
        }
        let keep = open
            .iter()
            .copied()
            .find(|o| !before.contains(o))
            .unwrap_or(open[0]);
        let mut fx = Vec::new();
        for o in open {
            // An owner of two (moves; the board) closes both on one message, so re-check before each. Each owner
            // keeps its own two apart, so `keep` never shares an owner with one closed here.
            if o != keep && self.overlays().contains(&o) {
                fx.extend(self.close(o, now));
            }
        }
        fx
    }

    pub fn update(&mut self, msg: Msg, now: Instant) -> Vec<Effect<Msg>> {
        let before = self.overlays();
        let mut fx = self.step(msg, now);
        fx.extend(self.one_overlay(&before, now));
        // Sign-out is final for reads too (kb/ADR-0015 R5): the one choke point for every owner's requests.
        if self.session.signed_out {
            fx.retain(|e| !matches!(e, Effect::Http { .. } | Effect::Connect { .. }));
        }
        fx
    }

    fn step(&mut self, msg: Msg, now: Instant) -> Vec<Effect<Msg>> {
        match msg {
            Msg::Done {
                sent,
                completion,
                route,
                write,
            } => {
                let stamp = Stamp {
                    sent,
                    at_done: self.seq,
                };
                let was_signed_out = self.session.signed_out;
                let mut fx = self.classify(&completion, write.as_ref(), now);
                let m = route(stamp, completion);
                fx.extend(self.step(m, now));
                // The owner ran second and may have worded the 401 its own way; the sign-out's wording wins.
                if !was_signed_out && self.session.signed_out {
                    self.set_status(SIGNED_OUT, now);
                }
                fx
            }
            Msg::Input(Event::Key(k)) if k.kind != KeyEventKind::Release => self.key(k, now),
            Msg::Input(Event::Resize(w, h)) => {
                self.ui.width = w;
                self.ui.height = h;
                if let (Some(max), Some(s)) = (self.drawer_max(), self.ui.drawer.as_mut()) {
                    *s = (*s).min(max);
                }
                Vec::new()
            }
            Msg::Input(_) | Msg::Tick => Vec::new(),
            Msg::Board(m) => self.board_msg(m, now),
            Msg::Move(m) => self.moves_msg(m, now),
            Msg::SetEpic(m) => self.set_epic_msg(m, now),
            Msg::Live(m) => self.live_msg(m, now),
            Msg::Status(t) => {
                self.set_status(t, now);
                Vec::new()
            }
            // A late success after a sign-out (overlapping `/me` requests from `r`) must not restart the session.
            Msg::MeLoaded(Completion::Ok(Reply::Me(me))) if !self.session.signed_out => {
                self.session.me = Some(me);
                self.ui.status = None;
                self.begin_session(now)
            }
            // The server's own refusal of the login ends it, as at startup (`first_me`): a 401 was signed out by
            // `classify`; a 403 (a disabled user) is final the same way.
            Msg::MeLoaded(Completion::Failed(Failure::Forbidden(_)))
                if !self.session.signed_out && self.session.me.is_none() =>
            {
                self.sign_out(now)
            }
            // No answer, or a proxy's 5xx while the server restarts; a late failure after a success changes nothing.
            Msg::MeLoaded(Completion::Failed(Failure::Transport(e) | Failure::Server(e)))
                if !self.session.signed_out && self.session.me.is_none() =>
            {
                let text = format!("cannot reach {}: {e}", self.session.url);
                self.retry_me(text, now)
            }
            // Any other answer the server wrote: shown, and asked again on the same schedule.
            Msg::MeLoaded(Completion::Failed(
                f @ (Failure::Validation(_) | Failure::NotFound(_) | Failure::Conflict),
            )) if !self.session.signed_out && self.session.me.is_none() => {
                let text = format!("{}: {}", self.session.url, board::describe(&f));
                self.retry_me(text, now)
            }
            Msg::MeLoaded(_) => Vec::new(),
            Msg::RetryMe(g)
                if self.session.me_gen.is(g)
                    && self.session.me.is_none()
                    && !self.session.signed_out =>
            {
                vec![Effect::http(Request::Me, |_, c| Msg::MeLoaded(c))]
            }
            Msg::RetryMe(_) => Vec::new(),
            Msg::TakeDone { key, stamp, result } => {
                let outcome = WriteOutcome::of(&result);
                // A lost answer and a conflict were worded by `classify`; failures by it too.
                if let Completion::Ok(_) = result {
                    self.set_status(format!("{key} is yours"), now);
                }
                self.notes(
                    vec![Note::WriteFinished {
                        key,
                        stamp,
                        outcome,
                    }],
                    now,
                )
            }
            Msg::ClearUrgentDone { key, stamp, result } => {
                let outcome = WriteOutcome::of(&result);
                // Failures were worded by `classify`; it does not word a refusal, so this does.
                match &result {
                    Completion::Ok(_) => self.set_status(format!("{key} urgent mark cleared"), now),
                    Completion::Refused { reasons, .. } => {
                        let shorts = reasons
                            .iter()
                            .map(words::short)
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.set_status(format!("{key}: {shorts}"), now);
                    }
                    Completion::Failed(_) => {}
                }
                self.notes(
                    vec![Note::WriteFinished {
                        key,
                        stamp,
                        outcome,
                    }],
                    now,
                )
            }
        }
    }

    /// Shows why `/me` did not answer and asks again after the next backoff step.
    fn retry_me(&mut self, why: String, now: Instant) -> Vec<Effect<Msg>> {
        let delay = self.me_backoff[self.session.me_attempt.min(self.me_backoff.len() - 1)];
        self.session.me_attempt += 1;
        self.set_status(why, now);
        vec![Effect::after(
            delay,
            Msg::RetryMe(self.session.me_gen.bump()),
        )]
    }

    fn key(&mut self, k: KeyEvent, now: Instant) -> Vec<Effect<Msg>> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return Vec::new();
        }
        // An outcome holds until the next key (or five seconds); whatever this key does may set a new one.
        self.ui.status = None;
        match self.overlay() {
            Some(Overlay::Help) => {
                self.ui.help = false;
                Vec::new()
            }
            Some(Overlay::ActionMenu) => self.menu_key(k, now),
            Some(Overlay::MovePrompt) => self.moves_msg(moves::Msg::PromptKey(k), now),
            Some(Overlay::MovePicker) => self.moves_msg(moves::Msg::PickerKey(k), now),
            Some(Overlay::EpicPicker) => self.set_epic_msg(set_epic::Msg::Key(k), now),
            Some(Overlay::FilterMenu) => self.board_msg(board::Msg::FilterKey(k), now),
            Some(Overlay::Search) => self.board_msg(board::Msg::SearchKey(k), now),
            None => match keys::action(&k) {
                Some(a) => self.act(a, now),
                None => Vec::new(),
            },
        }
    }

    /// The open drawer's furthest scroll at the terminal's size; `None` without a selected story.
    fn drawer_max(&self) -> Option<u16> {
        let e = self.board.selected_entry()?;
        let terminal = ratatui::layout::Rect::new(0, 0, self.ui.width, self.ui.height);
        Some(drawer::max_scroll(
            &self.board,
            e,
            drawer::area(terminal),
            self.ui.wall,
        ))
    }

    fn selected_key(&self) -> Option<Key> {
        self.board
            .selected_entry()
            .map(|e| e.item.story.key.clone())
    }

    fn act(&mut self, a: Action, now: Instant) -> Vec<Effect<Msg>> {
        use board::Nav;
        match a {
            Action::Left => self.board_msg(board::Msg::Nav(Nav::Left), now),
            Action::Right => self.board_msg(board::Msg::Nav(Nav::Right), now),
            Action::Up => self.board_msg(board::Msg::Nav(Nav::Up), now),
            Action::Down => self.board_msg(board::Msg::Nav(Nav::Down), now),
            Action::First => self.board_msg(board::Msg::Nav(Nav::First), now),
            Action::Last => self.board_msg(board::Msg::Nav(Nav::Last), now),
            Action::Open => {
                if self.board.selected_entry().is_some() {
                    self.ui.drawer = Some(0);
                }
                Vec::new()
            }
            Action::Menu => {
                if self.board.selected_entry().is_some() {
                    self.ui.menu = Some(0);
                } else {
                    self.set_status("no story selected", now);
                }
                Vec::new()
            }
            Action::Back => {
                if self.ui.drawer.take().is_some() {
                    Vec::new()
                } else if !self.board.query().is_empty() {
                    self.board_msg(board::Msg::ClearQuery, now)
                } else {
                    Vec::new()
                }
            }
            Action::MoveEarlier => self.moves_msg(moves::Msg::Step { delta: -1 }, now),
            Action::MoveLater => self.moves_msg(moves::Msg::Step { delta: 1 }, now),
            Action::MoveTo => self.moves_msg(moves::Msg::Pick, now),
            Action::SetEpic => self.set_epic_msg(set_epic::Msg::Open, now),
            Action::Take => self.take(now),
            Action::ClearUrgent => self.clear_urgent(now),
            // The guards only: the reason prompt that sends the mark comes with the urgent slice.
            Action::MarkUrgent => {
                match self.board.writable(self.session.read_only()) {
                    Err(why) => self.set_status(why, now),
                    Ok(e) if e.item.story.delivery == DeliveryState::Done => {
                        let k = e.item.story.key.clone();
                        self.set_status(format!("{k}: {NOT_WHEN_DONE}"), now);
                    }
                    Ok(_) => {}
                }
                Vec::new()
            }
            Action::Yank => match self.selected_key() {
                Some(k) => {
                    self.set_status(format!("copied {k}"), now);
                    vec![Effect::Copy(k.to_string())]
                }
                None => Vec::new(),
            },
            Action::OpenWeb => match self.selected_key() {
                Some(k) => vec![Effect::Open(format!(
                    "{}/board?story={k}",
                    self.session.url
                ))],
                None => Vec::new(),
            },
            Action::OpenPr => match self.board.selected_entry() {
                Some(e) => match e.item.story.prs.first() {
                    Some(pr) => vec![Effect::Open(pr.url.clone())],
                    None => {
                        let k = e.item.story.key.clone();
                        self.set_status(format!("{k} has no pull request"), now);
                        Vec::new()
                    }
                },
                None => Vec::new(),
            },
            Action::Filter => self.board_msg(board::Msg::FilterOpen, now),
            Action::Search => self.board_msg(board::Msg::SearchOpen, now),
            Action::Refresh => {
                if self.session.me.is_none() {
                    // Still connecting: retry identity now; the board starts when it answers.
                    if self.session.signed_out {
                        return Vec::new();
                    }
                    return vec![Effect::http(Request::Me, |_, c| Msg::MeLoaded(c))];
                }
                self.board_msg(board::Msg::Refresh, now)
            }
            // The stored scroll never passes the last page, so K/PgUp move the drawer from the first press.
            Action::ScrollDown => {
                if let (Some(max), Some(s)) = (self.drawer_max(), self.ui.drawer.as_mut()) {
                    *s = s.saturating_add(5).min(max);
                }
                Vec::new()
            }
            Action::ScrollUp => {
                // From the last page when the data has shrunk under a stored scroll.
                if let (Some(max), Some(s)) = (self.drawer_max(), self.ui.drawer.as_mut()) {
                    *s = (*s).min(max).saturating_sub(5);
                }
                Vec::new()
            }
            Action::Help => {
                self.ui.help = true;
                Vec::new()
            }
            Action::Quit => {
                self.quit = true;
                Vec::new()
            }
        }
    }

    /// The action menu's rows for the selected story: each `keys::MENU` action with why it would do nothing, if so.
    pub fn menu_items(&self) -> Vec<MenuItem> {
        let Some(e) = self.board.selected_entry() else {
            return Vec::new();
        };
        let story = &e.item.story;
        let writable = self.board.writable(self.session.read_only()).err();
        // A pending move steps from its target, not from where the story is now.
        let at = self
            .moves
            .intent(&story.key)
            .map_or(story.delivery, |i| i.target);
        let mine = match (&story.owner, &self.session.me) {
            (Some(o), Some(me)) => normalise_email(o) == normalise_email(&me.email),
            _ => false,
        };
        keys::MENU
            .iter()
            .map(|&action| {
                let disabled = match action {
                    Action::MoveEarlier
                    | Action::MoveLater
                    | Action::MoveTo
                    | Action::Take
                    | Action::SetEpic
                    | Action::MarkUrgent
                    | Action::ClearUrgent
                        if writable.is_some() =>
                    {
                        writable.clone()
                    }
                    Action::MoveEarlier if at == words::STATES[0] => {
                        Some(format!("nothing before {}", words::state(at)))
                    }
                    Action::MoveLater if at == words::STATES[words::STATES.len() - 1] => {
                        Some(format!("nothing after {}", words::state(at)))
                    }
                    Action::Take if mine => Some("already yours".into()),
                    Action::MarkUrgent if story.delivery == DeliveryState::Done => {
                        Some(NOT_WHEN_DONE.into())
                    }
                    Action::ClearUrgent if story.urgent_mark.is_none() => Some(NO_MARK.into()),
                    Action::OpenPr if story.prs.is_empty() => Some("no pull request".into()),
                    _ => None,
                };
                MenuItem { action, disabled }
            })
            .collect()
    }

    /// Keys while the action menu is open: `j`/`k` and the arrows choose, `Enter` runs the highlighted row, an
    /// action's own key runs it directly, `Esc`, `Space` and `q` close. Running an action closes the menu first, so
    /// one it opens (a picker, the drawer) is the one on screen.
    fn menu_key(&mut self, k: KeyEvent, now: Instant) -> Vec<Effect<Msg>> {
        let items = self.menu_items();
        let row = self.ui.menu.unwrap_or(0);
        let chosen = match (k.code, keys::action(&k)) {
            (KeyCode::Esc, _) | (_, Some(Action::Menu | Action::Quit)) => {
                self.ui.menu = None;
                return Vec::new();
            }
            (_, Some(Action::Up)) => {
                self.ui.menu = Some(row.saturating_sub(1));
                return Vec::new();
            }
            (_, Some(Action::Down)) => {
                self.ui.menu = Some((row + 1).min(items.len().saturating_sub(1)));
                return Vec::new();
            }
            (KeyCode::Enter, _) => items.get(row),
            (_, Some(a)) => items.iter().find(|i| i.action == a),
            _ => None,
        };
        let Some(item) = chosen.cloned() else {
            return Vec::new();
        };
        self.ui.menu = None;
        match item.disabled {
            Some(why) => {
                let key = self
                    .selected_key()
                    .map(|k| k.to_string())
                    .unwrap_or_default();
                self.set_status(format!("{key}: {why}"), now);
                Vec::new()
            }
            None => self.act(item.action, now),
        }
    }

    fn take(&mut self, now: Instant) -> Vec<Effect<Msg>> {
        let (key, version, owner, me) = match (
            self.board.writable(self.session.read_only()),
            &self.session.me,
        ) {
            (Ok(e), Some(me)) => (
                e.item.story.key.clone(),
                e.item.story.version,
                e.item.story.owner.clone(),
                me.email.clone(),
            ),
            (Err(why), _) => {
                self.set_status(why, now);
                return Vec::new();
            }
            // Unreachable: writes are read-only ("connecting") until `/me` answers.
            (Ok(_), None) => {
                self.set_status("read-only: connecting", now);
                return Vec::new();
            }
        };
        if owner.as_deref().map(normalise_email) == Some(normalise_email(&me)) {
            self.set_status(format!("{key} is already yours"), now);
            return Vec::new();
        }
        let mut fx = self.notes(vec![Note::WriteStarted(key.clone())], now);
        let body = EditItem {
            version,
            owner: Some(Some(me)),
            ..Default::default()
        };
        let k = key.clone();
        fx.push(Effect::http(
            Request::Edit { key, body },
            move |stamp, result| Msg::TakeDone {
                key: k,
                stamp,
                result,
            },
        ));
        fx
    }

    /// Clears the selected story's own mark. No version: marks do not take one (kb/ADR-0017 R5).
    fn clear_urgent(&mut self, now: Instant) -> Vec<Effect<Msg>> {
        let (key, marked) = match self.board.writable(self.session.read_only()) {
            Ok(e) => (e.item.story.key.clone(), e.item.story.urgent_mark.is_some()),
            Err(why) => {
                self.set_status(why, now);
                return Vec::new();
            }
        };
        if !marked {
            self.set_status(format!("{key}: {NO_MARK}"), now);
            return Vec::new();
        }
        let mut fx = self.notes(vec![Note::WriteStarted(key.clone())], now);
        let k = key.clone();
        fx.push(Effect::http(
            Request::ClearUrgent { key },
            move |stamp, result| Msg::ClearUrgentDone {
                key: k,
                stamp,
                result,
            },
        ));
        fx
    }
}
