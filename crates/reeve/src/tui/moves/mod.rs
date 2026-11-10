//! Moves: H/L, the state picker, the resolution and override prompts, and the write. An operation is
//! bound to the story it started on (key, origin, version), never to the current selection. The first H/L sends at
//! once; presses while that move is in flight steer one follow-up move, sent from the reply.
pub mod view;

use crate::tui::board;
use crate::tui::effect::{Completion, Effect, Reply, Request, Stamp};
use crate::tui::line::{LineEditor, LineEvent};
use crate::tui::note::{Note, Out, WriteOutcome};
use crate::tui::words::{self, STATES};
use crossterm::event::{KeyCode, KeyEvent};
use reeve_protocol::{DeliveryState, Key, MoveStory, Reason, Resolution};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub key: Key,
    pub origin: DeliveryState,
    pub version: i64,
    pub target: DeliveryState,
    pub resolution: Option<Resolution>,
    pub override_reason: Option<String>,
}

/// A move of the story is in flight; `target` is where H/L presses since want it to end (the move's own target until
/// one is pressed). A different target is sent from the reply as one follow-up move.
#[derive(Debug, Clone)]
pub struct Intent {
    pub target: DeliveryState,
}

#[derive(Debug)]
pub enum Prompt {
    Resolution {
        op: Op,
        options: Vec<Resolution>,
        choice: usize,
    },
    Override {
        op: Op,
        reasons: Vec<Reason>,
        line: LineEditor,
    },
}

#[derive(Debug)]
pub struct Picker {
    pub key: Key,
    pub origin: DeliveryState,
    pub version: i64,
    pub cursor: usize,
}

#[derive(Debug, Default)]
pub struct State {
    intents: HashMap<Key, Intent>,
    prompt: Option<Prompt>,
    picker: Option<Picker>,
}

pub struct Ctx<'a> {
    pub board: board::View<'a>,
    /// Why writes are disabled (signed out, automation token, connecting), or `None`.
    pub read_only: Option<&'a str>,
}

#[expect(
    clippy::large_enum_variant,
    reason = "built once per request and moved through the channel; boxing buys nothing measurable"
)]
#[derive(Debug)]
pub enum Msg {
    /// H/L on the selected story.
    Step {
        delta: i8,
    },
    /// `m`.
    Pick,
    PickerKey(KeyEvent),
    PromptKey(KeyEvent),
    Done {
        op: Op,
        stamp: Stamp,
        result: Completion,
    },
    /// The board's data changed: cancel what was built on an older version.
    BoardChanged,
    /// Another overlay opened: close the picker and the prompt, saying so; moves in flight and their follow-ups stay.
    Close,
    /// Signed out: drop every follow-up, the picker and the prompt, silently.
    Reset,
}

fn stepped(s: DeliveryState, delta: i8) -> DeliveryState {
    let i = words::index(s) as i32 + delta as i32;
    STATES[i.clamp(0, STATES.len() as i32 - 1) as usize]
}

