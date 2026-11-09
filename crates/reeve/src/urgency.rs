//! Urgency in words, for the CLI and the TUI (kb/ADR-0017): a port of `web/src/lib/urgency.ts` (`daysAgo`, `expiry`,
//! `ownMarkText`, `urgentLine`, `via`) and of the expired and done lines in `UrgentLine.svelte` and `StoryDrawer.svelte`,
//! with the same words and rounding, lowercase as the CLI is. The server decides what is urgent (`urgent_sources`) and
//! whether a mark is active; this only words its answer.
// Used from the CLI and TUI tasks that follow.
#![allow(dead_code)]
use crate::glyphs::Level;
use chrono::{DateTime, Utc};
use reeve_protocol::{ItemRefDto, Key, UrgentMarkDto};

const DAY_MS: f64 = 86_400_000.0;

/// An item is urgent exactly when the server names a source for it (kb/ADR-0017 R3); never decided from the mark.
pub fn is_urgent(sources: &[ItemRefDto]) -> bool {
    !sources.is_empty()
}

/// The marker after a key in one-line renderings: `" !"` for an urgent item, `""` otherwise.
pub fn urgent_marker(sources: &[ItemRefDto]) -> &'static str {
    if is_urgent(sources) { " !" } else { "" }
}

/// "1 mark", "2 marks".
pub fn marks(n: usize) -> String {
    format!("{n} {}", if n == 1 { "mark" } else { "marks" })
}

/// "dana" from "dana@example.com".
pub fn local_part(email: &str) -> &str {
    email.split('@').next().unwrap_or(email)
}

/// "today" or "N d ago": whole days between two instants, never negative.
pub fn days_ago(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let d = ((now - at).num_milliseconds() as f64 / DAY_MS)
        .floor()
        .max(0.0) as i64;
    if d == 0 {
        "today".into()
    } else {
        format!("{d} d ago")
    }
}

/// "expires in N d", "expires today" or "expired N d ago", from the server's `expires_at` and `active`.
pub fn expiry(m: &UrgentMarkDto, now: DateTime<Utc>) -> String {
    if m.active {
        let d = ((m.expires_at - now).num_milliseconds() as f64 / DAY_MS).ceil() as i64;
        if d <= 1 {
            "expires today".into()
        } else {
            format!("expires in {d} d")
        }
    } else {
        format!("expired {}", days_ago(m.expires_at, now))
    }
}

/// "marked by dana, 2 d ago · 'reason' · expires in 5 d".
pub fn own_mark_text(m: &UrgentMarkDto, now: DateTime<Utc>) -> String {
    let mut parts = vec![format!(
        "marked by {}, {}",
        local_part(&m.by),
        days_ago(m.at, now)
    )];
    if let Some(r) = &m.reason {
        parts.push(format!("'{r}'"));
    }
    parts.push(expiry(m, now));
    parts.join(" · ")
}

