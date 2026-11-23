//! The CLI version this binary was built with (kb/ADR-0030): the 40-character commit id of the last change to the CLI's
//! build inputs, compiled in as `REEVE_CLI_VERSION` by CI, and shown to people as a "build". A binary built without it
//! is unstamped (people see `dev build`): it never shows the update notice. `reeve --version` is unrelated and unchanged
//! (`reeve-planner <semver>`, kb/ADR-0005 R1).
use reeve_protocol::is_cli_version;
use std::io::Write;
use std::sync::Mutex;

/// The start of the notice. A contract agents match on (kb/ADR-0030 R6): change it only through a record.
pub const PREFIX: &str = "note: this reeve is build";

/// This binary's CLI version; `None` when unstamped.
///
/// Debug builds also honour `REEVE_CLI_VERSION_OVERRIDE`, so the tests can run one binary as several builds (a value
/// that is not a CLI version, such as `dev`, makes it unstamped). The read sits inside `#[cfg(debug_assertions)]`, so a
/// release build does not contain it: a released binary's CLI version cannot change at run time.
pub fn current() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Some(v) = std::env::var_os("REEVE_CLI_VERSION_OVERRIDE").filter(|v| !v.is_empty()) {
        let v = v.to_string_lossy().into_owned();
        return is_cli_version(&v).then_some(v);
    }
    option_env!("REEVE_CLI_VERSION")
        .filter(|v| is_cli_version(v))
        .map(str::to_owned)
}

/// The first seven characters, as builds are shown.
pub fn short(v: &str) -> &str {
    v.get(..7).unwrap_or(v)
}

/// The notice for a stamped CLI whose build differs from the one the server offers; `None` when unstamped, when the
/// server sent no (or a garbled) CLI version, or when the builds are equal. Neutral wording: after a rollback the offered
/// build is the older one; "between commands" tells an agent when to update (kb/ADR-0030 R8).
pub fn notice(current: Option<&str>, offered: Option<&str>, ascii: bool) -> Option<String> {
    let (c, s) = (current?, offered?);
    (is_cli_version(c) && is_cli_version(s) && c != s).then(|| {
        format!(
            "{PREFIX} {}, the server offers {} {} run reeve update between commands",
            short(c),
            short(s),
            crate::glyphs::dash_with(ascii)
        )
    })
}

/// Whether the TUI shows its note: exactly when a command would print the notice, under the same switch.
pub fn tui_note(current: Option<&str>, offered: Option<&str>, setting: Option<&str>) -> bool {
    enabled(setting) && notice(current, offered, false).is_some()
}

/// `REEVE_UPDATE_NOTICE=0` silences the notice and the TUI note; anything else (or nothing) leaves them on.
pub fn enabled(setting: Option<&str>) -> bool {
    setting != Some("0")
}

/// The `REEVE_UPDATE_NOTICE` setting of this process.
pub fn setting() -> Option<String> {
    std::env::var("REEVE_UPDATE_NOTICE").ok()
}

/// The build a server offers, as its `X-Reeve-Cli` header names it: `None` when the header is absent or is not a CLI
/// version (a garbled value is ignored, never trusted). The one reading of that header, for `Client` and `reeve update`.
pub fn offered_in(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get(reeve_protocol::CLI_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| is_cli_version(v))
        .map(str::to_owned)
}

/// The last build a server offered: the answer that last named one wins, and an answer that names none keeps it.
pub struct Offered(Mutex<Option<String>>);