impl State {
    pub fn intent(&self, k: &Key) -> Option<&Intent> {
        self.intents.get(k)
    }
    /// The resolution or override prompt while one is open.
    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }
    /// The state picker while it is open.
    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    /// The selected story, if a write on it may start now; otherwise the status says why.
    fn writable(&self, cx: &Ctx, o: &mut Out<Msg>) -> Option<board::Selected> {
        cx.board
            .writable(cx.read_only)
            .map_err(|why| o.status(why))
            .ok()
    }

    /// The picker and a prompt are never open together: a prompt closes the picker.
    fn open_prompt(&mut self, p: Prompt) {
        self.picker = None;
        self.prompt = Some(p);
    }

    /// Sends the move, unless writes became disabled (signed out) or another write on the story started since this
    /// operation began: the guard runs at dispatch, on the operation's own key, not on the current selection.
    fn send(&mut self, op: Op, cx: &Ctx, o: &mut Out<Msg>) {
        if let Some(why) = cx.read_only {
            o.status(format!("read-only: {why} — {}: move cancelled", op.key));
            return;
        }
        if cx.board.locked(&op.key) {
            o.status(format!(
                "{}: a change is still saving — move cancelled",
                op.key
            ));
            return;
        }
        self.dispatch(op, o);
    }

    /// Sends the move with no lock check: `send` after its guards, and a follow-up from the reply of the move that
    /// held the story (its `WriteFinished` is told first, so the board unlocks before this write locks it again).
    fn dispatch(&mut self, op: Op, o: &mut Out<Msg>) {
        self.intents
            .insert(op.key.clone(), Intent { target: op.target });
        o.tell(Note::WriteStarted(op.key.clone()));
        let body = MoveStory {
            version: op.version,
            to: op.target,
            resolution: op.resolution,
            override_reason: op.override_reason.clone(),
        };
        let key = op.key.clone();
        o.push(Effect::http(
            Request::Move { key, body },
            move |stamp, result| Msg::Done { op, stamp, result },
        ));
    }

    fn begin(&mut self, op: Op, cx: &Ctx, o: &mut Out<Msg>) {
        if op.target != DeliveryState::Done {
            return self.send(op, cx, o);
        }
        self.ask_resolution(op, o);
    }

    /// A move into done asks for the resolution first. A follow-up into done that comes due while this slice's
    /// prompt or picker is open never replaces it: the operation is cancelled with a notice.
    fn ask_resolution(&mut self, op: Op, o: &mut Out<Msg>) {
        if self.prompt.is_some() || self.picker.is_some() {
            o.status(format!(
                "{}: move cancelled — select it and move again",
                op.key
            ));
            return;
        }
        let options = if op.origin.is_active() {
            vec![Resolution::Completed, Resolution::Dropped]
        } else {
            vec![Resolution::Dropped]
        };
        self.open_prompt(Prompt::Resolution {
            op,
            options,
            choice: 0,
        });
    }

    pub fn update(&mut self, msg: Msg, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        match msg {
            Msg::Step { delta } => {
                // A move of this story is in flight: the press steers where it ends.
                if cx.read_only.is_none()
                    && let Some(i) = cx.board.selected().and_then(|k| self.intents.get_mut(k))
                {
                    i.target = stepped(i.target, delta);
                    return o;
                }
                let Some(s) = self.writable(cx, &mut o) else {
                    return o;
                };
                let target = stepped(s.delivery, delta);
                if target != s.delivery {
                    let op = Op {
                        key: s.key,
                        origin: s.delivery,
                        version: s.version,
                        target,
                        resolution: None,
                        override_reason: None,
                    };
                    self.begin(op, cx, &mut o);
                }
            }
            Msg::Pick => {
                let Some(s) = self.writable(cx, &mut o) else {
                    return o;
                };
                let picker = Picker {
                    key: s.key,
                    origin: s.delivery,
                    version: s.version,
                    cursor: words::index(s.delivery),
                };
                self.picker = Some(picker);
            }
            Msg::PickerKey(k) => {
                let Some(mut p) = self.picker.take() else {
                    return o;
                };
                match k.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        p.cursor = (p.cursor + 1).min(STATES.len() - 1);
                        self.picker = Some(p);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        p.cursor = p.cursor.saturating_sub(1);
                        self.picker = Some(p);
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {}
                    KeyCode::Enter => {
                        let target = STATES[p.cursor];
                        if target != p.origin {
                            let op = Op {
                                key: p.key,
                                origin: p.origin,
                                version: p.version,
                                target,
                                resolution: None,
                                override_reason: None,
                            };
                            self.begin(op, cx, &mut o);
                        }
                    }
                    _ => self.picker = Some(p),
                }
            }
            Msg::PromptKey(k) => o.extend(self.prompt_key(&k, cx)),
            Msg::Close => {
                if let Some(p) = self.picker.take() {
                    o.status(format!("{}: move cancelled", p.key));
                }
                if let Some(Prompt::Resolution { op, .. } | Prompt::Override { op, .. }) =
                    self.prompt.take()
                {
                    o.status(format!("{}: move cancelled", op.key));
                }
            }
            Msg::Reset => {
                // Moves in flight finish; no follow-up is sent after them.
                self.intents.clear();
                self.prompt = None;
                self.picker = None;
            }
            Msg::Done {
                mut op,
                stamp,
                result,
            } => {
                o.tell(Note::WriteFinished {
                    key: op.key.clone(),
                    stamp,
                    outcome: WriteOutcome::of(&result),
                });
                let wanted = self.intents.remove(&op.key).map(|i| i.target);
                match result {
                    Completion::Ok(Reply::Story(s)) => {
                        let tag = if op.override_reason.is_some() {
                            " (override)"
                        } else {
                            ""
                        };
                        o.status(format!("{} → {}{tag}", s.key, words::state(s.delivery)));
                        if let Some(target) = wanted.filter(|t| *t != s.delivery) {
                            self.follow_up(
                                Op {
                                    key: s.key.clone(),
                                    origin: s.delivery,
                                    version: s.version,
                                    target,
                                    resolution: None,
                                    override_reason: None,
                                },
                                cx,
                                &mut o,
                            );
                        }
                    }
                    Completion::Refused {
                        reasons,
                        overridable,
                    } => {
                        // The override is for where the presses wanted the story, from where it still is; a move
                        // into done also needs a resolution, so that one keeps the refused step's target.
                        if let Some(t) =
                            wanted.filter(|t| *t != op.origin && *t != DeliveryState::Done)
                        {
                            op.target = t;
                        }
                        let shorts = words::shorts(&reasons);
                        if op.override_reason.is_none()
                            && cx.read_only.is_none()
                            // The server's word on whether an override can get past these reasons.
                            && overridable
                        {
                            if self.prompt.is_none() && self.picker.is_none() {
                                self.open_prompt(Prompt::Override {
                                    op,
                                    reasons,
                                    line: LineEditor::default(),
                                });
                            } else {
                                o.status(format!(
                                    "{} refused: {shorts} — select it and move again",
                                    op.key
                                ));
                            }
                        } else {
                            o.status(format!("{}: {shorts}", op.key));
                        }
                    }
                    // Every failure is the core's to word (`app::State::classify`, by the write's story); it also asks the
                    // board to refetch on a conflict.
                    Completion::Failed(_) | Completion::Ok(_) => {}
                }
            }
            Msg::BoardChanged => {
                // A move in flight is the server's to judge: its reply, or a conflict, decides; only open dialogs
                // built on an older version close.
                let stale = |key: &Key, v: i64| cx.board.version(key) != Some(v);
                let prompt_stale = match &self.prompt {
                    Some(Prompt::Resolution { op, .. } | Prompt::Override { op, .. }) => {
                        stale(&op.key, op.version).then(|| op.key.clone())
                    }
                    None => None,
                };
                if let Some(k) = prompt_stale {
                    self.prompt = None;
                    o.status(format!("{k} changed — move cancelled"));
                }
                if self
                    .picker
                    .as_ref()
                    .is_some_and(|p| stale(&p.key, p.version))
                {
                    self.picker = None;
                }
            }
        }
        o
    }

    /// The move the presses during a write asked for, from the reply's state and version.
    fn follow_up(&mut self, op: Op, cx: &Ctx, o: &mut Out<Msg>) {
        if let Some(why) = cx.read_only {
            o.status(format!("read-only: {why} — {}: move cancelled", op.key));
            return;
        }
        if op.target == DeliveryState::Done {
            return self.ask_resolution(op, o);
        }
        self.dispatch(op, o);
    }

    fn prompt_key(&mut self, k: &KeyEvent, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        match self.prompt.take() {
            Some(Prompt::Resolution {
                mut op,
                options,
                mut choice,
            }) => match k.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    choice = (choice + 1).min(options.len() - 1);
                    self.prompt = Some(Prompt::Resolution {
                        op,
                        options,
                        choice,
                    });
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    choice = choice.saturating_sub(1);
                    self.prompt = Some(Prompt::Resolution {
                        op,
                        options,
                        choice,
                    });
                }
                KeyCode::Enter => {
                    op.resolution = Some(options[choice]);
                    self.send(op, cx, &mut o);
                }
                KeyCode::Esc => o.status(format!("{}: move cancelled", op.key)),
                _ => {
                    self.prompt = Some(Prompt::Resolution {
                        op,
                        options,
                        choice,
                    })
                }
            },
            Some(Prompt::Override {
                mut op,
                reasons,
                mut line,
            }) => match line.handle(k) {
                LineEvent::Submit if line.text().trim().is_empty() => {
                    o.status("an override needs a reason");
                    self.prompt = Some(Prompt::Override { op, reasons, line });
                }
                LineEvent::Submit => {
                    op.override_reason = Some(line.text().trim().to_string());
                    self.send(op, cx, &mut o);
                }
                LineEvent::Cancel => o.status(format!("{}: move cancelled", op.key)),
                LineEvent::Edited | LineEvent::Ignored => {
                    self.prompt = Some(Prompt::Override { op, reasons, line })
                }
            },
            None => {}
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::effect::Failure;
    use crate::tui::fixtures::*;
    use crossterm::event::KeyModifiers;
    use reeve_protocol::DeliveryState::*;

    fn cx(b: &board::State) -> Ctx<'_> {
        Ctx {
            board: b.view(),
            read_only: None,
        }
    }
    fn select(b: &mut board::State, k: &str) {
        b.test_select(key(k));
    }
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    /// A write on `k` holds the story, as the core tells the board when a move is sent.
    fn lock(b: &mut board::State, k: &str) {
        let _ = b.on_note(
            &Note::WriteStarted(key(k)),
            &board::Ctx {
                me: None,
                wide: true,
                seq: 0,
            },
        );
    }
    /// The server's reply to the one request in `o`, routed back as the core would.
    fn reply_to(o: Out<Msg>, result: Completion) -> Msg {
        match o
            .effects
            .into_iter()
            .find(|e| matches!(e, Effect::Http { .. }))
        {
            Some(Effect::Http { route, .. }) => route(stamp(1, 1), result),
            _ => panic!("no request to reply to"),
        }
    }
    fn moved(k: &str, to: DeliveryState, version: i64) -> Completion {
        Completion::Ok(Reply::Story(story(k, to, version).story))
    }
    fn the_move(o: &Out<Msg>) -> MoveStory {
        match o.effects.as_slice() {
            [
                Effect::Http {
                    req: Request::Move { body, .. },
                    ..
                },
            ] => body.clone(),
            other => panic!("expected one move request, got {other:?}"),
        }
    }
    fn statuses(o: &Out<Msg>) -> Vec<String> {
        o.notes
            .iter()
            .filter_map(|n| {
                if let Note::Status(s) = n {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn the_first_press_sends_at_once_and_presses_in_flight_steer_one_follow_up() {
        let mut b = board_with(vec![story("ABC-1", Todo, 4)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let first = m.update(Msg::Step { delta: 1 }, &cx(&b));
        assert_eq!(
            (the_move(&first).to, the_move(&first).version),
            (InProgress, 4)
        );
        assert!(
            first
                .notes
                .iter()
                .any(|n| matches!(n, Note::WriteStarted(x) if *x == key("ABC-1")))
        );
        lock(&mut b, "ABC-1");
        for _ in 0..2 {
            assert!(m.update(Msg::Step { delta: 1 }, &cx(&b)).effects.is_empty());
        }
        assert_eq!(
            m.intent(&key("ABC-1")).unwrap().target,
            Testing,
            "the card shows where the presses want the story"
        );
        // A change elsewhere while the move is in flight does not cancel it: the server judges the write.
        let _ = m.update(
            Msg::BoardChanged,
            &cx(&board_with(vec![story("ABC-1", Todo, 5)])),
        );
        let o = m.update(reply_to(first, moved("ABC-1", InProgress, 6)), &cx(&b));
        let follow = the_move(&o);
        assert_eq!(
            (follow.to, follow.version),
            (Testing, 6),
            "one follow-up, from the reply's version"
        );
        let finished = o
            .notes
            .iter()
            .position(|n| matches!(n, Note::WriteFinished { .. }));
        let started = o
            .notes
            .iter()
            .position(|n| matches!(n, Note::WriteStarted(_)));
        assert!(
            finished < started,
            "the board unlocks before the follow-up locks it"
        );
        let o = m.update(reply_to(o, moved("ABC-1", Testing, 7)), &cx(&b));
        assert!(o.effects.is_empty() && m.intent(&key("ABC-1")).is_none());
    }

    #[test]
    fn two_stories_moves_are_independent() {
        let mut b = board_with(vec![story("ABC-1", Todo, 1), story("ABC-2", Todo, 1)]);
        let mut m = State::default();
        select(&mut b, "ABC-1");
        assert_eq!(
            the_move(&m.update(Msg::Step { delta: 1 }, &cx(&b))).to,
            InProgress
        );
        lock(&mut b, "ABC-1");
        select(&mut b, "ABC-2");
        assert_eq!(
            the_move(&m.update(Msg::Step { delta: 1 }, &cx(&b))).to,
            InProgress
        );
        assert_eq!(m.intent(&key("ABC-1")).unwrap().target, InProgress);
    }

    #[test]
    fn pressing_back_to_the_target_in_flight_sends_no_follow_up() {
        let mut b = board_with(vec![story("ABC-1", InReview, 1)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let first = m.update(Msg::Step { delta: 1 }, &cx(&b));
        lock(&mut b, "ABC-1");
        let _ = m.update(Msg::Step { delta: 1 }, &cx(&b));
        let _ = m.update(Msg::Step { delta: -1 }, &cx(&b));
        let o = m.update(reply_to(first, moved("ABC-1", Testing, 2)), &cx(&b));
        assert!(o.effects.is_empty());
    }

    #[test]
    fn a_press_at_the_edge_sends_nothing() {
        let mut b = board_with(vec![story("ABC-1", Todo, 1)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        assert!(
            m.update(Msg::Step { delta: -1 }, &cx(&b))
                .effects
                .is_empty()
        );
        assert!(m.intent(&key("ABC-1")).is_none());
    }

    #[test]
    fn a_refusal_offers_the_override_for_where_the_presses_wanted_the_story() {
        for (presses, want) in [(2, InReview), (4, InProgress)] {
            let mut b = board_with(vec![story("ABC-1", Todo, 3)]);
            select(&mut b, "ABC-1");
            let mut m = State::default();
            let first = m.update(Msg::Step { delta: 1 }, &cx(&b));
            lock(&mut b, "ABC-1");
            for _ in 1..presses {
                let _ = m.update(Msg::Step { delta: 1 }, &cx(&b));
            }
            let _ = m.update(reply_to(first, refused(vec![Reason::NoCriteria])), &cx(&b));
            match &m.prompt {
                // Into done needs a resolution too, so that one keeps the refused step.
                Some(Prompt::Override { op, .. }) => {
                    assert_eq!((op.origin, op.target, op.version), (Todo, want, 3))
                }
                other => panic!("{other:?}"),
            }
            assert!(m.intent(&key("ABC-1")).is_none());
        }
    }

    #[test]
    fn a_press_during_a_write_is_ignored_with_a_notice() {
        let mut b = board_with(vec![story("ABC-1", Todo, 1)]);
        select(&mut b, "ABC-1");
        let _ = b.on_note(
            &Note::WriteStarted(key("ABC-1")),
            &board::Ctx {
                me: None,
                wide: true,
                seq: 0,
            },
        );
        let mut m = State::default();
        let o = m.update(Msg::Step { delta: 1 }, &cx(&b));
        assert!(o.effects.is_empty());
        assert!(statuses(&o)[0].contains("still saving"));
    }

    #[test]
    fn read_only_refuses_locally() {
        let mut b = board_with(vec![story("ABC-1", Todo, 1)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let o = m.update(
            Msg::Step { delta: 1 },
            &Ctx {
                board: b.view(),
                read_only: Some("automation token"),
            },
        );
        assert!(o.effects.is_empty());
        assert_eq!(statuses(&o), ["read-only: automation token"]);
    }

    #[test]
    fn into_done_asks_the_resolution_and_offers_completed_only_from_an_active_state() {
        for (origin, want) in [
            (Testing, vec![Resolution::Completed, Resolution::Dropped]),
            (Todo, vec![Resolution::Dropped]),
        ] {
            let mut b = board_with(vec![story("ABC-1", origin, 1)]);
            select(&mut b, "ABC-1");
            let mut m = State::default();
            let _ = m.update(Msg::Pick, &cx(&b));
            for _ in 0..4 {
                let _ = m.update(Msg::PickerKey(press(KeyCode::Char('j'))), &cx(&b));
            }
            let o = m.update(Msg::PickerKey(press(KeyCode::Enter)), &cx(&b));
            assert!(
                o.effects.is_empty(),
                "nothing is sent before the resolution"
            );
            match &m.prompt {
                Some(Prompt::Resolution { options, op, .. }) => {
                    assert_eq!(*options, want);
                    assert_eq!((op.origin, op.target), (origin, Done));
                }
                other => panic!("{other:?}"),
            }
            let o = m.update(Msg::PromptKey(press(KeyCode::Enter)), &cx(&b));
            assert_eq!(the_move(&o).resolution, Some(want[0]));
        }
    }

    #[test]
    fn a_readiness_refusal_opens_the_override_bound_to_the_operation() {
        let mut b = board_with(vec![story("ABC-1", Todo, 3)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let sent = m.update(Msg::Step { delta: 1 }, &cx(&b));
        let Effect::Http { route, .. } = sent.effects.into_iter().next().unwrap() else {
            panic!()
        };
        let done = route(
            stamp(2, 2),
            refused(vec![Reason::Untriaged, Reason::NoCriteria]),
        );
        let o = m.update(done, &cx(&b));
        assert!(o.notes.iter().any(|n| matches!(
            n,
            Note::WriteFinished {
                outcome: WriteOutcome::Unchanged,
                ..
            }
        )));
        assert!(matches!(m.prompt, Some(Prompt::Override { .. })));
        // An empty reason is not sent.
        assert!(
            m.update(Msg::PromptKey(press(KeyCode::Enter)), &cx(&b))
                .effects
                .is_empty()
        );
        for c in "hotfix".chars() {
            let _ = m.update(Msg::PromptKey(press(KeyCode::Char(c))), &cx(&b));
        }
        // Meanwhile someone else selects another card: the override still belongs to ABC-1.
        let mut b2 = board_with(vec![story("ABC-1", Todo, 3), story("ABC-2", Todo, 1)]);
        select(&mut b2, "ABC-2");
        let body = the_move(&m.update(Msg::PromptKey(press(KeyCode::Enter)), &cx(&b2)));
        assert_eq!(
            (body.to, body.version, body.override_reason.as_deref()),
            (InProgress, 3, Some("hotfix"))
        );
    }

    #[test]
    fn a_delivery_rule_reason_opens_no_prompt_and_an_override_refusal_is_not_retried() {
        let mut b = board_with(vec![story("ABC-1", Todo, 1)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let op = Op {
            key: key("ABC-1"),
            origin: Todo,
            version: 1,
            target: Done,
            resolution: None,
            override_reason: None,
        };
        let o = m.update(
            Msg::Done {
                op: op.clone(),
                stamp: stamp(1, 1),
                result: refused(vec![Reason::ResolutionRequired]),
            },
            &cx(&b),
        );
        assert!(m.prompt.is_none());
        assert!(statuses(&o)[0].contains("resolution required"));
        let with_reason = Op {
            target: InProgress,
            override_reason: Some("x".into()),
            ..op
        };
        let _ = m.update(
            Msg::Done {
                op: with_reason,
                stamp: stamp(2, 2),
                result: refused(vec![Reason::Untriaged]),
            },
            &cx(&b),
        );
        assert!(
            m.prompt.is_none(),
            "an override that was refused is shown, never retried"
        );
    }

    #[test]
    fn the_server_flag_decides_whether_the_override_prompt_opens() {
        let b = board_with(vec![story("ABC-1", Todo, 1)]);
        let op = Op {
            key: key("ABC-1"),
            origin: Todo,
            version: 1,
            target: InProgress,
            resolution: None,
            override_reason: None,
        };
        // Readiness facts the server nonetheless says no override can get past: a notice, no prompt.
        let mut m = State::default();
        let o = m.update(
            Msg::Done {
                op: op.clone(),
                stamp: stamp(1, 1),
                result: Completion::Refused {
                    reasons: vec![Reason::NoCriteria],
                    overridable: false,
                },
            },
            &cx(&b),
        );
        assert!(m.prompt.is_none());
        assert_eq!(statuses(&o), ["ABC-1: no criteria"]);
        // A delivery rule the server says an override can get past: the prompt opens.
        let mut m = State::default();
        let _ = m.update(
            Msg::Done {
                op,
                stamp: stamp(1, 1),
                result: Completion::Refused {
                    reasons: vec![Reason::ResolutionRequired],
                    overridable: true,
                },
            },
            &cx(&b),
        );
        assert!(matches!(m.prompt, Some(Prompt::Override { .. })));
    }

    #[test]
    fn transport_and_conflict_are_the_cores_to_word() {
        let b = board_with(vec![story("ABC-1", Todo, 1)]);
        let op = Op {
            key: key("ABC-1"),
            origin: Todo,
            version: 1,
            target: InProgress,
            resolution: None,
            override_reason: None,
        };
        for result in [
            Completion::Failed(Failure::Transport("timeout".into())),
            Completion::Failed(Failure::Conflict),
        ] {
            let mut m = State::default();
            let o = m.update(
                Msg::Done {
                    op: op.clone(),
                    stamp: stamp(1, 1),
                    result: result.clone(),
                },
                &cx(&b),
            );
            assert!(statuses(&o).is_empty(), "{result:?}: {:?}", statuses(&o));
        }
    }

    #[test]
    fn every_completion_ends_with_write_finished() {
        let b = board_with(vec![story("ABC-1", Todo, 1)]);
        let op = Op {
            key: key("ABC-1"),
            origin: Todo,
            version: 1,
            target: InProgress,
            resolution: None,
            override_reason: None,
        };
        for result in [
            Completion::Ok(Reply::Story(story("ABC-1", InProgress, 2).story)),
            refused(vec![Reason::NoCriteria]),
            Completion::Failed(Failure::Conflict),
            Completion::Failed(Failure::Forbidden("no".into())),
            Completion::Failed(Failure::Validation("bad".into())),
            Completion::Failed(Failure::Server("502".into())),
            Completion::Failed(Failure::Transport("timeout".into())),
        ] {
            let mut m = State::default();
            let o = m.update(
                Msg::Done {
                    op: op.clone(),
                    stamp: stamp(1, 1),
                    result,
                },
                &cx(&b),
            );
            let finished = o
                .notes
                .iter()
                .filter(|n| matches!(n, Note::WriteFinished { .. }))
                .count();
            assert_eq!(finished, 1);
        }
    }

    #[test]
    fn one_prompt_at_a_time() {
        let mut b = board_with(vec![story("ABC-1", Testing, 1), story("ABC-2", Todo, 1)]);
        let mut m = State::default();
        select(&mut b, "ABC-2");
        let gra2 = m.update(Msg::Step { delta: 1 }, &cx(&b));
        lock(&mut b, "ABC-2");
        // ABC-1's move into done asks the resolution at once, sending nothing; ABC-2's move stays in flight.
        select(&mut b, "ABC-1");
        assert!(m.update(Msg::Step { delta: 1 }, &cx(&b)).effects.is_empty());
        assert!(matches!(m.prompt, Some(Prompt::Resolution { .. })));
        // A refusal for another story while the prompt is open: status only.
        let o = m.update(reply_to(gra2, refused(vec![Reason::NoCriteria])), &cx(&b));
        assert!(matches!(m.prompt, Some(Prompt::Resolution { .. })));
        assert!(statuses(&o)[0].contains("select it and move again"));
    }

    #[test]
    fn no_follow_up_after_sign_out_or_reset() {
        let mut b = board_with(vec![story("ABC-1", Todo, 1)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let first = m.update(Msg::Step { delta: 1 }, &cx(&b));
        lock(&mut b, "ABC-1");
        let _ = m.update(Msg::Step { delta: 1 }, &cx(&b));
        let o = m.update(
            reply_to(first, moved("ABC-1", InProgress, 2)),
            &Ctx {
                board: b.view(),
                read_only: Some("signed out"),
            },
        );
        assert!(o.effects.is_empty(), "the follow-up re-checks permission");
        assert!(
            statuses(&o)
                .iter()
                .any(|s| s.contains("read-only: signed out"))
        );

        let mut m = State::default();
        let mut b = board_with(vec![story("ABC-1", Todo, 1)]);
        select(&mut b, "ABC-1");
        let first = m.update(Msg::Step { delta: 1 }, &cx(&b));
        lock(&mut b, "ABC-1");
        let _ = m.update(Msg::Step { delta: 1 }, &cx(&b));
        let _ = m.update(Msg::Reset, &cx(&b));
        let o = m.update(reply_to(first, moved("ABC-1", InProgress, 2)), &cx(&b));
        assert!(o.effects.is_empty(), "reset drops the follow-up");
    }

    #[test]
    fn a_readiness_refusal_after_sign_out_opens_no_prompt() {
        let b = board_with(vec![story("ABC-1", Todo, 1)]);
        let mut m = State::default();
        let op = Op {
            key: key("ABC-1"),
            origin: Todo,
            version: 1,
            target: InProgress,
            resolution: None,
            override_reason: None,
        };
        let _ = m.update(
            Msg::Done {
                op,
                stamp: stamp(1, 1),
                result: refused(vec![Reason::NoCriteria]),
            },
            &Ctx {
                board: b.view(),
                read_only: Some("signed out"),
            },
        );
        assert!(m.prompt.is_none());
    }

    #[test]
    fn board_changed_closes_a_stale_prompt() {
        let mut b = board_with(vec![story("ABC-1", Testing, 1)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let _ = m.update(Msg::Pick, &cx(&b));
        for _ in 0..4 {
            let _ = m.update(Msg::PickerKey(press(KeyCode::Char('j'))), &cx(&b));
        }
        let _ = m.update(Msg::PickerKey(press(KeyCode::Enter)), &cx(&b));
        assert!(m.prompt.is_some());
        let b2 = board_with(vec![story("ABC-1", Testing, 2)]);
        let o = m.update(Msg::BoardChanged, &cx(&b2));
        assert!(m.prompt.is_none());
        assert!(statuses(&o).iter().any(|s| s.contains("changed")));
        assert!(
            m.update(Msg::PromptKey(press(KeyCode::Enter)), &cx(&b2))
                .effects
                .is_empty()
        );
    }

    #[test]
    fn board_changed_closes_a_stale_picker() {
        let mut b = board_with(vec![story("ABC-1", Todo, 1)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let _ = m.update(Msg::Pick, &cx(&b));
        assert!(m.picker.is_some());
        let b2 = board_with(vec![story("ABC-1", Todo, 2)]);
        let _ = m.update(Msg::BoardChanged, &cx(&b2));
        assert!(m.picker.is_none());
    }

    #[test]
    fn a_refusal_while_the_picker_is_open_does_not_replace_it() {
        let mut b = board_with(vec![story("ABC-1", Todo, 1), story("ABC-2", Todo, 1)]);
        select(&mut b, "ABC-2");
        let mut m = State::default();
        let _ = m.update(Msg::Pick, &cx(&b));
        let op = Op {
            key: key("ABC-1"),
            origin: Todo,
            version: 1,
            target: InProgress,
            resolution: None,
            override_reason: None,
        };
        let o = m.update(
            Msg::Done {
                op,
                stamp: stamp(1, 1),
                result: refused(vec![Reason::NoCriteria]),
            },
            &cx(&b),
        );
        assert!(m.picker.is_some() && m.prompt.is_none());
        assert!(
            statuses(&o)
                .iter()
                .any(|s| s.starts_with("ABC-1 refused:") && s.contains("select it and move again")),
            "{:?}",
            statuses(&o)
        );
    }

    #[test]
    fn a_follow_up_into_done_behind_the_picker_is_cancelled_and_the_picker_stays() {
        let mut b = board_with(vec![story("ABC-1", InReview, 1), story("ABC-2", Todo, 1)]);
        let mut m = State::default();
        select(&mut b, "ABC-1");
        let first = m.update(Msg::Step { delta: 1 }, &cx(&b));
        lock(&mut b, "ABC-1");
        let _ = m.update(Msg::Step { delta: 1 }, &cx(&b));
        select(&mut b, "ABC-2");
        let _ = m.update(Msg::Pick, &cx(&b));
        let o = m.update(reply_to(first, moved("ABC-1", Testing, 2)), &cx(&b));
        assert!(o.effects.is_empty());
        assert!(
            statuses(&o).contains(&"ABC-1: move cancelled — select it and move again".to_string()),
            "{:?}",
            statuses(&o)
        );
        assert!(m.picker.is_some() && m.prompt.is_none());
    }

    #[test]
    fn picking_the_current_state_is_a_no_op() {
        let mut b = board_with(vec![story("ABC-1", Done, 1)]);
        select(&mut b, "ABC-1");
        let mut m = State::default();
        let _ = m.update(Msg::Pick, &cx(&b));
        let o = m.update(Msg::PickerKey(press(KeyCode::Enter)), &cx(&b));
        assert!(o.effects.is_empty() && m.prompt.is_none() && m.picker.is_none());
    }
}
