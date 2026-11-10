//! The board slice: the one authoritative story list, its freshness rules, the filter, search
//! and selection. The drawer is a view over this state (`tui/drawer.rs`).
pub mod view;

use crate::tui::effect::{Completion, Effect, Failure, Reply, Request, Seq, Stamp};
use crate::tui::line::{LineEditor, LineEvent};
use crate::tui::note::{Note, Out, WriteOutcome};
use crate::tui::words::{self, STATES};
use crate::urgency::is_urgent;
use crossterm::event::KeyEvent;
use reeve_protocol::{DeliveryState, Key, StoryDto, StoryWithReadiness, normalise_email};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct Entry {
    pub item: StoryWithReadiness,
    /// The `sent` number of the request this entry's data came from.
    seq: Seq,
    /// A write reply replaced the fields; readiness waits for the next load.
    pub readiness_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Filter {
    pub all: bool,
    pub ready_only: bool,
    /// Only stories the server says are urgent (`is_urgent`; kb/ADR-0017 R3).
    pub urgent_only: bool,
}

/// What the core passes the board read-only (kb/ADR-0011 R2).
pub struct Ctx<'a> {
    /// The caller's email; `None` while `/me` has not answered.
    pub me: Option<&'a str>,
    /// Five columns (true) or the grouped list.
    pub wide: bool,
    /// The core's request sequence now: every request sent so far has `sent <= seq`.
    pub seq: Seq,
}

#[derive(Debug, Default)]
pub struct State {
    entries: HashMap<Key, Entry>,
    loaded: bool,
    stale: Option<String>,
    done_window_days: Option<u32>,
    filter: Filter,
    query: String,
    search: Option<LineEditor>,
    filter_menu: Option<usize>,
    selected: Option<Key>,
    /// The focused column (an index into `STATES`).
    focus: usize,
    /// The user moved the focus into an empty column: a load must not pull it out.
    parked: bool,
    in_flight: bool,
    queued: bool,
    /// The highest `at_done` of a write that may have changed the server, or the sequence when an activity entry
    /// arrived: loads sent at or before it are stale.
    written: Seq,
    locks: HashSet<Key>,
}

#[derive(Debug)]
pub enum Msg {
    /// `r`: a load, and the configuration again if it has not answered yet.
    Refresh,
    Loaded {
        stamp: Stamp,
        result: Completion,
    },
    ConfigLoaded(Completion),
    Nav(Nav),
    SearchOpen,
    SearchKey(KeyEvent),
    ClearQuery,
    FilterOpen,
    FilterKey(KeyEvent),
    /// Another overlay opened (a move prompt): close the search line (its query stays, as on Enter) and the filter
    /// menu.
    CloseOverlays,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    First,
    Last,
}

/// The selected story as a write starts from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub key: Key,
    pub delivery: DeliveryState,
    pub version: i64,
}

/// What the workflow slices (moves, set_epic, urgent) may read of the board, and nothing more: read-only, so every change
/// to the board still goes through its own messages (kb/ADR-0011 R2).
#[derive(Clone, Copy)]
pub struct View<'a> {
    board: &'a State,
}

impl View<'_> {
    /// The selected story if a write may start on it now, else why not (for the status line).
    pub fn writable(&self, read_only: Option<&str>) -> Result<Selected, String> {
        self.board.writable(read_only).map(|e| Selected {
            key: e.item.story.key.clone(),
            delivery: e.item.story.delivery,
            version: e.item.story.version,
        })
    }
    pub fn version(&self, k: &Key) -> Option<i64> {
        self.board.version(k)
    }
    /// The selected story's key, whether or not a write may start on it.
    pub fn selected(&self) -> Option<&Key> {
        self.board.selected()
    }
    /// A write on `k` is in flight.
    pub fn locked(&self, k: &Key) -> bool {
        self.board.locked(k)
    }
}

/// A failure as the status line and the stale marker word it.
pub fn describe(f: &Failure) -> String {
    match f {
        Failure::Unauthenticated => "signed out".into(),
        Failure::Forbidden(m) => format!("forbidden: {m}"),
        Failure::Conflict => "changed on the server".into(),
        Failure::Validation(m) | Failure::NotFound(m) | Failure::Server(m) => m.clone(),
        Failure::Transport(m) => format!("no answer: {m}"),
    }
}

impl State {
    /// The first requests: the instance configuration and the stories.
    pub fn start(&mut self) -> Out<Msg> {
        let mut o = Out::none();
        o.push(Effect::http(Request::Config, |_, c| Msg::ConfigLoaded(c)));
        o.extend(self.request_load());
        o
    }

    pub fn stale(&self) -> Option<&str> {
        self.stale.as_deref()
    }
    pub fn filter(&self) -> Filter {
        self.filter
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    /// The search line while it is open.
    pub fn search(&self) -> Option<&LineEditor> {
        self.search.as_ref()
    }
    /// The filter menu's highlighted row while it is open.
    pub fn filter_menu(&self) -> Option<usize> {
        self.filter_menu
    }
    /// The focused column (an index into `STATES`): the selected story's, or where the focus is parked.
    pub fn focus(&self) -> usize {
        self.focus
    }
    pub fn selected(&self) -> Option<&Key> {
        self.selected.as_ref()
    }
    pub fn get(&self, k: &Key) -> Option<&Entry> {
        self.entries.get(k)
    }
    pub fn version(&self, k: &Key) -> Option<i64> {
        self.entries.get(k).map(|e| e.item.story.version)
    }
    pub fn locked(&self, k: &Key) -> bool {
        self.locks.contains(k)
    }
    pub fn selected_entry(&self) -> Option<&Entry> {
        self.selected.as_ref().and_then(|k| self.entries.get(k))
    }
    /// The selected story if a write may start on it now, else why not (for the status line).
    pub fn writable(&self, read_only: Option<&str>) -> Result<&Entry, String> {
        if let Some(why) = read_only {
            return Err(format!("read-only: {why}"));
        }
        let e = self
            .selected_entry()
            .ok_or_else(|| "no story selected".to_string())?;
        if self.locked(&e.item.story.key) {
            return Err(format!("{}: a change is still saving", e.item.story.key));
        }
        Ok(e)
    }

    /// The narrow, read-only view the workflow slices get.
    pub fn view(&self) -> View<'_> {
        View { board: self }
    }

