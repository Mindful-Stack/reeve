//! The screen: a header line, the body (board or empty state, with the drawer and any overlay on top), a status line.
use crate::tui::app::State;
use crate::tui::live::Conn;
use crate::tui::{board, drawer, keys, moves, set_epic, urgent, words};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use std::time::Instant;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const MIN_W: u16 = 40;
pub const MIN_H: u16 = 10;

/// `s` cut to `w` display columns, ending in `…` when cut.
pub fn fit(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw + 1 > w {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

/// A buffer's rows as text, skipping the continuation cell after each wide character. A row's character count is its
/// column count only for text of single code points: a combining mark or a ZWJ sequence is several characters in one
/// cell.
pub fn rows(buf: &ratatui::buffer::Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| {
            let mut row = String::new();
            let mut skip = 0;
            for x in 0..buf.area.width {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let sym = buf[(x, y)].symbol();
                row.push_str(sym);
                skip = sym.width().saturating_sub(1);
            }
            row
        })
        .collect()
}

pub fn selected_style() -> Style {
    Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// A centred rectangle of at most `w`×`h` inside `area`.
pub fn centred(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// The header cut to `w` columns. The connection state and the notes are never cut while they fit; the host gives way
/// first, then me, then the filter (each shrinks to `…`, then goes).
fn header(s: &State, w: usize) -> String {
    let host = s.session.url.split("://").nth(1).unwrap_or(&s.session.url);
    let me = s
        .session
        .me
        .as_ref()
        .map_or("…", |m| words::local_part(&m.email));
    let mut filter = if s.board.filter().all {
        "all".to_string()
    } else {
        "mine".to_string()
    };
    if s.board.filter().ready_only {
        filter.push_str(" · ready");
    }
    if s.board.filter().urgent_only {
        filter.push_str(" · urgent");
    }
    if !s.board.query().is_empty() {
        filter.push_str(&format!(" · \"{}\"", s.board.query()));
    }
    let conn = if s.session.signed_out {
        crate::tui::app::SIGNED_OUT
    } else if s.session.me.is_none() {
        "connecting"
    } else if s.board.stale().is_some() {
        "stale"
    } else {
        match s.live.conn() {
            Conn::Live => "live",
            Conn::Connecting => "connecting",
            Conn::Reconnecting => "reconnecting",
            Conn::Off => "off",
        }
    };
    let automation = matches!(&s.session.me, Some(m) if m.token_kind != reeve_protocol::TokenKind::Interactive)
        && !s.session.signed_out;
    let notes = |short: bool| {
        let mut t = vec![conn];
        if automation {
            t.push(if short {
                "automation token"
            } else {
                "automation token — run reeve auth login"
            });
        }
        if s.session.reeve_token_set {
            t.push("REEVE_TOKEN ignored");
        }
        t.join(" · ")
    };
    // The advice after the automation note gives way only when the state and the notes cannot fit whole otherwise.
    let tail = match notes(false) {
        t if t.width() <= w => t,
        _ => notes(true),
    };
    let mut head = vec![
        "reeve".to_string(),
        host.to_string(),
        me.to_string(),
        filter,
    ];
    let joined = |head: &[String]| {
        let mut parts: Vec<&str> = head
            .iter()
            .map(String::as_str)
            .filter(|p| !p.is_empty())
            .collect();
        parts.push(&tail);
        parts.join(" · ")
    };
    for i in [1, 2, 3, 0] {
        let over = joined(&head).width().saturating_sub(w);
        if over == 0 {
            break;
        }
        let have = head[i].width();
        head[i] = if have > over + 1 {
            fit(&head[i], have - over)
        } else {
            String::new()
        };
    }
    fit(&joined(&head), w)
}

/// The main keys, right-aligned in the header when they fit beside it; they give way before anything else.
const HINTS: [&str; 2] = [
    "hjkl ←↓↑→ navigate · space actions · enter open · ? help",
    "hjkl · space · enter · ?",
];

/// The header line: `header` on the left and, when there is room for a two-column gap, the longest hint that fits.
fn header_line(s: &State, w: usize) -> Line<'static> {
    let left = header(s, w);
    let room = w.saturating_sub(left.width() + 2);
    match HINTS.iter().find(|h| h.width() <= room) {
        Some(h) => Line::from(vec![
            Span::raw(left.clone()),
            Span::raw(" ".repeat(w - left.width() - h.width())),
            Span::styled(*h, Style::new().add_modifier(Modifier::DIM)),
        ]),
        None => Line::raw(left),
    }
}

fn status_line(s: &State, now: Instant, width: usize) -> String {
    if let Some(ed) = s.board.search() {
        return format!("/ {}", ed.window(width.saturating_sub(2)));
    }
    if let Some(st) = &s.ui.status
        && now < st.until
    {
        return st.text.clone();
    }
    if let Some(t) = s.live.ticker() {
        return format!(
            "{} · {}s",
            t.text,
            now.saturating_duration_since(t.at).as_secs()
        );
    }
    "? help".into()
}

/// The screen's rows: the header line, the body, the status line.
pub fn layout(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area)
}

pub fn draw(f: &mut Frame, s: &State, now: Instant) {
    let area = f.area();
    if area.width < MIN_W || area.height < MIN_H {
        f.render_widget(Paragraph::new("terminal too small"), area);
        return;
    }
    let [top, body, bottom] = layout(area);
    f.render_widget(Paragraph::new(header_line(s, top.width as usize)), top);
    f.render_widget(
        Paragraph::new(fit(
            &status_line(s, now, bottom.width as usize),
            bottom.width as usize,
        )),
        bottom,
    );
    let bcx = board::Ctx {
        me: s.session.me.as_ref().map(|m| m.email.as_str()),
        wide: area.width >= crate::tui::app::WIDE,
        seq: s.seq(),
    };
    board::view::draw(f, body, s, &bcx, now);
    if let Some(scroll) = s.ui.drawer
        && let Some(e) = s.board.selected_entry()
    {
        drawer::draw(f, drawer::area(area), &s.board, e, scroll, s.ui.wall);
    }
    if let Some(row) = s.board.filter_menu() {
        board::view::filter_menu(f, body, &s.board, row);
    }
    moves::view::draw(f, body, &s.moves, &s.board);
    set_epic::view::draw(f, body, &s.set_epic, &s.board);
    urgent::view::draw(f, body, &s.urgent, &s.board);
    if let Some(row) = s.ui.menu {
        menu(f, body, s, row);
    }
    if s.ui.help {
        help(f, body);
    }
}

/// The action menu over the selected story: one row per `keys::MENU` action, its key and wording from the binding
/// table; a row that would do nothing is dimmed and says why.
fn menu(f: &mut Frame, area: Rect, s: &State, row: usize) {
    let Some(e) = s.board.selected_entry() else {
        return;
    };
    let items = s.menu_items();
    let title = format!(" {} {} ", e.item.story.key, words::kind(e.item.story.kind));
    let texts: Vec<String> = items
        .iter()
        .map(|it| {
            let b = keys::binding(it.action);
            match &it.disabled {
                Some(why) => format!(" {:<6} {} — {why} ", b.keys, b.help),
                None => format!(" {:<6} {} ", b.keys, b.help),
            }
        })
        .collect();
    let w = texts.iter().map(|t| t.width()).max().unwrap_or(0).max(40) + 2;
    let r = centred(area, w as u16, items.len() as u16 + 2);
    let inner_w = r.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = items
        .iter()
        .zip(texts)
        .enumerate()
        .map(|(i, (it, text))| {
            let style = match (i == row, &it.disabled) {
                (true, _) => selected_style(),
                (false, Some(_)) => Style::new().add_modifier(Modifier::DIM),
                (false, None) => Style::new(),
            };
            Line::styled(fit(&text, inner_w), style)
        })
        .collect();
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(title)
                .title_bottom(" enter run · esc close "),
        ),
        r,
    );
}

