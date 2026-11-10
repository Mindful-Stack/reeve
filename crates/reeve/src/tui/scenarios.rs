//! Root reducer scenarios: real slices, requests completed the way the runtime completes them.
use crate::tui::effect::{Completion, Effect, Ended, Failure, Reply, Request, StreamEvent};
use crate::tui::fixtures::*;
use crate::tui::moves::Prompt;
use crossterm::event::{KeyCode, KeyModifiers};
use reeve_protocol::{DeliveryState::*, Reason, TokenKind};

fn is_move(r: &Request) -> bool {
    matches!(r, Request::Move { .. })
}
fn is_stories(r: &Request) -> bool {
    matches!(r, Request::Stories)
}

#[test]
fn startup_requests_the_config_the_stories_and_the_stream() {
    let sim = Sim::start(Some(me(TokenKind::Interactive)));
    assert!(sim.has_pending(|r| matches!(r, Request::Config)));
    assert!(sim.has_pending(is_stories));
    assert!(
        sim.other
            .iter()
            .any(|e| matches!(e, Effect::Connect { after_id: None, .. }))
    );
    assert!(
        !sim.has_pending(|r| matches!(r, Request::Me)),
        "a known identity is not asked again"
    );
}

#[test]
fn q_typed_into_the_override_prompt_is_text() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('L');
    sim.fire_timers();
    sim.reply(is_move, refused(vec![Reason::NoCriteria]));
    assert!(matches!(
        sim.s.moves.prompt(),
        Some(Prompt::Override { .. })
    ));
    sim.char('q');
    assert!(!sim.s.quit);
    let Some(Prompt::Override { line, .. }) = sim.s.moves.prompt() else {
        panic!()
    };
    assert_eq!(line.text(), "q");
}

#[test]
fn esc_unwinds_one_layer_per_press() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('/');
    sim.char('a');
    sim.press(KeyCode::Enter, KeyModifiers::NONE);
    sim.press(KeyCode::Enter, KeyModifiers::NONE); // open the drawer
    assert!(sim.s.ui.drawer.is_some());
    sim.press(KeyCode::Esc, KeyModifiers::NONE);
    assert!(sim.s.ui.drawer.is_none());
    assert_eq!(sim.s.board.query(), "a");
    sim.press(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(sim.s.board.query(), "");
    sim.press(KeyCode::Esc, KeyModifiers::NONE);
    assert!(!sim.s.quit);
}

#[test]
fn actions_work_with_the_drawer_open_and_take_sends_owner_me() {
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 3)]);
    sim.char('f');
    sim.press(KeyCode::Enter, KeyModifiers::NONE); // all
    sim.press(KeyCode::Esc, KeyModifiers::NONE);
    sim.press(KeyCode::Enter, KeyModifiers::NONE); // drawer
    assert!(sim.s.ui.drawer.is_some(), "the drawer is open");
    sim.char('t');
    let (_, req, _) = sim
        .pending
        .iter()
        .find(|(_, r, _)| matches!(r, Request::Edit { .. }))
        .expect("take sent");
    let Request::Edit { body, .. } = req else {
        unreachable!()
    };
    assert_eq!(
        (body.version, body.owner.clone()),
        (3, Some(Some("dan@example.com".to_string())))
    );
    assert!(sim.s.board.locked(&key("ABC-1")));
}

#[test]
fn take_on_my_own_story_is_a_notice() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "DAN@example.com")],
    );
    sim.char('t');
    assert!(!sim.has_pending(|r| matches!(r, Request::Edit { .. })));
    assert_eq!(sim.status(), "ABC-1 is already yours");
}

#[test]
fn an_automation_token_is_read_only() {
    let mut sim = Sim::loaded(TokenKind::Automation, vec![story("ABC-1", Todo, 1)]);
    sim.char('f');
    sim.press(KeyCode::Enter, KeyModifiers::NONE);
    sim.press(KeyCode::Esc, KeyModifiers::NONE);
    for c in ['t', 'L', 'm', 'e'] {
        sim.char(c);
        assert_eq!(sim.status(), "read-only: automation token", "{c}");
    }
    sim.fire_timers();
    assert!(!sim.has_pending(|r| matches!(
        r,
        Request::Edit { .. } | Request::Move { .. } | Request::Epics
    )));
    assert_eq!(sim.status(), "read-only: automation token");
}

#[test]
fn a_401_signs_out_for_good_and_a_late_success_does_not_undo_it() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('L');
    sim.fire_timers();
    sim.char('r');
    sim.reply(is_stories, Completion::Failed(Failure::Unauthenticated));
    assert!(sim.s.session.signed_out);
    assert!(sim.other.iter().any(|e| matches!(e, Effect::Disconnect)));
    sim.reply(
        is_move,
        Completion::Ok(Reply::Story(story("ABC-1", InProgress, 2).story)),
    );
    assert!(sim.s.session.signed_out);
    sim.char('t');
    assert!(!sim.has_pending(|r| matches!(r, Request::Edit { .. })));
    assert_eq!(sim.status(), "read-only: signed out");
}

#[test]
fn with_me_unknown_only_me_is_asked_until_it_answers() {
    let mut sim = Sim::start(None);
    assert!(sim.has_pending(|r| matches!(r, Request::Me)));
    assert!(!sim.has_pending(is_stories) && !sim.has_pending(|r| matches!(r, Request::Config)));
    assert!(sim.other.is_empty(), "no stream before identity");
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Ok(Reply::Me(me(TokenKind::Interactive))),
    );
    assert!(sim.has_pending(is_stories) && sim.has_pending(|r| matches!(r, Request::Config)));
    assert!(
        sim.other
            .iter()
            .any(|e| matches!(e, Effect::Connect { .. }))
    );
}