    fn request_load(&mut self) -> Out<Msg> {
        let mut o = Out::none();
        if self.in_flight {
            self.queued = true;
        } else {
            self.in_flight = true;
            o.push(Effect::http(Request::Stories, |stamp, result| {
                Msg::Loaded { stamp, result }
            }));
        }
        o
    }

    /// The `e.seq > sent` checks are unreachable today: every applied reply has seq <= at_done <= `written` < `sent`
    /// of any non-stale load, and only one load is in flight. They stay as a guard against that changing.
    fn apply_load(&mut self, sent: Seq, list: Vec<StoryWithReadiness>) {
        let incoming: HashSet<Key> = list.iter().map(|s| s.story.key.clone()).collect();
        self.entries
            .retain(|k, e| incoming.contains(k) || e.seq > sent);
        for item in list {
            let keep = self
                .entries
                .get(&item.story.key)
                .is_some_and(|e| e.seq > sent || e.item.story.version > item.story.version);
            if !keep {
                self.entries.insert(
                    item.story.key.clone(),
                    Entry {
                        item,
                        seq: sent,
                        readiness_pending: false,
                    },
                );
            }
        }
    }

    fn apply_reply(&mut self, sent: Seq, story: &StoryDto) {
        if let Some(e) = self.entries.get_mut(&story.key)
            && e.seq < sent
            && story.version >= e.item.story.version
        {
            e.item.story = story.clone();
            e.seq = sent;
            e.readiness_pending = true;
        }
    }

    pub fn on_note(&mut self, n: &Note, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        match n {
            Note::WriteStarted(k) => {
                self.locks.insert(k.clone());
            }
            Note::WriteFinished {
                key,
                stamp,
                outcome,
            } => {
                self.locks.remove(key);
                match outcome {
                    WriteOutcome::Applied(story) => {
                        let before = self.place(cx);
                        self.apply_reply(stamp.sent, story);
                        self.written = self.written.max(stamp.at_done);
                        o.extend(self.reselect(before, cx));
                        o.extend(self.request_load());
                    }
                    WriteOutcome::Unknown => {
                        // R8: the outcome is unknown, so readiness is too, until a fresh read lands.
                        let before = self.place(cx);
                        if let Some(e) = self.entries.get_mut(key) {
                            e.readiness_pending = true;
                        }
                        self.written = self.written.max(stamp.at_done);
                        o.extend(self.reselect(before, cx));
                        o.extend(self.request_load());
                    }
                    WriteOutcome::Unchanged => {}
                }
            }
            Note::Refetch => o.extend(self.request_load()),
            // Only the reads already sent go stale; live's debounce asks for the fresh one.
            Note::Invalidate => self.written = self.written.max(cx.seq),
            Note::Status(_) | Note::SignedOut | Note::SelectionLost(_) => {}
        }
        o
    }

    pub fn update(&mut self, msg: Msg, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        match msg {
            Msg::Refresh => {
                if self.done_window_days.is_none() {
                    o.push(Effect::http(Request::Config, |_, c| Msg::ConfigLoaded(c)));
                }
                o.extend(self.request_load());
            }
            Msg::Loaded { stamp, result } => {
                self.in_flight = false;
                if stamp.sent <= self.written {
                    // Sent before a write completed or an activity entry arrived: its snapshot may predate that change.
                    self.queued = false;
                    o.extend(self.request_load());
                    return o;
                }
                match result {
                    Completion::Ok(Reply::Stories(list)) => {
                        let before = self.place(cx);
                        self.apply_load(stamp.sent, list);
                        self.loaded = true;
                        self.stale = None;
                        o.extend(self.reselect(before, cx));
                    }
                    Completion::Failed(f) => {
                        o.status(format!("refresh failed: {}", describe(&f)));
                        self.stale = Some(describe(&f));
                    }
                    other => self.stale = Some(format!("unexpected reply: {other:?}")),
                }
                if std::mem::take(&mut self.queued) {
                    o.extend(self.request_load());
                }
            }
            Msg::ConfigLoaded(Completion::Ok(Reply::Config(c))) => {
                self.done_window_days = Some(c.done_window_days).filter(|d| *d > 0);
            }
            Msg::ConfigLoaded(_) => {}
            Msg::Nav(n) => o.extend(self.nav(n, cx)),
            Msg::SearchOpen => self.search = Some(LineEditor::with_text(&self.query)),
            Msg::SearchKey(k) => o.extend(self.search_key(&k, cx)),
            Msg::ClearQuery => o.extend(self.set_query(String::new(), cx)),
            Msg::FilterOpen => self.filter_menu = Some(0),
            Msg::FilterKey(k) => o.extend(self.filter_key(&k, cx)),
            Msg::CloseOverlays => {
                self.search = None;
                self.filter_menu = None;
            }
        }
        o
    }
}

impl State {
    fn matches(&self, e: &Entry, me: Option<&str>, q: &str) -> bool {
        let s = &e.item.story;
        if !self.filter.all {
            let Some(me) = me else { return false };
            if s.owner.as_deref().map(normalise_email).as_deref() != Some(me) {
                return false;
            }
        }
        if self.filter.ready_only && (!e.item.readiness.ready || e.readiness_pending) {
            return false;
        }
        if self.filter.urgent_only && !is_urgent(&s.urgent_sources) {
            return false;
        }
        if !q.is_empty()
            && !s.key.to_string().to_lowercase().contains(q)
            && !s.title.to_lowercase().contains(q)
        {
            return false;
        }
        true
    }

