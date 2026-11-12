//! The event stream: connect, resume with the last event id, reconnect with backoff, and
//! after any entry or (re)connect ask the board to refetch, debounced as the web client is (`web/src/lib/events.svelte.ts`:
//! 500 ms after the last change, at most 5 s after the first of a burst).
use crate::tui::effect::{Effect, Ended, StreamEvent};
use crate::tui::generation::Gen;
use crate::tui::note::{Note, Out};
use crate::tui::words;
use reeve_protocol::{ActivityEntryDto, Key};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub const TRAIL: Duration = Duration::from_millis(500);
pub const CAP: Duration = Duration::from_secs(5);
pub const BACKOFF: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conn {
    Connecting,
    Live,
    Reconnecting,
    Off,
}

#[derive(Debug, Clone)]
pub struct Ticker {
    pub text: String,
    pub at: Instant,
}

#[derive(Debug)]
pub struct State {
    conn: Conn,
    epoch: Gen,
    last_id: Option<i64>,
    attempt: usize,
    trail: Gen,
    cap: Gen,
    burst: bool,
    ticker: Option<Ticker>,
    last_changed: Option<(Key, Instant)>,
    override_for: Option<String>,
    backoff: Vec<Duration>,
}

impl Default for State {
    fn default() -> Self {
        Self::with_backoff(BACKOFF.to_vec())
    }
}

pub struct Ctx {
    pub now: Instant,
}

#[derive(Debug)]
pub enum Msg {
    Start,
    Stream { epoch: u64, ev: StreamEvent },
    Trail(u64),
    Cap(u64),
    Retry(u64),
    Stop,
}

impl State {
    pub fn conn(&self) -> Conn {
        self.conn
    }
    /// The newest activity entry, worded for the status line.
    pub fn ticker(&self) -> Option<&Ticker> {
        self.ticker.as_ref()
    }
    /// The story the newest activity entry changed, and when it arrived.
    pub fn last_changed(&self) -> Option<&(Key, Instant)> {
        self.last_changed.as_ref()
    }

    pub fn with_backoff(backoff: Vec<Duration>) -> Self {
        assert!(!backoff.is_empty());
        Self {
            conn: Conn::Connecting,
            epoch: Gen::default(),
            last_id: None,
            attempt: 0,
            trail: Gen::default(),
            cap: Gen::default(),
            burst: false,
            ticker: None,
            last_changed: None,
            override_for: None,
            backoff,
        }
    }

    fn connect(&mut self) -> Out<Msg> {
        let epoch = self.epoch.bump();
        let mut o = Out::none();
        o.push(Effect::Connect {
            after_id: self.last_id,
            route: Arc::new(move |ev| Msg::Stream { epoch, ev }),
        });
        o
    }

    /// A change: restart the trailing timer; start the cap if this is the first change of a burst.
    fn changed(&mut self) -> Out<Msg> {
        let mut o = Out::none();
        o.push(Effect::after(TRAIL, Msg::Trail(self.trail.bump())));
        if !self.burst {
            self.burst = true;
            o.push(Effect::after(CAP, Msg::Cap(self.cap.bump())));
        }
        o
    }

    fn fire(&mut self) -> Out<Msg> {
        self.burst = false;
        self.trail.bump();
        self.cap.bump();
        let mut o = Out::none();
        o.tell(Note::Refetch);
        o
    }

