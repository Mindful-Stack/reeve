use super::finish;
use crate::client::{Client, Outcome};
use crate::render;
use crate::urgency::marks;
use clap::Subcommand;
use reeve_protocol::{WarRoomEntryDto, WarRoomItems};

#[derive(Subcommand)]
pub enum WarRoom {
    /// List every urgent mark: active first, then expired.
    Ls,
    /// Clear every listed mark; rehearses unless --yes (interactive tokens only).
    Clear {
        #[arg(long)]
        yes: bool,
    },
    /// Re-mark every listed expired mark for another cycle (done stories left out); rehearses unless --yes (interactive tokens only).
    #[command(name = "re-mark", alias = "renew")]
    ReMark {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Verb {
    Clear,
    ReMark,
}

/// The rows `verb` acts on, in the server's order, and how many expired marks on done stories `re-mark` leaves out.
/// `clear` takes every row; `re-mark` takes the expired ones that are not done (re-marking a done story's mark would
/// make nothing urgent), as the web's "Re-mark all expired" does.
fn selection(rows: &[WarRoomEntryDto], verb: Verb) -> (Vec<&WarRoomEntryDto>, usize) {
    match verb {
        Verb::Clear => (rows.iter().collect(), 0),
        Verb::ReMark => (
            rows.iter()
                .filter(|r| !r.mark.active && !r.mark.done)
                .collect(),
            rows.iter()
                .filter(|r| !r.mark.active && r.mark.done)
                .count(),
        ),
    }
}

/// " (1 on a done story left out)", " (N on done stories left out)", or nothing.
fn note(left: usize) -> String {
    match left {
        0 => String::new(),
        1 => " (1 on a done story left out)".into(),
        n => format!(" ({n} on done stories left out)"),
    }
}

fn empty_text(verb: Verb, left: usize) -> String {
    match verb {
        Verb::Clear => "Nothing to clear.".into(),
        Verb::ReMark => format!("No expired marks to re-mark{}.", note(left)),
    }
}

fn rehearsal_line(verb: Verb, n: usize, left: usize) -> String {
    match verb {
        Verb::Clear => format!("Would clear {}; run again with --yes.", marks(n)),
        Verb::ReMark => format!(
            "Would re-mark {} for another cycle{}; run again with --yes.",
            marks(n),
            note(left)
        ),
    }
}

pub async fn run(c: &Client, cmd: WarRoom, json: bool) -> anyhow::Result<i32> {
    let now = chrono::Utc::now();
    let listed = c.get::<Vec<WarRoomEntryDto>>("/war-room").await?;
    let (verb, yes) = match cmd {
        WarRoom::Ls => {
            return Ok(finish(listed, json, |rows| {
                render::war_room_rows(rows, now)
            }));
        }
        WarRoom::Clear { yes } => (Verb::Clear, yes),
        WarRoom::ReMark { yes } => (Verb::ReMark, yes),
    };
    let Outcome::Ok(rows) = &listed else {
        return Ok(finish(listed, json, |_| String::new()));
    };
    let (selected, left) = selection(rows, verb);
    if selected.is_empty() {
        let text = empty_text(verb, left);
        return Ok(finish(listed, json, |_| text.clone()));
    }
    if !yes {
        let text = format!(
            "{}\n{}",
            render::war_room_rows(selected.iter().copied(), now),
            rehearsal_line(verb, selected.len(), left)
        );
        return Ok(finish(listed, json, |_| text.clone()));
    }
    let items = selected.iter().map(|r| r.item.clone()).collect();
    let path = match verb {
        Verb::Clear => "/war-room/clear",
        Verb::ReMark => "/war-room/renew",
    };
    let answer = c
        .post::<Vec<WarRoomEntryDto>>(path, serde_json::to_value(WarRoomItems { items })?)
        .await?;
    Ok(finish(answer, json, |rows| {
        render::war_room_rows(rows, now)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use reeve_protocol::{ItemRefDto, ItemType, UrgentMarkDto};

    fn row(key: &str, item: ItemType, active: bool, done: bool) -> WarRoomEntryDto {
        WarRoomEntryDto {
            item: ItemRefDto {
                key: key.parse().unwrap(),
                item,
            },
            title: format!("t-{key}"),
            initiative: None,
            mark: UrgentMarkDto {
                at: Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap(),
                by: "dana@example.com".into(),
                reason: None,
                expires_at: Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap(),
                active,
                done,
            },
        }
    }

    fn four() -> Vec<WarRoomEntryDto> {
        vec![
            row("ABC-1", ItemType::Story, true, false),
            row("ABC-2", ItemType::Story, false, false),
            row("ABC-3", ItemType::Story, false, true),
            row("ABC-4", ItemType::Story, true, true),
        ]
    }

    fn keys(v: &[&WarRoomEntryDto]) -> Vec<String> {
        v.iter().map(|r| r.item.key.to_string()).collect()
    }

    #[test]
    fn clear_selects_every_row_and_leaves_none_out() {
        let rows = four();
        let (sel, left) = selection(&rows, Verb::Clear);
        assert_eq!(keys(&sel), ["ABC-1", "ABC-2", "ABC-3", "ABC-4"]);
        assert_eq!(left, 0);
    }

    #[test]
    fn re_mark_selects_only_expired_not_done_and_counts_expired_done_as_left_out() {
        let rows = four();
        let (sel, left) = selection(&rows, Verb::ReMark);
        assert_eq!(keys(&sel), ["ABC-2"]);
        // The active-and-done row is neither selected nor counted.
        assert_eq!(left, 1);
    }

    #[test]
    fn rehearsal_lines_count_marks_and_word_the_note() {
        assert_eq!(
            rehearsal_line(Verb::Clear, 1, 0),
            "Would clear 1 mark; run again with --yes."
        );
        assert_eq!(
            rehearsal_line(Verb::Clear, 2, 0),
            "Would clear 2 marks; run again with --yes."
        );
        assert_eq!(
            rehearsal_line(Verb::ReMark, 2, 0),
            "Would re-mark 2 marks for another cycle; run again with --yes."
        );
        assert_eq!(
            rehearsal_line(Verb::ReMark, 1, 1),
            "Would re-mark 1 mark for another cycle (1 on a done story left out); run again with --yes."
        );
        assert_eq!(
            rehearsal_line(Verb::ReMark, 2, 2),
            "Would re-mark 2 marks for another cycle (2 on done stories left out); run again with --yes."
        );
    }

    #[test]
    fn empty_texts() {
        assert_eq!(empty_text(Verb::Clear, 0), "Nothing to clear.");
        assert_eq!(empty_text(Verb::ReMark, 0), "No expired marks to re-mark.");
        assert_eq!(
            empty_text(Verb::ReMark, 1),
            "No expired marks to re-mark (1 on a done story left out)."
        );
        assert_eq!(
            empty_text(Verb::ReMark, 3),
            "No expired marks to re-mark (3 on done stories left out)."
        );
    }
}
