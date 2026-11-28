//! A tokenizer for Rust source, shared by the source checks (`print_sites.rs`, `renderer_tests.rs`): just enough to
//! tell a name from a mention of it in a comment, a string or a character literal.
#![allow(dead_code)] // each test file uses part of it
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Punct(char),
}

/// The tokens of Rust source that matter here, each with its line: identifiers and single punctuation characters.
/// Comments (`//`, nested `/* */`), string literals (escapes, raw with any number of `#`, byte and C strings, across
/// lines), character literals (`'"'`, `'\''`) and lifetimes produce none, so a macro named in a doc comment, a
/// message or a character literal is not a call.
pub fn tokens(src: &str) -> Vec<(usize, Tok)> {
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

/// Every `.rs` file under `dir`, recursively.
pub fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}