/// The item's urgency lines: the urgent line when sources exist, then one line for an own mark the sources leave out
/// (expired, or a done story's); empty when there is nothing to say.
pub fn urgent_lines(
    own: Option<&UrgentMarkDto>,
    sources: &[ItemRefDto],
    key: &Key,
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut out = Vec::new();
    let own_counts = sources.iter().any(|s| &s.key == key);
    if is_urgent(sources) {
        let mut parts = vec!["urgent".to_string()];
        if let (true, Some(m)) = (own_counts, own) {
            parts.push(own_mark_text(m, now));
        }
        let others: Vec<_> = sources.iter().filter(|s| &s.key != key).collect();
        if let Some(first) = others.first() {
            let more = if others.len() > 1 {
                format!(" and {} more", others.len() - 1)
            } else {
                String::new()
            };
            parts.push(format!(
                "via {} {}{more}",
                Level::of_item(first.item).name(),
                first.key
            ));
        }
        out.push(parts.join(" · "));
    }
    if let (false, Some(m)) = (own_counts, own) {
        if !m.active {
            out.push(format!(
                "urgent mark expired {}",
                days_ago(m.expires_at, now)
            ));
        } else if m.done {
            out.push(format!("urgent mark · done · {}", expiry(m, now)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use reeve_protocol::ItemType;

    fn t(d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap()
    }
    fn k(s: &str) -> Key {
        s.parse().unwrap()
    }
    fn r(s: &str, item: ItemType) -> ItemRefDto {
        ItemRefDto { key: k(s), item }
    }
    fn mark(
        at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        active: bool,
        done: bool,
        reason: Option<&str>,
    ) -> UrgentMarkDto {
        UrgentMarkDto {
            at,
            by: "dana@example.com".into(),
            reason: reason.map(Into::into),
            expires_at,
            active,
            done,
        }
    }

    #[test]
    fn days_ago_floors_whole_days_and_never_goes_negative() {
        let now = t(10, 12);
        assert_eq!(days_ago(t(10, 1), now), "today");
        assert_eq!(days_ago(t(9, 13), now), "today");
        assert_eq!(days_ago(t(9, 12), now), "1 d ago");
        assert_eq!(days_ago(t(7, 11), now), "3 d ago");
        assert_eq!(days_ago(t(11, 12), now), "today");
    }

    #[test]
    fn expiry_ceils_for_active_marks_and_says_today_at_one_day_or_less() {
        let now = t(10, 12);
        let at = t(1, 0);
        assert_eq!(
            expiry(&mark(at, t(10, 13), true, false, None), now),
            "expires today"
        );
        assert_eq!(
            expiry(&mark(at, t(11, 12), true, false, None), now),
            "expires today"
        );
        assert_eq!(
            expiry(&mark(at, t(11, 13), true, false, None), now),
            "expires in 2 d"
        );
        assert_eq!(
            expiry(&mark(at, t(7, 12), false, false, None), now),
            "expired 3 d ago"
        );
    }

    #[test]
    fn own_mark_text_matches_the_web() {
        let now = t(10, 12);
        let m = mark(t(8, 12), t(20, 12), true, false, Some("Customer X blocked"));
        assert_eq!(
            own_mark_text(&m, now),
            "marked by dana, 2 d ago · 'Customer X blocked' · expires in 10 d"
        );
        let m = mark(t(10, 11), t(20, 12), true, false, None);
        assert_eq!(
            own_mark_text(&m, now),
            "marked by dana, today · expires in 10 d"
        );
    }

    #[test]
    fn an_own_active_mark_is_not_also_named_as_via() {
        let now = t(10, 12);
        let m = mark(t(8, 12), t(20, 12), true, false, None);
        assert_eq!(
            urgent_lines(Some(&m), &[r("ABC-1", ItemType::Story)], &k("ABC-1"), now),
            vec!["urgent · marked by dana, 2 d ago · expires in 10 d"]
        );
    }

    #[test]
    fn sources_elsewhere_read_via_the_first_and_count_the_rest() {
        let now = t(10, 12);
        assert_eq!(
            urgent_lines(None, &[r("ABC-40", ItemType::Epic)], &k("ABC-1"), now),
            vec!["urgent · via epic ABC-40"]
        );
        assert_eq!(
            urgent_lines(
                None,
                &[
                    r("ABC-40", ItemType::Epic),
                    r("ABC-2", ItemType::Initiative),
                    r("ABC-7", ItemType::Story)
                ],
                &k("ABC-1"),
                now
            ),
            vec!["urgent · via epic ABC-40 and 2 more"]
        );
        let m = mark(t(8, 12), t(20, 12), true, false, None);
        assert_eq!(
            urgent_lines(
                Some(&m),
                &[r("ABC-1", ItemType::Story), r("ABC-40", ItemType::Epic)],
                &k("ABC-1"),
                now
            ),
            vec!["urgent · marked by dana, 2 d ago · expires in 10 d · via epic ABC-40"]
        );
    }

    #[test]
    fn an_expired_own_mark_is_a_second_line_and_does_not_make_the_item_urgent() {
        let now = t(10, 12);
        let m = mark(t(1, 12), t(7, 12), false, false, None);
        assert_eq!(
            urgent_lines(Some(&m), &[], &k("ABC-1"), now),
            vec!["urgent mark expired 3 d ago"]
        );
        assert_eq!(
            urgent_lines(
                Some(&m),
                &[r("ABC-2", ItemType::Initiative)],
                &k("ABC-1"),
                now
            ),
            vec![
                "urgent · via initiative ABC-2",
                "urgent mark expired 3 d ago"
            ]
        );
    }

    #[test]
    fn a_done_storys_active_mark_makes_nothing_urgent() {
        let now = t(10, 12);
        let m = mark(t(8, 12), t(20, 12), true, true, None);
        assert!(!is_urgent(&[]));
        assert_eq!(urgent_marker(&[]), "");
        assert_eq!(
            urgent_lines(Some(&m), &[], &k("ABC-1"), now),
            vec!["urgent mark · done · expires in 10 d"]
        );
    }

    #[test]
    fn no_mark_and_no_sources_is_no_line_and_marks_pluralises() {
        assert!(urgent_lines(None, &[], &k("ABC-1"), t(10, 12)).is_empty());
        assert_eq!(urgent_marker(&[r("ABC-1", ItemType::Story)]), " !");
        assert_eq!(
            (marks(1), marks(2), marks(0)),
            ("1 mark".into(), "2 marks".into(), "0 marks".into())
        );
    }
}
