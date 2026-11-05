//! Set epic (`e`): a picker over `GET /epics`, filtered as you type, then `PATCH /stories/{key}` with the version.
pub mod view;

use crate::tui::board;
use crate::tui::effect::{Completion, Effect, Reply, Request, Stamp};
use crate::tui::generation::Gen;
use crate::tui::line::{LineEditor, LineEvent};
use crate::tui::note::{Note, Out, WriteOutcome};
use crossterm::event::{KeyCode, KeyEvent};
use reeve_protocol::{EditItem, EpicDto, Key, KeyRef};

#[derive(Debug)]
pub struct Picker {
    pub key: Key,
    pub version: i64,
    epoch: u64,
    /// `None` while loading.
    pub epics: Option<Vec<EpicDto>>,
    pub error: Option<String>,
    pub filter: LineEditor,
    pub cursor: usize,
}

#[derive(Debug, Default)]
pub struct State {
    picker: Option<Picker>,
    epoch: Gen,
}

pub struct Ctx<'a> {
    pub board: board::View<'a>,
    pub read_only: Option<&'a str>,
}

#[derive(Debug)]
pub enum Msg {
    Open,
    Loaded {
        epoch: u64,
        result: Completion,
    },
    Key(KeyEvent),
    Done {
        key: Key,
        stamp: Stamp,
        result: Completion,
    },
    /// Another overlay opened (a move prompt): close the picker, saying so.
    Close,
    /// The board's data changed: close a picker built on an older version of its story.
    BoardChanged,
    /// Signed out: close the picker, silently.
    Reset,
}

impl State {
    /// The picker while it is open.
    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    /// The loaded epics matching the filter, by key number.
    pub fn visible(&self) -> Vec<&EpicDto> {
        let Some(p) = &self.picker else { return vec![] };
        let q = p.filter.text().to_lowercase();
        let mut v: Vec<&EpicDto> = p
            .epics
            .iter()
            .flatten()
            .filter(|e| {
                q.is_empty()
                    || e.key.to_string().to_lowercase().contains(&q)
                    || e.title.to_lowercase().contains(&q)
            })
            .collect();
        v.sort_by_key(|e| e.key.number());
        v
    }

    /// Enter in the picker: the highlighted epic, written with the version captured when the picker opened, unless
    /// writes became disabled or another write on the story started meanwhile (the guard runs at dispatch).
    fn submit(&mut self, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        let chosen = {
            let v = self.visible();
            let c = self.picker.as_ref().map_or(0, |p| p.cursor);
            v.get(c.min(v.len().saturating_sub(1)))
                .map(|e| e.key.clone())
        };
        let Some(p) = self.picker.as_ref() else {
            return o;
        };
        if let Some(why) = cx.read_only {
            self.picker = None;
            o.status(format!("read-only: {why}"));
            return o;
        }
        if cx.board.locked(&p.key) {
            o.status(format!("{}: a change is still saving", p.key));
            self.picker = None;
            return o;
        }
        if chosen.is_none() {
            // Still loading, or nothing matches: stay open rather than close silently.
            o.status("no epic to choose");
            return o;
        }
        let Some(p) = self.picker.take() else {
            return o;
        };
        if let Some(epic) = chosen {
            let body = EditItem {
                version: p.version,
                epic: Some(Some(KeyRef::from(epic))),
                ..Default::default()
            };
            o.tell(Note::WriteStarted(p.key.clone()));
            let key = p.key.clone();
            o.push(Effect::http(
                Request::Edit { key: p.key, body },
                move |stamp, result| Msg::Done { key, stamp, result },
            ));
        }
        o
    }

