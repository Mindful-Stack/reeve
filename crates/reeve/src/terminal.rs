//! What reaches the reader's terminal. Two rules for server-supplied text, and the only functions in this crate that
//! write to standard output or standard error (`tests/print_sites.rs` refuses a print macro or an opened standard
//! stream anywhere else).
//!
//! Text mode is for reading; `--json` is exact and never comes through `line` or `block`.
use std::borrow::Cow;

/// One-line text (a title, a name, a label, an owner, a URL, a criterion) as text output prints it: every control
/// character (Unicode category Cc: ESC, NUL, a line break, a tab and the rest), the line and paragraph separators
/// (U+2028, U+2029) and the bidirectional controls (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069)
/// written as their Rust escape (`\u{1b}`, `\n`, `\u{202e}`), so stored text cannot drive the reader's terminal, pass
/// for another line of output or reorder what is printed after it. The zero-width joiner and non-joiner (U+200D,
/// U+200C) print as they are, so a joined emoji does. A backslash is not escaped: a title that says `\n` reads like
/// an escaped line break, and `--json` is the exact form.
pub fn line(s: &str) -> Cow<'_, str> {
    if !s.chars().any(escaped_in_line) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if escaped_in_line(c) {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Long text (a brief, a description, a comment, a template body, an error message) as text output prints it: as
/// `line`, except that line breaks and tabs are kept (`\r\n` and a lone `\r` become `\n`) and so are the three
/// direction marks editors put in mixed right-to-left text (U+200E, U+200F, U+061C). The embeddings, overrides and
/// isolates, which reorder what follows, are escaped. Applying it twice changes nothing.
pub fn block(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| c == '\r' || escaped_in_block(c)) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            c if escaped_in_block(c) => out.extend(c.escape_default()),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// What `line` escapes.
fn escaped_in_line(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}'
                | '\u{2029}'
                | '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

/// What `block` escapes: what `line` does, less the line break, the tab and the three direction marks. A carriage
/// return never reaches here: `block` turns it into a line break first.
fn escaped_in_block(c: char) -> bool {
    !matches!(c, '\n' | '\t' | '\u{200e}' | '\u{200f}' | '\u{061c}') && escaped_in_line(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A title stored before the server refused control characters, or by another server, cannot clear the screen,
    /// set the window title or start a line of its own.
    #[test]
    fn line_escapes_every_control_character() {
        assert_eq!(
            line("Report\u{1b}[2J\u{1b}[Hfake\noutput\ttab\0\u{9b}K\u{7f}\r"),
            "Report\\u{1b}[2J\\u{1b}[Hfake\\noutput\\ttab\\u{0}\\u{9b}K\\u{7f}\\r"
        );
        // Text without one is printed as it is, whatever its script, and is not copied to be so.
        for plain in [
            "Scenario page",
            "Scenariosida för ärenden",
            "シナリオ 🚀",
            "a\\nb",
        ] {
            assert!(matches!(line(plain), Cow::Borrowed(s) if s == plain));
        }
    }

    /// A line or paragraph separator would start a line of its own, and a bidirectional control would reorder what
    /// follows it on the line.
    #[test]
    fn line_escapes_separators_and_bidirectional_controls() {
        assert_eq!(
            line("Report\u{2028}second line\u{2029}paragraph"),
            "Report\\u{2028}second line\\u{2029}paragraph"
        );
        for (c, escaped) in [
            ('\u{061c}', "\\u{61c}"),
            ('\u{200e}', "\\u{200e}"),
            ('\u{200f}', "\\u{200f}"),
            ('\u{202a}', "\\u{202a}"),
            ('\u{202b}', "\\u{202b}"),
            ('\u{202c}', "\\u{202c}"),
            ('\u{202d}', "\\u{202d}"),
            ('\u{202e}', "\\u{202e}"),
            ('\u{2066}', "\\u{2066}"),
            ('\u{2067}', "\\u{2067}"),
            ('\u{2068}', "\\u{2068}"),
            ('\u{2069}', "\\u{2069}"),
        ] {
            assert_eq!(
                line(&format!("Report{c}page")),
                format!("Report{escaped}page"),
                "{c:?}"
            );
        }
        // The joiners are not escaped: a joined emoji prints as it is, and is not copied to be so.
        for joined in [
            "Family \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} page",
            "Rainbow flag \u{1f3f3}\u{fe0f}\u{200d}\u{1f308}",
            "\u{645}\u{200c}\u{6cc}\u{62e}\u{648}\u{627}\u{647}\u{645}",
        ] {
            assert!(
                matches!(line(joined), Cow::Borrowed(s) if s == joined),
                "{joined:?}"
            );
        }
    }

    /// Long text keeps its shape: its lines, its tabs and the marks a right-to-left sentence needs.
    #[test]
    fn block_keeps_line_breaks_tabs_and_direction_marks() {
        let brief = "# Title\n\n| a\t| b |\nשלום\u{200f} hello\u{200e} \u{61c}مرحبا";
        assert!(matches!(block(brief), Cow::Borrowed(s) if s == brief));
    }

    /// A CRLF file's lines are lines, and a lone carriage return cannot move the cursor back over what was printed.
    #[test]
    fn block_turns_carriage_returns_into_line_breaks() {
        assert_eq!(block("one\r\ntwo\rthree\r"), "one\ntwo\nthree\n");
        assert_eq!(block("\r\r\nx"), "\n\nx");
        assert_eq!(block("\rx"), "\nx");
    }

    /// Everything else that can drive the terminal or reorder what follows is escaped, in long text too.
    #[test]
    fn block_escapes_what_can_drive_the_terminal() {
        assert_eq!(
            block("a\u{1b}]0;owned\u{7}\0\u{9b}K\u{7f}b\u{2028}c\u{2029}d"),
            "a\\u{1b}]0;owned\\u{7}\\u{0}\\u{9b}K\\u{7f}b\\u{2028}c\\u{2029}d"
        );
        for (c, escaped) in [
            ('\u{202a}', "\\u{202a}"),
            ('\u{202b}', "\\u{202b}"),
            ('\u{202c}', "\\u{202c}"),
            ('\u{202d}', "\\u{202d}"),
            ('\u{202e}', "\\u{202e}"),
            ('\u{2066}', "\\u{2066}"),
            ('\u{2067}', "\\u{2067}"),
            ('\u{2068}', "\\u{2068}"),
            ('\u{2069}', "\\u{2069}"),
        ] {
            assert_eq!(block(&format!("x{c}y")), format!("x{escaped}y"), "{c:?}");
        }
    }

    /// The guard runs over text whose one-line fields `line` already escaped, and may run twice: neither changes it.
    #[test]
    fn block_changes_nothing_already_escaped() {
        let hostile = "a\u{1b}[2Jb\nc\u{202e}d\r";
        let once = line(hostile).into_owned();
        // Documents the layering (`line` first, then the guard); it cannot fail, the twice-applied check below is the real one.
        assert_eq!(block(&once), once);
        let shown = block(hostile).into_owned();
        assert_eq!(block(&shown), shown);
    }
}
