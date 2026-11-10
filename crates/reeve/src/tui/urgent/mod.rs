//! Mark urgent (`u`): a one-line reason prompt, then `POST /stories/{key}/urgent` (kb/ADR-0017). The slice owns the
//! write from the prompt to its one `WriteFinished`, as `set_epic` does. A mark takes no version (kb/ADR-0017 R5), so
//! nothing in the prompt can go stale while it is open.
pub mod view;

use crate::glyphs::Level;
use crate::tui::board;
use crate::tui::effect::{Completion, Effect, Request, Stamp};
use crate::tui::keys::Action;
use crate::tui::line::{LineEditor, LineEvent};
use crate::tui::note::{Note, Out, WriteOutcome};
use crate::tui::words;
use crossterm::event::KeyEvent;
use reeve_protocol::{DeliveryState, Key, StoryDto};

/// Why `u` does nothing for a done story: a mark on it would make nothing urgent (kb/ADR-0017 R3).
const NOT_WHEN_DONE: &str = "done stories cannot be urgent";
/// Why `U` does nothing for a story with neither a mark of its own nor a source.
const NO_MARK: &str = "no urgent mark";
/// The server's cap on a reason, in characters; the prompt keeps a longer one, as the web's `maxlength` does.
const REASON_MAX: usize = 200;

