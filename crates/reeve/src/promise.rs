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

/// The recipient of the promise that sets a story's date: its own, or its epic's.
pub fn holder_to(p: &StoryPromiseDto) -> Option<&str> {
    let fact = match p.holder {
        PromiseHolder::Own => p.own.as_ref(),
        PromiseHolder::Epic => p.epic.as_ref().map(|e| &e.fact),
    };
    fact.and_then(|f| f.to.as_deref()).filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    //! Row for row the tables in `web/src/lib/promise.test.ts`.
    use super::*;
    use reeve_protocol::EpicPromiseRefDto;

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn fact_by(on: &str, to: Option<&str>, by: &str) -> PromiseFactDto {
        PromiseFactDto {
            on: date(on),
            by: by.into(),
            at: "2026-11-01T09:00:00Z".parse().unwrap(),
            to: to.map(Into::into),
        }
    }

    fn fact(on: &str) -> PromiseFactDto {
        fact_by(on, Some("Acme"), "Dana")
    }

    fn epic_ref(on: &str) -> EpicPromiseRefDto {
        EpicPromiseRefDto {
            key: "ABC-7".parse().unwrap(),
            fact: fact(on),
        }
    }

    /// The web's `sp()`: an own open promise on 20 Nov, due later.
    fn sp() -> StoryPromiseDto {
        StoryPromiseDto {
            own: Some(fact("2026-11-20")),
            epic: None,
            on: date("2026-11-20"),
            holder: PromiseHolder::Own,
            state: Some(PromiseState::Open),
            due: Some(PromiseDue::Later),
            days: None,
        }
    }

    fn with(
        state: Option<PromiseState>,
        due: Option<PromiseDue>,
        days: Option<u32>,
    ) -> StoryPromiseDto {
        StoryPromiseDto {
            state,
            due,
            days,
            ..sp()
        }
    }

    use PromiseDue::*;
    use PromiseState::*;

    #[test]
    fn chip_text_rows() {
        let rows: [(&str, StoryPromiseDto, &str); 10] = [
            (
                "open, this sprint",
                with(Some(Open), Some(ThisSprint), None),
                "Promised 20 Nov · this sprint",
            ),
            (
                "open, next sprint",
                with(Some(Open), Some(NextSprint), None),
                "Promised 20 Nov · next sprint",
            ),
            (
                "open, later",
                with(Some(Open), Some(Later), None),
                "Promised 20 Nov",
            ),
            (
                "overdue",
                with(Some(Overdue), None, Some(3)),
                "Overdue 3 d · 20 Nov",
            ),
            (
                "overdue, no days",
                with(Some(Overdue), None, None),
                "Overdue · 20 Nov",
            ),
            ("kept", with(Some(Kept), None, None), "Kept · 20 Nov"),
            (
                "late",
                with(Some(Late), None, Some(3)),
                "Done 3 d late · 20 Nov",
            ),
            (
                "late, no days",
                with(Some(Late), None, None),
                "Done late · 20 Nov",
            ),
            (
                "dropped",
                with(Some(Dropped), None, None),
                "Dropped · 20 Nov",
            ),
            ("state null", with(None, None, None), "Promised 20 Nov"),
        ];
        for (label, p, want) in rows {
            assert_eq!(chip_text(&p, true), want, "{label}");
        }
    }

    fn inherited() -> StoryPromiseDto {
        StoryPromiseDto {
            own: None,
            epic: Some(epic_ref("2026-11-20")),
            holder: PromiseHolder::Epic,
            due: Some(ThisSprint),
            ..sp()
        }
    }

    #[test]
    fn chip_names_the_epic_when_the_epic_holds_the_date_unless_asked_not_to() {
        assert_eq!(
            chip_text(&inherited(), true),
            "Promised 20 Nov · this sprint · via ABC-7"
        );
        assert_eq!(
            chip_text(&inherited(), false),
            "Promised 20 Nov · this sprint"
        );
    }

    #[test]
    fn epic_chip_text_rows() {
        let ep = |due, past| EpicPromiseDto {
            fact: fact("2026-11-20"),
            due,
            past,
        };
        let rows = [
            (ep(Some(ThisSprint), false), "Promised 20 Nov · this sprint"),
            (ep(Some(NextSprint), false), "Promised 20 Nov · next sprint"),
            (ep(Some(Later), false), "Promised 20 Nov"),
            (ep(None, true), "Past 20 Nov"),
        ];
        for (p, want) in rows {
            assert_eq!(epic_chip_text(&p), want);
        }
    }

    #[test]
    fn line_text_rows() {
        let both_epic_earlier = StoryPromiseDto {
            own: Some(fact_by("2026-11-30", Some("Beta"), "Dana")),
            epic: Some(epic_ref("2026-11-18")),
            on: date("2026-11-18"),
            holder: PromiseHolder::Epic,
            ..sp()
        };
        let both_own_earlier = StoryPromiseDto {
            own: Some(fact_by("2026-11-18", Some("Beta"), "Dana")),
            epic: Some(epic_ref("2026-11-30")),
            on: date("2026-11-18"),
            holder: PromiseHolder::Own,
            ..sp()
        };
        let rows: Vec<(&str, StoryPromiseDto, &str)> = vec![
            ("own only", sp(), "Promised 20 Nov to Acme · set by Dana"),
            (
                "own only, no recipient",
                StoryPromiseDto {
                    own: Some(fact_by("2026-11-20", None, "Dana")),
                    ..sp()
                },
                "Promised 20 Nov · set by Dana",
            ),
            (
                "epic only",
                StoryPromiseDto {
                    own: None,
                    epic: Some(epic_ref("2026-11-20")),
                    holder: PromiseHolder::Epic,
                    ..sp()
                },
                "Promised 20 Nov via epic ABC-7 to Acme · set by Dana",
            ),
            (
                "both, epic earlier",
                both_epic_earlier,
                "Promised 30 Nov to Beta. Epic ABC-7 is promised 18 Nov; the earlier date counts.",
            ),
            (
                "both, own earlier",
                both_own_earlier,
                "Promised 18 Nov to Beta. Epic ABC-7 is promised 30 Nov.",
            ),
            (
                "overdue",
                with(Some(Overdue), None, Some(3)),
                "Promised 20 Nov to Acme · set by Dana · overdue 3 d",
            ),
            (
                "kept",
                with(Some(Kept), None, None),
                "Promised 20 Nov to Acme · set by Dana · kept",
            ),
            (
                "late",
                with(Some(Late), None, Some(3)),
                "Promised 20 Nov to Acme · set by Dana · done 3 d late",
            ),
            (
                "dropped",
                with(Some(Dropped), None, None),
                "Promised 20 Nov to Acme · set by Dana · dropped",
            ),
            (
                "overdue, no days",
                with(Some(Overdue), None, None),
                "Promised 20 Nov to Acme · set by Dana · overdue",
            ),
            (
                "late, no days",
                with(Some(Late), None, None),
                "Promised 20 Nov to Acme · set by Dana · done late",
            ),
        ];
        for (label, p, want) in rows {
            assert_eq!(line_text(&p), want, "{label}");
        }
    }

    /// With two promises the line already ends in a sentence, so the state is a sentence of its own.
    #[test]
    fn line_text_with_both_promises_states_the_state_as_a_sentence() {
        let both = |state, due, days| StoryPromiseDto {
            own: Some(fact_by("2026-11-30", Some("Beta"), "Dana")),
            epic: Some(epic_ref("2026-11-18")),
            on: date("2026-11-18"),
            holder: PromiseHolder::Epic,
            state,
            due,
            days,
        };
        let lead =
            "Promised 30 Nov to Beta. Epic ABC-7 is promised 18 Nov; the earlier date counts.";
        let rows = [
            (
                "overdue",
                both(Some(Overdue), None, Some(3)),
                format!("{lead} Overdue 3 d."),
            ),
            (
                "overdue, no days",
                both(Some(Overdue), None, None),
                format!("{lead} Overdue."),
            ),
            (
                "kept",
                both(Some(Kept), None, None),
                format!("{lead} Kept."),
            ),
            (
                "late",
                both(Some(Late), None, Some(3)),
                format!("{lead} Done 3 d late."),
            ),
            (
                "late, no days",
                both(Some(Late), None, None),
                format!("{lead} Done late."),
            ),
            (
                "dropped",
                both(Some(Dropped), None, None),
                format!("{lead} Dropped."),
            ),
            (
                "open",
                both(Some(Open), Some(Later), None),
                lead.to_string(),
            ),
        ];
        for (label, p, want) in rows {
            assert_eq!(line_text(&p), want, "{label}");
        }
    }

    #[test]
    fn line_text_shortens_an_email_to_its_local_part() {
        let p = StoryPromiseDto {
            own: Some(fact_by("2026-11-20", Some("Acme"), "dana@example.com")),
            ..sp()
        };
        assert_eq!(line_text(&p), "Promised 20 Nov to Acme · set by dana");
    }

    #[test]
    fn epic_line_text_names_the_recipient_and_the_setter_and_says_past_once_the_date_is() {
        let ep = |due, past| EpicPromiseDto {
            fact: fact("2026-11-20"),
            due,
            past,
        };
        assert_eq!(
            epic_line_text(&ep(Some(Later), false)),
            "Promised 20 Nov to Acme · set by Dana"
        );
        assert_eq!(
            epic_line_text(&ep(None, true)),
            "Promised 20 Nov to Acme · set by Dana · past"
        );
    }

    /// The web's `STATE_WORDS` table, with and without a day count.
    #[test]
    fn state_words_is_the_one_table() {
        let rows = [
            (Overdue, "overdue 3 d", "overdue"),
            (Kept, "kept", "kept"),
            (Late, "done 3 d late", "done late"),
            (Dropped, "dropped", "dropped"),
        ];
        for (s, with_days, without) in rows {
            assert_eq!(state_words(s, Some(3)).as_deref(), Some(with_days));
            assert_eq!(state_words(s, None).as_deref(), Some(without));
        }
        assert_eq!(state_words(Open, Some(3)), None);
    }

    #[test]
    fn day_labels_have_no_leading_zero() {
        assert_eq!(day_label(date("2026-11-02")), "2 Nov");
        assert_eq!(day_label(date("2026-01-20")), "20 Jan");
    }

    #[test]
    fn the_recipient_is_the_holders() {
        assert_eq!(holder_to(&sp()), Some("Acme"));
        let mut p = inherited();
        p.own = Some(fact_by("2026-11-30", Some("Beta"), "Dana"));
        assert_eq!(holder_to(&p), Some("Acme"));
        p.epic.as_mut().unwrap().fact.to = None;
        assert_eq!(holder_to(&p), None);
    }
}
