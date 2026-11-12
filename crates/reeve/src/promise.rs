//! The words of a promise (kb/ADR-0025), as the web says them (`web/src/lib/promise.ts`): every value comes from the
//! server, which derives state, due period and days on read, so nothing here reads a clock or computes a date
//! (kb/ADR-0011 R9).
use chrono::NaiveDate;
use reeve_protocol::{
    EpicPromiseDto, PromiseDue, PromiseFactDto, PromiseHolder, PromiseState, SprintPromiseDto,
    StoryPromiseDto,
};

/// "2026-11-20" → "20 Nov", from the date's own parts (no time zone in between).
pub fn day_label(d: NaiveDate) -> String {
    d.format("%-d %b").to_string()
}

/// "2026-11-20" → "20 Nov 2026": a line's date, with the year (a kept promise stays on a done story for good).
pub fn date_label(d: NaiveDate) -> String {
    d.format("%-d %b %Y").to_string()
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

/// The one table of state words, lower case (the web's `STATE_WORDS`); `None` for open and for a state this build does
/// not know (shown neutrally, "Promised by 20 Nov").
pub fn state_words(state: PromiseState, days: Option<u32>) -> Option<String> {
    match state {
        PromiseState::Open | PromiseState::Unknown => None,
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

/// A story chip's words without the epic: "Overdue 3 d · 20 Nov", "Promised by 20 Nov · this sprint", …
fn story_chip_base(p: &StoryPromiseDto) -> String {
    let d = day_label(p.on);
    match story_state_words(p) {
        Some(w) => format!("{} · {d}", capital(&w)),
        None if p.state == Some(PromiseState::Open) => format!("Promised by {d}{}", period(p.due)),
        None => format!("Promised by {d}"),
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
        format!("Promised by {d}{}", period(p.due))
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

/// The drawer's promise line: who promised what by which date (with its year) to whom, both promises when the story
/// has its own and its epic's, then the state (a sentence of its own after two promises).
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
                "Promised by {}{}. Epic {} is promised by {}{}{counts}{state}",
                date_label(own.on),
                to(own),
                epic.key,
                date_label(epic.fact.on),
                to(&epic.fact)
            )
        }
        (own, epic) => {
            let lead = match (own, epic) {
                (_, Some(e)) => format!(
                    "Promised by {} via epic {}{}{}",
                    date_label(e.fact.on),
                    e.key,
                    to(&e.fact),
                    set_by(&e.fact)
                ),
                (Some(o), None) => {
                    format!("Promised by {}{}{}", date_label(o.on), to(o), set_by(o))
                }
                (None, None) => format!("Promised by {}", date_label(p.on)),
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
        "Promised by {}{}{}{}",
        date_label(p.fact.on),
        to(&p.fact),
        set_by(&p.fact),
        if p.past { " · past" } else { "" }
    )
}

/// A row of a sprint's promise list after the story's key and title (the web's `sprintRowParts`): the recipient, then the
/// chip's words, which on a sprint row never carry a period.
pub fn sprint_row_text(p: &SprintPromiseDto) -> String {
    let d = day_label(p.on);
    let base = match state_words(p.state, p.days) {
        Some(w) => format!("{} · {d}", capital(&w)),
        None => format!("Promised by {d}"),
    };
    let base = match (&p.epic_key, p.holder) {
        (Some(e), PromiseHolder::Epic) => format!("{base} · via {e}"),
        _ => base,
    };
    match p.to.as_deref() {
        Some(t) if !t.is_empty() => format!("to {t} · {base}"),
        _ => base,
    }
}

/// A sprint's promise list heading (the web's `promisesHeading`): "Promises due 11–24 Nov" from the day after the
/// previous sprint's planned end (`after`, from the server) to this one's end, "Promises due 28 Oct–10 Nov" across
/// months, "Promises due up to 27 Oct" for the first sprint. Calendar arithmetic on the server's dates only.
pub fn promises_heading(after: Option<NaiveDate>, ends_on: NaiveDate) -> String {
    let end = day_label(ends_on);
    let Some(from) = after.and_then(|a| a.succ_opt()) else {
        return format!("Promises due up to {end}");
    };
    let from = if from.format("%Y-%m").to_string() == ends_on.format("%Y-%m").to_string() {
        from.format("%-d").to_string()
    } else {
        day_label(from)
    };
    format!("Promises due {from}–{end}")
}

/// A payload field as a non-empty string.
fn field<'a>(v: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    v.get(name)
        .and_then(|x| x.as_str())
        .filter(|x| !x.is_empty())
}

/// An ISO date as a line gives it ("20 Nov 2026"); anything else unchanged (the web's `dateLabel`).
fn payload_date(iso: &str) -> String {
    iso.parse::<NaiveDate>()
        .map(date_label)
        .unwrap_or_else(|_| iso.to_string())
}

/// A promise activity entry's words, as the web's `actionLabel` gives them (`web/src/lib/format.ts`): a `promised`
/// entry's chosen by comparing `on` and `to` with `previous`, a `promise_cleared` entry's fixed. `key` names the item
/// where the words are not already about it. `None` for every other action.
pub fn activity_words(
    action: &str,
    payload: &serde_json::Value,
    key: Option<&str>,
) -> Option<String> {
    match action {
        "promise_cleared" => Some(match key {
            Some(k) => format!("cleared the promise on {k}"),
            None => "cleared the promise".into(),
        }),
        "promised" => Some(promised_words(payload, key)),
        _ => None,
    }
}

fn promised_words(payload: &serde_json::Value, key: Option<&str>) -> String {
    let Some(on) = field(payload, "on") else {
        return match key {
            Some(k) => format!("promised {k}"),
            None => "promised".into(),
        };
    };
    let to = field(payload, "to");
    let promise = key.map_or("the promise".into(), |k| format!("the promise on {k}"));
    let recipient = key.map_or("the promise's recipient".into(), |k| {
        format!("the recipient of the promise on {k}")
    });
    let fresh = format!(
        "promised {}by {}{}",
        key.map(|k| format!("{k} ")).unwrap_or_default(),
        payload_date(on),
        to.map(|t| format!(" to {t}")).unwrap_or_default()
    );
    let prev = match payload.get("previous") {
        Some(p) if p.is_object() => p,
        _ => return fresh,
    };
    let (prev_on, prev_to) = (field(prev, "on"), field(prev, "to"));
    if Some(on) != prev_on {
        let change = format!("changed {promise} to {}", payload_date(on));
        return match to {
            _ if to == prev_to => change,
            None => format!("{change} and removed its recipient"),
            Some(t) => format!("{change} and its recipient to {t}"),
        };
    }
    match to {
        _ if to == prev_to => fresh,
        None => format!("removed {recipient}"),
        Some(t) => format!("changed {recipient} to {t}"),
    }
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
    struct ActivityRow {
        label: String,
        action: String,
        payload: serde_json::Value,
        plain: String,
        keyed: String,
    }

    #[derive(Deserialize)]
    struct HeadingRow {
        label: String,
        after: Option<NaiveDate>,
        ends_on: NaiveDate,
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
        sprint_row: Vec<Row<SprintPromiseDto>>,
        activity: Vec<ActivityRow>,
        promises_heading: Vec<HeadingRow>,
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
        let rows = words().epic_chip;
        assert!(!rows.is_empty());
        for r in rows {
            assert_eq!(epic_chip_text(&r.promise), r.text, "{}", r.label);
        }
    }

    #[test]
    fn line_text_rows() {
        let rows = words().line;
        assert!(!rows.is_empty());
        for r in rows {
            assert_eq!(line_text(&r.promise), r.text, "{}", r.label);
        }
    }

    #[test]
    fn epic_line_text_rows() {
        let rows = words().epic_line;
        assert!(!rows.is_empty());
        for r in rows {
            assert_eq!(epic_line_text(&r.promise), r.text, "{}", r.label);
        }
    }

    #[test]
    fn sprint_row_text_rows() {
        let rows = words().sprint_row;
        assert!(!rows.is_empty());
        for r in rows {
            assert_eq!(sprint_row_text(&r.promise), r.text, "{}", r.label);
        }
    }

    /// The TUI ticker's words are the web's activity words, row for row.
    #[test]
    fn activity_words_rows() {
        let rows = words().activity;
        assert!(!rows.is_empty());
        for r in rows {
            let words = |key| activity_words(&r.action, &r.payload, key);
            assert_eq!(
                words(None).as_deref(),
                Some(r.plain.as_str()),
                "{}",
                r.label
            );
            assert_eq!(
                words(Some("ABC-12")).as_deref(),
                Some(r.keyed.as_str()),
                "{}",
                r.label
            );
        }
        for a in ["moved", "goal_linked", "urgent_marked"] {
            assert_eq!(
                activity_words(a, &serde_json::Value::Null, Some("ABC-12")),
                None
            );
        }
    }

    /// The fixture's `promisesHeading` rows, which the web's `sprint.test.ts` reads too.
    #[test]
    fn promises_heading_runs_from_the_day_after_the_previous_end() {
        let rows = words().promises_heading;
        assert!(!rows.is_empty());
        for r in rows {
            assert_eq!(promises_heading(r.after, r.ends_on), r.text, "{}", r.label);
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
        assert_eq!(date_label(date("2026-01-02")), "2 Jan 2026");
    }
}
