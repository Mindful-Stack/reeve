//! The TUI against a scripted server: reconnects, sign-out, malformed frames, the idle watchdog, timeouts,
//! conflicts and quit, each measured at the server where it can be.
mod support;
use crossterm::event::KeyCode;
use reeve::tui::Options;
use reeve_protocol::{DeliveryState::*, TokenKind};
use serde_json::json;
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(5);

async fn started(script: std::sync::Arc<Script>) -> Tui {
    let url = serve(script).await;
    tui(&url, "t", Some(me(TokenKind::Interactive)), quick())
}

#[tokio::test]
async fn a_reconnect_sends_the_last_event_id() {
    let s = Script::new();
    s.events.lock().unwrap().extend([
        // Only the cursor: the reconnect resumes from the cursor's id.
        Conn::Frames {
            frames: vec![cursor(5)],
            close: true,
        },
        // Then an activity entry: the reconnect resumes from the entry's id.
        Conn::Frames {
            frames: vec![cursor(5), activity(6, "edited", "ABC-1", json!({}))],
            close: true,
        },
        Conn::Frames {
            frames: vec![cursor(6)],
            close: false,
        },
    ]);
    let mut t = started(s.clone()).await;
    t.until("three stream connections", WAIT, |_| {
        s.seen(|x| x.last_event_ids.len()) >= 3
    })
    .await;
    assert_eq!(
        s.seen(|x| x.last_event_ids[..3].to_vec()),
        [None, Some("5".to_string()), Some("6".to_string())]
    );
}

#[tokio::test]
async fn close_then_401_signs_out_and_stops_reconnecting() {
    let s = Script::new();
    s.events.lock().unwrap().extend([
        Conn::Frames {
            frames: vec![cursor(5)],
            close: true,
        },
        Conn::Status(401),
    ]);
    let mut t = started(s.clone()).await;
    t.until("signed out", WAIT, |d| {
        d.screen().contains("signed out — run reeve auth login")
    })
    .await;
    t.idle_for(Duration::from_millis(500)).await; // ten backoff periods
    assert_eq!(
        s.seen(|x| x.last_event_ids.len()),
        2,
        "no reconnect after a 401"
    );
}

#[tokio::test]
async fn a_malformed_frame_is_skipped_and_the_stream_goes_on() {
    let s = Script::new();
    let bad = "event: activity\nid: 6\ndata: not json\n\n".to_string();
    let good = activity(
        7,
        "moved",
        "ABC-1",
        json!({"from": "todo", "to": "in_review", "resolution": null}),
    );
    s.events.lock().unwrap().push_back(Conn::Frames {
        frames: vec![cursor(5), bad, good],
        close: false,
    });
    let mut t = started(s.clone()).await;
    t.until("the ticker shows the good frame", WAIT, |d| {
        d.screen().contains("ABC-1 → in review · anna")
    })
    .await;
    t.idle_for(Duration::from_millis(200)).await; // a dropped connection would reconnect within this
    assert_eq!(
        s.seen(|x| x.last_event_ids.len()),
        1,
        "a bad frame does not drop the connection"
    );
}

#[tokio::test]
async fn a_silent_stream_trips_the_watchdog_and_reconnects() {
    let s = Script::new();
    s.events.lock().unwrap().push_back(Conn::Frames {
        frames: vec![cursor(5)],
        close: false,
    });
    let url = serve(s.clone()).await;
    let mut t = tui(
        &url,
        "t",
        Some(me(TokenKind::Interactive)),
        Options {
            idle_timeout: Duration::from_millis(400),
            ..quick()
        },
    );
    t.until("a second connection", WAIT, |_| {
        s.seen(|x| x.last_event_ids.len()) >= 2
    })
    .await;
    assert_eq!(
        s.seen(|x| x.last_event_ids[1].clone()),
        Some("5".to_string()),
        "the reconnect resumes from the silent stream's cursor"
    );
    // The client dropped the silent body; the server notices on its side once the connection closes.
    t.until("the silent stream closed by the client", WAIT, |_| {
        s.seen(|x| x.closed_streams) >= 1
    })
    .await;
}

#[tokio::test]
async fn a_stream_that_never_answers_its_headers_is_retried() {
    let s = Script::new();
    s.events.lock().unwrap().extend([
        Conn::NoHeaders,
        Conn::Frames {
            frames: vec![cursor(5)],
            close: false,
        },
    ]);
    let mut t = started(s.clone()).await;
    t.until("a second connection", WAIT, |_| {
        s.seen(|x| x.last_event_ids.len()) >= 2
    })
    .await;
    t.until("live", WAIT, |d| header(d).contains("live")).await;
}

