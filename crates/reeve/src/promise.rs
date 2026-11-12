//! The words of a promise (kb/ADR-0025), as the web says them (`web/src/lib/promise.ts`): every value comes from the
//! server, which derives state, due period and days on read, so nothing here reads a clock or computes a date
//! (kb/ADR-0011 R9).
use chrono::NaiveDate;
use reeve_protocol::{
    EpicPromiseDto, PromiseDue, PromiseFactDto, PromiseHolder, PromiseState, StoryPromiseDto,
};

/// "2026-11-20" → "20 Nov", from the date's own parts (no time zone in between).
pub fn day_label(d: NaiveDate) -> String {
    d.format("%-d %b").to_string()
}

/// The local part of an email address.
fn short(email: &str) -> &str {
    match email.find('@') {
        Some(at) if at > 0 => &email[..at],
        _ => email,
    }
}

/// " · this sprint", " · next sprint", or nothing (later, or no period).
fn period(due: Option<PromiseDue>) -> &'static str {
    match due {
        Some(PromiseDue::ThisSprint) => " · this sprint",
        Some(PromiseDue::NextSprint) => " · next sprint",
        _ => "",
    }
}

/// " 3 d", or "" when the server sent no count.
fn days_text(days: Option<u32>) -> String {
    days.map(|d| format!(" {d} d")).unwrap_or_default()
}

/// The one table of state words, lower case (the web's `STATE_WORDS`); `None` for open.
pub fn state_words(state: PromiseState, days: Option<u32>) -> Option<String> {
    match state {
        PromiseState::Open => None,
        PromiseState::Overdue => Some(format!("overdue{}", days_text(days))),
        PromiseState::Kept => Some("kept".into()),
        PromiseState::Late => Some(format!("done{} late", days_text(days))),
        PromiseState::Dropped => Some("dropped".into()),
    }
}

fn capital(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// A story's state words, or `None` for open and no state.
fn story_state_words(p: &StoryPromiseDto) -> Option<String> {
    p.state.and_then(|s| state_words(s, p.days))
}

/// A story chip's words without the epic: "Overdue 3 d · 20 Nov", "Promised 20 Nov · this sprint", …
fn story_chip_base(p: &StoryPromiseDto) -> String {
    let d = day_label(p.on);
    match story_state_words(p) {
        Some(w) => format!("{} · {d}", capital(&w)),
        None if p.state == Some(PromiseState::Open) => format!("Promised {d}{}", period(p.due)),
        None => format!("Promised {d}"),
    }
}

/// A story chip's text, from the server's effective date, state, period and days; `via` names the epic whose promise
/// sets the date (off where every row is under that epic).
pub fn chip_text(p: &StoryPromiseDto, via: bool) -> String {
    let base = story_chip_base(p);
    match (&p.epic, p.holder) {
        (Some(e), PromiseHolder::Epic) if via => format!("{base} · via {}", e.key),
        _ => base,
    }
}

/// An epic chip's text: an epic has no state of its own (R4), only a period while the date is not past.
pub fn epic_chip_text(p: &EpicPromiseDto) -> String {
    let d = day_label(p.fact.on);
    if p.past {
        format!("Past {d}")
    } else {
        format!("Promised {d}{}", period(p.due))
    }
}

fn to(f: &PromiseFactDto) -> String {
    match f.to.as_deref() {
        Some(t) if !t.is_empty() => format!(" to {t}"),
        _ => String::new(),
    }
}

fn set_by(f: &PromiseFactDto) -> String {
    format!(" · set by {}", short(&f.by))
}

/// The drawer's promise line: who promised what to whom, both promises when the story has its own and its epic's, then
/// the state (a sentence of its own after two promises).
pub fn line_text(p: &StoryPromiseDto) -> String {
    let w = story_state_words(p);
    match (&p.own, &p.epic) {
        (Some(own), Some(epic)) => {
            let counts = if p.holder == PromiseHolder::Epic {
                "; the earlier date counts."
            } else {
                "."
            };
            let state = w.map(|w| format!(" {}.", capital(&w))).unwrap_or_default();
            format!(
                "Promised {}{}. Epic {} is promised {}{counts}{state}",
                day_label(own.on),
                to(own),
                epic.key,
                day_label(epic.fact.on)
            )
        }
        (own, epic) => {
            let lead = match (own, epic) {
                (_, Some(e)) => format!(
                    "Promised {} via epic {}{}{}",
                    day_label(e.fact.on),
                    e.key,
                    to(&e.fact),
                    set_by(&e.fact)
                ),
                (Some(o), None) => format!("Promised {}{}{}", day_label(o.on), to(o), set_by(o)),
                (None, None) => format!("Promised {}", day_label(p.on)),
            };
            match w {
                Some(w) => format!("{lead} · {w}"),
                None => lead,
            }
        }
    }
}

/// The epic page's promise line.
pub fn epic_line_text(p: &EpicPromiseDto) -> String {
    format!(
        "Promised {}{}{}{}",
        day_label(p.fact.on),
        to(&p.fact),
        set_by(&p.fact),
        if p.past { " · past" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    //! The word rows are `web/src/lib/promise.fixture.json`, which the web's `promise.test.ts` reads too, so the two
    //! cannot drift.
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Deserialize)]
    struct Row<P> {
        label: String,
        promise: P,
        #[serde(default)]
        via: Option<bool>,
        text: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Words {
        chip: Vec<Row<StoryPromiseDto>>,
        epic_chip: Vec<Row<EpicPromiseDto>>,
        line: Vec<Row<StoryPromiseDto>>,
        epic_line: Vec<Row<EpicPromiseDto>>,
        state_words: BTreeMap<String, [String; 2]>,
    }

    fn words() -> Words {
        serde_json::from_str(include_str!("../../../web/src/lib/promise.fixture.json")).unwrap()
    }

    #[test]
    fn chip_text_rows() {
        let rows = words().chip;
        assert!(!rows.is_empty());
        for r in rows {
            assert_eq!(
                chip_text(&r.promise, r.via.unwrap_or(true)),
                r.text,
                "{}",
                r.label
            );
        }
    }

    #[test]
    fn epic_chip_text_rows() {
        for r in words().epic_chip {
            assert_eq!(epic_chip_text(&r.promise), r.text, "{}", r.label);
        }
    }

    #[test]
    fn line_text_rows() {
        for r in words().line {
            assert_eq!(line_text(&r.promise), r.text, "{}", r.label);
        }
    }

    #[test]
    fn epic_line_text_rows() {
        for r in words().epic_line {
            assert_eq!(epic_line_text(&r.promise), r.text, "{}", r.label);
        }
    }

    /// The web's `STATE_WORDS` table, with and without a day count; open has no words.
    #[test]
    fn state_words_is_the_one_table() {
        let mut got = BTreeMap::new();
        for s in [
            PromiseState::Overdue,
            PromiseState::Kept,
            PromiseState::Late,
            PromiseState::Dropped,
        ] {
            let name = serde_json::to_value(s)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();
            got.insert(
                name,
                [
                    state_words(s, Some(3)).unwrap(),
                    state_words(s, None).unwrap(),
                ],
            );
        }
        assert_eq!(got, words().state_words);
        assert_eq!(state_words(PromiseState::Open, Some(3)), None);
    }

    #[test]
    fn day_labels_have_no_leading_zero() {
        let date = |s: &str| s.parse::<NaiveDate>().unwrap();
        assert_eq!(day_label(date("2026-11-02")), "2 Nov");
        assert_eq!(day_label(date("2026-01-20")), "20 Jan");
    }
}