#[test]
fn startup_connecting_then_a_401_on_the_retry_is_signed_out() {
    let mut sim = Sim::start(None);
    assert_eq!(sim.s.session.read_only(), Some("connecting"));
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Failed(Failure::Transport("refused".into())),
    );
    assert!(sim.status().contains("cannot reach"));
    sim.fire_timers();
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Failed(Failure::Unauthenticated),
    );
    assert!(sim.s.session.signed_out);
}

#[test]
fn a_load_sent_before_a_moves_completion_is_dropped_and_replaced() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('L');
    sim.fire_timers(); // the move is sent
    sim.char('r'); // a load is sent after the move, before its completion
    sim.reply(
        is_move,
        Completion::Ok(Reply::Story(
            owned(story("ABC-1", InProgress, 2), "dan@example.com").story,
        )),
    );
    // That load's snapshot predates the commit.
    sim.reply(
        is_stories,
        stories(vec![
            owned(story("ABC-1", Todo, 1), "dan@example.com"),
            owned(story("ABC-2", Todo, 1), "dan@example.com"),
        ]),
    );
    assert_eq!(
        sim.s.board.get(&key("ABC-1")).unwrap().item.story.delivery,
        InProgress
    );
    // ABC-1's version alone would keep its fields; the snapshot as a whole must not be applied.
    assert!(
        sim.s.board.get(&key("ABC-2")).is_none(),
        "the stale snapshot was applied"
    );
    assert!(
        sim.has_pending(is_stories),
        "a fresh load replaces the dropped one"
    );
}

#[test]
fn a_conflict_refetches_and_names_the_story() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('L');
    sim.fire_timers();
    sim.reply(is_move, Completion::Failed(Failure::Conflict));
    assert_eq!(
        sim.status(),
        "ABC-1 changed on the server — refreshing; try again"
    );
    assert!(sim.has_pending(is_stories));
    assert!(!sim.s.board.locked(&key("ABC-1")));
}

#[test]
fn yank_and_open() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('y');
    sim.char('o');
    sim.char('O');
    assert!(
        sim.other
            .iter()
            .any(|e| matches!(e, Effect::Copy(t) if t == "ABC-1"))
    );
    assert!(sim.other.iter().any(
        |e| matches!(e, Effect::Open(u) if u == "https://reeve.example.com/board?story=ABC-1")
    ));
    assert_eq!(sim.status(), "ABC-1 has no pull request");
}

#[test]
fn the_drawer_closes_with_a_notice_when_its_story_leaves() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.press(KeyCode::Enter, KeyModifiers::NONE);
    sim.char('r');
    sim.reply(is_stories, stories(vec![]));
    assert!(sim.s.ui.drawer.is_none());
    assert_eq!(sim.status(), "ABC-1 is no longer on the board");
}

#[test]
fn a_follow_up_due_after_sign_out_never_sends() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('L'); // sent at once
    sim.char('L'); // steers a follow-up
    sim.stream(StreamEvent::Ended(Ended::Unauthorized));
    sim.reply(
        is_move,
        Completion::Ok(Reply::Story(
            owned(story("ABC-1", InProgress, 2), "dan@example.com").story,
        )),
    );
    assert!(
        !sim.has_pending(is_move),
        "the follow-up after sign-out sends nothing"
    );
}

#[test]
fn every_move_outcome_releases_the_lock() {
    for result in [
        Completion::Ok(Reply::Story(
            owned(story("ABC-1", InProgress, 2), "dan@example.com").story,
        )),
        refused(vec![Reason::ResolutionRequired]),
        Completion::Failed(Failure::Conflict),
        Completion::Failed(Failure::Forbidden("no".into())),
        Completion::Failed(Failure::Validation("bad".into())),
        Completion::Failed(Failure::Server("502".into())),
        Completion::Failed(Failure::Transport("timed out".into())),
    ] {
        let mut sim = Sim::loaded(
            TokenKind::Interactive,
            vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
        );
        sim.char('L');
        sim.fire_timers();
        assert!(sim.s.board.locked(&key("ABC-1")));
        sim.reply(is_move, result.clone());
        assert!(!sim.s.board.locked(&key("ABC-1")), "{result:?}");
        assert!(sim.s.moves.prompt().is_none(), "{result:?}");
    }
}

#[test]
fn a_late_identity_success_after_a_sign_out_does_not_restart_the_session() {
    let mut sim = Sim::start(None);
    sim.char('r'); // a second /me while the first is pending
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Failed(Failure::Unauthenticated),
    );
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Ok(Reply::Me(me(TokenKind::Interactive))),
    );
    assert!(sim.s.session.signed_out);
    assert!(sim.pending.is_empty(), "no board requests after sign-out");
    assert!(
        !sim.other
            .iter()
            .any(|e| matches!(e, Effect::Connect { .. }))
    );
}

#[test]
fn a_move_prompt_closes_the_epic_picker() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![
            owned(story("ABC-1", InReview, 1), "dan@example.com"),
            owned(story("ABC-2", InReview, 1), "dan@example.com"),
        ],
    );
    sim.char('L'); // ABC-1, selected first: sent into testing
    sim.char('L'); // the follow-up will ask a resolution
    sim.char('j');
    sim.char('e'); // the epic picker for ABC-2 opens while ABC-1's move is in flight
    assert!(sim.s.set_epic.picker().is_some());
    sim.reply(
        is_move,
        Completion::Ok(Reply::Story(
            owned(story("ABC-1", Testing, 2), "dan@example.com").story,
        )),
    );
    assert!(matches!(
        sim.s.moves.prompt(),
        Some(Prompt::Resolution { .. })
    ));
    assert!(sim.s.set_epic.picker().is_none(), "never two overlays");
}