    /// The visible stories in board order: state, then (todo only) rank, then key number.
    pub fn visible(&self, cx: &Ctx) -> Vec<&Entry> {
        let me = cx.me.map(normalise_email);
        let q = self.query.to_lowercase();
        let mut v: Vec<&Entry> = self
            .entries
            .values()
            .filter(|e| self.matches(e, me.as_deref(), &q))
            .collect();
        v.sort_by_key(|e| {
            let s = &e.item.story;
            let rank = if s.delivery == DeliveryState::Todo {
                s.rank
            } else {
                0
            };
            (words::index(s.delivery), rank, s.key.number())
        });
        v
    }

    pub fn column(&self, col: usize, cx: &Ctx) -> Vec<&Entry> {
        self.visible(cx)
            .into_iter()
            .filter(|e| words::index(e.item.story.delivery) == col)
            .collect()
    }

    /// Where the selection is now: (column, index in it), or `None` when nothing visible is selected.
    fn place(&self, cx: &Ctx) -> Option<(usize, usize)> {
        let k = self.selected.as_ref()?;
        let e = self.entries.get(k)?;
        let col = words::index(e.item.story.delivery);
        let idx = self
            .column(col, cx)
            .iter()
            .position(|x| &x.item.story.key == k)?;
        Some((col, idx))
    }

    /// Keeps the selection on its story when it is still visible (the focus follows it); otherwise hands it to the
    /// nearest card in the column it was in, and says so.
    fn reselect(&mut self, before: Option<(usize, usize)>, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        if let Some(k) = &self.selected
            && let Some(e) = self
                .visible(cx)
                .into_iter()
                .find(|e| &e.item.story.key == k)
        {
            self.focus = words::index(e.item.story.delivery);
            return o;
        }
        let lost = self.selected.take();
        if self.parked && lost.is_none() {
            return o;
        }
        let (col, idx) = before.unwrap_or((self.focus, 0));
        // Owned before any write to `self`: the borrow of the entries ends here.
        let pick: Option<(usize, Key)> = {
            let in_col = self.column(col, cx);
            let e = if in_col.is_empty() {
                self.visible(cx).first().copied()
            } else {
                Some(in_col[idx.min(in_col.len() - 1)])
            };
            e.map(|e| {
                (
                    words::index(e.item.story.delivery),
                    e.item.story.key.clone(),
                )
            })
        };
        if let Some((focus, k)) = pick {
            self.focus = focus;
            self.selected = Some(k);
            self.parked = false;
        }
        if let Some(k) = lost {
            o.tell(Note::SelectionLost(k));
        }
        o
    }

    fn select_in(&mut self, col: usize, idx: usize, cx: &Ctx) {
        self.focus = col;
        let c = self.column(col, cx);
        self.selected = c
            .get(idx.min(c.len().saturating_sub(1)))
            .map(|e| e.item.story.key.clone());
    }

    fn nav(&mut self, n: Nav, cx: &Ctx) -> Out<Msg> {
        if self.selected.is_none() {
            // Nothing selected: the first key lands on the first card instead of skipping it.
            if cx.wide {
                if !self.column(self.focus, cx).is_empty() {
                    self.select_in(self.focus, 0, cx);
                    return Out::none();
                }
            } else if let Some(e) = self.visible(cx).first() {
                let (k, c) = (
                    e.item.story.key.clone(),
                    words::index(e.item.story.delivery),
                );
                self.selected = Some(k);
                self.focus = c;
                self.parked = false;
                return Out::none();
            }
        }
        let here = self.place(cx);
        if cx.wide {
            let (col, idx) = here.unwrap_or((self.focus, 0));
            let len = self.column(col, cx).len();
            match n {
                Nav::Up => self.select_in(col, idx.saturating_sub(1), cx),
                Nav::Down => self.select_in(col, (idx + 1).min(len.saturating_sub(1)), cx),
                Nav::Left => self.select_in(col.saturating_sub(1), idx, cx),
                Nav::Right => self.select_in((col + 1).min(STATES.len() - 1), idx, cx),
                Nav::First => self.select_in(col, 0, cx),
                Nav::Last => self.select_in(col, len.saturating_sub(1), cx),
            }
            // Park only on a deliberate Left/Right into an empty column while something is visible.
            self.parked = if matches!(n, Nav::Left | Nav::Right) {
                self.selected.is_none() && !self.visible(cx).is_empty()
            } else {
                self.parked && self.selected.is_none()
            };
        } else {
            let flat: Vec<Key> = self
                .visible(cx)
                .iter()
                .map(|e| e.item.story.key.clone())
                .collect();
            if flat.is_empty() {
                return Out::none();
            }
            let i = self
                .selected
                .as_ref()
                .and_then(|k| flat.iter().position(|x| x == k))
                .unwrap_or(0);
            let col_of = |b: &Self, k: &Key| words::index(b.entries[k].item.story.delivery);
            let target = match n {
                Nav::Up => i.saturating_sub(1),
                Nav::Down => (i + 1).min(flat.len() - 1),
                Nav::First => 0,
                Nav::Last => flat.len() - 1,
                Nav::Left => {
                    let cur = col_of(self, &flat[i]);
                    let group = (0..cur)
                        .rev()
                        .find(|c| flat.iter().any(|k| col_of(self, k) == *c));
                    match group {
                        Some(g) => flat.iter().position(|k| col_of(self, k) == g).unwrap_or(i),
                        None => i,
                    }
                }
                Nav::Right => {
                    let cur = col_of(self, &flat[i]);
                    flat.iter().position(|k| col_of(self, k) > cur).unwrap_or(i)
                }
            };
            self.selected = Some(flat[target].clone());
            self.parked = false;
            self.focus = col_of(self, &flat[target]);
        }
        Out::none()
    }

