//! The mark-urgent reason prompt: one input row, like the override prompt's.
use super::State;
use crate::tui::board;
use crate::tui::moves::view::tagged;
use crate::tui::view::centred;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Clear, Paragraph},
};

const LABEL: &str = "reason (optional): ";

pub fn draw(f: &mut Frame, area: Rect, u: &State, board: &board::State) {
    let Some(p) = &u.prompt else { return };
    let r = centred(area, 64, 3);
    let inner_w = r.width.saturating_sub(2) as usize;
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(format!(
            "{LABEL}{}",
            p.line.window(inner_w.saturating_sub(LABEL.len()))
        ))
        .block(Block::bordered().title(format!(" mark {} urgent ", tagged(board, &p.key)))),
        r,
    );
}
