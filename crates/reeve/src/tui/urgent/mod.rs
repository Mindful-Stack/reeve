//! Mark urgent (`u`): a one-line reason prompt, then `POST /stories/{key}/urgent` (kb/ADR-0017). The slice owns the
//! write from the prompt to its one `WriteFinished`, as `set_epic` does. A mark takes no version (kb/ADR-0017 R5), so
//! nothing in the prompt can go stale while it is open.
pub mod view;

use crate::tui::board;
use crate::tui::effect::{Completion, Effect, Request, Stamp};
use crate::tui::line::{LineEditor, LineEvent};
use crate::tui::note::{Note, Out, WriteOutcome};
use crate::tui::words;
use crossterm::event::KeyEvent;
use reeve_protocol::Key;

#[derive(Debug)]
struct Prompt {
    key: Key,
    line: LineEditor,
}

#[derive(Debug, Default)]
pub struct State {
    prompt: Option<Prompt>,
}

pub struct Ctx<'a> {
    pub board: board::View<'a>,
    pub read_only: Option<&'a str>,
}

#[expect(
    clippy::large_enum_variant,
    reason = "built once per request and moved through the channel; boxing buys nothing measurable"
)]
#[derive(Debug)]
pub enum Msg {
    /// Opens the prompt for `key`, prefilled with its current reason so a re-mark keeps it.
    Open {
        key: Key,
        reason: Option<String>,
    },
    Key(KeyEvent),
    Done {
        key: Key,
        stamp: Stamp,
        result: Completion,
    },
    /// Another overlay opened: close the prompt, saying so.
    Close,
    /// Signed out: close the prompt, silently.
    Reset,
}

impl State {
    /// The prompt's story while it is open.
    pub fn open(&self) -> Option<&Key> {
        self.prompt.as_ref().map(|p| &p.key)
    }

    /// Enter: one mark with the trimmed reason (none when blank), unless writes became disabled or another write on
    /// the story started while the prompt was open (a pending move). Either way the prompt closes.
    fn submit(&mut self, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        let Some(p) = self.prompt.take() else {
            return o;
        };
        if let Some(why) = cx.read_only {
            o.status(format!("read-only: {why}"));
            return o;
        }
        if cx.board.locked(&p.key) {
            o.status(format!("{}: a change is still saving", p.key));
            return o;
        }
        let text = p.line.text().trim();
        let reason = (!text.is_empty()).then(|| text.to_string());
        o.tell(Note::WriteStarted(p.key.clone()));
        let key = p.key.clone();
        o.push(Effect::http(
            Request::MarkUrgent { key: p.key, reason },
            move |stamp, result| Msg::Done { key, stamp, result },
        ));
        o
    }