    pub fn update(&mut self, msg: Msg, cx: &Ctx) -> Out<Msg> {
        let mut o = Out::none();
        match msg {
            Msg::Open => match cx.board.writable(cx.read_only) {
                Err(why) => o.status(why),
                Ok(s) => {
                    let epoch = self.epoch.bump();
                    self.picker = Some(Picker {
                        key: s.key,
                        version: s.version,
                        epoch,
                        epics: None,
                        error: None,
                        filter: LineEditor::default(),
                        cursor: 0,
                    });
                    o.push(Effect::http(Request::Epics, move |_, result| Msg::Loaded {
                        epoch,
                        result,
                    }));
                }
            },
            Msg::Loaded { epoch, result } => {
                if let Some(p) = self.picker.as_mut().filter(|p| p.epoch == epoch) {
                    match result {
                        Completion::Ok(Reply::Epics(list)) => p.epics = Some(list),
                        Completion::Failed(f) => p.error = Some(board::describe(&f)),
                        other => p.error = Some(format!("unexpected reply: {other:?}")),
                    }
                }
            }
            Msg::Key(k) => {
                let Some(p) = self.picker.as_mut() else {
                    return o;
                };
                let ev = match k.code {
                    KeyCode::Down => {
                        p.cursor += 1;
                        None
                    }
                    KeyCode::Up => {
                        p.cursor = p.cursor.saturating_sub(1);
                        None
                    }
                    _ => {
                        let before = p.filter.text().to_string();
                        Some(match p.filter.handle(&k) {
                            // Left, Right, Home and End are edits that leave the text alone.
                            LineEvent::Edited if p.filter.text() == before => LineEvent::Ignored,
                            ev => ev,
                        })
                    }
                };
                match ev {
                    Some(LineEvent::Cancel) => self.picker = None,
                    Some(LineEvent::Edited) => {
                        if let Some(p) = self.picker.as_mut() {
                            p.cursor = 0;
                        }
                    }
                    Some(LineEvent::Submit) => o.extend(self.submit(cx)),
                    Some(LineEvent::Ignored) | None => {}
                }
                // After any key the cursor stays within the visible list.
                let n = self.visible().len();
                if let Some(p) = self.picker.as_mut() {
                    p.cursor = p.cursor.min(n.saturating_sub(1));
                }
            }
            Msg::Done { key, stamp, result } => {
                o.tell(Note::WriteFinished {
                    key: key.clone(),
                    stamp,
                    outcome: WriteOutcome::of(&result),
                });
                // Failures are the core's to word (`app::State::classify`, by the write's story).
                if let Completion::Ok(Reply::Story(s)) = result {
                    o.status(format!(
                        "{key} → epic {}",
                        s.epic
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "none".into())
                    ));
                }
            }
            Msg::Close => {
                if self.picker.take().is_some() {
                    o.status("epic picker closed");
                }
            }
            Msg::BoardChanged => {
                let stale = self
                    .picker
                    .as_ref()
                    .filter(|p| cx.board.version(&p.key) != Some(p.version))
                    .map(|p| p.key.clone());
                if let Some(k) = stale {
                    self.picker = None;
                    o.status(format!("{k} changed — epic picker closed"));
                }
            }
            Msg::Reset => self.picker = None,
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

    fn epic(k: &str, title: &str) -> EpicDto {
        EpicDto {
            key: key(k),
            title: title.into(),
            initiative: None,
            brief: String::new(),
            discovery_complete: None,
            external_ref: None,
            version: 1,
        }
    }
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn opened(b: &board::State) -> (State, u64) {
        let mut e = State::default();
        let o = e.update(
            Msg::Open,
            &Ctx {
                board: b.view(),
                read_only: None,
            },
        );
        let [
            Effect::Http {
                req: Request::Epics,
                route,
            },
        ] = <[_; 1]>::try_from(o.effects).unwrap()
        else {
            panic!()
        };
        let Msg::Loaded { epoch, .. } = route(stamp(1, 1), Completion::Ok(Reply::Epics(vec![])))
        else {
            panic!()
        };
        (e, epoch)
    }

    #[test]
    fn open_loads_the_epics_and_a_reply_for_an_earlier_opening_is_ignored() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        b.test_select(key("ABC-1"));
        let (mut e, first) = opened(&b);
        let cx = Ctx {
            board: b.view(),
            read_only: None,
        };
        let _ = e.update(Msg::Key(press(KeyCode::Esc)), &cx);
        let _ = e.update(Msg::Open, &cx);
        let _ = e.update(
            Msg::Loaded {
                epoch: first,
                result: Completion::Ok(Reply::Epics(vec![epic("ABC-9", "Old")])),
            },
            &cx,
        );
        assert!(
            e.picker.as_ref().unwrap().epics.is_none(),
            "a reply for an earlier opening is dropped"
        );
    }

