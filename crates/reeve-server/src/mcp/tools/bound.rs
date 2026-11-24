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
/// How many rows of a list of summaries that is not paged (epics, initiatives) are sent: the first ones, in the
/// list's own order.
pub(super) const MAX_LIST: usize = 200;
/// How many rows of a list of whole items (required decisions) are sent: fewer than summaries, since each is larger.
pub(super) const MAX_WHOLE_LIST: usize = 50;
/// The budget for a list's rows: their serialized size in bytes. Twice this, with the envelope, stays well under a
/// client's limit.
pub(super) const MAX_ANSWER: usize = 60_000;

/// How many of an item's item links are sent: the first ones, in the list's own order (by name, `out` before `in`,
/// then the other item's key number).
pub(super) const MAX_LINKS: usize = 50;

/// A linked item's title is cut at this many bytes. Bytes, not characters: fifty titles of 200 four-byte
/// characters would be 40 000 bytes of one answer. Bytes of text, not of JSON: escaping can still inflate a cut
/// title, which is why the list is also fitted (`bound_links`).
pub(super) const MAX_LINK_TITLE_BYTES: usize = 200;

/// How many of an item's web links are sent: the first ones, which are the oldest.
pub(super) const MAX_WEB_LINKS: usize = 10;

/// The two bytes of an empty JSON list, `[]`. A baseline (see `room`) holds them for its list, so the list's rows
/// may take them back.
const EMPTY_LIST: usize = 2;

pub(super) use crate::text::{cut, echo};

/// Keeps the last `keep` of `items`, which are oldest first, and returns how many were left out (0 when all fit).
pub(super) fn most_recent<T>(items: &mut Vec<T>, keep: usize) -> usize {
    let omitted = items.len().saturating_sub(keep);
    items.drain(..omitted);
    omitted
}

/// Keeps the first `keep` of `items` and returns how many were left out (0 when all fit). `keep` is `MAX_LIST` for
/// summaries and `MAX_WHOLE_LIST` for whole items.
pub(super) fn capped<T>(items: &mut Vec<T>, keep: usize) -> usize {
    let omitted = items.len().saturating_sub(keep);
    items.truncate(keep);
    omitted
}

/// Cuts `text` to at most `limit` bytes, on a character boundary, when it is longer, and appends
/// `[cut: <n> more characters]`, n being the number of characters removed: `cut`'s marker, at a byte limit.
pub(super) fn cut_bytes(text: &mut String, limit: usize) {
    if text.len() <= limit {
        return;
    }
    // The last character boundary at or before the limit; byte 0 always is one.
    let mut at = limit;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let removed = text[at..].chars().count();
    text.truncate(at);
    text.push_str(&format!("[cut: {removed} more characters]"));
}

/// The size of `answer` as `call_tool` measures it: the bytes of its JSON after `tools::sent`'s `to_value` (which
/// widens an `f32`). 0 for an answer that does not serialize; sending it reports that.
pub(super) fn sent_bytes<T: serde::Serialize>(answer: &T) -> usize {
    serde_json::to_value(answer).map_or(0, |value| value.to_string().len())
}

/// A list as an answer carries it when none of its rows is sent, and how many rows that leaves out: empty, with
/// the list's length; or absent, with 0, for an item read without the list. A tool puts the two into its answer to
/// measure the answer's baseline (see `room`).
pub(super) fn emptied<T>(list: &Option<Vec<T>>) -> (Option<Vec<T>>, usize) {
    (
        list.as_ref().map(|_| Vec::new()),
        list.as_ref().map_or(0, Vec::len),
    )
}

/// The most bytes a list of links may take as JSON, brackets included, in an answer whose baseline is `baseline`
/// bytes. The baseline is the complete answer as `call_tool` measures it (`sent_bytes`) with the list empty and
/// every one of its rows counted as left out: the list's property name, its two brackets and its count are in it,
/// so only rows are added to it, and the count only gets shorter as rows are sent. The rows get what the baseline
/// leaves of `tools::MAX_SENT`, and the two bytes of the empty list they replace. A baseline at the limit or over
/// it leaves those two bytes: no row.
fn room(baseline: usize) -> usize {
    super::MAX_SENT.saturating_sub(baseline) + EMPTY_LIST
}