    fn on_entry(&mut self, e: &ActivityEntryDto, now: Instant) {
        let who = words::local_part(&e.actor);
        let k = &e.entity_key;
        let who_what = if e.entity_type == "story" {
            k.clone()
        } else {
            format!("{} {k}", e.entity_type)
        };
        let k = &who_what;
        let text = match e.action.as_str() {
            "override" => {
                self.override_for = Some(k.clone());
                format!("{k} override · {who}")
            }
            "moved" => {
                let to = e.payload["to"].as_str().unwrap_or("?").replace('_', " ");
                let to = match e.payload["resolution"].as_str() {
                    Some(r) if to == "done" => format!("{to} ({})", r.replace('_', " ")),
                    _ => to,
                };
                let tag = if self.override_for.as_deref() == Some(k.as_str()) {
                    " (override)"
                } else {
                    ""
                };
                self.override_for = None;
                format!("{k} → {to}{tag} · {who}")
            }
            "urgent_marked" => {
                self.override_for = None;
                format!("{k} marked urgent · {who}")
            }
            "urgent_cleared" => {
                self.override_for = None;
                format!("{k} urgent mark cleared · {who}")
            }
            // A promise's words are the web's activity note's (`actionLabel`), the actor first, as `freshNote` reads.
            "promised" | "promise_cleared" => {
                self.override_for = None;
                let words = crate::promise::activity_words(&e.action, &e.payload, Some(k))
                    .unwrap_or_else(|| e.action.replace('_', " "));
                format!("{who} {words}")
            }
            other => {
                self.override_for = None;
                format!("{k} {} · {who}", other.replace('_', " "))
            }
        };
        self.ticker = Some(Ticker { text, at: now });
        // The mark is on the entry's story until the next entry, whatever that entry is about.
        self.last_changed = if e.entity_type == "story" {
            e.entity_key.parse::<Key>().ok().map(|key| (key, now))
        } else {
            None
        };
    }

    pub fn update(&mut self, msg: Msg, cx: &Ctx) -> Out<Msg> {
        match msg {
            Msg::Start => self.connect(),
            Msg::Stop => {
                self.epoch.bump();
                self.trail.bump();
                self.cap.bump();
                self.burst = false;
                self.conn = Conn::Off;
                let mut o = Out::none();
                o.push(Effect::Disconnect);
                o
            }
            Msg::Retry(g) if self.epoch.is(g) && self.conn == Conn::Reconnecting => self.connect(),
            Msg::Retry(_) => Out::none(),
            Msg::Trail(_) | Msg::Cap(_) if self.conn == Conn::Off => Out::none(),
            Msg::Trail(g) if self.trail.is(g) => self.fire(),
            Msg::Cap(g) if self.cap.is(g) => self.fire(),
            Msg::Trail(_) | Msg::Cap(_) => Out::none(),
            Msg::Stream { epoch, .. } if !self.epoch.is(epoch) || self.conn == Conn::Off => {
                Out::none()
            }
            Msg::Stream { ev, .. } => match ev {
                StreamEvent::Opened => {
                    self.conn = Conn::Live;
                    self.changed()
                }
                StreamEvent::Frame(f) => match f.event.as_deref() {
                    Some("cursor") => {
                        self.last_id =
                            f.id.as_deref()
                                .and_then(|s| s.parse().ok())
                                .or_else(|| f.data.parse().ok())
                                .or(self.last_id);
                        self.attempt = 0;
                        Out::none()
                    }
                    Some("activity") => match serde_json::from_str::<ActivityEntryDto>(&f.data) {
                        Ok(e) => {
                            self.last_id = Some(e.id);
                            self.attempt = 0;
                            self.on_entry(&e, cx.now);
                            let mut o = self.changed();
                            o.tell(Note::Invalidate);
                            o
                        }
                        Err(_) => Out::none(),
                    },
                    _ => Out::none(),
                },
                StreamEvent::Ended(Ended::Unauthorized) => {
                    self.conn = Conn::Off;
                    let mut o = Out::none();
                    o.tell(Note::SignedOut);
                    o
                }
                StreamEvent::Ended(_) => {
                    // The next Opened refetches anyway: drop the pending debounce so a drop costs one refetch.
                    self.trail.bump();
                    self.cap.bump();
                    self.burst = false;
                    self.conn = Conn::Reconnecting;
                    let delay = self.backoff[self.attempt.min(self.backoff.len() - 1)];
                    self.attempt += 1;
                    let mut o = Out::none();
                    o.push(Effect::after(delay, Msg::Retry(self.epoch.current())));
                    o
                }
            },
        }
    }
}

