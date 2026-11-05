//! The epic picker.
use super::State;
use crate::glyphs::{Level, mark};
use crate::tui::board;
use crate::tui::moves::view::tagged;
use crate::tui::view::{centred, fit, selected_style};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Block, Clear, Paragraph},
};

pub fn draw(f: &mut Frame, area: Rect, e: &State, board: &board::State) {
    let Some(p) = &e.picker else { return };
    let v = e.visible();
    // Border (2) and the filter and blank lines (2) leave this many rows for epics; the window follows the cursor so
    // the highlighted epic, the one Enter picks, is always on screen.
    let r = centred(area, 64, (v.len().max(1) as u16 + 4).min(area.height));
    let rows = (r.height as usize).saturating_sub(4).max(1);
    // The border and the "› " mark take four columns.
    let filter_w = (r.width as usize).saturating_sub(2 + "filter: ".len());
    let mut lines = vec![
        Line::raw(format!("filter: {}", p.filter.window(filter_w))),
        Line::raw(""),
    ];
    let text_w = (r.width as usize).saturating_sub(4);
    match (&p.epics, &p.error) {
        (_, Some(err)) => lines.push(Line::raw(err.clone())),
        (None, None) => lines.push(Line::raw("loading…")),
        (Some(_), None) => {
            if v.is_empty() {
                lines.push(Line::raw("no epic matches"));
            }
            let start = (p.cursor + 1).saturating_sub(rows);
            for (i, ep) in v.iter().enumerate().skip(start).take(rows) {
                let t = fit(
                    &format!("{} {}", mark(Level::Epic, &ep.key), ep.title),
                    text_w,
                );
                lines.push(if i == p.cursor {
                    Line::styled(format!("› {t}"), selected_style())
                } else {
                    Line::raw(format!("  {t}"))
                });
            }
        }
    }
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(format!(" epic for {} ", tagged(board, &p.key)))),
        r,
    );
}