#[test]
fn a_late_refusal_closes_the_epic_picker_it_opens_over() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![
            owned(story("ABC-1", Todo, 1), "dan@example.com"),
            owned(story("ABC-2", Todo, 1), "dan@example.com"),
        ],
    );
    sim.char('L');
    sim.fire_timers(); // ABC-1's move is in flight
    sim.char('j');
    sim.char('e'); // the epic picker for ABC-2
    sim.reply(is_move, refused(vec![Reason::NoCriteria]));
    assert!(matches!(
        sim.s.moves.prompt(),
        Some(Prompt::Override { .. })
    ));
    assert!(sim.s.set_epic.picker().is_none());
}

#[test]
fn an_outcome_clears_on_the_next_key() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('y');
    assert_eq!(sim.status(), "copied ABC-1");
    sim.char('j');
    assert_eq!(sim.status(), "");
}

#[test]
fn refresh_retries_a_configuration_that_never_answered() {
    let mut sim = Sim::start(Some(me(TokenKind::Interactive)));
    sim.reply(
        |r| matches!(r, Request::Config),
        Completion::Failed(Failure::Transport("down".into())),
    );
    sim.reply(is_stories, stories(vec![]));
    sim.char('r');
    assert!(sim.has_pending(|r| matches!(r, Request::Config)));
}

#[test]
fn ctrl_c_and_q_quit_at_the_board() {
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![]);
    sim.press(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(sim.s.quit);
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![]);
    sim.char('q');
    assert!(sim.s.quit);
}

#[test]
fn two_identity_answers_start_the_session_once() {
    let mut sim = Sim::start(None);
    sim.char('r'); // a second /me while the first is pending
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Ok(Reply::Me(me(TokenKind::Interactive))),
    );
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Ok(Reply::Me(me(TokenKind::Interactive))),
    );
    assert_eq!(
        sim.pending.iter().filter(|(_, r, _)| is_stories(r)).count(),
        1
    );
    assert_eq!(
        sim.pending
            .iter()
            .filter(|(_, r, _)| matches!(r, Request::Config))
            .count(),
        1
    );
    assert_eq!(
        sim.other
            .iter()
            .filter(|e| matches!(e, Effect::Connect { .. }))
            .count(),
        1
    );
}

#[test]
fn sign_out_closes_every_prompt_and_picker() {
    // The resolution prompt (a move into done), the state picker, the epic picker, the urgent reason prompt.
    for open in [&['L'][..], &['m'], &['e'], &['u']] {
        let mut sim = Sim::loaded(
            TokenKind::Interactive,
            vec![owned(story("ABC-1", Testing, 1), "dan@example.com")],
        );
        for c in open {
            sim.char(*c);
        }
        sim.fire_timers();
        assert!(
            sim.s.moves.prompt().is_some()
                || sim.s.moves.picker().is_some()
                || sim.s.set_epic.picker().is_some()
                || sim.s.urgent.open().is_some(),
            "{open:?}"
        );
        // The overlay owns the keys, so the sign-out comes from the stream.
        sim.stream(StreamEvent::Ended(Ended::Unauthorized));
        assert!(sim.s.session.signed_out, "{open:?}");
        assert!(sim.s.moves.prompt().is_none(), "{open:?}");
        assert!(sim.s.moves.picker().is_none(), "{open:?}");
        assert!(sim.s.set_epic.picker().is_none(), "{open:?}");
        assert!(sim.s.urgent.open().is_none(), "{open:?}");
    }
}

fn is_config(r: &Request) -> bool {
    matches!(r, Request::Config)
}
fn connects(sim: &Sim) -> usize {
    sim.other
        .iter()
        .filter(|e| matches!(e, Effect::Connect { .. }))
        .count()
}

/// ABC-1's move in flight, then a 401 on a refresh signs out. With `queue`, a second refresh was queued behind it.
fn signed_out_with_a_move_in_flight(queue: bool) -> Sim {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('L');
    sim.fire_timers();
    sim.char('r');
    if queue {
        sim.char('r');
    }
    sim.reply(is_stories, Completion::Failed(Failure::Unauthenticated));
    assert!(sim.s.session.signed_out);
    sim
}

#[test]
fn after_a_sign_out_nothing_reaches_the_network() {
    let quiet = |sim: &Sim, case: &str| {
        assert!(!sim.has_pending(is_stories), "{case}: a load was sent");
        assert!(!sim.has_pending(is_config), "{case}: the config was asked");
        assert_eq!(connects(sim), 1, "{case}: the stream reconnected");
    };
    // The load queued behind the one that drew the 401 lands with it.
    quiet(&signed_out_with_a_move_in_flight(true), "queued load");
    let late: [(&str, Completion); 3] = [
        (
            "late write success",
            Completion::Ok(Reply::Story(
                owned(story("ABC-1", InProgress, 2), "dan@example.com").story,
            )),
        ),
        (
            "late transport failure",
            Completion::Failed(Failure::Transport("timed out".into())),
        ),
        ("conflict", Completion::Failed(Failure::Conflict)),
    ];
    for (case, c) in late {
        let mut sim = signed_out_with_a_move_in_flight(false);
        sim.reply(is_move, c);
        quiet(&sim, case);
    }
    let mut sim = signed_out_with_a_move_in_flight(false);
    sim.char('r');
    quiet(&sim, "r");

    // The reason prompt open when the sign-out lands: its Enter can no longer send a mark.
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('u');
    sim.char('x');
    assert!(sim.s.urgent.open().is_some());
    sim.stream(StreamEvent::Ended(Ended::Unauthorized));
    sim.press(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        !sim.has_pending(|r| matches!(r, Request::MarkUrgent { .. })),
        "a mark was sent after the sign-out"
    );
}

