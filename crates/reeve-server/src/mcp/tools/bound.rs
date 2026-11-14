//! How a tool bounds what it answers: the planner limits neither text nor most lists, and a client cuts a tool
//! result that is too long (claude.ai at about 150 000 characters, and the result is sent twice, structured and as
//! text). Every tool uses these; which fields it walks is the tool's own. `get_story` takes its context apart
//! field by field, with no `..`, so that a field added to the API's type does not compile until someone decides
//! how to bound it; that rule is `get_story`'s only. The other tools change their answer in place, and the last
//! guard in `call_tool` (`tools::too_large`) is what holds for a field nobody walked.

/// An item's own main text (a story's description when the story is what was asked for) is cut at this many
/// characters.
pub(super) const MAX_OWN_TEXT: usize = 10_000;
/// Every other text (a criterion, a brief, a blocker's description, a comment's body) is cut at this many.
/// Together with `MAX_RECENT` this keeps a typical answer far below a client's limit; it is not a bound on the
/// whole answer, because the number of criteria, blockers, decisions, goals and attachments is not capped.
pub(super) const MAX_OTHER_TEXT: usize = 2_000;
/// How many of a list that grows without limit (a story's comments) are sent: the most recent ones.
pub(super) const MAX_RECENT: usize = 20;
/// The budget for a list's rows: their serialized size in bytes. Twice this, with the envelope, stays well under a
/// client's limit.
pub(super) const MAX_ANSWER: usize = 60_000;
/// How much of a caller's own argument an error message repeats.
pub(super) const MAX_ECHO: usize = 200;

/// Cuts `text` to its first `limit` characters when it has more, and appends `[cut: <n> more characters]`, n being
/// the number of characters removed. Characters, not bytes: the cut is always on a character boundary.
pub(super) fn cut(text: &mut String, limit: usize) {
    // The byte offset of the character after the last one kept; `None` when the text is within the limit.
    if let Some((at, _)) = text.char_indices().nth(limit) {
        let removed = text[at..].chars().count();
        text.truncate(at);
        text.push_str(&format!("[cut: {removed} more characters]"));
    }
}

/// A caller's argument as an error message repeats it: its first `MAX_ECHO` characters, and how many were left out.
pub(super) fn echo(text: &str) -> String {
    let mut text = text.to_string();
    cut(&mut text, MAX_ECHO);
    text
}

/// Keeps the last `keep` of `items`, which are oldest first, and returns how many were left out (0 when all fit).
pub(super) fn most_recent<T>(items: &mut Vec<T>, keep: usize) -> usize {
    let omitted = items.len().saturating_sub(keep);
    items.drain(..omitted);
    omitted
}

