//! The drawer: the selected story's working set, from board data.
use crate::glyphs::{Level, mark};
use crate::tui::app::WIDE;
use crate::tui::board::{Entry, State};
use crate::tui::{view, words};
use crate::urgency::urgent_lines;
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Block, Clear, Paragraph, Wrap},
};

pub fn lines(board: &State, e: &Entry, wall: DateTime<Utc>) -> Vec<String> {
    let s = &e.item.story;
    let mut out = vec![s.title.clone()];
    out.extend(urgent_lines(
        s.urgent_mark.as_ref(),
        &s.urgent_sources,
        &s.key,
        wall,
    ));
    out.push(format!(
        "owner {} · epic {} · {}",
        s.owner.as_deref().map(words::local_part).unwrap_or("none"),
        s.epic
            .as_ref()
            .map(|k| k.to_string())
            .unwrap_or_else(|| "none".into()),
        words::state(s.delivery)
    ));
    out.push(String::new());
    out.push("readiness".into());
    if e.readiness_pending {
        out.push("  …".into());
    } else if e.item.readiness.ready {
        out.push("  ready".into());
    } else {
        out.extend(
            e.item
                .readiness
                .reasons
                .iter()
                .map(|r| format!("  {}", words::full(r))),
        );
    }
    if !s.criteria.is_empty() {
        out.push(String::new());
        out.push("criteria".into());
        out.extend(
            s.criteria
                .iter()
                .map(|c| format!("  {} {}", if c.done { "☑" } else { "☐" }, c.text)),
        );
    }
    if !s.blocked_by.is_empty() {
        out.push(String::new());
        out.push("blockers".into());
        for k in &s.blocked_by {
            out.push(match board.get(k) {
                Some(b) => format!(
                    "  {k} {} {} ({})",
                    words::kind(b.item.story.kind),
                    b.item.story.title,
                    words::state(b.item.story.delivery)
                ),
                None => format!("  {k}"),
            });
        }
    }
    if !s.prs.is_empty() {
        out.push(String::new());
        out.push("pull requests".into());
        out.extend(s.prs.iter().map(|p| {
            format!(
                "  {}#{} {} {}",
                p.repo,
                p.number,
                format!("{:?}", p.status).to_lowercase(),
                p.title
            )
        }));
    }
    if !s.description.is_empty() {
        out.push(String::new());
        out.extend(s.description.lines().map(str::to_string));
    }
    out
}

/// Where the drawer goes on a terminal of `terminal`'s size: the right half of the body when wide, the whole body when
/// narrow. The view draws it there and the core caps the scroll from it, so the two cannot disagree.
pub fn area(terminal: Rect) -> Rect {
    let body = view::layout(terminal)[1];
    if terminal.width >= WIDE {
        Rect {
            x: body.x + body.width / 2,
            width: body.width - body.width / 2,
            ..body
        }
    } else {
        body
    }
}

fn block(e: &Entry) -> Block<'static> {
    let s = &e.item.story;
    Block::bordered().title(format!(
        " {} · {} ",
        mark(Level::of_kind(s.kind), &s.key),
        words::kind(s.kind)
    ))
}

fn paragraph(board: &State, e: &Entry, wall: DateTime<Utc>) -> Paragraph<'static> {
    // Wrapped, never cut: the full reasons and the whole description must be readable, and the scroll counts wrapped
    // lines, so J/K reach the end of a long description.
    let text: Vec<Line> = lines(board, e, wall).into_iter().map(Line::raw).collect();
    Paragraph::new(text).wrap(Wrap { trim: false })
}

fn last(p: &Paragraph, inner: Rect) -> u16 {
    let n = p
        .line_count(inner.width)
        .saturating_sub(inner.height as usize);
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// The furthest the drawer at `area` scrolls: its wrapped line count less its inner height (0 when it all fits).
///
/// The core has no clock (kb/ADR-0011 R4), so the urgent lines are counted with the own mark's creation time as "now":
/// the dates only change a number's width inside a line, never how many lines there are.
pub fn max_scroll(board: &State, e: &Entry, area: Rect) -> u16 {
    let wall = e
        .item
        .story
        .urgent_mark
        .as_ref()
        .map_or(DateTime::<Utc>::UNIX_EPOCH, |m| m.at);
    last(&paragraph(board, e, wall), block(e).inner(area))
}

pub fn draw(f: &mut Frame, area: Rect, board: &State, e: &Entry, scroll: u16, wall: DateTime<Utc>) {
    let block = block(e);
    let p = paragraph(board, e, wall);
    // The core caps the stored scroll; the data or the size may since have changed, so a scroll past the end shows
    // the last page rather than a blank box.
    let scroll = scroll.min(last(&p, block.inner(area)));
    f.render_widget(Clear, area);
    f.render_widget(p.block(block).scroll((scroll, 0)), area);
}
