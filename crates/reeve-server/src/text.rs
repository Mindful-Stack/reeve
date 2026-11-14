//! Text the server cuts before it repeats it: shared by the actions (a refusal that repeats a caller's value) and
//! the MCP adapter (every text a tool answers or a refusal lists), so neither depends on the other for it.
/// How much of a caller's own argument an error message repeats.
pub(crate) const MAX_ECHO: usize = 200;

/// Cuts `text` to its first `limit` characters when it has more, and appends `[cut: <n> more characters]`, n being
/// the number of characters removed. Characters, not bytes: the cut is always on a character boundary.
pub(crate) fn cut(text: &mut String, limit: usize) {
    // The byte offset of the character after the last one kept; `None` when the text is within the limit.
    if let Some((at, _)) = text.char_indices().nth(limit) {
        let removed = text[at..].chars().count();
        text.truncate(at);
        text.push_str(&format!("[cut: {removed} more characters]"));
    }
}

/// The most characters a cut's marker takes: its words and the most digits a count can have.
const MAX_MARKER: usize = "[cut:  more characters]".len() + 20;

/// Cuts `text` so that, its marker included, it is at most `limit` characters; a text within `limit` is left as
/// it is. `limit` is larger than `MAX_MARKER`.
pub(crate) fn cut_within(text: &mut String, limit: usize) {
    if text.chars().count() > limit {
        cut(text, limit - MAX_MARKER);
    }
}

/// A caller's argument as an error message repeats it: its first `MAX_ECHO` characters, and how many were left out.
/// Also where an action's refusal repeats a caller's value (`actions::stories`), so the words after it survive.
pub(crate) fn echo(text: &str) -> String {
    let mut text = text.to_string();
    cut(&mut text, MAX_ECHO);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two limits the adapter cuts at (`mcp::tools::bound`): an item's own text and every other text.
    const LIMITS: [usize; 2] = [10_000, 2_000];
    const MAX_OWN_TEXT: usize = LIMITS[0];
    const MAX_OTHER_TEXT: usize = LIMITS[1];

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

    #[test]
    fn a_text_cut_within_a_limit_is_at_most_that_long_with_its_marker() {
        for text in ["a".repeat(2_000), "世".repeat(2_000)] {
            let mut same = text.clone();
            cut_within(&mut same, 2_000);
            assert_eq!(same, text, "a text at the limit is left as it is");
        }
        for over in [1, 9, 1_000_000] {
            let mut text = "å".repeat(2_000 + over);
            cut_within(&mut text, 2_000);
            assert!(text.chars().count() <= 2_000, "{over}");
            assert!(text.ends_with(&format!("[cut: {} more characters]", over + MAX_MARKER)));
        }
    }
}