/// Test-only, for tests outside this module; every other change goes through `update` (kb/ADR-0011 R2).
#[cfg(test)]
impl State {
    pub fn test_set_last_changed(&mut self, changed: Option<(Key, Instant)>) {
        self.last_changed = changed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::sse::Frame;
    use std::time::Instant;

    fn cx() -> Ctx {
        Ctx {
            now: Instant::now(),
        }
    }
    fn activity(id: i64, action: &str, key: &str, payload: serde_json::Value) -> StreamEvent {
        let dto = serde_json::json!({
            "id": id, "at": "2026-10-28T10:00:00Z", "actor": "anna@example.com", "via": "interactive",
            "entity_type": "story", "entity_key": key, "action": action, "payload": payload,
        });
        StreamEvent::Frame(Frame {
            event: Some("activity".into()),
            id: Some(id.to_string()),
            data: dto.to_string(),
        })
    }
    fn cursor(id: Option<&str>, data: &str) -> StreamEvent {
        StreamEvent::Frame(Frame {
            event: Some("cursor".into()),
            id: id.map(Into::into),
            data: data.into(),
        })
    }
    fn ticker_after(action: &str, entity: &str, key: &str, payload: serde_json::Value) -> String {
        let (mut l, epoch) = open();
        let dto = serde_json::json!({
            "id": 6, "at": "2026-10-28T10:00:00Z", "actor": "anna@example.com", "via": "interactive",
            "entity_type": entity, "entity_key": key, "action": action, "payload": payload,
        });
        let ev = StreamEvent::Frame(Frame {
            event: Some("activity".into()),
            id: Some("6".into()),
            data: dto.to_string(),
        });
        let _ = l.update(Msg::Stream { epoch, ev }, &cx());
        l.ticker.unwrap().text
    }
    /// Closes the connection, retries, and returns the `after_id` of the new Connect and its epoch.
    fn reconnect(l: &mut State, epoch: u64) -> (Option<i64>, u64) {
        let o = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Ended(Ended::Closed),
            },
            &cx(),
        );
        let (_, Msg::Retry(g)) = timers(&o)[0] else {
            panic!()
        };
        let o = l.update(Msg::Retry(*g), &cx());
        let [Effect::Connect { after_id, route }] = <[_; 1]>::try_from(o.effects).unwrap() else {
            panic!()
        };
        let Msg::Stream { epoch, .. } = route(StreamEvent::Opened) else {
            panic!()
        };
        (after_id, epoch)
    }
    fn first_delay(l: &mut State, epoch: u64) -> u64 {
        let o = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Ended(Ended::Closed),
            },
            &cx(),
        );
        timers(&o)[0].0.as_secs()
    }

    #[test]
    fn a_cursor_frame_sets_the_resume_id_from_its_id_else_its_data() {
        let (mut l, epoch) = open();
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: cursor(Some("42"), "42"),
            },
            &cx(),
        );
        let (after, epoch) = reconnect(&mut l, epoch);
        assert_eq!(after, Some(42));
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Opened,
            },
            &cx(),
        );
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: cursor(Some("junk"), "43"),
            },
            &cx(),
        );
        assert_eq!(reconnect(&mut l, epoch).0, Some(43));
    }

    #[test]
    fn a_drop_discards_the_pending_debounce_so_it_costs_one_refetch() {
        let (mut l, epoch) = open();
        let armed = l.update(
            Msg::Stream {
                epoch,
                ev: activity(6, "edited", "ABC-1", serde_json::json!({})),
            },
            &cx(),
        );
        let (trail, cap) = (trail_gen(&armed), cap_gen(&armed));
        let (_, epoch) = reconnect(&mut l, epoch);
        assert_eq!(
            refetches(&l.update(Msg::Trail(trail), &cx())),
            0,
            "the pre-drop trailing timer is dead"
        );
        if let Some(cap) = cap {
            assert_eq!(
                refetches(&l.update(Msg::Cap(cap), &cx())),
                0,
                "and so is the cap"
            );
        }
        let o = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Opened,
            },
            &cx(),
        );
        assert_eq!(refetches(&l.update(Msg::Trail(trail_gen(&o)), &cx())), 1);
    }

    #[test]
    fn backoff_resets_on_the_first_frame_not_on_opening() {
        let (mut l, mut epoch) = open();
        let mut delays = vec![];
        for _ in 0..3 {
            delays.push(first_delay(&mut l, epoch));
            let o = l.update(Msg::Retry(l.epoch.current()), &cx());
            let [Effect::Connect { route, .. }] = <[_; 1]>::try_from(o.effects).unwrap() else {
                panic!()
            };
            let Msg::Stream { epoch: e2, .. } = route(StreamEvent::Opened) else {
                panic!()
            };
            epoch = e2;
            let _ = l.update(
                Msg::Stream {
                    epoch,
                    ev: StreamEvent::Opened,
                },
                &cx(),
            );
        }
        assert_eq!(
            delays,
            [1, 2, 5],
            "opened-then-closed without a frame keeps backing off"
        );
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: cursor(Some("9"), "9"),
            },
            &cx(),
        );
        assert_eq!(first_delay(&mut l, epoch), 1);
    }

    #[test]
    fn the_ticker_words_non_story_entries_actions_and_resolutions() {
        assert_eq!(
            ticker_after("commented", "decision", "17", serde_json::json!({})),
            "decision 17 commented · anna"
        );
        assert_eq!(
            ticker_after("pr_linked", "story", "ABC-1", serde_json::json!({})),
            "ABC-1 pr linked · anna"
        );
        assert_eq!(
            ticker_after(
                "moved",
                "story",
                "ABC-1",
                serde_json::json!({"from": "in_review", "to": "done", "resolution": "dropped"})
            ),
            "ABC-1 → done (dropped) · anna"
        );
        assert_eq!(
            ticker_after(
                "moved",
                "story",
                "ABC-1",
                serde_json::json!({"from": "in_review", "to": "done", "resolution": null})
            ),
            "ABC-1 → done · anna"
        );
    }

    /// Starts and opens the stream; returns the state and the connection's generation.
    fn open() -> (State, u64) {
        let mut l = State::default();
        let o = l.update(Msg::Start, &cx());
        let [Effect::Connect { route, .. }] = <[_; 1]>::try_from(o.effects).unwrap() else {
            panic!()
        };
        let Msg::Stream { epoch, .. } = route(StreamEvent::Opened) else {
            panic!()
        };
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Opened,
            },
            &cx(),
        );
        (l, epoch)
    }
    fn timers(o: &Out<Msg>) -> Vec<(Duration, &Msg)> {
        o.effects
            .iter()
            .filter_map(|e| {
                if let Effect::After { delay, msg } = e {
                    Some((*delay, msg))
                } else {
                    None
                }
            })
            .collect()
    }
    fn refetches(o: &Out<Msg>) -> usize {
        o.notes
            .iter()
            .filter(|n| matches!(n, Note::Refetch))
            .count()
    }
    fn trail_gen(o: &Out<Msg>) -> u64 {
        timers(o)
            .iter()
            .find_map(|(_, m)| {
                if let Msg::Trail(g) = m {
                    Some(*g)
                } else {
                    None
                }
            })
            .unwrap()
    }
    fn cap_gen(o: &Out<Msg>) -> Option<u64> {
        timers(o)
            .iter()
            .find_map(|(_, m)| if let Msg::Cap(g) = m { Some(*g) } else { None })
    }

    #[test]
    fn an_entry_refetches_500ms_after_the_last_change() {
        let (mut l, epoch) = open();
        let a = l.update(
            Msg::Stream {
                epoch,
                ev: activity(6, "edited", "ABC-1", serde_json::json!({})),
            },
            &cx(),
        );
        let b = l.update(
            Msg::Stream {
                epoch,
                ev: activity(7, "edited", "ABC-1", serde_json::json!({})),
            },
            &cx(),
        );
        assert!(timers(&b).iter().any(|(d, _)| *d == TRAIL));
        assert_eq!(
            refetches(&l.update(Msg::Trail(trail_gen(&a)), &cx())),
            0,
            "a superseded trailing timer does nothing"
        );
        assert_eq!(refetches(&l.update(Msg::Trail(trail_gen(&b)), &cx())), 1);
    }

    #[test]
    fn the_cap_fires_5s_after_the_first_change_while_trailing_timers_keep_being_superseded() {
        let (mut l, epoch) = open();
        // Opening already started a burst (refetch on connect); let it fire so this burst starts clean.
        let _ = l.update(Msg::Cap(l.cap.current()), &cx());
        let first = l.update(
            Msg::Stream {
                epoch,
                ev: activity(6, "edited", "ABC-1", serde_json::json!({})),
            },
            &cx(),
        );
        let cap = cap_gen(&first).expect("the first change of a burst starts the cap");
        assert!(
            timers(&first)
                .iter()
                .any(|(d, m)| *d == CAP && matches!(m, Msg::Cap(_)))
        );
        for id in 7..20 {
            let o = l.update(
                Msg::Stream {
                    epoch,
                    ev: activity(id, "edited", "ABC-1", serde_json::json!({})),
                },
                &cx(),
            );
            assert!(
                cap_gen(&o).is_none(),
                "later changes in the burst do not restart the cap"
            );
        }
        assert_eq!(
            refetches(&l.update(Msg::Cap(cap), &cx())),
            1,
            "a new trailing timer never invalidates the cap"
        );
    }

    #[test]
    fn opening_marks_the_connection_live_and_refetches() {
        let mut l = State::default();
        let o = l.update(Msg::Start, &cx());
        let [Effect::Connect { route, .. }] = <[_; 1]>::try_from(o.effects).unwrap() else {
            panic!()
        };
        let Msg::Stream { epoch, .. } = route(StreamEvent::Opened) else {
            panic!()
        };
        let o = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Opened,
            },
            &cx(),
        );
        assert_eq!(l.conn, Conn::Live);
        assert_eq!(refetches(&l.update(Msg::Trail(trail_gen(&o)), &cx())), 1);
    }

    #[test]
    fn drops_reconnect_with_backoff_and_the_last_event_id() {
        let (mut l, mut epoch) = open();
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: activity(6, "edited", "ABC-1", serde_json::json!({})),
            },
            &cx(),
        );
        let mut delays = vec![];
        for _ in 0..6 {
            let o = l.update(
                Msg::Stream {
                    epoch,
                    ev: StreamEvent::Ended(Ended::Closed),
                },
                &cx(),
            );
            assert_eq!(l.conn, Conn::Reconnecting);
            let (d, Msg::Retry(g)) = timers(&o)[0] else {
                panic!()
            };
            delays.push(d.as_secs());
            let o = l.update(Msg::Retry(*g), &cx());
            let [Effect::Connect { after_id, route }] = <[_; 1]>::try_from(o.effects).unwrap()
            else {
                panic!()
            };
            assert_eq!(after_id, Some(6));
            let Msg::Stream { epoch: g2, .. } = route(StreamEvent::Opened) else {
                panic!()
            };
            epoch = g2;
        }
        assert_eq!(delays, [1, 2, 5, 10, 30, 30]);
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Opened,
            },
            &cx(),
        );
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: cursor(Some("7"), "7"),
            },
            &cx(),
        );
        let o = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Ended(Ended::Idle),
            },
            &cx(),
        );
        assert_eq!(
            timers(&o)[0].0.as_secs(),
            1,
            "a successful open resets the backoff; an idle drop reconnects too"
        );
    }

    #[test]
    fn close_then_401_signs_out_and_stops_reconnecting() {
        let (mut l, epoch) = open();
        let o = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Ended(Ended::Closed),
            },
            &cx(),
        );
        let (_, Msg::Retry(g)) = timers(&o)[0] else {
            panic!()
        };
        let o = l.update(Msg::Retry(*g), &cx());
        let [Effect::Connect { route, .. }] = <[_; 1]>::try_from(o.effects).unwrap() else {
            panic!()
        };
        let Msg::Stream { epoch: g2, .. } = route(StreamEvent::Opened) else {
            panic!()
        };
        let o = l.update(
            Msg::Stream {
                epoch: g2,
                ev: StreamEvent::Ended(Ended::Unauthorized),
            },
            &cx(),
        );
        assert!(o.notes.iter().any(|n| matches!(n, Note::SignedOut)));
        assert!(timers(&o).is_empty());
        assert_eq!(l.conn, Conn::Off);
    }

    #[test]
    fn events_from_an_older_connection_are_ignored() {
        let (mut l, old) = open();
        let o = l.update(
            Msg::Stream {
                epoch: old,
                ev: StreamEvent::Ended(Ended::Closed),
            },
            &cx(),
        );
        let (_, Msg::Retry(g)) = timers(&o)[0] else {
            panic!()
        };
        let _ = l.update(Msg::Retry(*g), &cx());
        let o = l.update(
            Msg::Stream {
                epoch: old,
                ev: activity(9, "edited", "ABC-1", serde_json::json!({})),
            },
            &cx(),
        );
        assert!(o.effects.is_empty() && o.notes.is_empty());
    }

    #[test]
    fn the_ticker_folds_an_override_into_the_move_that_follows_it() {
        let (mut l, epoch) = open();
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: activity(
                    6,
                    "override",
                    "ABC-31",
                    serde_json::json!({"reason": "hotfix"}),
                ),
            },
            &cx(),
        );
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: activity(
                    7,
                    "moved",
                    "ABC-31",
                    serde_json::json!({"from": "todo", "to": "in_progress", "resolution": null}),
                ),
            },
            &cx(),
        );
        assert_eq!(
            l.ticker.as_ref().unwrap().text,
            "ABC-31 → in progress (override) · anna"
        );
        assert_eq!(l.last_changed.as_ref().unwrap().0.to_string(), "ABC-31");
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: activity(
                    8,
                    "moved",
                    "ABC-32",
                    serde_json::json!({"from": "todo", "to": "in_review", "resolution": null}),
                ),
            },
            &cx(),
        );
        assert_eq!(l.ticker.as_ref().unwrap().text, "ABC-32 → in review · anna");
    }

    #[test]
    fn every_entry_invalidates_the_reads_sent_so_far_and_a_cursor_or_open_does_not() {
        let mut l = State::default();
        let o = l.update(Msg::Start, &cx());
        let [Effect::Connect { route, .. }] = <[_; 1]>::try_from(o.effects).unwrap() else {
            panic!()
        };
        let Msg::Stream { epoch, .. } = route(StreamEvent::Opened) else {
            panic!()
        };
        let invalidates = |o: &Out<Msg>| {
            o.notes
                .iter()
                .filter(|n| matches!(n, Note::Invalidate))
                .count()
        };
        let o = l.update(
            Msg::Stream {
                epoch,
                ev: StreamEvent::Opened,
            },
            &cx(),
        );
        assert_eq!(invalidates(&o), 0);
        let o = l.update(
            Msg::Stream {
                epoch,
                ev: cursor(Some("5"), "5"),
            },
            &cx(),
        );
        assert_eq!(invalidates(&o), 0);
        for id in 6..8 {
            let o = l.update(
                Msg::Stream {
                    epoch,
                    ev: activity(id, "edited", "ABC-1", serde_json::json!({})),
                },
                &cx(),
            );
            assert_eq!(invalidates(&o), 1, "entry {id}");
            assert_eq!(refetches(&o), 0, "the refetch itself stays debounced");
        }
    }

    #[test]
    fn a_malformed_activity_frame_is_ignored() {
        let (mut l, epoch) = open();
        let bad = StreamEvent::Frame(Frame {
            event: Some("activity".into()),
            id: Some("6".into()),
            data: "not json".into(),
        });
        let o = l.update(Msg::Stream { epoch, ev: bad }, &cx());
        assert!(o.effects.is_empty() && o.notes.is_empty());
        assert!(l.ticker.is_none());
    }

    #[test]
    fn stop_disconnects_and_ignores_later_events_and_timers() {
        let (mut l, epoch) = open();
        let pending = l.update(
            Msg::Stream {
                epoch,
                ev: activity(6, "edited", "ABC-1", serde_json::json!({})),
            },
            &cx(),
        );
        let (trail, cap) = (trail_gen(&pending), l.cap.current());
        let o = l.update(Msg::Stop, &cx());
        assert!(matches!(o.effects.as_slice(), [Effect::Disconnect]));
        assert!(
            l.update(
                Msg::Stream {
                    epoch,
                    ev: StreamEvent::Ended(Ended::Closed)
                },
                &cx()
            )
            .effects
            .is_empty()
        );
        assert_eq!(
            refetches(&l.update(Msg::Trail(trail), &cx())),
            0,
            "a debounce scheduled before stop does not refetch"
        );
        assert_eq!(refetches(&l.update(Msg::Cap(cap), &cx())), 0);
    }

    #[test]
    fn a_non_story_entry_clears_the_last_changed_mark() {
        let (mut l, epoch) = open();
        let _ = l.update(
            Msg::Stream {
                epoch,
                ev: activity(6, "edited", "ABC-1", serde_json::json!({})),
            },
            &cx(),
        );
        assert!(l.last_changed.is_some());
        let epic = serde_json::json!({"id": 7, "at": "2026-10-28T10:00:00Z", "actor": "anna@example.com", "via": "interactive",
            "entity_type": "epic", "entity_key": "ABC-9", "action": "edited", "payload": {}});
        let ev = StreamEvent::Frame(Frame {
            event: Some("activity".into()),
            id: Some("7".into()),
            data: epic.to_string(),
        });
        let _ = l.update(Msg::Stream { epoch, ev }, &cx());
        assert!(l.last_changed.is_none());
    }

    /// A promise entry reads as the web's activity note does: the actor, then the payload's words naming the item, never
    /// the raw action ("ABC-1 promise cleared", "promised" for a change of date).
    #[test]
    fn the_ticker_words_promises_from_the_payload() {
        let prev = serde_json::json!({"on": "2026-11-20", "to": "Acme"});
        for (action, entity, key, payload, want) in [
            (
                "promised",
                "story",
                "ABC-12",
                serde_json::json!({"item_type": "story", "on": "2026-11-20", "to": "Acme", "previous": null}),
                "anna promised ABC-12 by 20 Nov 2026 to Acme",
            ),
            (
                "promised",
                "story",
                "ABC-12",
                serde_json::json!({"item_type": "story", "on": "2026-11-27", "to": "Acme", "previous": prev}),
                "anna changed the promise on ABC-12 to 27 Nov 2026",
            ),
            (
                "promised",
                "epic",
                "ABC-7",
                serde_json::json!({"item_type": "epic", "on": "2026-11-20", "to": "Beta", "previous": prev}),
                "anna changed the recipient of the promise on epic ABC-7 to Beta",
            ),
            (
                "promise_cleared",
                "story",
                "ABC-12",
                serde_json::json!({"item_type": "story", "previous": prev}),
                "anna cleared the promise on ABC-12",
            ),
            (
                "promise_cleared",
                "epic",
                "ABC-7",
                serde_json::json!({"item_type": "epic", "previous": prev}),
                "anna cleared the promise on epic ABC-7",
            ),
        ] {
            assert_eq!(ticker_after(action, entity, key, payload), want);
        }
    }

    #[test]
    fn the_ticker_names_urgent_marks_and_clearings_at_every_level() {
        for (action, entity, key, want) in [
            (
                "urgent_marked",
                "story",
                "ABC-1",
                "ABC-1 marked urgent · anna",
            ),
            (
                "urgent_marked",
                "epic",
                "ABC-40",
                "epic ABC-40 marked urgent · anna",
            ),
            (
                "urgent_marked",
                "initiative",
                "ABC-2",
                "initiative ABC-2 marked urgent · anna",
            ),
            (
                "urgent_cleared",
                "story",
                "ABC-1",
                "ABC-1 urgent mark cleared · anna",
            ),
            (
                "urgent_cleared",
                "epic",
                "ABC-40",
                "epic ABC-40 urgent mark cleared · anna",
            ),
        ] {
            assert_eq!(
                ticker_after(action, entity, key, serde_json::json!({})),
                want
            );
        }
    }
}
