//! Only `src/terminal.rs` writes to standard output or standard error. Everything a command prints for reading
//! passes its guard there, so a command added later cannot print stored text raw, neither by reaching for
//! `println!` nor by writing to `std::io::stderr()` itself.
use std::path::{Path, PathBuf};

/// The print macros, by name: `println!(…)` and `println !(…)` are both a call.
const PRINTS: [&str; 5] = ["print", "println", "eprint", "eprintln", "dbg"];

/// The functions that open a standard stream: `stdout()`, `std::io::stderr()`, `io::stdout().lock()`.
const STREAMS: [&str; 2] = ["stdout", "stderr"];

/// The files that may open a standard stream, each for the TUI's own terminal and never for text a command prints;
/// neither may hold a print macro:
/// - `tui/term.rs` enters and leaves the alternate screen and writes the OSC 52 clipboard sequence itself;
/// - `tui/mod.rs` hands `std::io::stdout()` to the ratatui backend.
fn stream_owners() -> [PathBuf; 2] {
    [
        ["tui", "term.rs"].iter().collect(),
        ["tui", "mod.rs"].iter().collect(),
    ]
}

#[derive(Debug, PartialEq)]
enum Tok {
    Ident(String),
    Punct(char),
}

/// The tokens of Rust source that matter here, each with its line: identifiers and single punctuation characters.
/// Comments (`//`, nested `/* */`), string literals (escapes, raw with any number of `#`, byte and C strings, across
/// lines), character literals (`'"'`, `'\''`) and lifetimes produce none, so a macro named in a doc comment, a
/// message or a character literal is not a call.
fn tokens(src: &str) -> Vec<(usize, Tok)> {
    let s: Vec<char> = src.chars().collect();
    let at = |i: usize| s.get(i).copied();
    let (mut out, mut i, mut line) = (vec![], 0, 1);
    while i < s.len() {
        let c = s[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == Some('/') {
            while i < s.len() && s[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 0;
            while i < s.len() {
                if s[i] == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if s[i] == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    line += usize::from(s[i] == '\n');
                    i += 1;
                }
            }
        } else if c == '"' {
            i = string_end(&s, i + 1, None, &mut line);
        } else if c == '\'' {
            if at(i + 1) == Some('\\') {
                // An escaped character literal, `'\''`, `'\n'`, `'\u{1b}'`: past the quote, the backslash and the
                // escaped character, then to the closing quote.
                i += 3;
                while i < s.len() && s[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if at(i + 2) == Some('\'') {
                // A character literal: `'"'`, `'a'`.
                i += 3;
            } else {
                // A lifetime or a label: `'a`, `'static`.
                i += 1;
                while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_') {
                    i += 1;
                }
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_') {
                i += 1;
            }
            let word: String = s[start..i].iter().collect();
            let hashes = s[i..].iter().take_while(|&&h| h == '#').count();
            if matches!(word.as_str(), "r" | "br" | "cr") && at(i + hashes) == Some('"') {
                i = string_end(&s, i + hashes + 1, Some(hashes), &mut line);
            } else if matches!(word.as_str(), "b" | "c") && at(i) == Some('"') {
                i = string_end(&s, i + 1, None, &mut line);
            } else if word == "r" && hashes == 1 {
                // A raw identifier, `r#match`: its name is the next token.
                i += 1;
            } else {
                out.push((line, Tok::Ident(word)));
            }
        } else {
            out.push((line, Tok::Punct(c)));
            i += 1;
        }
    }
    out
}

/// The index just past a string literal whose contents start at `i`, counting the lines it spans. A raw string
/// (`Some(hashes)`) ends at a `"` followed by that many `#` and has no escapes; any other ends at a `"` that no
/// backslash escapes.
fn string_end(s: &[char], mut i: usize, raw: Option<usize>, line: &mut usize) -> usize {
    let hashes = raw.unwrap_or(0);
    while i < s.len() {
        match s[i] {
            '\\' if raw.is_none() => {
                *line += usize::from(s.get(i + 1) == Some(&'\n'));
                i += 2;
            }
            '"' if (1..=hashes).all(|k| s.get(i + k) == Some(&'#')) => return i + 1 + hashes,
            c => {
                *line += usize::from(c == '\n');
                i += 1;
            }
        }
    }
    i
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

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
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
}
