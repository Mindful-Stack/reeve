//! What reaches the reader's terminal. Two rules for server-supplied text, and the only functions in this crate that
//! write to standard output or standard error (`tests/print_sites.rs` refuses a print macro or an opened standard
//! stream anywhere else).
//!
//! Text mode is for reading; `--json` carries server values exactly, never through `line` or `block`. A sentence the
//! CLI itself writes about a value it refuses to use (a record's repository name or path, refused in
//! `results[].error` and `immediate_check.error`) may name that value in escaped form.
use std::borrow::Cow;
use std::io::{self, Write};

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

/// A line of text for a person on standard output: guarded by `block`, then one line break, and flushed. A reader
/// that went away (a closed pipe) is success; any other failure is returned, for `status`, as for `exact`: text
/// output redirected to a full disk (`reeve story context X > ctx.md`) is exit status 1, never a silent 0.
#[must_use = "a failed write is exit status 1: pass the result to `terminal::status`"]
pub fn out(text: &str) -> io::Result<()> {
    guarded(&mut io::stdout(), text)
}

/// A line of text for a person on standard error, guarded as `out` is: a refusal, an error, a note, the update
/// notice. A failed write is ignored: standard error is where a failure would be said, so there is nowhere left to
/// say it, and a note has no status of its own.
pub fn err(text: &str) {
    let _ = guarded(&mut io::stderr(), text);
}

/// One line on standard output exactly as given: a `--json` envelope. It does not pass the guard, which would turn a
/// raw U+202E into an escape JSON does not have. A reader that went away (a closed pipe) is success; any other
/// failure is returned, for `status`.
pub fn exact(text: &str) -> io::Result<()> {
    untouched(&mut io::stdout(), text, true)
}

/// A whole document on standard output exactly as given, with no line break added: `reeve export`, which writes JSON
/// with or without `--json`. A closed pipe is success; any other failure is returned, for `status`, so a full disk
/// cannot leave a truncated document behind exit status 0.
pub fn document(text: &str) -> io::Result<()> {
    untouched(&mut io::stdout(), text, false)
}

/// The exit status after a write to standard output (`out`, `exact`, `document`): `code` when it was written, else 1,
/// with the failure said on standard error.
pub fn status(written: io::Result<()>, code: i32) -> i32 {
    match written {
        Ok(()) => code,
        Err(e) => {
            err(&format!("error: writing standard output: {e}"));
            1
        }
    }
}

/// A person's line: `text` through `block`, written as `untouched` writes.
fn guarded(to: &mut dyn Write, text: &str) -> io::Result<()> {
    untouched(to, &block(text), true)
}