#[tokio::test]
async fn a_stalled_load_marks_the_board_stale() {
    let s = Script::new();
    *s.stories.lock().unwrap() = vec![story("ABC-1", Todo, 1, Some("dan@example.com"))];
    let mut t = started(s.clone()).await;
    t.until("the board", WAIT, |d| d.screen().contains("ABC-1"))
        .await;
    *s.stories_hang.lock().unwrap() = true;
    t.press(KeyCode::Char('r'));
    t.until("stale", WAIT, |d| header(d).contains("stale"))
        .await;
    assert!(t.d.screen().contains("ABC-1"), "the last good board stays");
}

#[tokio::test]
async fn a_conflict_says_so_and_refetches() {
    let s = Script::new();
    *s.stories.lock().unwrap() = vec![story("ABC-1", Todo, 1, Some("dan@example.com"))];
    s.moves
        .lock()
        .unwrap()
        .push_back(Answer::Json(409, conflict(2)));
    let mut t = started(s.clone()).await;
    t.until("the board", WAIT, |d| d.screen().contains("ABC-1"))
        .await;
    t.settled(&s).await;
    let before = s.seen(|x| x.stories_calls);
    t.press(KeyCode::Char('L'));
    t.until("the conflict notice", WAIT, |d| {
        d.screen().contains("ABC-1 changed on the server")
    })
    .await;
    t.until("a refetch caused by the conflict", WAIT, |_| {
        s.seen(|x| x.stories_calls) > before
    })
    .await;
    assert_eq!(s.seen(|x| x.moves[0]["version"].clone()), json!(1));
}

#[tokio::test]
async fn a_lost_write_reply_refreshes_without_resending() {
    let s = Script::new();
    *s.stories.lock().unwrap() = vec![story("ABC-1", Todo, 1, Some("dan@example.com"))];
    s.moves.lock().unwrap().push_back(Answer::Hang);
    let mut t = started(s.clone()).await;
    t.until("the board", WAIT, |d| d.screen().contains("ABC-1"))
        .await;
    t.settled(&s).await;
    let before = s.seen(|x| x.stories_calls);
    t.press(KeyCode::Char('L'));
    t.until("no answer", WAIT, |d| {
        d.screen().contains("ABC-1: no answer — refreshing")
    })
    .await;
    t.until("a refetch caused by the lost reply", WAIT, |_| {
        s.seen(|x| x.stories_calls) > before
    })
    .await;
    t.idle_for(Duration::from_secs(1)).await; // long enough for any retry to show
    assert_eq!(s.seen(|x| x.moves.len()), 1, "the write is not resent");
}

#[tokio::test]
async fn quit_closes_the_stream() {
    let s = Script::new();
    let mut t = started(s.clone()).await;
    t.until("connected", WAIT, |d| header(d).contains("live"))
        .await;
    assert_eq!(s.seen(|x| x.closed_streams), 0, "open before quit");
    t.press(KeyCode::Char('q'));
    let code = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(c) = t.d.step().await {
                break c;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(code, 0);
    // No driver is left to step: a bounded poll of an observable fact at the server.
    let deadline = std::time::Instant::now() + WAIT;
    while s.seen(|x| x.closed_streams) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the stream was not closed after quit"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn yank_and_open_reach_the_io_sink() {
    let s = Script::new();
    *s.stories.lock().unwrap() = vec![story("ABC-1", Todo, 1, Some("dan@example.com"))];
    let url = serve(s.clone()).await;
    let mut t = tui(&url, "t", Some(me(TokenKind::Interactive)), quick());
    t.until("the board", WAIT, |d| d.screen().contains("ABC-1"))
        .await;
    t.press(KeyCode::Char('y'));
    t.press(KeyCode::Char('o'));
    t.until("copied", WAIT, |d| d.screen().contains("copied ABC-1"))
        .await;
    // Opening runs off the loop (spawn_blocking): wait for the sink, bounded.
    let io = t.io.clone();
    t.until("opened", WAIT, |_| !io.opened.lock().unwrap().is_empty())
        .await;
    assert_eq!(*t.io.copied.lock().unwrap(), ["ABC-1"]);
    assert_eq!(
        *t.io.opened.lock().unwrap(),
        [format!("{url}/board?story=ABC-1")]
    );
}
