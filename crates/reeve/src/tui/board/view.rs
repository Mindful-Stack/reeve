//! The board: five columns (wide) or one list grouped by state (narrow); empty states; the filter menu.
use super::{Ctx, Entry};
use crate::glyphs::{Level, mark};
use crate::tui::app::State;
use crate::tui::view::{centred, fit, selected_style};
use crate::tui::words::{self, STATES};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph},
};
use std::time::{Duration, Instant};

const HIGHLIGHT: Duration = Duration::from_secs(2);

fn title(s: &State, col: usize, n: usize) -> String {
    match (STATES[col], s.board.done_window_days) {
        (reeve_protocol::DeliveryState::Done, Some(d)) => format!("done · {d} days ({n})"),
        (st, _) => format!("{} ({n})", words::state(st)),
    }
}

/// The third card line (columns) or the second (list): the pending marker, the pending target, or readiness.
fn status_of(s: &State, e: &Entry) -> String {
    let k = &e.item.story.key;
    // A move in flight shows where it is heading (with presses since, where they want it to end).
    if let Some(i) = s.moves.intent(k) {
        return format!("→ {}", words::state(i.target));
    }
    if s.board.locked(k) || e.readiness_pending {
        return "…".into();
    }
    words::readiness_short(&e.item.readiness)
}

fn card_style(s: &State, e: &Entry, now: Instant) -> Style {
    let k = &e.item.story.key;
    if s.board.selected() == Some(k) {
        return selected_style();
    }
    match s.live.last_changed() {
        Some((c, at)) if c == k && now.saturating_duration_since(*at) < HIGHLIGHT => {
            Style::new().add_modifier(Modifier::BOLD)
        }
        _ => Style::new(),
    }
}

fn marker(s: &State, e: &Entry) -> &'static str {
    match s.live.last_changed() {
        Some((c, _)) if *c == e.item.story.key => "• ",
        _ => "  ",
    }
}

/// The focused column's border and title, or group header: bold (a thick border too, in columns), and yellow when
/// colour is allowed, so the focus reads without colour.
fn focus_style(s: &State) -> Style {
    let st = Style::new().add_modifier(Modifier::BOLD);
    if s.ui.color { st.fg(Color::Yellow) } else { st }
}

fn readiness_style(s: &State, e: &Entry) -> Style {
    if !s.ui.color {
        return Style::new();
    }
    if e.item.readiness.ready {
        Style::new().fg(Color::Green)
    } else {
        Style::new().fg(Color::Yellow)
    }
}

/// The tags after a card's status line, in both layouts: epic, then PR, then goal. `fit` cuts a too-narrow line from the
/// end, so these tags go before the status, and the goal tag, last, goes first.
fn meta(e: &Entry) -> String {
    let story = &e.item.story;
    let mut m = String::new();
    if let Some(ep) = &story.epic {
        m.push_str(&format!(" · epic {ep}"));
    }
    if let Some(pr) = story.prs.first() {
        m.push_str(&format!(
            " · PR {}",
            format!("{:?}", pr.status).to_lowercase()
        ));
    }
    if let Some(g) = story.goals.first() {
        m.push_str(&format!(" · goal {}", g.id));
    }
    m
}

fn owner_tag(s: &State, e: &Entry) -> String {
    match (&e.item.story.owner, s.board.filter.all) {
        (Some(o), true) => format!(" {}", words::local_part(o)),
        _ => String::new(),
    }
}

fn empty_text(s: &State, cx: &Ctx, visible: &[&Entry]) -> Option<String> {
    if !s.board.loaded {
        return Some(match (&s.board.stale, cx.me) {
            (Some(e), _) => format!("cannot reach {}: {e}", s.session.url),
            (None, None) => format!("connecting to {}…", s.session.url),
            (None, Some(_)) => "loading…".into(),
        });
    }
    if !visible.is_empty() {
        return None;
    }
    Some(if !s.board.query.is_empty() {
        format!("nothing matches \"{}\"", s.board.query)
    } else if !s.board.filter.all {
        "no stories owned by you — f shows all".into()
    } else {
        "no stories".into()
    })
}

/// The first index to draw so that `selected` (an index into `n` items of `per` rows) fits in `rows`.
fn scroll(selected: Option<usize>, per: usize, rows: usize) -> usize {
    let fits = (rows / per).max(1);
    selected.map_or(0, |i| (i + 1).saturating_sub(fits))
}