impl Offered {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }
    /// Records the build `headers` name, if they name one.
    pub fn observe(&self, headers: &reqwest::header::HeaderMap) {
        if let Some(v) = offered_in(headers) {
            *self.0.lock().unwrap() = Some(v);
        }
    }
    pub fn get(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

/// The one store of the build the server offers in this process: every `Client` answer writes it (`observe`); the
/// notice, `reeve auth status` and the TUI's startup decision read it (`offered`). Process-wide: unit tests must not
/// assert on it (they share the process) and use their own `Offered`; the notice is covered by subprocess tests.
static OFFERED: Offered = Offered::new();

/// Records the build an answer's headers name (called by `Client` for every response).
pub fn observe(headers: &reqwest::header::HeaderMap) {
    OFFERED.observe(headers);
}

/// The last build any `Client` in this process was offered; `None` before an answer named one.
pub fn offered() -> Option<String> {
    OFFERED.get()
}

/// Prints the notice on standard error when it applies (kb/ADR-0030 R6), with or without a terminal or `--json`.
/// `lib.rs::cli()` is its only caller, once per process, after the command's output and whatever its outcome, and never
/// for `reeve tui`.
pub fn notify() {
    if !enabled(setting().as_deref()) {
        return;
    }
    if let Some(n) = notice(
        current().as_deref(),
        offered().as_deref(),
        crate::glyphs::ascii(),
    ) {
        // Not `eprintln!`: it panics when standard error cannot be written (a closed pipe), and the notice must never
        // change the exit status (kb/ADR-0030 R6).
        let _ = writeln!(std::io::stderr(), "{n}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "9f8e7d6c5b4a39281706f5e4d3c2b1a098765432";
    const B: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";

    #[test]
    fn the_notice_names_both_builds_only_when_they_differ() {
        assert_eq!(
            notice(Some(A), Some(B), false).as_deref(),
            Some(
                "note: this reeve is build 9f8e7d6, the server offers 1a2b3c4 — run reeve update between commands"
            )
        );
        assert_eq!(
            notice(Some(A), Some(B), true).as_deref(),
            Some(
                "note: this reeve is build 9f8e7d6, the server offers 1a2b3c4 -- run reeve update between commands"
            ),
            "--ascii"
        );
        assert_eq!(notice(Some(A), Some(A), false), None, "equal builds");
        assert_eq!(
            notice(None, Some(B), false),
            None,
            "an unstamped build never sees it"
        );
        assert_eq!(
            notice(Some(A), None, false),
            None,
            "a server without a bundle sends no header"
        );
        assert_eq!(
            notice(Some(A), Some("not-a-version"), false),
            None,
            "a garbled header is ignored"
        );
    }

    #[test]
    fn the_notice_starts_with_the_prefix_agents_match_on() {
        // kb/ADR-0030 R6: this prefix is a contract; changing it needs a record.
        assert!(notice(Some(A), Some(B), false).unwrap().starts_with(PREFIX));
        assert_eq!(PREFIX, "note: this reeve is build");
    }

    #[test]
    fn the_tui_note_follows_the_notice_and_the_same_switch() {
        assert!(tui_note(Some(A), Some(B), None));
        assert!(
            !tui_note(Some(A), Some(A), None)
                && !tui_note(None, Some(B), None)
                && !tui_note(Some(A), None, None)
        );
        assert!(
            !tui_note(Some(A), Some(B), Some("0")),
            "REEVE_UPDATE_NOTICE=0 silences the TUI note too"
        );
    }

    #[test]
    fn only_zero_silences_it() {
        assert!(enabled(None));
        assert!(enabled(Some("1")) && enabled(Some("")) && enabled(Some("no")));
        assert!(!enabled(Some("0")));
    }

    fn answer(cli: Option<&str>) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Some(v) = cli {
            h.insert(reeve_protocol::CLI_VERSION_HEADER, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn only_a_cli_version_in_the_header_is_an_offered_build() {
        assert_eq!(offered_in(&answer(Some(A))).as_deref(), Some(A));
        assert_eq!(offered_in(&answer(None)), None);
        for garbled in [
            "latest",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "9f8e7d6",
        ] {
            assert_eq!(offered_in(&answer(Some(garbled))), None, "{garbled}");
        }
    }

    #[test]
    fn the_last_offered_build_wins_and_an_answer_without_one_keeps_it() {
        // A store of its own: the process-wide one is shared by every test in this process.
        let o = Offered::new();
        assert_eq!(o.get(), None, "nothing before the first answer");
        o.observe(&answer(Some(A)));
        assert_eq!(o.get().as_deref(), Some(A));
        o.observe(&answer(Some(B)));
        assert_eq!(o.get().as_deref(), Some(B), "the last one wins");
        o.observe(&answer(None));
        assert_eq!(
            o.get().as_deref(),
            Some(B),
            "an answer without the header keeps it"
        );
        o.observe(&answer(Some("not-a-version")));
        assert_eq!(o.get().as_deref(), Some(B), "a garbled header is ignored");
    }

    #[test]
    fn short_is_the_first_seven_characters() {
        assert_eq!(short(B), "1a2b3c4");
        assert_eq!(short("abc"), "abc");
    }
}
