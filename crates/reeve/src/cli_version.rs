//! The CLI version this binary was built with (kb/ADR-0030): the 40-character commit id of the last change to the CLI's
//! build inputs, compiled in as `REEVE_CLI_VERSION` by CI, and shown to people as a "build". A binary built without it
//! is unstamped (people see `dev build`): it never shows the update notice. `reeve --version` is unrelated and unchanged
//! (`reeve-planner <semver>`, kb/ADR-0005 R1).
use reeve_protocol::is_cli_version;
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
    (is_cli_version(s) && c != s).then(|| {
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

/// The last offered build any `Client` in this process was sent.
static SEEN: Mutex<Option<String>> = Mutex::new(None);

/// Records a server's `X-Reeve-Cli` (called by `Client` for every response that carries one).
pub fn observe(offered: &str) {
    *SEEN.lock().unwrap() = Some(offered.to_string());
}

/// Prints the notice on standard error when it applies (kb/ADR-0030 R6), with or without a terminal or `--json`.
/// `lib.rs::cli()` is its only caller, once per process, after the command's output and whatever its outcome, and never
/// for `reeve tui`.
pub fn notify() {
    if !enabled(setting().as_deref()) {
        return;
    }
    let offered = SEEN.lock().unwrap().clone();
    if let Some(n) = notice(
        current().as_deref(),
        offered.as_deref(),
        crate::glyphs::ascii(),
    ) {
        eprintln!("{n}");
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

    #[test]
    fn short_is_the_first_seven_characters() {
        assert_eq!(short(B), "1a2b3c4");
        assert_eq!(short("abc"), "abc");
    }
}