#[test]
fn a_401_on_a_load_words_the_sign_out() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('r');
    sim.reply(is_stories, Completion::Failed(Failure::Unauthenticated));
    assert_eq!(sim.status(), "signed out — run reeve auth login");
}

#[test]
fn identity_retries_a_server_error() {
    let mut sim = Sim::start(None);
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Failed(Failure::Server("502 bad gateway".into())),
    );
    assert!(sim.status().contains("cannot reach"), "{}", sim.status());
    sim.fire_timers();
    assert!(sim.has_pending(|r| matches!(r, Request::Me)));
}

#[test]
fn a_late_identity_failure_after_a_success_changes_nothing() {
    let mut sim = Sim::start(None);
    sim.char('r'); // a second /me while the first is pending
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Ok(Reply::Me(me(TokenKind::Interactive))),
    );
    sim.reply(
        |r| matches!(r, Request::Me),
        Completion::Failed(Failure::Transport("refused".into())),
    );
    assert!(!sim.status().contains("cannot reach"), "{}", sim.status());
    assert!(sim.timers.is_empty(), "no retry is scheduled");
}

#[test]
fn every_take_outcome_releases_the_lock() {
    let cases: [(Completion, &str, bool); 6] = [
        (
            Completion::Ok(Reply::Story(
                owned(story("ABC-1", Todo, 2), "dan@example.com").story,
            )),
            "ABC-1 is yours",
            true,
        ),
        (
            Completion::Failed(Failure::Conflict),
            "ABC-1 changed on the server — refreshing; try again",
            true,
        ),
        (
            Completion::Failed(Failure::Transport("timed out".into())),
            "ABC-1: no answer — refreshing",
            true,
        ),
        (
            Completion::Failed(Failure::Forbidden("no".into())),
            "forbidden: no",
            false,
        ),
        (
            Completion::Failed(Failure::Validation("bad".into())),
            "bad",
            false,
        ),
        (
            Completion::Failed(Failure::Server("502 bad gateway".into())),
            "502 bad gateway",
            true,
        ),
    ];
    for (result, status, refetch) in cases {
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 1)]);
        sim.char('f');
        sim.press(KeyCode::Enter, KeyModifiers::NONE); // all
        sim.press(KeyCode::Esc, KeyModifiers::NONE);
        sim.char('t');
        assert!(sim.s.board.locked(&key("ABC-1")));
        sim.reply(|r| matches!(r, Request::Edit { .. }), result.clone());
        assert!(!sim.s.board.locked(&key("ABC-1")), "{result:?}");
        assert_eq!(sim.status(), status, "{result:?}");
        assert_eq!(sim.has_pending(is_stories), refetch, "{result:?}");
    }
}

#[test]
fn a_move_prompt_closes_the_help() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", InReview, 1), "dan@example.com")],
    );
    sim.char('L'); // sent: into testing
    sim.char('L'); // the follow-up will ask a resolution
    sim.char('?');
    assert!(sim.s.ui.help);
    sim.reply(
        is_move,
        Completion::Ok(Reply::Story(
            owned(story("ABC-1", Testing, 2), "dan@example.com").story,
        )),
    );
    assert!(matches!(
        sim.s.moves.prompt(),
        Some(Prompt::Resolution { .. })
    ));
    assert!(!sim.s.ui.help, "never two overlays");
}

#[test]
fn a_load_that_changes_the_story_closes_the_epic_picker() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('e');
    assert!(sim.s.set_epic.picker().is_some());
    // The picker owns the keys: the refetch comes from the stream opening.
    sim.stream(StreamEvent::Opened);
    sim.fire_timers();
    sim.reply(
        is_stories,
        stories(vec![owned(story("ABC-1", Todo, 2), "dan@example.com")]),
    );
    assert!(sim.s.set_epic.picker().is_none());
    assert_eq!(sim.status(), "ABC-1 changed — epic picker closed");
}

#[test]
fn a_move_that_times_out_leaves_readiness_pending_until_the_next_load() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('L');
    sim.fire_timers();
    sim.reply(
        is_move,
        Completion::Failed(Failure::Transport("timed out".into())),
    );
    assert!(sim.s.board.get(&key("ABC-1")).unwrap().readiness_pending);
    assert!(sim.has_pending(is_stories));
    sim.reply(
        is_stories,
        stories(vec![owned(story("ABC-1", Todo, 1), "dan@example.com")]),
    );
    assert!(!sim.s.board.get(&key("ABC-1")).unwrap().readiness_pending);
}

#[test]
fn a_server_error_on_any_write_refetches_the_board() {
    let server = || Completion::Failed(Failure::Server("502 bad gateway".into()));
    let is_edit = |r: &Request| matches!(r, Request::Edit { .. });

    // move
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char('L');
    sim.fire_timers();
    assert!(!sim.has_pending(is_stories));
    sim.reply(is_move, server());
    assert!(sim.has_pending(is_stories), "move");

    // take
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 1)]);
    sim.char('f');
    sim.press(KeyCode::Enter, KeyModifiers::NONE); // all
    sim.press(KeyCode::Esc, KeyModifiers::NONE);
    sim.char('t');
    assert!(!sim.has_pending(is_stories));
    sim.reply(is_edit, server());
    assert!(sim.has_pending(is_stories), "take");

    // epic
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 1)]);
    sim.char('f');
    sim.press(KeyCode::Enter, KeyModifiers::NONE); // all
    sim.press(KeyCode::Esc, KeyModifiers::NONE);
    sim.char('e');
    sim.reply(
        |r| matches!(r, Request::Epics),
        Completion::Ok(Reply::Epics(vec![epic("ABC-7", "Billing")])),
    );
    sim.press(KeyCode::Enter, KeyModifiers::NONE);
    assert!(sim.has_pending(is_edit), "the epic write is sent");
    assert!(!sim.has_pending(is_stories));
    sim.reply(is_edit, server());
    assert!(sim.has_pending(is_stories), "epic");
}