/// The key table: one column, two when it does not fit in one and the screen is wide enough, and cut with a final `…`
/// line when it still does not fit.
fn help(f: &mut Frame, area: Rect) {
    let all = keys::help_lines();
    let col_w = all.iter().map(|l| l.width()).max().unwrap_or(0);
    let inner_h = area.height.saturating_sub(2) as usize;
    let two = all.len() > inner_h && area.width as usize >= 2 * col_w + 5;
    let (rows, cols): (Vec<String>, usize) = if two {
        let half = all.len().div_ceil(2);
        let rows = (0..half)
            .map(|i| {
                let right = all.get(half + i).map_or("", String::as_str);
                format!("{:<col_w$}   {right}", all[i])
            })
            .collect();
        (rows, 2)
    } else {
        (all, 1)
    };
    let mut rows = rows;
    if rows.len() > inner_h {
        rows.truncate(inner_h.saturating_sub(1));
        rows.push("…".into());
    }
    let w = (cols * col_w + (cols - 1) * 3 + 2).max(64.min(area.width as usize)) as u16;
    let r = centred(area, w, rows.len() as u16 + 2);
    let inner_w = r.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = rows.iter().map(|l| Line::from(fit(l, inner_w))).collect();
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" keys ")),
        r,
    );
}

#[cfg(test)]
mod tests {
    use crate::tui::effect::{Completion, Reply, Request, StreamEvent};
    use crate::tui::fixtures::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::style::{Color, Modifier};
    use reeve_protocol::{DeliveryState::*, Kind, Reason, TokenKind};

    fn mine(list: Vec<&str>) -> Sim {
        Sim::loaded(
            TokenKind::Interactive,
            list.into_iter()
                .map(|k| owned(story(k, Todo, 1), "dan@example.com"))
                .collect(),
        )
    }
    fn has(rows: &[String], needle: &str) -> bool {
        rows.iter().any(|r| r.contains(needle))
    }
    /// The (column, row) of the first cell of `needle`.
    fn at(rows: &[String], needle: &str) -> (u16, u16) {
        let y = rows.iter().position(|r| r.contains(needle)).unwrap();
        let x = rows[y][..rows[y].find(needle).unwrap()].chars().count();
        (x as u16, y as u16)
    }
    fn refuse(sim: &mut Sim, reasons: Vec<Reason>) {
        sim.char('L');
        sim.fire_timers();
        sim.reply(|r| matches!(r, Request::Move { .. }), refused(reasons));
    }
    fn activity(sim: &mut Sim, key: &str, action: &str) {
        let dto = serde_json::json!({
            "id": 1, "at": "2026-10-28T10:00:00Z", "actor": "anna@example.com", "via": "interactive",
            "entity_type": "story", "entity_key": key, "action": action, "payload": {},
        });
        sim.stream(StreamEvent::Opened);
        sim.stream(StreamEvent::Frame(crate::tui::sse::Frame {
            event: Some("activity".into()),
            id: Some("1".into()),
            data: dto.to_string(),
        }));
    }
    const LONG_URL: &str = "https://reeve.a-very-long-internal-host-name.example.com";

    #[test]
    fn at_120_columns_five_columns_and_at_119_the_grouped_list() {
        let mut sim = mine(vec!["ABC-1"]);
        sim.s.board.test_set_done_window_days(Some(14));
        let (wide, _) = sim.render(120, 30);
        assert!(
            has(&wide, "todo (1)")
                && has(&wide, "in progress (0)")
                && has(&wide, "done · 14 days (0)"),
            "{wide:#?}"
        );
        let (narrow, _) = sim.render(119, 30);
        assert!(
            has(&narrow, "── todo (1)") && has(&narrow, "── done · 14 days (0)"),
            "{narrow:#?}"
        );
        assert!(
            !has(&narrow, "in progress (0)│"),
            "no column boxes in the list layout"
        );
        // The titles above read the same in both layouts; these tell the layouts apart, so the breakpoint is pinned.
        assert!(
            has(&wide, "┏ todo (1) ") && !has(&wide, "── todo"),
            "{wide:#?}"
        );
        assert!(!has(&narrow, "┌"), "{narrow:#?}");
    }