/// Bounds an item's item links in place: by count (the first `MAX_LINKS`), by text (each linked item's title cut at
/// `MAX_LINK_TITLE_BYTES`), then by serialized bytes (rows dropped from the end until the list fits
/// `room(baseline)`). `baseline` is `sent_bytes` of the tool's whole answer with this list `emptied` and its count
/// at the number of rows. So an answer whose baseline is within `tools::MAX_SENT` stays within it. Returns how many
/// rows the cap or the fit left out (0 when all are sent, and for an item read without its links).
pub(super) fn bound_links(
    links: &mut Option<Vec<reeve_protocol::ItemLinkDto>>,
    baseline: usize,
) -> usize {
    let Some(links) = links.as_mut() else {
        return 0;
    };
    let omitted = capped(links, MAX_LINKS);
    for link in links.iter_mut() {
        cut_bytes(&mut link.item.title, MAX_LINK_TITLE_BYTES);
    }
    // Last, on the rows as they will be sent: the caps bound neither the bytes of four-byte text nor what JSON
    // escaping adds.
    fit_within(links, omitted, room(baseline))
}

/// Bounds an item's web links in place: by count (the first `MAX_WEB_LINKS`), then by serialized bytes (rows
/// dropped from the end until the list fits `room(baseline)`). A row is sent whole or not at all: neither a title
/// (at most 200 characters by validation) nor a URL (at most 1 000; a cut URL would be a wrong URL) is cut.
/// `baseline` is `sent_bytes` of the tool's whole answer with this list `emptied` and its count at the number of
/// rows, as for `bound_links`. Returns how many rows the cap or the fit left out.
pub(super) fn bound_web_links(
    links: &mut Option<Vec<reeve_protocol::WebLinkDto>>,
    baseline: usize,
) -> usize {
    let Some(links) = links.as_mut() else {
        return 0;
    };
    let omitted = capped(links, MAX_WEB_LINKS);
    // Whole rows only: nothing here cuts a title or a URL.
    fit_within(links, omitted, room(baseline))
}

/// A list of summaries that is not paged: the first `MAX_LIST` rows, each as its summary, fitted to the answer
/// budget; and how many rows the cap or the budget left out.
pub(super) fn summaries<D, S: From<D> + serde::Serialize>(mut rows: Vec<D>) -> (Vec<S>, usize) {
    let omitted = capped(&mut rows, MAX_LIST);
    let mut rows: Vec<S> = rows.into_iter().map(S::from).collect();
    let omitted = fit(&mut rows, omitted);
    (rows, omitted)
}

/// Drops rows from the end while the rows, serialized as one JSON list, are over `MAX_ANSWER` bytes. Returns
/// `omitted` (the rows a cap already left out) plus the rows dropped here. A list tool calls it after its row cap.
/// It measures each row's own serialization; the form that is sent goes through `serde_json::to_value` and may
/// differ by a few bytes a row (an `f32` is widened), which the gap between `MAX_ANSWER` and `tools::MAX_SENT`
/// covers.
pub(super) fn fit<T: serde::Serialize>(rows: &mut Vec<T>, omitted: usize) -> usize {
    fit_within(rows, omitted, MAX_ANSWER)
}

/// `fit` against a budget the caller names: drops rows from the end while the rows, serialized as one JSON list,
/// are over `budget` bytes, and returns `omitted` plus the rows dropped here.
pub(super) fn fit_within<T: serde::Serialize>(
    rows: &mut Vec<T>,
    omitted: usize,
    budget: usize,
) -> usize {
    // The size of the list up to and including each row: the opening bracket, then each row and the comma or
    // closing bracket after it.
    let mut size = 1;
    let mut keep = rows.len();
    for (n, row) in rows.iter().enumerate() {
        // A row that does not serialize counts as nothing here; serializing the answer reports it.
        size += serde_json::to_string(row).map_or(0, |json| json.len()) + 1;
        if size > budget {
            keep = n;
            break;
        }
    }
    let dropped = rows.len() - keep;
    rows.truncate(keep);
    omitted + dropped
}

