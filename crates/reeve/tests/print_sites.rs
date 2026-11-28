//! Only `src/terminal.rs` writes what a command prints to standard output or standard error. Everything a command
//! prints for reading passes its guard there, so a command added later cannot print stored text raw, neither by
//! reaching for `println!` nor by writing to `std::io::stderr()` itself. Two other writers are not a command's text:
//! `src/tui/term.rs`, the TUI's own terminal (the alternate screen, the ratatui backend, the clipboard sequence), and
//! clap's `e.print()` in `src/lib.rs` (help, version and usage errors), a method call this check does not see.
#[path = "rust_source/mod.rs"]
mod rust_source;
use rust_source::{Tok, rust_files, tokens};
use std::path::{Path, PathBuf};

/// The print macros, by name: `println!(…)` and `println !(…)` are both a call.
const PRINTS: [&str; 5] = ["print", "println", "eprint", "eprintln", "dbg"];

/// The functions that open a standard stream: `stdout()`, `std::io::stderr()`, `io::stdout().lock()`.
const STREAMS: [&str; 2] = ["stdout", "stderr"];

/// The files besides `terminal.rs` that may open a standard stream, for the TUI's own terminal and never for text a
/// command prints; none may hold a print macro: `tui/term.rs` enters and leaves the alternate screen, builds the
/// ratatui backend on standard output and writes the OSC 52 clipboard sequence itself.
fn stream_owners() -> [PathBuf; 1] {
    [["tui", "term.rs"].iter().collect()]
}

/// Each call in `src`, with its line: a print macro, a standard stream opened by a path or a bare name, or either
/// renamed on import (`use std::io::stdout as raw;`, `use std::println as say;`), whose later calls would carry
/// another name. A method or a field of that name (`.stdout(Stdio::piped())`, `out.stderr`) is not one.
///
/// It detects these syntactic forms, not every way to reach a stream: a stream function taken as a value
/// (`let f = std::io::stdout;`) is not seen. No such code exists in the crate, and a review would ask why.
fn calls(src: &str) -> Vec<(usize, String)> {
    let t = tokens(src);
    let mut found = vec![];
    for (n, (line, tok)) in t.iter().enumerate() {
        let Tok::Ident(w) = tok else { continue };
        let next = t.get(n + 1).map(|(_, x)| x);
        let after_dot = n > 0 && t[n - 1].1 == Tok::Punct('.');
        let named = PRINTS.contains(&w.as_str()) || STREAMS.contains(&w.as_str());
        if PRINTS.contains(&w.as_str()) && next == Some(&Tok::Punct('!')) {
            found.push((*line, format!("{w}!")));
        } else if STREAMS.contains(&w.as_str()) && next == Some(&Tok::Punct('(')) && !after_dot {
            found.push((*line, format!("{w}()")));
        } else if named && next == Some(&Tok::Ident("as".into())) {
            found.push((*line, format!("{w} as")));
        }
    }
    found
}

fn offenders(src: &Path) -> Vec<String> {
    let mut files = vec![];
    rust_files(src, &mut files);
    let owners = stream_owners();
    let mut found = vec![];
    for file in files {
        // As `PathBuf`s built from components, never a joined string: the separator differs on Windows.
        let rel = file.strip_prefix(src).unwrap().to_path_buf();
        if rel == Path::new("terminal.rs") {
            continue;
        }
        for (line, call) in calls(&std::fs::read_to_string(&file).unwrap()) {
            if call.ends_with("()") && owners.contains(&rel) {
                continue;
            }
            found.push(format!("{}:{line}: {call}", rel.display()));
        }
    }
    found.sort();
    found
}

#[test]
fn only_terminal_rs_writes_to_a_standard_stream() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let found = offenders(&src);
    assert!(
        found.is_empty(),
        "print through crate::terminal (out, err, exact, document) so stored text passes the guard:\n{}",
        found.join("\n")
    );
}

/// The check itself: it sees a call however it is spaced and however the stream is named, and does not mistake a
/// comment, a string, a character literal or a longer name for one.
#[test]
fn the_check_tells_a_call_from_a_mention() {
    let found = |src: &str| -> Vec<String> {
        calls(src)
            .into_iter()
            .map(|(line, call)| format!("{line}:{call}"))
            .collect()
    };
    // Calls: either spacing of a macro, and a stream opened by a full path or an imported name.
    assert_eq!(
        found("println!(\"x\");\neprintln !(\"y\");"),
        ["1:println!", "2:eprintln!"]
    );
    assert_eq!(
        found("let _ = writeln!(std::io::stderr(), \"x\");"),
        ["1:stderr()"]
    );
    assert_eq!(
        found("use std::io;\nsay(&mut io::stdout(), x);"),
        ["2:stdout()"]
    );
    // Comments: a line comment, a doc comment, a nested block comment.
    assert!(
        found("// println!(\"x\")\n/// eprintln!\n/* print!(1) /* dbg!(2) */ println!(3) */")
            .is_empty()
    );
    // Strings: escaped quotes, a raw string holding quotes, a string across lines.
    assert!(found("let s = \"use \\\"println!\\\" here\";").is_empty());
    assert!(found("let s = r#\"println!(\"x\")\"#;").is_empty());
    assert!(found("let s = \"first\nprintln!(x)\nlast\";").is_empty());
    // Longer names, a method and a field of a stream's name.
    assert!(found("my_println!(1); x.println_count; Command::new(\"git\").stdout(Stdio::piped()); out.stderr").is_empty());
    // A quote in a character literal opens no string, an escaped quote closes none, and a lifetime is no literal.
    assert_eq!(found("let q = '\"'; println!(\"{q}\");"), ["1:println!"]);
    assert_eq!(
        found("let e = '\\''; fn f<'a>(s: &'a str) { eprint!(\"{s}\") }"),
        ["1:eprint!"]
    );
    // Lines are counted through a block comment and a string that span lines.
    assert_eq!(found("/* a\nb */ let s = \"c\nd\";\ndbg!(s);"), ["4:dbg!"]);
    // A raw string with no `#`, a byte string, and a byte literal holding a quote open nothing that hides a call.
    assert!(found("let s = r\"println!(x)\"; let t = b\"eprintln!(y)\";").is_empty());
    assert_eq!(found("let q = b'\"'; println!(\"{q}\");"), ["1:println!"]);
    // A stream or a print macro renamed on import, alone or in a list.
    assert_eq!(
        found("use std::io::stdout as raw;\nwriteln!(raw(), \"x\");"),
        ["1:stdout as"]
    );
    assert_eq!(found("use std::io::{self, stderr as e};"), ["1:stderr as"]);
    assert_eq!(found("use std::println as say;"), ["1:println as"]);
    // A raw string with two `#` holding `"#`, a C string, a raw byte string, an escaped character literal and a raw
    // identifier hide no call; a path-qualified macro is one.
    assert!(found("let s = r##\"a\"# println!(x)\"##;").is_empty());
    assert!(found("let s = c\"println!(x)\";").is_empty());
    assert!(found("let s = br#\"eprintln!(\"x\")\"#;").is_empty());
    assert!(found("let e = '\\u{1b}'; let f = '\\u{1b}';").is_empty());
    assert!(found("let r#println = 1;").is_empty());
    assert_eq!(found("std::println!(\"x\");"), ["1:println!"]);
}