/// Writes `text` (and a line break) and flushes, so a failure shows here and not in the exit-time flush, which
/// ignores it. A closed pipe (`ErrorKind::BrokenPipe`, on Windows too) is success.
fn untouched(to: &mut dyn Write, text: &str, line_break: bool) -> io::Result<()> {
    let written = if line_break {
        writeln!(to, "{text}")
    } else {
        write!(to, "{text}")
    };
    match written.and_then(|()| to.flush()) {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

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
        // Pins that the guard leaves `line`'s escapes alone (it fails if `block` ever escaped a backslash or brace).
        assert_eq!(block(&once), once);
        let shown = block(hostile).into_owned();
        assert_eq!(block(&shown), shown);
    }

    /// A stream whose every write and flush fails with `kind`: a closed pipe, or a full disk. In process, so it runs on
    /// Windows and macOS too.
    struct Failing(ErrorKind);
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(self.0.into())
        }
    }

    /// `println!` panics on a stream it cannot write (exit status 101). A person's line, an envelope or a document
    /// whose reader went away (`| head -1`) is a success; any other failure is returned, and `status` makes it exit
    /// status 1, so `reeve export > backup.json` or `reeve story ls > stories.txt` on a full disk cannot exit 0.
    #[test]
    fn a_closed_pipe_is_success_and_any_other_failure_is_reported() {
        assert!(guarded(&mut Failing(ErrorKind::BrokenPipe), "reeve is up to date").is_ok());
        let e = guarded(&mut Failing(ErrorKind::Other), "reeve is up to date").unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Other);
        for line_break in [true, false] {
            assert!(untouched(&mut Failing(ErrorKind::BrokenPipe), "{}", line_break).is_ok());
            let e = untouched(&mut Failing(ErrorKind::Other), "{}", line_break).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::Other, "{line_break}");
        }
        assert_eq!(status(Ok(()), 2), 2);
        assert_eq!(
            status(Err(io::Error::other("no space left on device")), 0),
            1
        );
    }

    /// A stream that takes every byte and then fails to flush them: a buffered pipe whose reader went away, or a
    /// buffered file on a full disk.
    struct FlushFails(ErrorKind, Vec<u8>);
    impl Write for FlushFails {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.1.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(self.0.into())
        }
    }

    /// Every writer flushes, so a failure that only the flush sees is treated as a failed write: a closed pipe is
    /// success, anything else is returned. The bytes were all handed over first.
    #[test]
    fn a_flush_that_fails_is_reported_like_a_write() {
        assert!(guarded(&mut FlushFails(ErrorKind::BrokenPipe, vec![]), "x").is_ok());
        let mut disk = FlushFails(ErrorKind::Other, vec![]);
        assert_eq!(
            guarded(&mut disk, "x").unwrap_err().kind(),
            ErrorKind::Other
        );
        assert_eq!(disk.1, b"x\n".to_vec());
        for line_break in [true, false] {
            let mut pipe = FlushFails(ErrorKind::BrokenPipe, vec![]);
            assert!(
                untouched(&mut pipe, "{}", line_break).is_ok(),
                "{line_break}"
            );
            let mut disk = FlushFails(ErrorKind::Other, vec![]);
            let e = untouched(&mut disk, "{}", line_break).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::Other, "{line_break}");
            assert_eq!(
                disk.1,
                if line_break {
                    b"{}\n".to_vec()
                } else {
                    b"{}".to_vec()
                }
            );
        }
    }

    /// A human line is guarded and ends in one line break; an envelope and a document are written as given.
    #[test]
    fn human_lines_are_guarded_and_exact_ones_are_not() {
        let hostile = "a\u{1b}[2Jb\r\nc\u{202e}d";
        let mut human = Vec::new();
        guarded(&mut human, hostile).unwrap();
        assert_eq!(
            String::from_utf8(human).unwrap(),
            "a\\u{1b}[2Jb\nc\\u{202e}d\n"
        );
        let mut envelope = Vec::new();
        untouched(&mut envelope, hostile, true).unwrap();
        assert_eq!(String::from_utf8(envelope).unwrap(), format!("{hostile}\n"));
        let mut doc = Vec::new();
        untouched(&mut doc, hostile, false).unwrap();
        assert_eq!(String::from_utf8(doc).unwrap(), hostile);
    }

    /// The notice line is what agents match on (kb/ADR-0030 R6): the guard writes it byte for byte, the dash
    /// included. `reeve update`'s `reason` values travel only in an envelope, which never passes the guard;
    /// `tests/update.rs` pins them end to end.
    #[test]
    fn the_guard_leaves_the_notice_as_it_is() {
        const A: &str = "9f8e7d6c5b4a39281706f5e4d3c2b1a098765432";
        const B: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";
        for ascii in [false, true] {
            let notice = crate::cli_version::notice(Some(A), Some(B), ascii)
                .expect("two different stamped builds");
            assert!(notice.starts_with("note: this reeve is build"), "{notice}");
            let mut written = Vec::new();
            guarded(&mut written, &notice).unwrap();
            assert_eq!(String::from_utf8(written).unwrap(), format!("{notice}\n"));
        }
    }
}