/// Drops rows from the end while the rows, serialized as one JSON list, are over `MAX_ANSWER` bytes. Returns
/// `omitted` (the rows a cap already left out) plus the rows dropped here. A list tool calls it after its row cap.
/// It measures each row's own serialization; the form that is sent goes through `serde_json::to_value` and may
/// differ by a few bytes a row (an `f32` is widened), which the gap between `MAX_ANSWER` and `tools::MAX_SENT`
/// covers.
pub(super) fn fit<T: serde::Serialize>(rows: &mut Vec<T>, omitted: usize) -> usize {
    // The size of the list up to and including each row: the opening bracket, then each row and the comma or
    // closing bracket after it.
    let mut size = 1;
    let mut keep = rows.len();
    for (n, row) in rows.iter().enumerate() {
        // A row that does not serialize counts as nothing here; serializing the answer reports it.
        size += serde_json::to_string(row).map_or(0, |json| json.len()) + 1;
        if size > MAX_ANSWER {
            keep = n;
            break;
        }
    }
    let dropped = rows.len() - keep;
    rows.truncate(keep);
    omitted + dropped
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: [usize; 2] = [MAX_OWN_TEXT, MAX_OTHER_TEXT];

    fn cut_of(text: &str, limit: usize) -> String {
        let mut text = text.to_string();
        cut(&mut text, limit);
        text
    }

    #[test]
    fn text_within_the_limit_is_unchanged() {
        for limit in LIMITS {
            for text in [
                String::new(),
                "short".to_string(),
                "a".repeat(limit),
                "世".repeat(limit),
            ] {
                assert_eq!(
                    cut_of(&text, limit),
                    text,
                    "{} characters, limit {limit}",
                    text.chars().count()
                );
            }
        }
    }

    #[test]
    fn one_character_over_the_limit_is_cut_and_counted() {
        for limit in LIMITS {
            let text = format!("{}b", "a".repeat(limit));
            assert_eq!(
                cut_of(&text, limit),
                format!("{}[cut: 1 more characters]", "a".repeat(limit))
            );
        }
        // The limit is the argument, not a constant: a text between the two limits is cut by one and not the other.
        let between = "a".repeat(MAX_OTHER_TEXT + 1);
        assert_eq!(cut_of(&between, MAX_OWN_TEXT), between);
        assert_ne!(cut_of(&between, MAX_OTHER_TEXT), between);
    }

    #[test]
    fn multi_byte_text_is_cut_on_a_character_boundary_and_counted_in_characters() {
        // Three-, four- and two-byte characters, 9 bytes a unit. 10 000 = 9 x 1111 + 1 and 2 000 = 9 x 222 + 2:
        // byte offset `limit` falls inside the three-byte `世` for both limits.
        let unit = "世🙂å";
        for limit in LIMITS {
            let text = unit.repeat(limit / 3 + 4);
            assert!(
                !text.is_char_boundary(limit),
                "the fixture has a character across byte {limit}"
            );
            let over = text.chars().count() - limit;
            let kept: String = text.chars().take(limit).collect();
            assert_eq!(
                cut_of(&text, limit),
                format!("{kept}[cut: {over} more characters]")
            );
        }
    }

    #[test]
    fn an_echoed_argument_is_at_most_two_hundred_characters() {
        assert_eq!(echo("TST-12"), "TST-12");
        assert_eq!(echo(&"å".repeat(MAX_ECHO)), "å".repeat(MAX_ECHO));
        assert_eq!(
            echo(&"å".repeat(MAX_ECHO + 5)),
            format!("{}[cut: 5 more characters]", "å".repeat(MAX_ECHO))
        );
    }

    /// Rows of a known size: a string of `n` characters serializes to `n + 2` bytes, and a list of them to the sum
    /// plus a comma between rows and the two brackets.
    #[test]
    fn rows_are_dropped_from_the_end_until_the_list_fits_the_budget() {
        // Fifty-nine rows of 1 000 bytes with their commas, and a last row that brings the list to the budget.
        let mut at_the_budget: Vec<String> = (0..59).map(|n| format!("{n:0>997}")).collect();
        at_the_budget.push("z".repeat(996));
        assert_eq!(
            serde_json::to_string(&at_the_budget).unwrap().len(),
            MAX_ANSWER,
            "the control: exactly the budget"
        );
        let whole = at_the_budget.clone();
        assert_eq!(fit(&mut at_the_budget, 0), 0);
        assert_eq!(at_the_budget, whole);

        // One byte more: the last row is dropped and counted, the rest keep their order.
        let mut one_over = whole.clone();
        one_over[59].push('z');
        assert_eq!(fit(&mut one_over, 0), 1);
        assert_eq!(one_over, whole[..59]);

        // Rows left out before (by a row cap) are counted with the ones dropped here.
        let mut after_a_cap = whole.clone();
        after_a_cap[59].push('z');
        after_a_cap.push("more".into());
        assert_eq!(fit(&mut after_a_cap, 7), 9);
        assert_eq!(after_a_cap, whole[..59]);

        let mut none: Vec<String> = Vec::new();
        assert_eq!(fit(&mut none, 0), 0);
    }

    #[test]
    fn the_most_recent_are_kept_and_the_rest_counted() {
        // Exactly the bound: all kept, none omitted.
        let mut at_the_bound: Vec<usize> = (1..=MAX_RECENT).collect();
        assert_eq!(most_recent(&mut at_the_bound, MAX_RECENT), 0);
        assert_eq!(at_the_bound, (1..=MAX_RECENT).collect::<Vec<_>>());
        // One more: the oldest (the first) is dropped and counted, the rest keep their order.
        let mut one_over: Vec<usize> = (1..=MAX_RECENT + 1).collect();
        assert_eq!(most_recent(&mut one_over, MAX_RECENT), 1);
        assert_eq!(one_over, (2..=MAX_RECENT + 1).collect::<Vec<_>>());
        let mut none: Vec<usize> = Vec::new();
        assert_eq!(most_recent(&mut none, MAX_RECENT), 0);
    }
}