    #[test]
    fn typing_filters_and_enter_sends_the_edit_with_the_version() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        b.test_select(key("ABC-1"));
        let (mut e, epoch) = opened(&b);
        let cx = Ctx {
            board: b.view(),
            read_only: None,
        };
        let _ = e.update(
            Msg::Loaded {
                epoch,
                result: Completion::Ok(Reply::Epics(vec![
                    epic("ABC-7", "Billing"),
                    epic("ABC-8", "Auth"),
                ])),
            },
            &cx,
        );
        for c in "auth".chars() {
            let _ = e.update(Msg::Key(press(KeyCode::Char(c))), &cx);
        }
        assert_eq!(
            e.visible()
                .iter()
                .map(|x| x.key.to_string())
                .collect::<Vec<_>>(),
            ["ABC-8"]
        );
        let o = e.update(Msg::Key(press(KeyCode::Enter)), &cx);
        match o.effects.as_slice() {
            [
                Effect::Http {
                    req: Request::Edit { key: k, body },
                    ..
                },
            ] => {
                assert_eq!(*k, key("ABC-1"));
                assert_eq!(body.version, 2);
                assert_eq!(
                    body.epic.as_ref().unwrap().as_ref().unwrap().as_str(),
                    "ABC-8"
                );
                assert!(body.owner.is_none(), "only the epic changes");
            }
            other => panic!("{other:?}"),
        }
        assert!(o.notes.iter().any(|n| matches!(n, Note::WriteStarted(_))));
        assert!(e.picker.is_none());
    }

    #[test]
    fn a_write_started_while_the_picker_was_open_blocks_the_submit() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        b.test_select(key("ABC-1"));
        let (mut e, epoch) = opened(&b);
        let _ = e.update(
            Msg::Loaded {
                epoch,
                result: Completion::Ok(Reply::Epics(vec![epic("ABC-7", "Billing")])),
            },
            &Ctx {
                board: b.view(),
                read_only: None,
            },
        );
        let _ = b.on_note(
            &Note::WriteStarted(key("ABC-1")),
            &board::Ctx {
                me: None,
                wide: true,
                seq: 0,
            },
        );
        let o = e.update(
            Msg::Key(press(KeyCode::Enter)),
            &Ctx {
                board: b.view(),
                read_only: None,
            },
        );
        assert!(o.effects.is_empty());
        assert!(
            o.notes
                .iter()
                .any(|n| matches!(n, Note::Status(s) if s.contains("still saving")))
        );
    }

    #[test]
    fn read_only_refuses_and_done_always_finishes_the_write() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        b.test_select(key("ABC-1"));
        let mut e = State::default();
        let o = e.update(
            Msg::Open,
            &Ctx {
                board: b.view(),
                read_only: Some("signed out"),
            },
        );
        assert!(o.effects.is_empty() && e.picker.is_none());
        let o = e.update(
            Msg::Done {
                key: key("ABC-1"),
                stamp: stamp(3, 3),
                result: Completion::Failed(Failure::Transport("t".into())),
            },
            &Ctx {
                board: b.view(),
                read_only: None,
            },
        );
        assert!(o.notes.iter().any(|n| matches!(
            n,
            Note::WriteFinished {
                outcome: WriteOutcome::Unknown,
                ..
            }
        )));
    }

    #[test]
    fn submit_rechecks_read_only_and_closes_without_writing() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        b.test_select(key("ABC-1"));
        let (mut e, epoch) = opened(&b);
        let _ = e.update(
            Msg::Loaded {
                epoch,
                result: Completion::Ok(Reply::Epics(vec![epic("ABC-7", "Billing")])),
            },
            &Ctx {
                board: b.view(),
                read_only: None,
            },
        );
        let o = e.update(
            Msg::Key(press(KeyCode::Enter)),
            &Ctx {
                board: b.view(),
                read_only: Some("signed out"),
            },
        );
        assert!(o.effects.is_empty());
        assert!(!o.notes.iter().any(|n| matches!(n, Note::WriteStarted(_))));
        assert!(
            o.notes
                .iter()
                .any(|n| matches!(n, Note::Status(s) if s == "read-only: signed out"))
        );
        assert!(e.picker.is_none());
    }

    #[test]
    fn enter_with_nothing_to_choose_keeps_the_picker_open() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        b.test_select(key("ABC-1"));
        let (mut e, epoch) = opened(&b);
        let cx = Ctx {
            board: b.view(),
            read_only: None,
        };
        // Loading: no epics yet.
        let mut e2 = State::default();
        let _ = e2.update(Msg::Open, &cx);
        let o = e2.update(Msg::Key(press(KeyCode::Enter)), &cx);
        assert!(o.effects.is_empty() && e2.picker.is_some());
        assert!(
            o.notes
                .iter()
                .any(|n| matches!(n, Note::Status(s) if s == "no epic to choose"))
        );
        // Loaded, but the filter matches nothing.
        let _ = e.update(
            Msg::Loaded {
                epoch,
                result: Completion::Ok(Reply::Epics(vec![epic("ABC-7", "Billing")])),
            },
            &cx,
        );
        let _ = e.update(Msg::Key(press(KeyCode::Char('z'))), &cx);
        let o = e.update(Msg::Key(press(KeyCode::Enter)), &cx);
        assert!(o.effects.is_empty() && e.picker.is_some());
        assert!(
            o.notes
                .iter()
                .any(|n| matches!(n, Note::Status(s) if s == "no epic to choose"))
        );
    }

    #[test]
    fn moving_the_text_cursor_does_not_reset_the_list_cursor() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        b.test_select(key("ABC-1"));
        let (mut e, epoch) = opened(&b);
        let cx = Ctx {
            board: b.view(),
            read_only: None,
        };
        let _ = e.update(
            Msg::Loaded {
                epoch,
                result: Completion::Ok(Reply::Epics(vec![
                    epic("ABC-7", "Billing"),
                    epic("ABC-8", "Auth"),
                ])),
            },
            &cx,
        );
        let _ = e.update(Msg::Key(press(KeyCode::Char('a'))), &cx);
        let _ = e.update(Msg::Key(press(KeyCode::Down)), &cx);
        let at = e.picker.as_ref().unwrap().cursor;
        assert_eq!(at, 1, "two epics match 'a'");
        let _ = e.update(Msg::Key(press(KeyCode::Left)), &cx);
        let _ = e.update(Msg::Key(press(KeyCode::Home)), &cx);
        assert_eq!(e.picker.as_ref().unwrap().cursor, at);
    }

    #[test]
    fn close_says_so_only_when_a_picker_was_open_and_reset_is_silent() {
        let mut b = board_with(vec![story("ABC-1", Todo, 2)]);
        b.test_select(key("ABC-1"));
        let cx = Ctx {
            board: b.view(),
            read_only: None,
        };
        let (mut e, _) = opened(&b);
        let o = e.update(Msg::Close, &cx);
        assert!(e.picker.is_none());
        assert!(matches!(&o.notes[..], [Note::Status(s)] if s == "epic picker closed"));
        let o = e.update(Msg::Close, &cx);
        assert!(o.notes.is_empty());

        let (mut e, _) = opened(&b);
        let o = e.update(Msg::Reset, &cx);
        assert!(e.picker.is_none());
        assert!(o.notes.is_empty() && o.effects.is_empty());
    }
}