    fn set_query(&mut self, q: String, cx: &Ctx) -> Out<Msg> {
        let before = self.place(cx);
        self.parked = false;
        self.query = q;
        self.reselect(before, cx)
    }

    fn search_key(&mut self, k: &KeyEvent, cx: &Ctx) -> Out<Msg> {
        let Some(ed) = self.search.as_mut() else {
            return Out::none();
        };
        match ed.handle(k) {
            LineEvent::Edited => {
                let q = ed.text().to_string();
                self.set_query(q, cx)
            }
            LineEvent::Submit => {
                self.search = None;
                Out::none()
            }
            LineEvent::Cancel => {
                self.search = None;
                self.set_query(String::new(), cx)
            }
            LineEvent::Ignored => Out::none(),
        }
    }

    /// Rows: 0 mine / all, 1 ready only, 2 urgent only.
    fn filter_key(&mut self, k: &KeyEvent, cx: &Ctx) -> Out<Msg> {
        use crossterm::event::KeyCode;
        let Some(row) = self.filter_menu else {
            return Out::none();
        };
        let before = self.place(cx);
        match k.code {
            KeyCode::Esc | KeyCode::Char('f') | KeyCode::Char('q') => self.filter_menu = None,
            KeyCode::Char('j') | KeyCode::Down => self.filter_menu = Some((row + 1).min(2)),
            KeyCode::Char('k') | KeyCode::Up => self.filter_menu = Some(row.saturating_sub(1)),
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.parked = false;
                match row {
                    0 => self.filter.all = !self.filter.all,
                    1 => self.filter.ready_only = !self.filter.ready_only,
                    _ => self.filter.urgent_only = !self.filter.urgent_only,
                }
                return self.reselect(before, cx);
            }
            _ => {}
        }
        Out::none()
    }
}