    #[test]
    fn the_selected_card_is_reversed_and_bold_not_only_coloured() {
        let mut sim = mine(vec!["ABC-1"]);
        let (rows, buf) = sim.render(120, 30);
        let y = rows.iter().position(|r| r.contains("ABC-1 story")).unwrap() as u16;
        // A column index, not a byte offset: the row holds multi-byte box-drawing characters.
        let row = &rows[y as usize];
        let x = row[..row.find("ABC-1").unwrap()].chars().count() as u16;
        let m = buf[(x, y)].modifier;
        assert!(
            m.contains(Modifier::REVERSED) && m.contains(Modifier::BOLD),
            "{m:?}"
        );
    }

    #[test]
    fn a_card_shows_the_first_reason_short_and_the_drawer_every_reason_in_full() {
        let mut sim = Sim::loaded(
            TokenKind::Interactive,
            vec![owned(
                not_ready(
                    story("ABC-1", Todo, 1),
                    vec![Reason::Untriaged, Reason::NoCriteria],
                ),
                "dan@example.com",
            )],
        );
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "untriaged") && !has(&rows, "no acceptance criteria"));
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "untriaged: no epic yet; triage places it")
                && has(&rows, "no acceptance criteria"),
            "{rows:#?}"
        );
    }

    #[test]
    fn the_override_prompt_names_both_ends_and_survives_a_resize() {
        let mut sim = mine(vec!["ABC-1"]);
        refuse(&mut sim, vec![Reason::NoCriteria]);
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "ABC-1 story: todo → in progress") && has(&rows, "override reason:"),
            "{rows:#?}"
        );
        let (rows, _) = sim.render(60, 20);
        assert!(has(&rows, "override reason:"), "{rows:#?}");
    }

    #[test]
    fn the_pending_target_shows_from_the_first_press() {
        let mut sim = mine(vec!["ABC-1"]);
        sim.char('L');
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "→ in progress"), "{rows:#?}");
    }

    #[test]
    fn headers_for_signed_out_stale_automation_and_an_ignored_reeve_token() {
        let mut sim = mine(vec!["ABC-1"]);
        sim.s.session.reeve_token_set = true;
        sim.s
            .board
            .test_set_stale(Some("no answer: timed out".into()));
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "stale") && has(&rows, "REEVE_TOKEN ignored"),
            "{}",
            rows[0]
        );
        sim.s.session.signed_out = true;
        let (rows, _) = sim.render(120, 30);
        assert!(
            rows[0].contains("signed out — run reeve auth login"),
            "{}",
            rows[0]
        );
        let mut bot = Sim::loaded(TokenKind::Automation, vec![]);
        let (rows, _) = bot.render(120, 30);
        assert!(
            rows[0].contains("automation token — run reeve auth login"),
            "{}",
            rows[0]
        );
    }

    #[test]
    fn the_header_names_the_ready_and_urgent_filters_after_the_menu_closes() {
        let mut sim = mine(vec!["ABC-1"]);
        let (rows, _) = sim.render(120, 30);
        assert!(
            !rows[0].contains("· ready") && !rows[0].contains("· urgent"),
            "{}",
            rows[0]
        );
        sim.char('f');
        sim.char('j');
        sim.char(' '); // row 1: ready only
        sim.char('j');
        sim.char(' '); // row 2: urgent only
        sim.press(KeyCode::Esc, KeyModifiers::NONE);
        let (rows, _) = sim.render(120, 30);
        assert!(rows[0].contains("mine · ready · urgent"), "{}", rows[0]);
    }

    #[test]
    fn empty_states() {
        let mut sim = Sim::loaded(
            TokenKind::Interactive,
            vec![owned(story("ABC-1", Todo, 1), "mia@example.com")],
        );
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "no stories owned by you — f shows all"),
            "{rows:#?}"
        );
        sim.s.board.test_set_filter_all(true);
        sim.s.board.test_set_query("zzz");
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "nothing matches \"zzz\""), "{rows:#?}");
    }

    #[test]
    fn long_and_wide_titles_are_truncated_by_display_width() {
        let mut s = owned(story("ABC-1", Todo, 1), "dan@example.com");
        s.story.title = "日本語のとても長いタイトルがここに続きます、さらに続きます".into();
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        let (rows, _) = sim.render(60, 20);
        let line = rows.iter().find(|r| r.contains("日本語")).unwrap();
        assert!(line.contains('…'), "{line}");
    }

    #[test]
    fn wide_cards_show_the_epic_when_there_is_room() {
        // "  no criteria · epic ABC-100" is 28 columns: past a 120-column board's 22, within a 160-column board's 30.
        let mut sim = Sim::loaded(
            TokenKind::Interactive,
            vec![owned(
                not_ready(story("ABC-1", Todo, 1), vec![Reason::NoCriteria]),
                "dan@example.com",
            )],
        );
        let (rows, _) = sim.render(120, 30);
        assert!(!has(&rows, "epic ABC-100"), "{rows:#?}");
        let (rows, _) = sim.render(160, 30);
        assert!(has(&rows, "epic ABC-100"), "{rows:#?}");
    }

    #[test]
    fn wide_cards_show_the_goal_word_last_and_cut_it_first() {
        let mut s = owned(
            not_ready(story("ABC-1", Todo, 1), vec![Reason::NoCriteria]),
            "dan@example.com",
        );
        s.story.goals = vec![reeve_protocol::GoalRefDto {
            id: 7,
            outcome: "Ship it".into(),
            via: reeve_protocol::GoalVia::Story,
        }];
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        // "  no criteria · epic ABC-100" (28 columns) fits a 160-column board's 30; the goal word (36) does not.
        let (rows, _) = sim.render(160, 30);
        assert!(has(&rows, "epic ABC-100"), "{rows:#?}");
        assert!(!has(&rows, "goal 7"), "{rows:#?}");
        assert!(has(&rows, "ABC-1"), "{rows:#?}");
        let (rows, _) = sim.render(200, 30);
        assert!(has(&rows, "epic ABC-100 · goal 7"), "{rows:#?}");
    }

    #[test]
    fn the_drawer_wraps_long_reasons_and_scrolls_to_the_end_of_the_description() {
        let long = format!("{} TAILMARK", "a very long decision title".repeat(6));
        let mut s = owned(
            not_ready(
                story("ABC-1", Todo, 1),
                vec![Reason::DecisionNoRecord {
                    decision_id: 3,
                    title: long,
                }],
            ),
            "dan@example.com",
        );
        s.story.description = (1..=60)
            .map(|i| format!("line {i}"))
            .chain(["LASTLINE".to_string()])
            .collect::<Vec<_>>()
            .join("\n");
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "TAILMARK"),
            "a long reason wraps instead of being cut: {rows:#?}"
        );
        assert!(!has(&rows, "LASTLINE"));
        for _ in 0..10 {
            sim.char('J');
        }
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "LASTLINE"), "{rows:#?}");
    }

    #[test]
    fn the_epic_picker_keeps_the_highlighted_epic_on_screen() {
        let mut sim = mine(vec!["ABC-1"]);
        sim.char('e');
        let epics = (0..30)
            .map(|i| epic(&format!("ABC-{}", 200 + i), &format!("Epic {i}")))
            .collect();
        sim.reply(
            |r| matches!(r, crate::tui::effect::Request::Epics),
            crate::tui::effect::Completion::Ok(crate::tui::effect::Reply::Epics(epics)),
        );
        for _ in 0..20 {
            sim.press(KeyCode::Down, KeyModifiers::NONE);
        }
        let (rows, _) = sim.render(40, 10);
        assert!(has(&rows, "› ■ ABC-220"), "{rows:#?}");
    }

    #[test]
    fn the_smallest_board_and_below_it() {
        let mut sim = mine(vec!["ABC-1"]);
        let (rows, _) = sim.render(40, 10);
        assert!(has(&rows, "ABC-1"), "{rows:#?}");
        let (rows, _) = sim.render(39, 10);
        assert!(has(&rows, "terminal too small"));
        assert!(!has(&rows, "ABC-1"));
    }

    /// The cell of the first `!` on the card of `title` and the row it sits on.
    fn bang(rows: &[String], title: &str) -> (u16, u16) {
        at(rows, &format!("! {title}"))
    }

    /// ABC-1 is selected (the first card). The urgent story is ABC-1 when `selected_first`, else ABC-2.
    fn urgent_board(selected_first: bool) -> Sim {
        let (a, b) = if selected_first {
            ("ABC-1", "ABC-2")
        } else {
            ("ABC-2", "ABC-1")
        };
        let list = vec![
            urgent(owned(story(a, Todo, 1), "dan@example.com")),
            owned(story(b, Todo, 1), "dan@example.com"),
        ];
        Sim::loaded(TokenKind::Interactive, list)
    }

    #[test]
    fn an_urgent_card_carries_a_red_bold_bang_with_colour_and_a_plain_one_without() {
        for w in [120, 100] {
            // ABC-2 is urgent and not selected (ABC-1, first, is).
            let mut sim = urgent_board(false);
            let (rows, buf) = sim.render(w, 30);
            let (x, y) = bang(&rows, "Title of ABC-2");
            assert_eq!(buf[(x, y)].symbol(), "!");
            assert_eq!(buf[(x, y)].fg, Color::Reset, "no colour unless allowed");
            assert!(
                !has(&rows, "! Title of ABC-1"),
                "only the urgent story gets it: {rows:#?}"
            );
            sim.s.ui.color = true;
            let (_, buf) = sim.render(w, 30);
            assert_eq!(buf[(x, y)].fg, Color::Red, "width {w}");
            assert!(buf[(x, y)].modifier.contains(Modifier::BOLD));
            assert!(!buf[(x, y)].modifier.contains(Modifier::REVERSED));
        }
    }

    #[test]
    fn the_bang_on_the_selected_card_keeps_the_selected_style_not_red() {
        for w in [120, 100] {
            let mut sim = urgent_board(true);
            sim.s.ui.color = true;
            let (rows, buf) = sim.render(w, 30);
            let (x, y) = bang(&rows, "Title of ABC-1");
            let c = &buf[(x, y)];
            assert_eq!(c.symbol(), "!");
            assert!(c.modifier.contains(Modifier::REVERSED | Modifier::BOLD));
            assert_eq!(c.fg, Color::Reset, "width {w}");
            assert_ne!(c.bg, Color::Red);
        }
    }

    #[test]
    fn a_done_story_with_an_active_own_mark_has_no_bang() {
        let sim_list = vec![done_marked(owned(
            story("ABC-1", Done, 1),
            "dan@example.com",
        ))];
        for w in [120, 100] {
            let mut sim = Sim::loaded(TokenKind::Interactive, sim_list.clone());
            sim.s.board.test_set_filter_all(true);
            let (rows, _) = sim.render(w, 30);
            assert!(has(&rows, "Title of ABC-1"), "{rows:#?}");
            assert!(!has(&rows, "! "), "{rows:#?}");
        }
    }

    #[test]
    fn the_bang_leaves_column_zero_to_the_live_marker_and_the_title_gives_up_two_columns() {
        let mut s = urgent(owned(story("ABC-1", Todo, 1), "dan@example.com"));
        s.story.title = "A very long title that cannot fit in a card".into();
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        sim.s
            .live
            .test_set_last_changed(Some((key("ABC-1"), sim.now)));
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "• ● ABC-1"), "{rows:#?}");
        // Column inner width is 120 / 5 - 2 = 22: "! " then the title cut to 20 columns.
        let cut = crate::tui::view::fit("A very long title that cannot fit in a card", 20);
        assert!(has(&rows, &format!("! {cut}")), "{rows:#?}");
        let (x, y) = at(&rows, &format!("! {cut}"));
        let card: String = rows[y as usize].chars().skip(x as usize).take(22).collect();
        assert_eq!(card.trim_end(), format!("! {cut}"), "{rows:#?}");
        // List layout: the bang is still before the title and the title is cut with an ellipsis.
        let (rows, _) = sim.render(50, 30);
        let head = rows.iter().find(|r| r.contains("! A very")).unwrap();
        assert!(
            head.starts_with("• ● ABC-1 story  ! A very"),
            "the bang directly follows the lead: {rows:#?}"
        );
        assert!(head.trim_end().ends_with('…'), "{rows:#?}");
    }

    /// The card shows the chip and the drawer the promise line, both from the server's fields (kb/ADR-0011 R9). The
    /// chip leaves out the epic it is via: the card already names the story's epic, the only one it can be via.
    #[test]
    fn a_promised_card_shows_the_chip_and_the_drawer_the_line() {
        let s = || promised(owned(story("ABC-1", Todo, 1), "dan@example.com"));
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s()]);
        for w in [260, 100] {
            let (rows, _) = sim.render(w, 30);
            assert!(has(&rows, "Overdue 3 d · 7 Nov"), "{w}: {rows:#?}");
            assert!(!has(&rows, "via ABC-100"), "{w}: {rows:#?}");
        }
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s()]);
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        let (rows, _) = sim.render(240, 30);
        let (_, y) = at(&rows, "Title of ABC-1");
        assert!(
            rows[y as usize + 1].contains(
                "Promised by 30 Nov 2026 to Beta. Epic ABC-100 is promised by 7 Nov 2026 to Acme; the earlier date counts. Overdue 3 d."
            ),
            "{rows:#?}"
        );
    }

    /// In columns the chip has the card's fourth row to itself, so a 120-column board (22 columns a card) still shows
    /// the state of a not-ready story's promise; it is drawn without the readiness colour.
    #[test]
    fn a_narrow_column_card_keeps_the_promise_state_on_its_own_row() {
        let s = promised(not_ready(
            owned(story("ABC-1", Todo, 1), "dan@example.com"),
            vec![Reason::NoCriteria],
        ));
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        sim.s.ui.color = true;
        let (rows, buf) = sim.render(120, 30);
        let (x, y) = at(&rows, "Overdue 3 d");
        let (_, status) = at(&rows, "no criteria");
        assert_eq!(y, status + 1, "{rows:#?}");
        let (sx, _) = at(&rows, "no criteria");
        assert_eq!(buf[(sx, status)].fg, Color::Yellow, "the status has it");
        assert_ne!(buf[(x, y)].fg, Color::Yellow, "no readiness colour");
    }

    /// The list's status line cuts from the end: the goal first, then the PR and the epic, and the promise chip, just
    /// after the status, last.
    #[test]
    fn the_list_cuts_the_goal_before_the_epic_and_the_promise_last() {
        let mut s = promised(owned(story("ABC-1", Todo, 1), "dan@example.com"));
        s.story.goals = vec![reeve_protocol::GoalRefDto {
            id: 7,
            outcome: "Ship it".into(),
            via: reeve_protocol::GoalVia::Story,
        }];
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        let full = "    ready · Overdue 3 d · 7 Nov · epic ABC-100 · goal 7";
        let (rows, _) = sim.render(100, 30);
        assert!(has(&rows, full), "{rows:#?}");
        // Without room for the goal: the epic stays.
        let (rows, _) = sim.render(48, 30);
        assert!(
            has(&rows, "ready · Overdue 3 d · 7 Nov · epic ABC-100"),
            "{rows:#?}"
        );
        assert!(!has(&rows, "goal 7"), "{rows:#?}");
        // Without room for the epic: the chip stays.
        let (rows, _) = sim.render(42, 30);
        assert!(has(&rows, "ready · Overdue 3 d · 7 Nov"), "{rows:#?}");
        assert!(!has(&rows, "epic ABC-100"), "{rows:#?}");
    }

    #[test]
    fn the_drawer_says_why_a_story_is_urgent_right_under_its_title() {
        let mut s = urgent(owned(story("ABC-1", Todo, 1), "dan@example.com"));
        s.story.title = "The title".into();
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        let (rows, _) = sim.render(120, 30);
        let (_, y) = at(&rows, "The title");
        let right: String = rows[y as usize].chars().skip(60).collect();
        assert!(right.contains("The title"), "{rows:#?}");
        assert!(
            rows[y as usize + 1].contains("urgent · marked by dana, 2 d ago · expires in 10 d"),
            "{rows:#?}"
        );
    }

    #[test]
    fn the_filter_keeps_only_urgent_stories_and_has_a_third_row() {
        let list = vec![
            urgent(owned(story("ABC-1", Todo, 1), "dan@example.com")),
            owned(story("ABC-2", Todo, 1), "dan@example.com"),
            done_marked(owned(story("ABC-3", Done, 1), "dan@example.com")),
        ];
        let mut sim = Sim::loaded(TokenKind::Interactive, list);
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "Title of ABC-2") && has(&rows, "Title of ABC-3"));
        sim.char('f');
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "[ ] urgent only"), "{rows:#?}");
        sim.char('j');
        sim.char('j');
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "[x] urgent only"), "{rows:#?}");
        sim.char('f');
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "Title of ABC-1"), "{rows:#?}");
        assert!(
            !has(&rows, "Title of ABC-2") && !has(&rows, "Title of ABC-3"),
            "{rows:#?}"
        );
    }

    #[test]
    fn an_empty_urgent_filter_says_so() {
        let mut sim = Sim::loaded(
            TokenKind::Interactive,
            vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
        );
        let open_urgent = |sim: &mut Sim| {
            sim.char('f');
            sim.char('j');
            sim.char('j');
            sim.press(KeyCode::Enter, KeyModifiers::NONE);
            sim.char('f');
        };
        open_urgent(&mut sim);
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "no urgent stories owned by you — f shows all"),
            "{rows:#?}"
        );
        sim.s.board.test_set_filter_all(true);
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "no urgent stories"), "{rows:#?}");
        assert!(!has(&rows, "owned by you"), "{rows:#?}");
    }

    #[test]
    fn urgency_comes_from_the_sources_never_from_the_mark() {
        let list = vec![
            urgent_via_epic(owned(story("ABC-1", Todo, 1), "dan@example.com")),
            marked_without_sources(owned(story("ABC-2", Todo, 1), "dan@example.com")),
        ];
        for w in [120, 100] {
            let mut sim = Sim::loaded(TokenKind::Interactive, list.clone());
            let (rows, _) = sim.render(w, 30);
            assert!(has(&rows, "! Title of ABC-1"), "width {w}: {rows:#?}");
            assert!(
                has(&rows, "Title of ABC-2") && !has(&rows, "! Title of ABC-2"),
                "{rows:#?}"
            );
        }
        let mut sim = Sim::loaded(TokenKind::Interactive, list);
        sim.char('f');
        sim.char('j');
        sim.char('j');
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        sim.char('f');
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "Title of ABC-1") && !has(&rows, "Title of ABC-2"),
            "{rows:#?}"
        );
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "urgent · via epic ABC-100"), "{rows:#?}");
    }

    #[test]
    fn a_long_owner_at_a_narrow_width_never_clips_the_bang() {
        let mut s = urgent(owned(
            story("ABC-1", Todo, 1),
            "a-remarkably-long-owner-name@example.com",
        ));
        s.story.title = "Title".into();
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        sim.s.board.test_set_filter_all(true);
        let (rows, _) = sim.render(40, 20);
        let head = rows.iter().find(|r| r.contains("ABC-1")).unwrap();
        assert!(head.contains("…! "), "{rows:#?}");
    }

    #[test]
    fn the_drawer_scroll_cap_agrees_with_what_is_drawn() {
        // The frame's wording ("2 d ago", "expires in 10 d") is longer than the mark's own date would give, so
        // the urgent line wraps in the drawer and a cap counted with other wording would miss a line.
        let mut s = urgent(owned(story("ABC-1", Todo, 1), "dan@example.com"));
        s.story.urgent_mark.as_mut().unwrap().reason = Some("abcde".into());
        s.story.description = (1..=60)
            .map(|i| format!("line {i}"))
            .chain(["LASTLINE".to_string()])
            .collect::<Vec<_>>()
            .join("\n");
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        let _ = sim.render(120, 30); // the core learns the terminal's size from the driver's frames
        for _ in 0..100 {
            sim.char('J');
        }
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "LASTLINE"), "{rows:#?}");
    }

    #[test]
    fn the_status_line_shows_an_outcome_then_the_ticker() {
        let mut sim = mine(vec!["ABC-1"]);
        activity(&mut sim, "ABC-1", "edited");
        sim.char('y');
        let (rows, _) = sim.render(120, 30);
        assert!(rows[29].contains("copied ABC-1"), "{}", rows[29]);
        sim.now += crate::tui::app::STATUS_FOR + std::time::Duration::from_millis(1);
        let (rows, _) = sim.render(120, 30);
        assert!(!rows[29].contains("copied"), "{}", rows[29]);
        assert!(rows[29].contains("ABC-1 edited · anna"), "{}", rows[29]);
    }

    #[test]
    fn a_narrow_header_keeps_the_connection_state_and_the_notes_whole() {
        let mut sim = mine(vec!["ABC-1"]);
        sim.s.session.url = LONG_URL.into();
        sim.s.session.signed_out = true;
        let (rows, _) = sim.render(40, 10);
        assert!(rows[0].contains(crate::tui::app::SIGNED_OUT), "{}", rows[0]);

        let mut sim = mine(vec!["ABC-1"]);
        sim.s.session.url = LONG_URL.into();
        sim.s.session.reeve_token_set = true;
        sim.s
            .board
            .test_set_stale(Some("no answer: timed out".into()));
        let (rows, _) = sim.render(40, 10);
        assert!(
            rows[0].contains("stale · REEVE_TOKEN ignored"),
            "{}",
            rows[0]
        );
        // The host shrinks first: at 60 columns it is cut, and me survives.
        let (rows, _) = sim.render(60, 10);
        assert!(
            rows[0].contains("…") && rows[0].contains("dan") && rows[0].contains("stale"),
            "{}",
            rows[0]
        );

        let mut bot = Sim::loaded(TokenKind::Automation, vec![]);
        bot.s.session.url = LONG_URL.into();
        let (rows, _) = bot.render(40, 10);
        // Whole: the short note, not the long one cut with `…`.
        assert!(
            rows[0]
                .trim_end()
                .ends_with("connecting · automation token"),
            "{}",
            rows[0]
        );
    }

    #[test]
    fn the_override_input_stays_visible_under_many_long_reasons() {
        let mut sim = mine(vec!["ABC-1"]);
        let reasons = (0..6)
            .map(|i| Reason::DecisionNoRecord {
                decision_id: i,
                title: "a decision with a rather long title that wraps".into(),
            })
            .collect();
        refuse(&mut sim, reasons);
        let (rows, _) = sim.render(40, 10);
        assert!(has(&rows, "override reason:"), "{rows:#?}");
    }

    #[test]
    fn a_long_override_reason_keeps_the_cursor_and_the_tail_visible() {
        let mut sim = mine(vec!["ABC-1"]);
        refuse(&mut sim, vec![Reason::NoCriteria]);
        let typed: String = (0..80)
            .map(|i| char::from(b'a' + u8::try_from(i % 26).unwrap()))
            .collect();
        for c in typed.chars() {
            sim.char(c);
        }
        let (rows, _) = sim.render(60, 20);
        let row = rows
            .iter()
            .find(|r| r.contains("override reason:"))
            .unwrap_or_else(|| panic!("{rows:#?}"));
        let tail = &typed[typed.len() - 10..];
        assert!(row.contains(&format!("{tail}▏")), "{row}");
    }

    #[test]
    fn drawer_blockers_and_overlay_titles_name_the_kind() {
        let mut gra1 = owned(story("ABC-1", Todo, 1), "dan@example.com");
        gra1.story.blocked_by = vec![key("ABC-12")];
        let mut gra12 = story("ABC-12", InProgress, 1);
        gra12.story.kind = Kind::Bug;
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![gra1, gra12]);
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "ABC-12 bug Title of ABC-12 (in progress)"),
            "{rows:#?}"
        );
        sim.press(KeyCode::Esc, KeyModifiers::NONE);
        sim.char('m');
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "move ● ABC-1 story to"), "{rows:#?}");
        sim.press(KeyCode::Esc, KeyModifiers::NONE);
        sim.char('e');
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "epic for ● ABC-1 story"), "{rows:#?}");
    }

    #[test]
    fn the_urgent_prompt_is_titled_with_the_tagged_key_and_prefilled() {
        let mut marked = urgent(owned(story("ABC-1", Todo, 1), "dan@example.com"));
        marked.story.urgent_mark.as_mut().unwrap().reason = Some("old".into());
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![marked]);
        sim.char('u');
        let (rows, _) = sim.render(120, 30);
        let title = format!(
            " mark {} urgent ",
            crate::tui::moves::view::tagged(&sim.s.board, &key("ABC-1"))
        );
        assert_eq!(title, " mark ● ABC-1 story urgent ");
        assert!(has(&rows, &title), "{rows:#?}");
        assert!(has(&rows, "reason (optional): old"), "{rows:#?}");
    }

    #[test]
    fn fit_to_nothing_is_empty() {
        assert_eq!(super::fit("abc", 0), "");
        assert_eq!(super::fit("abc", 2), "a…");
    }

    #[test]
    fn epic_lines_and_empty_texts_are_cut_to_the_box() {
        let mut sim = mine(vec!["ABC-1"]);
        sim.char('e');
        sim.reply(
            |r| matches!(r, Request::Epics),
            Completion::Ok(Reply::Epics(vec![epic(
                "ABC-200",
                "An epic whose title is far wider than a forty-column box",
            )])),
        );
        let (rows, _) = sim.render(40, 10);
        let line = rows.iter().find(|r| r.contains("ABC-200")).unwrap();
        assert!(line.contains('…'), "{line}");

        let mut sim = mine(vec![]);
        sim.s.session.url = LONG_URL.into();
        sim.s.board.test_set_loaded(false);
        sim.s
            .board
            .test_set_stale(Some("no answer: timed out".into()));
        let (rows, _) = sim.render(40, 10);
        let line = rows.iter().find(|r| r.contains("cannot reach")).unwrap();
        assert!(line.trim_end().ends_with('…'), "{line}");
    }

    #[test]
    fn help_uses_two_columns_when_wide_and_says_when_it_is_cut() {
        let last = crate::tui::keys::help_lines().pop().unwrap();
        let mut sim = mine(vec!["ABC-1"]);
        sim.char('?');
        let (rows, _) = sim.render(140, 20);
        assert!(has(&rows, last.trim_end()), "{rows:#?}");
        let (rows, _) = sim.render(40, 10);
        assert!(!has(&rows, last.trim_end()));
        assert!(
            rows.iter().any(|r| r.trim_matches(['│', ' ']) == "…"),
            "{rows:#?}"
        );
    }

    #[test]
    fn a_drawer_scrolled_past_the_end_shows_the_last_page() {
        let mut s = owned(story("ABC-1", Todo, 1), "dan@example.com");
        s.story.description = (1..=60)
            .map(|i| format!("line {i}"))
            .chain(["LASTLINE".to_string()])
            .collect::<Vec<_>>()
            .join("\n");
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        sim.render(120, 30);
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        for _ in 0..40 {
            sim.char('J');
        }
        let (rows, _) = sim.render(120, 30);
        assert!(has(&rows, "LASTLINE") && has(&rows, "line 40"), "{rows:#?}");
        // A terminal grown under the drawer (its stored scroll now past the end) still shows the last page.
        let (rows, _) = sim.render(120, 40);
        assert!(has(&rows, "LASTLINE"), "{rows:#?}");
    }

    #[test]
    fn k_after_scrolling_past_the_end_moves_the_drawer_up_at_once() {
        let mut s = owned(story("ABC-1", Todo, 1), "dan@example.com");
        s.story.description = (1..=60)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![s]);
        sim.render(120, 30);
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        for _ in 0..40 {
            sim.char('J');
        }
        let (bottom, _) = sim.render(120, 30);
        assert!(has(&bottom, "line 60"), "{bottom:#?}");
        sim.char('K');
        let (up, _) = sim.render(120, 30);
        assert_ne!(
            up, bottom,
            "K moves the drawer up from the last page at once"
        );
        assert!(!has(&up, "line 60"), "{up:#?}");
    }

    #[test]
    fn the_last_changed_card_is_marked_and_bold_for_two_seconds() {
        let mut sim = mine(vec!["ABC-1", "ABC-2"]);
        sim.s
            .live
            .test_set_last_changed(Some((key("ABC-2"), sim.now)));
        let (rows, buf) = sim.render(120, 30);
        assert!(has(&rows, "• ● ABC-2"), "{rows:#?}");
        let (x, y) = at(&rows, "ABC-2");
        assert!(buf[(x, y)].modifier.contains(Modifier::BOLD));
        assert!(!buf[(x, y)].modifier.contains(Modifier::REVERSED));
        sim.now += std::time::Duration::from_secs(2);
        let (rows, buf) = sim.render(120, 30);
        assert!(has(&rows, "• ● ABC-2"), "the mark stays: {rows:#?}");
        assert!(!buf[(x, y)].modifier.contains(Modifier::BOLD));
        let (x, y) = at(&rows, "ABC-1");
        assert!(!rows[y as usize][..].contains("• ABC-1"));
        let _ = x;
    }

    #[test]
    fn colour_is_added_only_when_allowed() {
        let mut sim = mine(vec!["ABC-1"]);
        let (rows, buf) = sim.render(120, 30);
        let (x, y) = at(&rows, "ready");
        assert_eq!(buf[(x, y)].fg, Color::Reset);
        sim.s.ui.color = true;
        let (_, buf) = sim.render(120, 30);
        assert_eq!(buf[(x, y)].fg, Color::Green);
    }

    #[test]
    fn the_focused_column_has_a_thick_bold_border_and_is_yellow_only_with_colour() {
        let mut sim = mine(vec!["ABC-1"]);
        let (rows, buf) = sim.render(120, 30);
        assert!(
            has(&rows, "┏ todo (1) ") && has(&rows, "┌ in progress (0) "),
            "{rows:#?}"
        );
        let (x, y) = at(&rows, "todo (1)");
        assert!(buf[(x, y)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(x, y)].fg, Color::Reset);
        sim.char('l');
        let (rows, _) = sim.render(120, 30);
        assert!(
            has(&rows, "┌ todo (1) ") && has(&rows, "┏ in progress (0) "),
            "{rows:#?}"
        );
        sim.char('h');
        sim.s.ui.color = true;
        let (rows, buf) = sim.render(120, 30);
        let (x, y) = at(&rows, "todo (1)");
        assert_eq!(buf[(x, y)].fg, Color::Yellow);
        // The list layout marks the focused group's header the same way, without a border.
        let (rows, buf) = sim.render(100, 30);
        let (x, y) = at(&rows, "── todo (1)");
        assert!(buf[(x, y)].modifier.contains(Modifier::BOLD));
        let (x, y) = at(&rows, "── in progress (0)");
        assert!(!buf[(x, y)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_header_shows_the_main_keys_on_the_right_and_drops_them_first() {
        let mut sim = mine(vec!["ABC-1"]);
        let (rows, _) = sim.render(120, 30);
        assert!(
            rows[0].trim_end().ends_with(super::HINTS[0]) && rows[0].contains("· connecting"),
            "{}",
            rows[0]
        );
        let (rows, _) = sim.render(80, 20);
        assert!(rows[0].trim_end().ends_with(super::HINTS[1]), "{}", rows[0]);
        let (rows, _) = sim.render(40, 10);
        assert!(
            !rows[0].contains("hjkl") && rows[0].contains("connecting"),
            "{}",
            rows[0]
        );
    }

    #[test]
    fn space_lists_the_story_actions_and_says_why_one_would_do_nothing() {
        let mut sim = mine(vec!["ABC-1"]);
        sim.char(' ');
        let (rows, buf) = sim.render(120, 30);
        assert!(
            has(&rows, " ABC-1 story ") && has(&rows, "enter run · esc close"),
            "{rows:#?}"
        );
        for a in crate::tui::keys::MENU {
            assert!(
                has(&rows, crate::tui::keys::binding(*a).help),
                "{a:?}: {rows:#?}"
            );
        }
        // Mine already, in todo, no pull request: three rows say why, dimmed.
        assert!(has(&rows, "take (owner = me) — already yours"), "{rows:#?}");
        assert!(
            has(&rows, "earlier state — nothing before todo"),
            "{rows:#?}"
        );
        assert!(
            has(&rows, "first pull request — no pull request"),
            "{rows:#?}"
        );
        let (x, y) = at(&rows, "already yours");
        assert!(buf[(x, y)].modifier.contains(Modifier::DIM));
        // The first row is highlighted; Enter runs it (the drawer) and closes the menu.
        let (x, y) = at(&rows, "open the drawer");
        assert!(buf[(x, y)].modifier.contains(Modifier::REVERSED));
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(sim.s.ui.menu, None);
        assert_eq!(sim.s.ui.drawer, Some(0));
    }

    #[test]
    fn in_the_menu_an_actions_own_key_runs_it_and_a_disabled_one_says_why() {
        let mut sim = Sim::loaded(TokenKind::Interactive, vec![story("ABC-1", Todo, 1)]);
        sim.char('f');
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        sim.press(KeyCode::Esc, KeyModifiers::NONE);
        sim.char(' ');
        sim.char('t');
        assert_eq!(sim.s.ui.menu, None);
        assert!(sim.has_pending(|r| matches!(r, Request::Edit { .. })));

        let mut sim = mine(vec!["ABC-1"]);
        sim.char(' ');
        sim.char('H');
        assert_eq!(sim.s.ui.menu, None);
        assert_eq!(sim.status(), "ABC-1: nothing before todo");
        // j moves the highlight to the next row and Enter runs it: take, which is a no-op here.
        sim.char(' ');
        sim.char('j');
        sim.press(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(sim.status(), "ABC-1: already yours");
        // Esc, Space and q close it without quitting.
        for close in [' ', 'q'] {
            sim.char(' ');
            sim.char(close);
            assert_eq!(sim.s.ui.menu, None);
            assert!(!sim.s.quit);
        }
        sim.char(' ');
        sim.press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(sim.s.ui.menu, None);
    }

    #[test]
    fn a_read_only_session_dims_every_write_in_the_menu() {
        let mut bot = Sim::loaded(
            TokenKind::Automation,
            vec![owned(story("ABC-1", Todo, 1), "dan@example.com")],
        );
        bot.char(' ');
        let (rows, _) = bot.render(120, 30);
        assert!(
            has(&rows, "set epic — read-only: automation token"),
            "{rows:#?}"
        );
        assert!(
            has(&rows, "copy the key") && !has(&rows, "copy the key —"),
            "{rows:#?}"
        );
    }

    #[test]
    fn space_without_a_selected_story_says_so() {
        let mut sim = mine(vec![]);
        sim.char(' ');
        assert_eq!(sim.s.ui.menu, None);
        assert_eq!(sim.status(), "no story selected");
    }
}