#[test]
fn a_load_in_flight_when_an_activity_entry_arrives_is_dropped_and_replaced() {
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 1)]);
    sim.stream(StreamEvent::Opened);
    sim.char('r'); // a load is in flight
    let entry = serde_json::json!({
        "id": 6, "at": "2026-10-28T10:00:00Z", "actor": "anna@example.com", "via": "interactive",
        "entity_type": "story", "entity_key": "ABC-2", "action": "created", "payload": {},
    });
    sim.stream(StreamEvent::Frame(crate::tui::sse::Frame {
        event: Some("activity".into()),
        id: Some("6".into()),
        data: entry.to_string(),
    }));
    // The load's snapshot may predate the change the entry reports (kb/ADR-0011 R5).
    sim.reply(
        is_stories,
        stories(vec![story("ABC-1", Todo, 1), story("ABC-3", Todo, 1)]),
    );
    assert!(
        sim.s.board.get(&key("ABC-3")).is_none(),
        "the stale snapshot was applied"
    );
    assert!(
        sim.has_pending(is_stories),
        "a fresh load replaces the dropped one"
    );
}

#[test]
fn the_core_words_a_write_that_got_no_answer_or_conflicted_by_its_story() {
    for (c, want) in [
        (
            Completion::Failed(Failure::Transport("timed out".into())),
            "ABC-7: no answer — refreshing",
        ),
        (
            Completion::Failed(Failure::Conflict),
            "ABC-7 changed on the server — refreshing; try again",
        ),
    ] {
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 1)]);
        // A write's completion whose owner says nothing: the wording is the core's, keyed by the write's story.
        sim.send(crate::tui::app::Msg::Done {
            sent: 99,
            completion: c.clone(),
            route: Box::new(|_, _| crate::tui::app::Msg::Tick),
            write: Some(key("ABC-7")),
        });
        assert_eq!(sim.status(), want, "{c:?}");
    }
    // A read that got no answer is the board's to show (its stale line), not a write's status.
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 1)]);
    sim.send(crate::tui::app::Msg::Done {
        sent: 99,
        completion: Completion::Failed(Failure::Transport("timed out".into())),
        route: Box::new(|_, _| crate::tui::app::Msg::Tick),
        write: None,
    });
    assert_eq!(sim.status(), "");
}

#[test]
fn every_epic_write_outcome_is_worded_once() {
    let cases = [
        (
            Completion::Ok(Reply::Story(story("ABC-1", Todo, 2).story)),
            "ABC-1 → epic ABC-100",
        ),
        (
            Completion::Failed(Failure::Conflict),
            "ABC-1 changed on the server — refreshing; try again",
        ),
        (
            Completion::Failed(Failure::Transport("timed out".into())),
            "ABC-1: no answer — refreshing",
        ),
        (
            Completion::Failed(Failure::Server("502 bad gateway".into())),
            "502 bad gateway",
        ),
    ];
    for (result, status) in cases {
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 1)]);
        sim.char('f');
        sim.press(KeyCode::Enter, KeyModifiers::NONE); // all
        sim.press(KeyCode::Esc, KeyModifiers::NONE);
        sim.char('e');
        sim.reply(
            |r| matches!(r, Request::Epics),
            Completion::Ok(Reply::Epics(vec![epic("ABC-7", "Billing")])),
        );
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        sim.reply(|r| matches!(r, Request::Edit { .. }), result.clone());
        assert_eq!(sim.status(), status, "{result:?}");
    }
}

/// Every overlay open in `s`, read from each owner's accessor (independently of the core's own `overlay`).
fn open_overlays(s: &crate::tui::app::State) -> Vec<&'static str> {
    let mut v = Vec::new();
    if s.ui.help {
        v.push("help");
    }
    if s.moves.prompt().is_some() {
        v.push("move prompt");
    }
    if s.moves.picker().is_some() {
        v.push("move picker");
    }
    if s.set_epic.picker().is_some() {
        v.push("epic picker");
    }
    if s.urgent.open().is_some() {
        v.push("urgent prompt");
    }
    if s.board.filter_menu().is_some() {
        v.push("filter menu");
    }
    if s.board.search().is_some() {
        v.push("search");
    }
    v
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Key(char),
    /// ABC-3's move answers into testing (its follow-up into done asks the resolution).
    Moved,
    /// ABC-1's move answers with a readiness refusal (an override prompt opens).
    Refuse,
}