/// Test-only setters for tests outside this module that need a board in a given shape; every other change to the
/// board goes through `update` or `on_note` (kb/ADR-0011 R2).
#[cfg(test)]
impl State {
    /// A board showing everyone's stories (`all`).
    pub fn test_showing_all() -> Self {
        Self {
            filter: Filter {
                all: true,
                ready_only: false,
                urgent_only: false,
            },
            ..Self::default()
        }
    }
    pub fn test_select(&mut self, k: Key) {
        self.selected = Some(k);
    }
    pub fn test_set_stale(&mut self, why: Option<String>) {
        self.stale = why;
    }
    pub fn test_set_loaded(&mut self, loaded: bool) {
        self.loaded = loaded;
    }
    pub fn test_set_done_window_days(&mut self, days: Option<u32>) {
        self.done_window_days = days;
    }
    pub fn test_set_filter_all(&mut self, all: bool) {
        self.filter.all = all;
    }
    pub fn test_set_query(&mut self, q: &str) {
        self.query = q.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures::*;
    use reeve_protocol::{DeliveryState::*, PrStatus, PullRequestDto, Reason};

    fn cx() -> Ctx<'static> {
        Ctx {
            me: Some("dan@example.com"),
            wide: true,
            seq: 0,
        }
    }
    #[test]
    fn the_workflow_view_reads_the_selected_story_its_versions_and_locks() {
        let mut b = board_with(vec![story("ABC-1", Testing, 4), story("ABC-2", Todo, 1)]);
        b.test_select(key("ABC-1"));
        let v = b.view();
        assert_eq!(
            v.writable(None),
            Ok(Selected {
                key: key("ABC-1"),
                delivery: Testing,
                version: 4
            })
        );
        assert_eq!(
            v.writable(Some("signed out")),
            Err("read-only: signed out".to_string())
        );
        assert_eq!(
            (v.version(&key("ABC-2")), v.version(&key("ABC-9"))),
            (Some(1), None)
        );
        assert!(!v.locked(&key("ABC-1")));
        let _ = b.on_note(&Note::WriteStarted(key("ABC-1")), &cx());
        let v = b.view();
        assert!(v.locked(&key("ABC-1")));
        assert_eq!(
            v.writable(None),
            Err("ABC-1: a change is still saving".to_string())
        );
    }
    /// A board after one load of `list` with sent number `sent`.
    fn loaded(list: Vec<StoryWithReadiness>, sent: u64) -> State {
        let mut b = State::default();
        let _ = b.request_load();
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(sent, sent),
                result: stories(list),
            },
            &cx(),
        );
        b
    }
    fn applied(s: &StoryWithReadiness) -> WriteOutcome {
        WriteOutcome::Applied(s.story.clone())
    }

    #[test]
    fn a_load_replaces_the_list_and_drops_stories_that_left() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1), story("ABC-2", Todo, 1)], 1);
        let _ = b.request_load();
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(2, 2),
                result: stories(vec![story("ABC-1", Todo, 2)]),
            },
            &cx(),
        );
        assert_eq!(b.version(&key("ABC-1")), Some(2));
        assert!(b.get(&key("ABC-2")).is_none());
    }

    #[test]
    fn loads_coalesce_to_one_in_flight_and_one_queued() {
        let mut b = State::default();
        assert_eq!(requests(&b.request_load().effects).len(), 1);
        assert!(requests(&b.request_load().effects).is_empty());
        assert!(requests(&b.on_note(&Note::Refetch, &cx()).effects).is_empty());
        let o = b.update(
            Msg::Loaded {
                stamp: stamp(1, 1),
                result: stories(vec![]),
            },
            &cx(),
        );
        assert_eq!(
            requests(&o.effects).len(),
            1,
            "the queued load goes out once the first lands"
        );
    }

    #[test]
    fn a_load_sent_before_a_write_completed_is_dropped_and_reloaded() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1), story("ABC-2", Todo, 1)], 1);
        let _ = b.on_note(&Note::WriteStarted(key("ABC-1")), &cx());
        let _ = b.request_load(); // load L, sent 5, in flight
        let moved = story("ABC-1", InProgress, 2);
        let _ = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(4, 6),
                outcome: applied(&moved),
            },
            &cx(),
        );
        // L's snapshot predates the move: it still says todo.
        let o = b.update(
            Msg::Loaded {
                stamp: stamp(5, 6),
                result: stories(vec![story("ABC-1", Todo, 1)]),
            },
            &cx(),
        );
        assert_eq!(
            b.get(&key("ABC-1")).unwrap().item.story.delivery,
            InProgress
        );
        assert!(
            b.get(&key("ABC-2")).is_some(),
            "the stale list is dropped whole, not applied"
        );
        assert_eq!(
            requests(&o.effects).len(),
            1,
            "a fresh load replaces the dropped one"
        );
    }

    #[test]
    fn an_invalidate_makes_the_loads_sent_so_far_stale_without_asking_for_one() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let _ = b.request_load(); // load L, sent 2, in flight
        let o = b.on_note(&Note::Invalidate, &Ctx { seq: 3, ..cx() });
        assert!(
            requests(&o.effects).is_empty(),
            "the debounced refetch asks; the note only marks what is in flight"
        );
        let o = b.update(
            Msg::Loaded {
                stamp: stamp(2, 3),
                result: stories(vec![story("ABC-1", Todo, 1), story("ABC-2", Todo, 1)]),
            },
            &cx(),
        );
        assert!(b.get(&key("ABC-2")).is_none(), "L predates the entry");
        assert_eq!(requests(&o.effects).len(), 1, "a fresh load replaces L");
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(4, 4),
                result: stories(vec![story("ABC-1", Todo, 1), story("ABC-2", Todo, 1)]),
            },
            &cx(),
        );
        assert!(
            b.get(&key("ABC-2")).is_some(),
            "a load sent after the entry applies"
        );
    }

    #[test]
    fn a_dependent_storys_readiness_from_a_stale_load_is_not_applied() {
        let blocked = not_ready(
            story("ABC-2", Todo, 1),
            vec![Reason::BlockedBy {
                story: key("ABC-1"),
            }],
        );
        let mut b = loaded(vec![story("ABC-1", Testing, 1), blocked.clone()], 1);
        let _ = b.request_load(); // sent 3, in flight across the write's completion
        let done = story("ABC-1", Done, 2);
        let _ = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(2, 3),
                outcome: applied(&done),
            },
            &cx(),
        );
        // The stale load reports ABC-2 with a different, same-version reason: applying it would be visible.
        let stale = not_ready(story("ABC-2", Todo, 1), vec![Reason::NoCriteria]);
        let o = b.update(
            Msg::Loaded {
                stamp: stamp(3, 3),
                result: stories(vec![done, stale]),
            },
            &cx(),
        );
        assert_eq!(
            b.get(&key("ABC-2")).unwrap().item.readiness.reasons,
            vec![Reason::BlockedBy {
                story: key("ABC-1")
            }],
            "the stale load's readiness was not applied"
        );
        assert_eq!(requests(&o.effects).len(), 1, "a fresh load replaces it");
    }

    #[test]
    fn a_write_reply_older_than_a_landed_load_is_not_applied() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let _ = b.on_note(&Note::WriteStarted(key("ABC-1")), &cx());
        // A load sent after the write (sent 4 > write's 3) lands first and already shows the move with a PR linked.
        let mut fresh = story("ABC-1", InProgress, 2);
        fresh.story.prs.push(PullRequestDto {
            repo: "orion".into(),
            number: 7,
            url: "https://github.com/x/orion/pull/7".into(),
            title: "t".into(),
            status: PrStatus::Open,
            updated_at: chrono::Utc::now(),
        });
        let _ = b.request_load();
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(4, 4),
                result: stories(vec![fresh]),
            },
            &cx(),
        );
        // The write's own reply (same version, no PR yet) arrives late.
        let o = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(3, 5),
                outcome: applied(&story("ABC-1", InProgress, 2)),
            },
            &cx(),
        );
        let e = b.get(&key("ABC-1")).unwrap();
        assert_eq!(
            e.item.story.prs.len(),
            1,
            "equal-version PR data from the newer load is kept"
        );
        assert!(!e.readiness_pending);
        assert_eq!(
            requests(&o.effects).len(),
            1,
            "the completion still asks for a load"
        );
    }

    #[test]
    fn a_write_reply_marks_readiness_pending_and_asks_for_a_load() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let o = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(2, 2),
                outcome: applied(&story("ABC-1", InProgress, 2)),
            },
            &cx(),
        );
        let e = b.get(&key("ABC-1")).unwrap();
        assert_eq!(e.item.story.delivery, InProgress);
        assert!(e.readiness_pending);
        assert_eq!(requests(&o.effects).len(), 1);
    }

    #[test]
    fn an_unknown_write_outcome_marks_readiness_pending_until_a_load_lands() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let _ = b.on_note(&Note::WriteStarted(key("ABC-1")), &cx());
        let o = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(2, 2),
                outcome: WriteOutcome::Unknown,
            },
            &cx(),
        );
        assert!(!b.locked(&key("ABC-1")));
        assert!(b.get(&key("ABC-1")).unwrap().readiness_pending);
        assert_eq!(requests(&o.effects).len(), 1);
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(3, 3),
                result: stories(vec![story("ABC-1", Todo, 1)]),
            },
            &cx(),
        );
        assert!(!b.get(&key("ABC-1")).unwrap().readiness_pending);
    }

    #[test]
    fn a_lower_version_never_replaces_a_higher_one() {
        let mut b = loaded(vec![story("ABC-1", Todo, 3)], 1);
        let _ = b.request_load();
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(7, 7),
                result: stories(vec![story("ABC-1", Todo, 2)]),
            },
            &cx(),
        );
        assert_eq!(b.version(&key("ABC-1")), Some(3));
    }

    #[test]
    fn the_write_lock_is_held_from_start_to_every_kind_of_finish() {
        for outcome in [
            WriteOutcome::Unchanged,
            WriteOutcome::Unknown,
            applied(&story("ABC-1", Todo, 2)),
        ] {
            let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
            let _ = b.on_note(&Note::WriteStarted(key("ABC-1")), &cx());
            assert!(b.locked(&key("ABC-1")));
            let _ = b.on_note(
                &Note::WriteFinished {
                    key: key("ABC-1"),
                    stamp: stamp(2, 2),
                    outcome,
                },
                &cx(),
            );
            assert!(!b.locked(&key("ABC-1")));
        }
    }

    #[test]
    fn an_unknown_write_outcome_invalidates_older_loads_but_unchanged_does_not() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let _ = b.request_load(); // sent 2
        let _ = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(2, 3),
                outcome: WriteOutcome::Unchanged,
            },
            &cx(),
        );
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(2, 3),
                result: stories(vec![story("ABC-1", Todo, 5)]),
            },
            &cx(),
        );
        assert_eq!(
            b.version(&key("ABC-1")),
            Some(5),
            "an unchanged write does not make the load stale"
        );

        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let _ = b.request_load();
        let _ = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(2, 3),
                outcome: WriteOutcome::Unknown,
            },
            &cx(),
        );
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(2, 3),
                result: stories(vec![story("ABC-1", Todo, 5)]),
            },
            &cx(),
        );
        assert_eq!(
            b.version(&key("ABC-1")),
            Some(1),
            "a lost write reply may have committed: the older load is dropped"
        );
    }

    #[test]
    fn a_failed_load_keeps_the_board_and_marks_it_stale() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let _ = b.request_load();
        let o = b.update(
            Msg::Loaded {
                stamp: stamp(2, 2),
                result: Completion::Failed(Failure::Transport("timed out".into())),
            },
            &cx(),
        );
        assert!(
            o.notes
                .iter()
                .any(|n| matches!(n, Note::Status(s) if s.contains("timed out"))),
            "the error reaches the status line"
        );
        assert!(b.get(&key("ABC-1")).is_some());
        assert!(b.stale.as_deref().unwrap().contains("timed out"));
        let _ = b.request_load();
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(3, 3),
                result: stories(vec![story("ABC-1", Todo, 1)]),
            },
            &cx(),
        );
        assert!(b.stale.is_none());
    }
    use crossterm::event::{KeyCode, KeyModifiers};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn keys_of(b: &State, cx: &Ctx) -> Vec<String> {
        b.visible(cx)
            .iter()
            .map(|e| e.item.story.key.to_string())
            .collect()
    }

    #[test]
    fn mine_compares_normalised_emails_and_all_shows_everyone() {
        let b = loaded(
            vec![
                owned(story("ABC-1", Todo, 1), " Dan@Example.com "),
                owned(story("ABC-2", Todo, 1), "mia@example.com"),
                story("ABC-3", Todo, 1),
            ],
            1,
        );
        assert_eq!(keys_of(&b, &cx()), ["ABC-1"]);
        let mut b = b;
        b.filter.all = true;
        assert_eq!(keys_of(&b, &cx()), ["ABC-1", "ABC-2", "ABC-3"]);
    }

    #[test]
    fn mine_shows_nothing_until_me_is_known() {
        let b = loaded(vec![owned(story("ABC-1", Todo, 1), "dan@example.com")], 1);
        assert!(
            b.visible(&Ctx {
                me: None,
                wide: true,
                seq: 0,
            })
            .is_empty()
        );
    }

    #[test]
    fn ready_only_hides_not_ready_and_pending_readiness() {
        let mut b = loaded(
            vec![
                owned(story("ABC-1", Todo, 1), "dan@example.com"),
                owned(
                    not_ready(story("ABC-2", Todo, 1), vec![Reason::NoCriteria]),
                    "dan@example.com",
                ),
                owned(story("ABC-3", Todo, 1), "dan@example.com"),
            ],
            1,
        );
        // A write reply on ABC-3 leaves its readiness pending until the next load.
        let _ = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-3"),
                stamp: stamp(2, 2),
                outcome: applied(&owned(story("ABC-3", Todo, 2), "dan@example.com")),
            },
            &cx(),
        );
        assert!(b.get(&key("ABC-3")).unwrap().readiness_pending);
        b.filter.ready_only = true;
        assert_eq!(keys_of(&b, &cx()), ["ABC-1"]);
    }

    #[test]
    fn search_narrows_by_key_or_title_case_insensitively_enter_keeps_esc_clears() {
        let mut b = loaded(
            vec![
                owned(story("ABC-1", Todo, 1), "dan@example.com"),
                owned(story("ABC-12", Todo, 1), "dan@example.com"),
            ],
            1,
        );
        b.entries.get_mut(&key("ABC-1")).unwrap().item.story.title = "Fix Auth".into();
        let _ = b.update(Msg::SearchOpen, &cx());
        for c in "auth".chars() {
            let _ = b.update(Msg::SearchKey(k(KeyCode::Char(c))), &cx());
        }
        assert_eq!(keys_of(&b, &cx()), ["ABC-1"]);
        let _ = b.update(Msg::SearchKey(k(KeyCode::Enter)), &cx());
        assert!(b.search.is_none());
        assert_eq!(b.query, "auth");
        let _ = b.update(Msg::SearchOpen, &cx());
        let _ = b.update(Msg::SearchKey(k(KeyCode::Esc)), &cx());
        assert_eq!(b.query, "");
        assert_eq!(keys_of(&b, &cx()), ["ABC-1", "ABC-12"]);
        b.query = "abc-12".into();
        assert_eq!(keys_of(&b, &cx()), ["ABC-12"]);
    }

    #[test]
    fn order_is_state_then_todo_rank_then_key_number() {
        let mut a = owned(story("ABC-10", Todo, 1), "dan@example.com");
        a.story.rank = 0;
        let mut c = owned(story("ABC-2", Todo, 1), "dan@example.com");
        c.story.rank = 1;
        let b = loaded(
            vec![
                owned(story("ABC-9", InReview, 1), "dan@example.com"),
                owned(story("ABC-3", InReview, 1), "dan@example.com"),
                c,
                a,
            ],
            1,
        );
        assert_eq!(keys_of(&b, &cx()), ["ABC-10", "ABC-2", "ABC-3", "ABC-9"]);
    }

    #[test]
    fn the_first_load_selects_the_first_card_and_the_selection_follows_a_moved_story() {
        let mut b = loaded(
            vec![
                owned(story("ABC-1", Todo, 1), "dan@example.com"),
                owned(story("ABC-2", Todo, 1), "dan@example.com"),
            ],
            1,
        );
        assert_eq!(b.selected, Some(key("ABC-1")));
        let o = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(2, 2),
                outcome: applied(&owned(story("ABC-1", InReview, 2), "dan@example.com")),
            },
            &cx(),
        );
        assert_eq!(b.selected, Some(key("ABC-1")), "selection is by key");
        assert_eq!(
            STATES[b.focus], InReview,
            "focus follows the story into its new column"
        );
        assert!(!o.notes.iter().any(|n| matches!(n, Note::SelectionLost(_))));
    }

    #[test]
    fn a_selected_story_leaving_hands_over_to_the_nearest_card_and_says_so() {
        let mut b = loaded(
            vec![
                owned(story("ABC-1", Todo, 1), "dan@example.com"),
                owned(story("ABC-2", Todo, 1), "dan@example.com"),
                owned(story("ABC-3", Todo, 1), "dan@example.com"),
            ],
            1,
        );
        let _ = b.update(Msg::Nav(Nav::Down), &cx());
        let _ = b.update(Msg::Nav(Nav::Down), &cx());
        assert_eq!(b.selected, Some(key("ABC-3")));
        let _ = b.request_load();
        let o = b.update(
            Msg::Loaded {
                stamp: stamp(2, 2),
                result: stories(vec![
                    owned(story("ABC-1", Todo, 1), "dan@example.com"),
                    owned(story("ABC-2", Todo, 1), "dan@example.com"),
                ]),
            },
            &cx(),
        );
        assert_eq!(
            b.selected,
            Some(key("ABC-2")),
            "the nearest card in the same column"
        );
        assert!(
            o.notes
                .iter()
                .any(|n| matches!(n, Note::SelectionLost(k) if *k == key("ABC-3")))
        );
    }

    #[test]
    fn wide_navigation_moves_within_and_between_columns() {
        let mut b = loaded(
            vec![
                owned(story("ABC-1", Todo, 1), "dan@example.com"),
                owned(story("ABC-2", Todo, 1), "dan@example.com"),
                owned(story("ABC-3", InReview, 1), "dan@example.com"),
            ],
            1,
        );
        let _ = b.update(Msg::Nav(Nav::Down), &cx());
        assert_eq!(b.selected, Some(key("ABC-2")));
        let _ = b.update(Msg::Nav(Nav::Right), &cx()); // in progress: empty
        assert_eq!(b.selected, None);
        assert_eq!(STATES[b.focus], InProgress);
        let _ = b.update(Msg::Nav(Nav::Right), &cx());
        assert_eq!(b.selected, Some(key("ABC-3")));
        let _ = b.update(Msg::Nav(Nav::Up), &cx());
        assert_eq!(
            b.selected,
            Some(key("ABC-3")),
            "the top of a column stays put"
        );
    }

    #[test]
    fn narrow_navigation_is_flat_and_left_right_jump_groups() {
        let narrow = Ctx {
            me: Some("dan@example.com"),
            wide: false,
            seq: 0,
        };
        let mut b = loaded(
            vec![
                owned(story("ABC-1", Todo, 1), "dan@example.com"),
                owned(story("ABC-2", Todo, 1), "dan@example.com"),
                owned(story("ABC-3", Testing, 1), "dan@example.com"),
            ],
            1,
        );
        let _ = b.update(Msg::Nav(Nav::Down), &narrow);
        let _ = b.update(Msg::Nav(Nav::Down), &narrow);
        assert_eq!(
            b.selected,
            Some(key("ABC-3")),
            "j runs across group boundaries"
        );
        let _ = b.update(Msg::Nav(Nav::Left), &narrow);
        assert_eq!(
            b.selected,
            Some(key("ABC-1")),
            "h jumps to the previous non-empty group's first card"
        );
        let _ = b.update(Msg::Nav(Nav::Last), &narrow);
        assert_eq!(b.selected, Some(key("ABC-3")));
    }

    #[test]
    fn the_filter_menu_toggles_mine_and_ready_and_closes_on_esc() {
        let mut b = loaded(vec![owned(story("ABC-1", Todo, 1), "mia@example.com")], 1);
        let _ = b.update(Msg::FilterOpen, &cx());
        let _ = b.update(Msg::FilterKey(k(KeyCode::Enter)), &cx()); // row 0: mine/all
        assert!(b.filter.all);
        let _ = b.update(Msg::FilterKey(k(KeyCode::Char('j'))), &cx());
        let _ = b.update(Msg::FilterKey(k(KeyCode::Char(' '))), &cx()); // row 1: ready only
        assert!(b.filter.ready_only);
        let _ = b.update(Msg::FilterKey(k(KeyCode::Esc)), &cx());
        assert!(b.filter_menu.is_none());
        assert_eq!(b.selected, Some(key("ABC-1")), "a filter change reselects");
    }

    #[test]
    fn the_filter_menu_reaches_a_third_row_that_keeps_only_urgent_stories() {
        let mut b = loaded(
            vec![
                urgent(owned(story("ABC-1", Todo, 1), "dan@example.com")),
                owned(story("ABC-2", Todo, 1), "dan@example.com"),
                done_marked(owned(story("ABC-3", Done, 1), "dan@example.com")),
            ],
            1,
        );
        assert_eq!(b.visible(&cx()).len(), 3);
        let _ = b.update(Msg::FilterOpen, &cx());
        for c in ['j', 'j', 'j'] {
            let _ = b.update(Msg::FilterKey(k(KeyCode::Char(c))), &cx());
        }
        assert_eq!(b.filter_menu, Some(2), "the clamp stops at the third row");
        let _ = b.update(Msg::FilterKey(k(KeyCode::Enter)), &cx());
        assert!(b.filter.urgent_only);
        let keys: Vec<_> = b
            .visible(&cx())
            .iter()
            .map(|e| e.item.story.key.clone())
            .collect();
        assert_eq!(keys, vec![key("ABC-1")]);
        let _ = b.update(Msg::FilterKey(k(KeyCode::Enter)), &cx());
        assert!(!b.filter.urgent_only);
    }

    #[test]
    fn a_write_reply_below_a_landed_third_party_version_is_not_applied() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let _ = b.on_note(&Note::WriteStarted(key("ABC-1")), &cx());
        let _ = b.request_load();
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(5, 5),
                result: stories(vec![story("ABC-1", Todo, 3)]),
            },
            &cx(),
        );
        let _ = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(6, 7),
                outcome: applied(&story("ABC-1", InProgress, 2)),
            },
            &cx(),
        );
        assert_eq!(b.version(&key("ABC-1")), Some(3));
    }

    #[test]
    fn an_unknown_write_outcome_with_no_load_in_flight_asks_for_exactly_one_load() {
        let mut b = loaded(vec![story("ABC-1", Todo, 1)], 1);
        let o = b.on_note(
            &Note::WriteFinished {
                key: key("ABC-1"),
                stamp: stamp(2, 2),
                outcome: WriteOutcome::Unknown,
            },
            &cx(),
        );
        assert_eq!(requests(&o.effects).len(), 1);
    }

    #[test]
    fn with_nothing_selected_the_first_nav_key_selects_the_first_card() {
        let mut b = loaded(
            vec![
                owned(story("ABC-1", Todo, 1), "dan@example.com"),
                owned(story("ABC-2", Todo, 1), "dan@example.com"),
            ],
            1,
        );
        b.selected = None;
        let _ = b.update(Msg::Nav(Nav::Down), &cx());
        assert_eq!(b.selected, Some(key("ABC-1")));
    }

    #[test]
    fn a_load_does_not_pull_the_focus_out_of_an_empty_column_the_user_moved_into() {
        let mut b = loaded(vec![owned(story("ABC-1", Todo, 1), "dan@example.com")], 1);
        let _ = b.update(Msg::Nav(Nav::Right), &cx());
        assert_eq!(b.selected, None);
        let _ = b.request_load();
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(2, 2),
                result: stories(vec![owned(story("ABC-1", Todo, 2), "dan@example.com")]),
            },
            &cx(),
        );
        assert_eq!(b.selected, None);
        assert_eq!(STATES[b.focus], InProgress);
    }

    #[test]
    fn a_key_before_the_first_load_does_not_stop_the_first_load_selecting() {
        let mut b = State::default();
        let _ = b.request_load();
        let _ = b.update(Msg::Nav(Nav::Right), &cx());
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(1, 1),
                result: stories(vec![owned(story("ABC-1", Todo, 1), "dan@example.com")]),
            },
            &cx(),
        );
        assert_eq!(b.selected, Some(key("ABC-1")));
    }

    #[test]
    fn a_key_in_an_empty_view_does_not_stop_a_filter_change_selecting() {
        let mut b = loaded(vec![owned(story("ABC-1", Todo, 1), "mia@example.com")], 1);
        let _ = b.update(Msg::Nav(Nav::Right), &cx());
        let _ = b.update(Msg::Nav(Nav::Down), &cx());
        let _ = b.update(Msg::FilterOpen, &cx());
        let _ = b.update(Msg::FilterKey(k(KeyCode::Enter)), &cx());
        assert_eq!(b.selected, Some(key("ABC-1")));
    }

    #[test]
    fn up_and_down_in_a_parked_empty_column_keep_it_parked() {
        let mut b = loaded(vec![owned(story("ABC-1", Todo, 1), "dan@example.com")], 1);
        let _ = b.update(Msg::Nav(Nav::Right), &cx());
        let _ = b.update(Msg::Nav(Nav::Down), &cx());
        let _ = b.request_load();
        let _ = b.update(
            Msg::Loaded {
                stamp: stamp(2, 2),
                result: stories(vec![owned(story("ABC-1", Todo, 2), "dan@example.com")]),
            },
            &cx(),
        );
        assert_eq!(b.selected, None);
        assert_eq!(STATES[b.focus], InProgress);
    }
}