/// The visible stories split by column, from one pass over the board.
fn by_column<'a>(visible: &[&'a Entry]) -> [Vec<&'a Entry>; 5] {
    let mut cols: [Vec<&Entry>; 5] = Default::default();
    for e in visible {
        cols[words::index(e.item.story.delivery)].push(e);
    }
    cols
}

pub fn draw(f: &mut Frame, area: Rect, s: &State, cx: &Ctx, now: Instant) {
    let visible = s.board.visible(cx);
    if let Some(t) = empty_text(s, cx, &visible) {
        f.render_widget(
            Paragraph::new(fit(&t, area.width as usize)),
            centred(area, area.width, 1),
        );
        return;
    }
    let cols = by_column(&visible);
    if cx.wide {
        columns(f, area, s, &cols, now)
    } else {
        list(f, area, s, &cols, now)
    }
}

fn columns(f: &mut Frame, area: Rect, s: &State, by_col: &[Vec<&Entry>; 5], now: Instant) {
    let cols = Layout::horizontal([Constraint::Ratio(1, 5); 5]).split(area);
    for (i, r) in cols.iter().enumerate() {
        let cards = &by_col[i];
        let name = format!(" {} ", title(s, i, cards.len()));
        let block = if i == s.board.focus() {
            Block::bordered()
                .border_type(BorderType::Thick)
                .border_style(focus_style(s))
                .title(Span::styled(name, focus_style(s)))
        } else {
            Block::bordered().title(name)
        };
        let inner = block.inner(*r);
        f.render_widget(block, *r);
        let w = inner.width as usize;
        let sel = cards
            .iter()
            .position(|e| s.board.selected.as_ref() == Some(&e.item.story.key));
        let mut lines: Vec<Line> = Vec::new();
        for e in cards.iter().skip(scroll(sel, 4, inner.height as usize)) {
            let st = card_style(s, e, now);
            let story = &e.item.story;
            let head = format!(
                "{}{} {}{}",
                marker(s, e),
                mark(Level::of_kind(story.kind), &story.key),
                words::kind(story.kind),
                owner_tag(s, e)
            );
            lines.push(Line::styled(fit(&head, w), st));
            lines.push(Line::styled(fit(&format!("  {}", story.title), w), st));
            let third = format!("  {}{}", status_of(s, e), meta(e));
            lines.push(Line::from(Span::styled(
                fit(&third, w),
                st.patch(readiness_style(s, e)),
            )));
            lines.push(Line::raw(""));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }
}

fn list(f: &mut Frame, area: Rect, s: &State, by_col: &[Vec<&Entry>; 5], now: Instant) {
    let w = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    let mut sel_line = None;
    for (i, _) in STATES.iter().enumerate() {
        let cards = &by_col[i];
        let header = fit(&format!("── {}", title(s, i, cards.len())), w);
        lines.push(if i == s.board.focus() {
            Line::styled(header, focus_style(s))
        } else {
            Line::raw(header)
        });
        for e in cards {
            if s.board.selected.as_ref() == Some(&e.item.story.key) {
                sel_line = Some(lines.len());
            }
            let st = card_style(s, e, now);
            let story = &e.item.story;
            let head = format!(
                "{}{} {}{}  {}",
                marker(s, e),
                mark(Level::of_kind(story.kind), &story.key),
                words::kind(story.kind),
                owner_tag(s, e),
                story.title
            );
            lines.push(Line::styled(fit(&head, w), st));
            let second = format!("    {}{}", status_of(s, e), meta(e));
            lines.push(Line::from(Span::styled(
                fit(&second, w),
                st.patch(readiness_style(s, e)),
            )));
        }
    }
    let top = sel_line.map_or(0, |l| (l + 2).saturating_sub(area.height as usize));
    f.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), area);
}

pub fn filter_menu(f: &mut Frame, area: Rect, b: &super::State, row: usize) {
    let mark = |on: bool| if on { "[x]" } else { "[ ]" };
    let items = [
        format!("{} all stories (off: mine)", mark(b.filter.all)),
        format!("{} ready only", mark(b.filter.ready_only)),
    ];
    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == row {
                Line::styled(t.clone(), selected_style())
            } else {
                Line::raw(t.clone())
            }
        })
        .collect();
    let r = centred(area, 36, 4);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" filter ")),
        r,
    );
}