/// A goal's six texts are people's text of any length that comes along with a sprint or a story: each is cut at
/// `MAX_OTHER_TEXT`. Used wherever a tool answers a goal (`get_sprint`, `get_story`).
pub(super) fn cut_goal(goal: &mut reeve_protocol::GoalDto) {
    cut(&mut goal.outcome, MAX_OTHER_TEXT);
    cut(&mut goal.check, MAX_OTHER_TEXT);
    cut(&mut goal.notes, MAX_OTHER_TEXT);
    for note in [
        &mut goal.confidence_note,
        &mut goal.suggested_note,
        &mut goal.verdict_note,
    ]
    .into_iter()
    .flatten()
    {
        cut(note, MAX_OTHER_TEXT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The budget is bytes, not characters: forty rows of 1 000 two-byte characters are 40 121 characters, which
    /// would fit, and 80 121 bytes, which do not.
    #[test]
    fn a_list_within_the_budget_in_characters_but_not_in_bytes_is_fitted_by_bytes() {
        let mut rows: Vec<String> = vec!["å".repeat(1_000); 40];
        let json = serde_json::to_string(&rows).unwrap();
        assert!(
            json.chars().count() <= MAX_ANSWER && json.len() > MAX_ANSWER,
            "the control: {} characters, {} bytes",
            json.chars().count(),
            json.len()
        );
        // A row is 2 002 bytes and its comma or bracket: 1 + 29 x 2 003 = 58 088 fits, a thirtieth row does not.
        assert_eq!(fit(&mut rows, 0), 11);
        assert_eq!(rows.len(), 29);
        assert!(serde_json::to_string(&rows).unwrap().len() <= MAX_ANSWER);
    }

    #[test]
    fn the_first_are_kept_and_the_rest_counted() {
        // Exactly the cap: all kept, none left out.
        let mut at_the_cap: Vec<usize> = (1..=MAX_LIST).collect();
        assert_eq!(capped(&mut at_the_cap, MAX_LIST), 0);
        assert_eq!(at_the_cap.len(), MAX_LIST);
        // One more: the last is dropped and counted, the rest keep their order.
        let mut one_over: Vec<usize> = (1..=MAX_LIST + 1).collect();
        assert_eq!(capped(&mut one_over, MAX_LIST), 1);
        assert_eq!(one_over, (1..=MAX_LIST).collect::<Vec<_>>());
        let mut none: Vec<usize> = Vec::new();
        assert_eq!(capped(&mut none, MAX_LIST), 0);
    }

    /// The cap of 200 and the budget for a list of summaries, tested where it is cheap: rows in memory, not 201
    /// creates through HTTP.
    #[test]
    fn summaries_keep_the_first_rows_under_the_cap_and_the_budget_and_count_the_rest() {
        use reeve_protocol::{EpicDto, EpicSummary};
        fn epic(n: u64) -> EpicDto {
            serde_json::from_value(serde_json::json!({
                "key": format!("TST-{n}"), "title": format!("Epic {n}"), "initiative": null, "brief": "",
                "discovery_complete": null, "external_ref": null, "version": 1
            }))
            .unwrap()
        }
        let (at_the_cap, omitted): (Vec<EpicSummary>, _) =
            summaries((1..=MAX_LIST as u64).map(epic).collect());
        assert_eq!((at_the_cap.len(), omitted), (MAX_LIST, 0));
        let (one_over, omitted): (Vec<EpicSummary>, _) =
            summaries((1..=MAX_LIST as u64 + 1).map(epic).collect());
        assert_eq!((one_over.len(), omitted), (200, 1));
        assert_eq!(one_over[0].key.to_string(), "TST-1");
        assert_eq!(one_over[199].key.to_string(), "TST-200");

        // Rows too large for the budget (titles no planner would accept) are left out after the cap, and counted
        // with the capped one.
        let (wide, omitted): (Vec<EpicSummary>, _) = summaries(
            (1..=MAX_LIST as u64 + 1)
                .map(|n| {
                    let mut epic = epic(n);
                    epic.title = "t".repeat(400);
                    epic
                })
                .collect(),
        );
        assert!(omitted > 1, "{omitted}");
        assert_eq!(wide.len() + omitted, MAX_LIST + 1);
        assert_eq!(wide[0].key.to_string(), "TST-1");
        assert!(serde_json::to_string(&wide).unwrap().len() <= MAX_ANSWER);
    }

    /// All six of a goal's texts, in memory: each long one is cut, and a short or absent one is left as it is.
    #[test]
    fn each_of_a_goal_s_six_texts_is_cut_and_short_or_absent_ones_are_not() {
        let long = "å".repeat(MAX_OTHER_TEXT + 3);
        let cut_long = format!("{}[cut: 3 more characters]", "å".repeat(MAX_OTHER_TEXT));
        let goal = |outcome: &str,
                    check: &str,
                    notes: &str,
                    confidence: Option<&str>,
                    suggested: Option<&str>,
                    verdict: Option<&str>| {
            let mut goal: reeve_protocol::GoalDto = serde_json::from_value(serde_json::json!({
                "id": 1, "sprint_id": 1, "outcome": outcome, "check": check, "owner": "dan@example.com",
                "notes": notes, "confidence": null, "confidence_note": confidence, "confidence_at": null,
                "suggested_confidence": null, "suggested_note": suggested, "suggested_at": null,
                "suggested_by": null, "verdict": null, "verdict_note": verdict, "version": 1,
                "created_at": "2026-11-01T09:00:00Z", "links": []
            }))
            .unwrap();
            cut_goal(&mut goal);
            (
                goal.outcome,
                goal.check,
                goal.notes,
                goal.confidence_note,
                goal.suggested_note,
                goal.verdict_note,
            )
        };
        let l = Some(long.as_str());
        let c = Some(cut_long.clone());
        assert_eq!(
            goal(&long, &long, &long, l, l, l),
            (
                cut_long.clone(),
                cut_long.clone(),
                cut_long.clone(),
                c.clone(),
                c.clone(),
                c
            )
        );
        let none: Option<&str> = None;
        assert_eq!(
            goal(
                "Customers can book",
                "one did",
                "",
                Some("on track"),
                none,
                none
            ),
            (
                "Customers can book".to_string(),
                "one did".to_string(),
                String::new(),
                Some("on track".to_string()),
                None,
                None
            )
        );
    }

    fn item_link(n: usize, title: &str) -> reeve_protocol::ItemLinkDto {
        serde_json::from_value(serde_json::json!({
            "type": "relates_to", "direction": "out", "label": "relates to",
            "item": {"key": format!("TST-{n}"), "level": "initiative", "title": title}
        }))
        .unwrap()
    }

    #[test]
    fn a_text_is_cut_at_a_byte_limit_on_a_character_boundary() {
        let cut_of = |text: &str| {
            let mut text = text.to_string();
            cut_bytes(&mut text, MAX_LINK_TITLE_BYTES);
            text
        };
        // Within the limit in bytes: unchanged, exactly at the limit included.
        for whole in [
            String::new(),
            "short".to_string(),
            "a".repeat(200),
            "å".repeat(100),
        ] {
            assert_eq!(cut_of(&whole), whole, "{} bytes", whole.len());
        }
        // 200 two-byte characters are 400 bytes: the first 100 are kept and the other 100 counted.
        assert_eq!(
            cut_of(&"å".repeat(200)),
            format!("{}[cut: 100 more characters]", "å".repeat(100))
        );
        // Three-byte characters do not end on byte 200: 66 are kept (198 bytes), never half of the 67th.
        assert_eq!(
            cut_of(&"世".repeat(200)),
            format!("{}[cut: 134 more characters]", "世".repeat(66))
        );
        assert_eq!(
            cut_of(&"a".repeat(201)),
            format!("{}[cut: 1 more characters]", "a".repeat(200))
        );
    }

    /// A list is emptied to what the answer carries when none of its rows is sent: empty, never absent, with every
    /// row counted; a list that was not loaded stays absent.
    #[test]
    fn a_list_is_emptied_to_what_is_sent_when_none_of_its_rows_is() {
        assert_eq!(emptied(&Some(vec![1, 2, 3])), (Some(vec![]), 3));
        assert_eq!(
            emptied(&Some(Vec::<u8>::new())),
            (Some(vec![]), 0),
            "none stays none"
        );
        assert_eq!(
            emptied(&None::<Vec<u8>>),
            (None, 0),
            "not loaded stays not loaded"
        );
    }

    /// Rows of a known size, as in `rows_are_dropped_from_the_end_until_the_list_fits_the_budget`: three
    /// four-letter strings are the 22 bytes of `["aaaa","aaaa","aaaa"]`, two are 15, one is 8.
    #[test]
    fn rows_are_fitted_to_a_budget_the_caller_names() {
        let rows = || vec!["aaaa".to_string(); 3];
        assert_eq!(
            serde_json::to_string(&rows()).unwrap().len(),
            22,
            "the control"
        );
        // (rows kept, rows left out), one row having been left out by a cap before.
        let fitted = |budget: usize| {
            let mut rows = rows();
            let omitted = fit_within(&mut rows, 1, budget);
            (rows.len(), omitted)
        };
        assert_eq!(
            fitted(22),
            (3, 1),
            "exactly the budget: all kept, the earlier omission carried"
        );
        assert_eq!(
            fitted(21),
            (2, 2),
            "one byte short: the last row is dropped and counted"
        );
        assert_eq!(fitted(15), (2, 2));
        assert_eq!(fitted(14), (1, 3));
        assert_eq!(
            fitted(0),
            (0, 4),
            "no room at all: every row is counted, none is sent"
        );
    }

    /// The fit at its edge, in bytes. Three links of one size, and baselines chosen against the dispatch limit: a
    /// baseline already holds the list's two brackets, the first row adds its own bytes to it, and each later row
    /// its bytes and a comma. A baseline a few bytes under the limit, at it or over it leaves room for no row, and
    /// every row is counted.
    #[test]
    fn rows_take_exactly_what_the_baseline_leaves_of_one_answer() {
        let limit = crate::mcp::tools::MAX_SENT;
        let row = serde_json::to_string(&item_link(1, "Billing"))
            .unwrap()
            .len();
        assert!(
            row > 3,
            "the control: a row is more than the few bytes left below"
        );
        // (rows sent, rows left out) of three, for an answer of this baseline.
        let sent = |baseline: usize| {
            let mut links = Some((1..=3).map(|n| item_link(n, "Billing")).collect::<Vec<_>>());
            let omitted = bound_links(&mut links, baseline);
            (links.unwrap().len(), omitted)
        };
        assert_eq!(
            sent(limit - row),
            (1, 2),
            "exactly one row's bytes are left"
        );
        assert_eq!(sent(limit - row + 1), (0, 3), "one byte short of a row");
        assert_eq!(
            sent(limit - 2 * row - 1),
            (2, 1),
            "two rows and the comma between them"
        );
        assert_eq!(
            sent(limit - 2 * row),
            (1, 2),
            "two rows, and no byte for the comma"
        );
        assert_eq!(
            sent(limit - 3),
            (0, 3),
            "three bytes under the limit: nothing is sent, all are counted"
        );
        assert_eq!(sent(limit), (0, 3), "exactly at the limit");
        assert_eq!(
            sent(limit + 1),
            (0, 3),
            "over the limit: nothing to give, and no underflow"
        );
        assert_eq!(
            sent(0),
            (3, 0),
            "the control: with room, all three are sent"
        );
    }

    /// Spec section 9, the item-link half of the caps: an item with 200 item links whose titles are as long as a
    /// title may be (200 characters, here of four bytes each) is answered with the first 50, the rest counted. With
    /// nothing else in the answer (a baseline of 0) the fit drops none of the fifty.
    #[test]
    fn the_first_fifty_item_links_are_sent_with_their_titles_cut_and_the_rest_counted() {
        let title = "𝄞".repeat(200);
        assert_eq!(title.len(), 800, "the control: the longest title in bytes");
        let mut links = Some((1..=200).map(|n| item_link(n, &title)).collect::<Vec<_>>());
        assert!(
            serde_json::to_string(&links).unwrap().len() > crate::mcp::tools::MAX_SENT,
            "the control: unbounded, the list alone is over the limit of one answer"
        );
        assert_eq!(bound_links(&mut links, 0), 150);
        let sent = links.unwrap();
        assert_eq!(sent.len(), MAX_LINKS);
        assert_eq!(
            sent[0].item.key.to_string(),
            "TST-1",
            "the first are kept, in order"
        );
        assert_eq!(sent[49].item.key.to_string(), "TST-50");
        assert_eq!(
            sent[0].item.title,
            format!("{}[cut: 150 more characters]", "𝄞".repeat(50))
        );
        assert!(
            serde_json::to_string(&sent).unwrap().len() <= 20_000,
            "fifty bounded links of unescaped titles leave most of an answer for the item itself"
        );

        // At the cap and below: nothing left out, short titles untouched; no list, nothing to bound.
        let mut at_the_cap = Some(
            (1..=MAX_LINKS)
                .map(|n| item_link(n, "Billing"))
                .collect::<Vec<_>>(),
        );
        let whole = at_the_cap.clone();
        assert_eq!(bound_links(&mut at_the_cap, 0), 0);
        assert_eq!(at_the_cap, whole);
        assert_eq!(bound_links(&mut None, 0), 0);
    }

    /// The caps count rows and bytes of text, not bytes of JSON. A title of characters JSON escapes grows when it is
    /// serialized, after it was cut: 200 control characters are 200 bytes of text and 1 200 of JSON. Whether the
    /// planner would store such a title is not this bound's business; it must hold for whatever comes back. After
    /// the caps the list is fitted to what the answer's baseline leaves of one answer, from the end, and what the
    /// fit drops is counted with what the cap dropped.
    #[test]
    fn item_links_are_fitted_to_the_bytes_the_baseline_leaves() {
        let escaped = "\u{1}".repeat(200);
        let links = || {
            Some(
                (1..=200)
                    .map(|n| item_link(n, &escaped))
                    .collect::<Vec<_>>(),
            )
        };
        // An item whose own text is 40 000 bytes: a description or a brief at its cut, in four-byte characters.
        let baseline = 40_000;

        // The control: the caps alone leave a list that does not fit an answer of that baseline.
        let mut capped_only = links().unwrap();
        capped(&mut capped_only, MAX_LINKS);
        assert_eq!(
            capped_only[0].item.title.len(),
            MAX_LINK_TITLE_BYTES,
            "the control: nothing for the title cut to cut"
        );
        let after_the_caps = serde_json::to_string(&capped_only).unwrap().len();
        assert!(
            after_the_caps > room(baseline),
            "the control: {after_the_caps} bytes after the caps, room for {}",
            room(baseline)
        );

        let mut fitted = links();
        let omitted = bound_links(&mut fitted, baseline);
        let sent = fitted.unwrap();
        assert!(
            !sent.is_empty() && sent.len() < MAX_LINKS,
            "{} sent",
            sent.len()
        );
        assert_eq!(
            sent.len() + omitted,
            200,
            "every row is either sent or counted"
        );
        assert_eq!(sent[0].item.key.to_string(), "TST-1", "the first are kept");
        assert_eq!(
            sent[sent.len() - 1].item.key.to_string(),
            format!("TST-{}", sent.len()),
            "dropped from the end"
        );
        assert_eq!(sent[0].item.title, escaped, "a kept row is whole");
        assert!(serde_json::to_string(&sent).unwrap().len() <= room(baseline));
        assert!(
            serde_json::to_string(&capped_only[..=sent.len()])
                .unwrap()
                .len()
                > room(baseline),
            "one more row would not have fitted: the fit drops no more than it must"
        );

        // Characters JSON doubles: fifty titles of 200 quotes are 10 000 bytes of text and twice that of JSON. The
        // list fits an otherwise empty answer whole, and not one whose baseline is 50 000 bytes.
        let quotes = "\"".repeat(200);
        let quoted = || Some((1..=50).map(|n| item_link(n, &quotes)).collect::<Vec<_>>());
        let mut alone = quoted();
        assert_eq!(bound_links(&mut alone, 0), 0);
        assert_eq!(alone, quoted(), "nothing cut, nothing dropped");
        let mut beside = quoted();
        let dropped = bound_links(&mut beside, 50_000);
        assert!(dropped > 0 && dropped < 50, "{dropped} dropped");
        assert!(serde_json::to_string(&beside).unwrap().len() <= room(50_000));

        // A baseline that already fills the answer leaves no room: nothing is sent and everything is counted.
        let mut none = links();
        assert_eq!(bound_links(&mut none, crate::mcp::tools::MAX_SENT), 200);
        assert_eq!(none, Some(vec![]));
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

    /// Ten web links at the longest allowed lengths are about 18 300 bytes. With room they are sent whole, the oldest
    /// ten; with less room, fewer, still whole and still the oldest; what the cap and the fit leave out is counted.
    #[test]
    fn the_first_ten_web_links_are_sent_whole_and_fitted_to_the_room_left() {
        let title = "𝄞".repeat(200);
        let url = format!("https://example.com/{}", "a".repeat(980));
        assert_eq!(
            (title.len(), url.len()),
            (800, 1_000),
            "the control: the longest of each"
        );
        let links = || {
            Some(
                (1..=60)
                    .map(|id| reeve_protocol::WebLinkDto {
                        id,
                        title: title.clone(),
                        url: url.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let whole = |sent: &[reeve_protocol::WebLinkDto]| {
            sent.iter().all(|w| w.url == url && w.title == title)
        };

        // Room for all ten: the cap alone.
        let mut all = links();
        assert_eq!(bound_web_links(&mut all, 0), 50);
        let ten = all.unwrap();
        assert_eq!(
            ten.iter().map(|w| w.id).collect::<Vec<_>>(),
            (1..=10).collect::<Vec<_>>(),
            "the oldest ten"
        );
        assert!(whole(&ten), "neither a URL nor a title is cut");

        // Room for some of them: a baseline of 60 000 bytes leaves 10 002 for the list.
        let baseline = 60_000;
        let ten_bytes = serde_json::to_string(&ten).unwrap().len();
        assert!(
            ten_bytes > room(baseline),
            "the control: the ten are {ten_bytes} bytes and do not fit"
        );
        let mut some = links();
        let omitted = bound_web_links(&mut some, baseline);
        let sent = some.unwrap();
        assert!(
            !sent.is_empty() && sent.len() < MAX_WEB_LINKS,
            "{} sent",
            sent.len()
        );
        assert_eq!(
            sent.len() + omitted,
            60,
            "every row is either sent or counted"
        );
        assert_eq!(
            sent.iter().map(|w| w.id).collect::<Vec<_>>(),
            (1..=sent.len() as i64).collect::<Vec<_>>(),
            "dropped from the end: the oldest stay"
        );
        assert!(whole(&sent), "a web link is dropped whole, never cut");
        assert!(serde_json::to_string(&sent).unwrap().len() <= room(baseline));

        // No room, at the limit and three bytes under it; and no list.
        for baseline in [crate::mcp::tools::MAX_SENT, crate::mcp::tools::MAX_SENT - 3] {
            let mut none = links();
            assert_eq!(
                bound_web_links(&mut none, baseline),
                60,
                "a baseline of {baseline}"
            );
            assert_eq!(none, Some(vec![]));
        }
        assert_eq!(bound_web_links(&mut None, 0), 0);
    }
}