#[test]
fn at_most_one_overlay_is_open_after_every_message() {
    use Step::*;
    // Each sequence opens an overlay, then something that opens another late: a refusal or a follow-up into done.
    let sequences: &[(&str, &[Step], &str)] = &[
        // ABC-1 (todo) moves and is refused; the overlay opened meanwhile may be another story's (`j`).
        (
            "refusal over search",
            &[Key('L'), Key('/'), Key('g'), Refuse],
            "move prompt",
        ),
        (
            "refusal over the filter menu",
            &[Key('L'), Key('f'), Refuse],
            "move prompt",
        ),
        (
            "refusal over help",
            &[Key('L'), Key('?'), Refuse],
            "move prompt",
        ),
        (
            "refusal over the epic picker",
            &[Key('L'), Key('j'), Key('e'), Refuse],
            "move prompt",
        ),
        (
            "refusal over the urgent prompt",
            &[Key('L'), Key('j'), Key('u'), Refuse],
            "move prompt",
        ),
        // On a board of ABC-3 and ABC-4 (in review), ABC-3 moves and is steered into done: the reply's follow-up opens
        // the resolution prompt.
        (
            "follow-up over the epic picker",
            &[Key('L'), Key('L'), Key('j'), Key('e'), Moved],
            "move prompt",
        ),
        (
            "follow-up over the urgent prompt",
            &[Key('L'), Key('L'), Key('j'), Key('u'), Moved],
            "move prompt",
        ),
        (
            "follow-up over help",
            &[Key('L'), Key('L'), Key('?'), Moved],
            "move prompt",
        ),
        (
            "follow-up over search",
            &[Key('L'), Key('L'), Key('/'), Moved],
            "move prompt",
        ),
        (
            "follow-up over the filter menu",
            &[Key('L'), Key('L'), Key('f'), Moved],
            "move prompt",
        ),
    ];
    for (case, steps, last) in sequences {
        let list = if case.starts_with("follow-up") {
            vec![
                owned(story("ABC-3", InReview, 1), "dan@example.com"),
                owned(story("ABC-4", InReview, 1), "dan@example.com"),
            ]
        } else {
            vec![
                owned(story("ABC-1", Todo, 1), "dan@example.com"),
                owned(story("ABC-2", Todo, 1), "dan@example.com"),
            ]
        };
        let mut sim = Sim::loaded(TokenKind::Interactive, list);
        for (i, step) in steps.iter().enumerate() {
            match *step {
                Key(c) => sim.char(c),
                Moved => sim.reply(
                    is_move,
                    Completion::Ok(Reply::Story(
                        owned(story("ABC-3", Testing, 2), "dan@example.com").story,
                    )),
                ),
                Refuse => sim.reply(is_move, refused(vec![Reason::NoCriteria])),
            }
            let open = open_overlays(&sim.s);
            assert!(
                open.len() <= 1,
                "{case}: after step {i} ({step:?}) {open:?} are open"
            );
        }
        assert_eq!(open_overlays(&sim.s), [*last], "{case}");
        assert_eq!(
            sim.s.overlay(),
            Some(crate::tui::app::Overlay::MovePrompt),
            "{case}: the core names the overlay that receives the keys"
        );
    }
}

#[test]
fn a_prompt_opened_over_search_keeps_the_query_typed_so_far() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![
            owned(story("ABC-1", Todo, 1), "dan@example.com"),
            owned(story("ABC-12", Todo, 1), "dan@example.com"),
        ],
    );
    sim.char('L');
    sim.fire_timers();
    sim.char('/');
    sim.char('1');
    sim.reply(is_move, refused(vec![Reason::NoCriteria]));
    assert!(sim.s.board.search().is_none());
    assert_eq!(sim.s.board.query(), "1");
    // The prompt receives the keys, not the closed search.
    sim.char('x');
    let Some(Prompt::Override { line, .. }) = sim.s.moves.prompt() else {
        panic!()
    };
    assert_eq!(line.text(), "x");
}

#[test]
fn a_refused_identity_retry_signs_out_like_startup() {
    for refused in [
        Completion::Failed(Failure::Unauthenticated),
        Completion::Failed(Failure::Forbidden("user disabled".into())),
    ] {
        let mut sim = Sim::start(None);
        sim.reply(
            |r| matches!(r, Request::Me),
            Completion::Failed(Failure::Transport("refused".into())),
        );
        sim.fire_timers(); // the retry
        sim.reply(|r| matches!(r, Request::Me), refused.clone());
        assert!(sim.s.session.signed_out, "{refused:?}");
        assert_eq!(sim.s.session.read_only(), Some("signed out"), "{refused:?}");
        assert_eq!(
            sim.status(),
            "signed out — run reeve auth login",
            "{refused:?}"
        );
        sim.fire_timers();
        assert!(sim.pending.is_empty(), "{refused:?}: nothing is retried");
    }
}

#[test]
fn another_envelope_error_on_an_identity_retry_is_shown_and_retried() {
    for (failure, want) in [
        (Failure::Validation("bad header".into()), "bad header"),
        (Failure::NotFound("no such route".into()), "no such route"),
    ] {
        let mut sim = Sim::start(None);
        sim.reply(
            |r| matches!(r, Request::Me),
            Completion::Failed(failure.clone()),
        );
        assert!(!sim.s.session.signed_out, "{failure:?}");
        assert_eq!(sim.s.session.read_only(), Some("connecting"), "{failure:?}");
        assert!(sim.status().contains(want), "{failure:?}: {}", sim.status());
        assert_eq!(sim.timers.len(), 1, "{failure:?}: a retry is scheduled");
        sim.fire_timers();
        assert!(sim.has_pending(|r| matches!(r, Request::Me)), "{failure:?}");
    }
}

fn is_clear(r: &Request) -> bool {
    matches!(r, Request::ClearUrgent { key: k } if *k == key("ABC-1"))
}

/// ABC-1, mine (so the default board shows it), urgent by its own mark.
fn my_urgent_story(
    state: reeve_protocol::DeliveryState,
    version: i64,
) -> reeve_protocol::StoryWithReadiness {
    urgent(owned(story("ABC-1", state, version), "dan@example.com"))
}

#[test]
fn clear_urgent_sends_delete_and_locks_without_changing_the_story() {
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![my_urgent_story(Todo, 3)]);
    sim.char('U');
    assert_eq!(
        sim.pending.iter().filter(|(_, r, _)| is_clear(r)).count(),
        1,
        "one clear for ABC-1"
    );
    assert!(sim.s.board.locked(&key("ABC-1")));
    let e = sim.s.board.get(&key("ABC-1")).unwrap();
    assert_eq!(
        e.item.story.urgent_mark,
        Some(own_mark()),
        "no optimistic write"
    );
    assert_eq!(e.item.story.urgent_sources.len(), 1);
    sim.char('U'); // again, while the first is in flight
    assert_eq!(
        sim.pending.iter().filter(|(_, r, _)| is_clear(r)).count(),
        1,
        "a locked story sends no second clear"
    );
    assert_eq!(sim.status(), "ABC-1: a change is still saving");
}

