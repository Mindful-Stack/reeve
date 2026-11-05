//! The move picker and the resolution and override prompts.
use super::{Op, Prompt, State};
use crate::glyphs::{Level, mark};
use crate::tui::board;
use crate::tui::line::LineEditor;
use crate::tui::view::{centred, selected_style};
use crate::tui::words::{self, STATES};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Block, Clear, Paragraph, Wrap},
};
use reeve_protocol::{Key, Resolution};

/// The key with its kind beside it (kb/ADR-0002 R2), when the story is on the board.
pub fn tagged(board: &board::State, k: &Key) -> String {
    match board.get(k) {
        Some(e) => format!(
            "{} {}",
            mark(Level::of_kind(e.item.story.kind), k),
            words::kind(e.item.story.kind)
        ),
        None => mark(Level::Story, k),
    }
}

fn arrow(board: &board::State, op: &Op) -> String {
    format!(
        " {}: {} → {} ",
        tagged(board, &op.key),
        words::state(op.origin),
        words::state(op.target)
    )
}

fn choice(i: usize, cursor: usize, t: &str) -> Line<'static> {
    if i == cursor {
        Line::styled(format!("› {t}"), selected_style())
    } else {
        Line::raw(format!("  {t}"))
    }
}

pub fn draw(f: &mut Frame, area: Rect, m: &State, board: &board::State) {
    // The picker (five states) and the resolution prompt (two options) fit in the smallest body (8 rows) whole.
    let (title, lines): (String, Vec<Line>) = if let Some(p) = &m.picker {
        let lines = STATES
            .iter()
            .enumerate()
            .map(|(i, st)| {
                let now = if *st == p.origin { " (now)" } else { "" };
                choice(i, p.cursor, &format!("{}{now}", words::state(*st)))
            })
            .collect();
        (format!(" move {} to… ", tagged(board, &p.key)), lines)
    } else {
        match &m.prompt {
            Some(Prompt::Resolution {
                op,
                options,
                choice: c,
            }) => {
                let lines = options
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let t = match r {
                            Resolution::Completed => "completed",
                            Resolution::Dropped => "dropped",
                        };
                        choice(i, *c, t)
                    })
                    .collect();
                (arrow(board, op), lines)
            }
            Some(Prompt::Override { op, reasons, line }) => {
                return override_prompt(f, area, &arrow(board, op), reasons, line);
            }
            None => return,
        }
    };
    let r = centred(area, 64, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(title)),
        r,
    );
}

/// The reasons, wrapped, above the input line, which has its own row at the bottom of the box so that no number of
/// reasons can push it out of sight.
fn override_prompt(
    f: &mut Frame,
    area: Rect,
    title: &str,
    reasons: &[reeve_protocol::Reason],
    input: &LineEditor,
) {
    const LABEL: &str = "override reason: ";
    let w = area.width.min(64);
    let inner_w = w.saturating_sub(2);
    let mut text: Vec<Line> = reasons
        .iter()
        .map(|r| Line::raw(format!("  {}", words::full(r))))
        .collect();
    text.push(Line::raw(""));
    let p = Paragraph::new(text).wrap(Wrap { trim: false });
    let h = u16::try_from(p.line_count(inner_w)).unwrap_or(u16::MAX);
    let r = centred(area, w, h.saturating_add(3));
    let block = Block::bordered().title(title.to_string());
    let [above, input_row] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(block.inner(r));
    f.render_widget(Clear, r);
    f.render_widget(block, r);
    f.render_widget(p, above);
    f.render_widget(
        Paragraph::new(format!(
            "{LABEL}{}",
            input.window((inner_w as usize).saturating_sub(LABEL.len()))
        )),
        input_row,
    );
}