/// Why `u` or `U` would do nothing for `story` (the token aside): the one place the done and no-mark conditions are
/// written, for the core's shortcuts and menu alike. Presentation only; the server decides (kb/ADR-0011 R9).
pub fn blocked(story: &StoryDto, a: Action) -> Option<String> {
    match a {
        Action::MarkUrgent if story.delivery == DeliveryState::Done => Some(NOT_WHEN_DONE.into()),
        // Urgent through its lineage only: the card shows `!`, so say where it comes from.
        Action::ClearUrgent if story.urgent_mark.is_none() => Some(
            match story.urgent_sources.iter().find(|s| s.key != story.key) {
                Some(src) => format!(
                    "no own mark: urgent via {} {}",
                    Level::of_item(src.item).name(),
                    src.key
                ),
                None => NO_MARK.into(),
            },
        ),
        _ => None,
    }
}

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

    /// Enter: one mark with the trimmed reason (none when blank), unless writes became disabled, the story left the
    /// board or became one `u` refuses (`blocked`, e.g. done by a live update), or another write on it started while
    /// the prompt was open (a pending move); in each case the prompt closes. Otherwise a reason over the server's cap
    /// keeps the prompt open and sends nothing (presentation, like the web's `maxlength`).
    fn submit(&mut self, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        let Some(p) = self.prompt.take() else {
            return o;
        };
        if let Some(why) = cx.read_only {
            o.status(format!("read-only: {why}"));
            return o;
        }
        let Some(story) = cx.board.story(&p.key) else {
            o.status(format!("{} is no longer on the board", p.key));
            return o;
        };
        if let Some(why) = blocked(story, Action::MarkUrgent) {
            o.status(format!("{}: {why}", p.key));
            return o;
        }
        if cx.board.locked(&p.key) {
            o.status(format!("{}: a change is still saving", p.key));
            return o;
        }
        let text = p.line.text().trim();
        if text.chars().count() > REASON_MAX {
            o.status(format!(
                "{}: reason must be at most {REASON_MAX} characters",
                p.key
            ));
            self.prompt = Some(p);
            return o;
        }
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
                        o.status(format!("{key}: {}", words::shorts(reasons)));
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
        let mut u = opened(&b, Some("Åsa"));
        let _ = u.update(press(KeyCode::Char('!')), &cx(&b));
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert_eq!(the_mark(&o), (key("ABC-1"), Some("Åsa!".into())));
    }

    #[test]
    fn done_finishes_its_own_write_once_whatever_the_prompt_holds() {
        use crate::tui::effect::{Failure, Reply};
        use crate::tui::note::WriteOutcome;
        use reeve_protocol::Reason;
        let b = board_with(vec![story("ABC-1", Todo, 2), story("ABC-2", Todo, 1)]);
        let marked = story("ABC-1", Todo, 2).story;
        let finished = |o: &Out<Msg>| -> Vec<(Key, WriteOutcome)> {
            o.notes
                .iter()
                .filter_map(|n| match n {
                    Note::WriteFinished { key, outcome, .. } => {
                        Some((key.clone(), outcome.clone()))
                    }
                    _ => None,
                })
                .collect()
        };
        let cases: [(Completion, WriteOutcome, &[&str]); 3] = [
            (
                Completion::Ok(Reply::Story(marked.clone())),
                WriteOutcome::Applied(marked.clone()),
                &["ABC-1 marked urgent"],
            ),
            (
                refused(vec![Reason::InteractiveOnly]),
                WriteOutcome::Unchanged,
                &["ABC-1: interactive only"],
            ),
            (
                Completion::Failed(Failure::Transport("t".into())),
                WriteOutcome::Unknown,
                &[],
            ),
        ];
        let mut u = State::default();
        let _ = u.update(
            Msg::Open {
                key: key("ABC-2"),
                reason: None,
            },
            &cx(&b),
        );
        for (result, outcome, status) in cases {
            let o = u.update(
                Msg::Done {
                    key: key("ABC-1"),
                    stamp: stamp(1, 1),
                    result: result.clone(),
                },
                &cx(&b),
            );
            assert_eq!(finished(&o), [(key("ABC-1"), outcome)], "{result:?}");
            assert_eq!(statuses(&o), status, "{result:?}");
            assert!(o.effects.is_empty(), "{result:?}");
            assert_eq!(u.open(), Some(&key("ABC-2")), "{result:?}");
        }
        let _ = u.update(Msg::Reset, &cx(&b));
        let o = u.update(
            Msg::Done {
                key: key("ABC-1"),
                stamp: stamp(2, 2),
                result: Completion::Failed(Failure::Transport("t".into())),
            },
            &cx(&b),
        );
        assert_eq!(
            finished(&o),
            [(key("ABC-1"), WriteOutcome::Unknown)],
            "a write finishes after a sign-out too"
        );
    }

    #[test]
    fn a_reason_over_200_characters_keeps_the_prompt_and_sends_nothing() {
        let b = board_with(vec![story("ABC-1", Todo, 2)]);
        // 201 characters, 402 bytes: the limit counts characters, and the surrounding blanks are trimmed first.
        let long = format!(" {} ", "å".repeat(201));
        let mut u = opened(&b, Some(&long));
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert!(o.effects.is_empty());
        assert!(!o.notes.iter().any(|n| matches!(n, Note::WriteStarted(_))));
        assert_eq!(
            statuses(&o),
            ["ABC-1: reason must be at most 200 characters"]
        );
        assert_eq!(u.open(), Some(&key("ABC-1")), "the prompt stays open");
        // Deleting one character brings it to 200, and Enter sends it.
        let _ = u.update(press(KeyCode::End), &cx(&b));
        let _ = u.update(press(KeyCode::Backspace), &cx(&b));
        let _ = u.update(press(KeyCode::Backspace), &cx(&b));
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert_eq!(the_mark(&o), (key("ABC-1"), Some("å".repeat(200))));
        assert!(u.open().is_none());
    }

    #[test]
    fn blocked_words_why_u_or_capital_u_does_nothing() {
        use crate::tui::keys::Action;
        let plain = story("ABC-1", Todo, 2).story;
        assert_eq!(blocked(&plain, Action::MarkUrgent), None);
        assert_eq!(
            blocked(&plain, Action::ClearUrgent).as_deref(),
            Some("no urgent mark")
        );
        let done = done_marked(story("ABC-1", Todo, 2)).story;
        assert_eq!(
            blocked(&done, Action::MarkUrgent).as_deref(),
            Some("done stories cannot be urgent")
        );
        assert_eq!(blocked(&done, Action::ClearUrgent), None);
        let via = urgent_via_epic(story("ABC-1", Todo, 2)).story;
        assert_eq!(
            blocked(&via, Action::ClearUrgent).as_deref(),
            Some("no own mark: urgent via epic ABC-100")
        );
        // The story itself listed first is skipped: the source named is the first one that is not the story.
        let mut both = urgent_via_epic(story("ABC-1", Todo, 2)).story;
        both.urgent_sources.insert(
            0,
            reeve_protocol::ItemRefDto {
                key: key("ABC-1"),
                item: reeve_protocol::ItemType::Story,
            },
        );
        assert_eq!(
            blocked(&both, Action::ClearUrgent).as_deref(),
            Some("no own mark: urgent via epic ABC-100")
        );
        assert_eq!(
            blocked(&urgent(story("ABC-1", Todo, 2)).story, Action::ClearUrgent),
            None
        );
        assert_eq!(blocked(&plain, Action::Take), None);
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

    /// The board as a live update leaves it: a refetch, then a load of `list`.
    fn reload(b: &mut board::State, list: Vec<reeve_protocol::StoryWithReadiness>) {
        let bcx = board::Ctx {
            me: Some("dan@example.com"),
            wide: true,
            seq: 0,
        };
        let _ = b.on_note(&Note::Refetch, &bcx);
        let _ = b.update(
            board::Msg::Loaded {
                stamp: stamp(2, 2),
                result: stories(list),
            },
            &bcx,
        );
    }

    #[test]
    fn a_story_done_while_the_prompt_is_open_refuses_enter() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        let mut u = opened(&b, None);
        reload(&mut b, vec![story("ABC-1", Done, 3)]);
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert!(o.effects.is_empty());
        assert!(!o.notes.iter().any(|n| matches!(n, Note::WriteStarted(_))));
        assert_eq!(statuses(&o), ["ABC-1: done stories cannot be urgent"]);
        assert!(u.open().is_none());
    }

    #[test]
    fn a_story_gone_while_the_prompt_is_open_refuses_enter() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2), story("ABC-2", Todo, 1)]);
        let mut u = opened(&b, None);
        reload(&mut b, vec![story("ABC-2", Todo, 1)]);
        let o = u.update(press(KeyCode::Enter), &cx(&b));
        assert!(o.effects.is_empty());
        assert!(!o.notes.iter().any(|n| matches!(n, Note::WriteStarted(_))));
        assert_eq!(statuses(&o), ["ABC-1 is no longer on the board"]);
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