#[test]
fn every_clear_outcome_releases_the_lock() {
    let mut cleared = owned(story("ABC-1", Todo, 3), "dan@example.com").story;
    cleared.urgent_mark = None;
    let cases: [(Completion, &str, bool); 4] = [
        (
            Completion::Ok(Reply::Story(cleared.clone())),
            "ABC-1 urgent mark cleared",
            true,
        ),
        (
            refused(vec![Reason::InteractiveOnly]),
            "ABC-1: interactive only",
            false,
        ),
        (
            Completion::Failed(Failure::Server("502 bad gateway".into())),
            "502 bad gateway",
            true,
        ),
        (
            Completion::Failed(Failure::Transport("timed out".into())),
            "ABC-1: no answer — refreshing",
            true,
        ),
    ];
    for (result, status, refetch) in cases {
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![my_urgent_story(Todo, 3)]);
        sim.char('U');
        assert!(sim.s.board.locked(&key("ABC-1")));
        sim.reply(is_clear, result.clone());
        assert!(!sim.s.board.locked(&key("ABC-1")), "{result:?}");
        assert_eq!(sim.status(), status, "{result:?}");
        // R8: a committed or unknown write leaves readiness pending until the refetch lands; a refusal changed nothing.
        assert_eq!(
            sim.s.board.get(&key("ABC-1")).unwrap().readiness_pending,
            refetch,
            "{result:?}"
        );
        assert_eq!(
            sim.pending.iter().filter(|(_, r, _)| is_stories(r)).count(),
            usize::from(refetch),
            "{result:?}"
        );
        assert!(!sim.has_pending(is_clear), "never sent again: {result:?}");
        if let Completion::Ok(_) = result {
            assert_eq!(
                sim.s
                    .board
                    .get(&key("ABC-1"))
                    .unwrap()
                    .item
                    .story
                    .urgent_mark,
                None,
                "the reply of the same version is applied"
            );
        }
    }
}

#[test]
fn a_read_started_during_the_clear_is_dropped() {
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![my_urgent_story(Todo, 3)]);
    sim.char('U'); // the clear is sent
    sim.char('r'); // a load is sent after the clear, before its completion
    let mut cleared = owned(story("ABC-1", Todo, 3), "dan@example.com").story;
    cleared.urgent_mark = None;
    sim.reply(is_clear, Completion::Ok(Reply::Story(cleared)));
    // That load's snapshot predates the clear: same version, mark still set, plus a story not on the board.
    sim.reply(
        is_stories,
        stories(vec![
            my_urgent_story(Todo, 3),
            owned(story("ABC-9", Todo, 1), "dan@example.com"),
        ]),
    );
    assert_eq!(
        sim.s
            .board
            .get(&key("ABC-1"))
            .unwrap()
            .item
            .story
            .urgent_mark,
        None,
        "the post-write state is kept"
    );
    assert!(
        sim.s.board.get(&key("ABC-9")).is_none(),
        "the stale snapshot was applied"
    );
    assert!(
        sim.has_pending(is_stories),
        "a fresh load replaces the dropped one"
    );
}

#[test]
fn urgent_shortcuts_respect_the_disabled_reasons() {
    let is_urgent_write =
        |r: &Request| matches!(r, Request::MarkUrgent { .. } | Request::ClearUrgent { .. });

    let mut sim = Sim::loaded(TokenKind::Automation, vec![my_urgent_story(Todo, 1)]);
    for c in ['u', 'U'] {
        sim.char(c);
        assert_eq!(sim.status(), "read-only: automation token", "{c}");
    }
    assert!(!sim.has_pending(is_urgent_write));

    let done = owned(story("ABC-1", Done, 1), "dan@example.com");
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![done]);
    sim.char('u');
    assert_eq!(sim.status(), "ABC-1: done stories cannot be urgent");
    assert!(!sim.has_pending(is_urgent_write));

    let unmarked = owned(story("ABC-1", Todo, 1), "dan@example.com");
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![unmarked]);
    sim.char('U');
    assert_eq!(sim.status(), "ABC-1: no urgent mark");
    assert!(!sim.has_pending(is_urgent_write));
    assert!(!sim.s.board.locked(&key("ABC-1")));

    // Urgent only through its epic: no mark of its own to clear.
    let via_epic = urgent_via_epic(owned(story("ABC-1", Todo, 1), "dan@example.com"));
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![via_epic]);
    sim.char('U');
    assert_eq!(sim.status(), "ABC-1: no urgent mark");
    assert!(!sim.has_pending(is_urgent_write));
}

#[test]
fn the_menu_words_the_urgent_actions_like_their_shortcuts() {
    use crate::tui::keys::Action;
    let why = |sim: &Sim, a: Action| {
        sim.s
            .menu_items()
            .into_iter()
            .find(|i| i.action == a)
            .expect("listed")
            .disabled
    };
    let sim = Sim::loaded(TokenKind::Automation, vec![my_urgent_story(Todo, 1)]);
    for a in [Action::MarkUrgent, Action::ClearUrgent] {
        assert_eq!(why(&sim, a).as_deref(), Some("read-only: automation token"));
    }
    let sim = Sim::loaded(
        TokenKind::Interactive,
        vec![done_marked(owned(
            story("ABC-1", Todo, 1),
            "dan@example.com",
        ))],
    );
    assert_eq!(
        why(&sim, Action::MarkUrgent).as_deref(),
        Some("done stories cannot be urgent")
    );
    assert_eq!(
        why(&sim, Action::ClearUrgent),
        None,
        "a done story's mark can go"
    );
    let sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    assert_eq!(why(&sim, Action::MarkUrgent), None);
    assert_eq!(
        why(&sim, Action::ClearUrgent).as_deref(),
        Some("no urgent mark")
    );

    // The disabled row run from the menu gives the shortcut's wording and sends nothing.
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
    );
    sim.char(' ');
    sim.char('U');
    assert_eq!(sim.status(), "ABC-1: no urgent mark");
    assert!(!sim.has_pending(|r| matches!(r, Request::ClearUrgent { .. })));
}