    pub fn update(&mut self, msg: Msg, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        match msg {
            Msg::Open { key, reason } => {
                self.prompt = Some(Prompt {
                    key,
                    line: LineEditor::with_text(reason.as_deref().unwrap_or_default()),
                });
            }
            Msg::Key(k) => {
                let Some(p) = self.prompt.as_mut() else {
                    return o;
                };
                match p.line.handle(&k) {
                    LineEvent::Submit => o.extend(self.submit(cx)),
                    LineEvent::Cancel => o.extend(self.update(Msg::Close, cx)),
                    LineEvent::Edited | LineEvent::Ignored => {}
                }
            }
            Msg::Done { key, stamp, result } => {
                o.tell(Note::WriteFinished {
                    key: key.clone(),
                    stamp,
                    outcome: WriteOutcome::of(&result),
                });
                // Failures are the core's to word (`app::State::classify`, by the write's story).
                match &result {
                    Completion::Ok(_) => o.status(format!("{key} marked urgent")),
                    Completion::Refused { reasons, .. } => {
                        let shorts = reasons
                            .iter()
                            .map(words::short)
                            .collect::<Vec<_>>()
                            .join(", ");
                        o.status(format!("{key}: {shorts}"));
                    }
                    Completion::Failed(_) => {}
                }
            }
            Msg::Close => {
                if let Some(p) = self.prompt.take() {
                    o.status(format!("{}: mark cancelled", p.key));
                }
            }
            Msg::Reset => self.prompt = None,
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::effect::{Effect, Request};
    use crate::tui::fixtures::*;
    use crate::tui::note::Note;
    use crossterm::event::{KeyCode, KeyModifiers};
    use reeve_protocol::DeliveryState::*;

    fn press(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }
    fn cx(b: &board::State) -> Ctx<'_> {
        Ctx {
            board: b.view(),
            read_only: None,
        }
    }
    fn opened(b: &board::State, reason: Option<&str>) -> State {
        let mut u = State::default();
        let o = u.update(
            Msg::Open {
                key: key("ABC-1"),
                reason: reason.map(str::to_string),
            },
            &cx(b),
        );
        assert!(o.effects.is_empty(), "opening sends nothing");
        assert_eq!(u.open(), Some(&key("ABC-1")));
        u
    }
    /// The one mark the output sends, as (key, reason).
    fn the_mark(o: &Out<Msg>) -> (Key, Option<String>) {
        match o.effects.as_slice() {
            [
                Effect::Http {
                    req: Request::MarkUrgent { key, reason },
                    ..
                },
            ] => (key.clone(), reason.clone()),
            other => panic!("{other:?}"),
        }
    }
    fn statuses(o: &Out<Msg>) -> Vec<&str> {
        o.notes
            .iter()
            .filter_map(|n| match n {
                Note::Status(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn enter_sends_one_mark_with_the_typed_reason_and_closes() {
        let b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, None);
        for c in "abc".chars() {
            let _ = u.update(press(KeyCode::Char(c)), &cx(&b));
        }
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert_eq!(the_mark(&o), (key("ABC-1"), Some("abc".into())));
        assert_eq!(
            o.notes
                .iter()
                .filter(|n| matches!(n, Note::WriteStarted(k) if *k == key("ABC-1")))
                .count(),
            1
        );
        assert!(u.open().is_none());
    }

    #[test]
    fn an_empty_reason_is_sent_as_none() {
        let b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, None);
        let _ = u.update(press(KeyCode::Char(' ')), &cx(&b));
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert_eq!(the_mark(&o), (key("ABC-1"), None));
    }

    #[test]
    fn a_re_mark_keeps_the_current_reason() {
        let b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, Some("old"));
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert_eq!(the_mark(&o), (key("ABC-1"), Some("old".into())));
    }

    #[test]
    fn esc_cancels_and_sends_nothing() {
        let b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, None);
        let _ = u.update(press(KeyCode::Char('x')), &cx(&b));
        let o = u.update(press(KeyCode::Esc), &cx(&b));
        assert!(o.effects.is_empty());
        assert_eq!(statuses(&o), ["ABC-1: mark cancelled"]);
        assert!(u.open().is_none());
    }

    #[test]
    fn a_story_locked_while_the_prompt_is_open_refuses_enter() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, None);
        let _ = b.on_note(
            &Note::WriteStarted(key("ABC-1")),
            &board::Ctx {
                me: None,
                wide: true,
                seq: 0,
            },
        );
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert!(o.effects.is_empty());
        assert!(!o.notes.iter().any(|n| matches!(n, Note::WriteStarted(_))));
        assert_eq!(statuses(&o), ["ABC-1: a change is still saving"]);
        assert!(u.open().is_none());
    }

    #[test]
    fn read_only_at_enter_refuses_and_closes() {
        let b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, None);
        let o = u.update(
            press(KeyCode::Enter),
            &Ctx {
                board: b.view(),
                read_only: Some("signed out"),
            },
        );
        assert!(o.effects.is_empty());
        assert_eq!(statuses(&o), ["read-only: signed out"]);
        assert!(u.open().is_none());
    }

    #[test]
    fn reset_closes_silently_so_a_late_enter_sends_nothing() {
        let b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, None);
        let o = u.update(Msg::Reset, &cx(&b));
        assert!(o.notes.is_empty() && o.effects.is_empty());
        assert!(u.open().is_none());
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert!(o.effects.is_empty());
    }

    #[test]
    fn close_words_the_cancel_only_when_the_prompt_was_open() {
        let b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, None);
        let o = u.update(Msg::Close, &cx(&b));
        assert_eq!(statuses(&o), ["ABC-1: mark cancelled"]);
        let o = u.update(Msg::Close, &cx(&b));
        assert!(o.notes.is_empty());
    }
}