#[test]
fn the_menu_lists_mark_and_clear_after_set_epic() {
    use crate::tui::keys::Action;
    let sim = Sim::loaded(TokenKind::Interactive, vec![my_urgent_story(Todo, 1)]);
    let actions: Vec<Action> = sim.s.menu_items().iter().map(|i| i.action).collect();
    let at = actions
        .iter()
        .position(|a| *a == Action::SetEpic)
        .expect("set epic listed");
    assert_eq!(
        actions[at + 1..at + 3],
        [Action::MarkUrgent, Action::ClearUrgent]
    );
}

fn is_mark(r: &Request) -> bool {
    matches!(r, Request::MarkUrgent { key: k, .. } if *k == key("ABC-1"))
}

#[test]
fn u_opens_the_prompt_and_enter_sends_one_mark_with_the_reason() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![owned(story("ABC-1", Todo, 3), "dan@example.com")],
    );
    sim.char('u');
    assert_eq!(
        sim.s.overlay(),
        Some(crate::tui::app::Overlay::UrgentPrompt)
    );
    assert!(
        !sim.has_pending(is_mark),
        "opening the prompt sends nothing"
    );
    for c in "on fire".chars() {
        sim.char(c);
    }
    sim.press(KeyCode::Enter, KeyModifiers::NONE);
    let marks: Vec<_> = sim
        .pending
        .iter()
        .filter_map(|(_, r, _)| match r {
            Request::MarkUrgent { key, reason } => Some((key.clone(), reason.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(marks, [(key("ABC-1"), Some("on fire".to_string()))]);
    assert!(
        sim.s.board.locked(&key("ABC-1")),
        "the story shows its pending mark"
    );
    assert_eq!(sim.s.overlay(), None);

    // A re-mark opens with the current reason.
    let mut marked = urgent(owned(story("ABC-1", Todo, 3), "dan@example.com"));
    marked.story.urgent_mark.as_mut().unwrap().reason = Some("old".into());
    let mut sim = Sim::loaded(TokenKind::Interactive, vec![marked]);
    sim.char('u');
    sim.press(KeyCode::Enter, KeyModifiers::NONE);
    assert!(sim.has_pending(|r| matches!(
        r,
        Request::MarkUrgent { reason: Some(x), .. } if x == "old"
    )));
}

#[test]
fn every_mark_outcome_releases_the_lock() {
    let marked = my_urgent_story(Todo, 3).story;
    let cases: [(Completion, &str, bool); 4] = [
        (
            Completion::Ok(Reply::Story(marked.clone())),
            "ABC-1 marked urgent",
            true,
        ),
        (
            refused(vec![Reason::InteractiveOnly]),
            "ABC-1: interactive only",
            false,
        ),
        (
            Completion::Failed(Failure::Server("502 bad gateway".into())),
            "502 bad gateway",
            true,
        ),
        (
            Completion::Failed(Failure::Transport("timed out".into())),
            "ABC-1: no answer — refreshing",
            true,
        ),
    ];
    for (result, status, refetch) in cases {
        let mut sim = Sim::loaded(
            TokenKind::Interactive,
            vec![owned(story("ABC-1", Todo, 3), "dan@example.com")],
        );
        sim.char('u');
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        assert!(sim.s.board.locked(&key("ABC-1")));
        sim.reply(is_mark, result.clone());
        assert!(!sim.s.board.locked(&key("ABC-1")), "{result:?}");
        assert_eq!(sim.status(), status, "{result:?}");
        // R8: a committed or unknown write leaves readiness pending until the refetch lands; a refusal changed nothing.
        assert_eq!(
            sim.s.board.get(&key("ABC-1")).unwrap().readiness_pending,
            refetch,
            "{result:?}"
        );
        assert_eq!(
            sim.pending.iter().filter(|(_, r, _)| is_stories(r)).count(),
            usize::from(refetch),
            "{result:?}"
        );
        assert!(!sim.has_pending(is_mark), "never sent again: {result:?}");
        if let Completion::Ok(_) = result {
            assert!(
                sim.s
                    .board
                    .get(&key("ABC-1"))
                    .unwrap()
                    .item
                    .story
                    .urgent_mark
                    .is_some(),
                "the reply of the same version is applied"
            );
        }
    }
}

#[test]
fn while_the_prompt_is_open_h_types_instead_of_moving() {
    let mut sim = Sim::loaded(
        TokenKind::Interactive,
        vec![
            owned(story("ABC-1", Todo, 1), "dan@example.com"),
            owned(story("ABC-2", Todo, 1), "dan@example.com"),
        ],
    );
    sim.char('u');
    for c in ['h', 'H', 'L', 'j', 'q', 'U'] {
        sim.char(c);
    }
    sim.fire_timers();
    assert!(!sim.s.quit);
    assert!(!sim.has_pending(is_move), "a move leaked to the board");
    assert!(!sim.has_pending(|r| matches!(r, Request::ClearUrgent { .. })));
    assert_eq!(sim.s.board.selected(), Some(&key("ABC-1")));
    sim.press(KeyCode::Enter, KeyModifiers::NONE);
    assert!(sim.has_pending(|r| matches!(
        r,
        Request::MarkUrgent { reason: Some(x), .. } if x == "hHLjqU"
    )));
}
