//! Every renderer has a hostile test. A function in `render.rs`, `relations.rs`, `promise.rs` or `urgency.rs` that is
//! `pub` or `pub(crate)` and returns text (`String`, `Option<String>` or `Vec<String>`) must be named inside a function
//! that calls `hostile::assert_one_line` or `hostile::assert_no_stored_text` (which make every string of the input
//! hostile), or be on `EXEMPT` with the reason it needs none. A renderer added later without such a test fails here,
//! so the hostile helper does not see only the renderers someone remembered to give it.
//!
//! It checks every text-returning function of the four files, not only those that take a protocol DTO: telling a DTO
//! from another reference by its tokens would be a guess. "Named inside" is by identifier: a test that mentions the
//! name some other way (a local variable of that name) would pass it too, which a review would see.
#[path = "rust_source/mod.rs"]
mod rust_source;
use rust_source::{Tok, rust_files, tokens};
use std::collections::BTreeSet;
use std::path::Path;

/// The files whose renderers are checked.
const RENDERERS: [&str; 4] = ["render.rs", "relations.rs", "promise.rs", "urgency.rs"];

/// The hostile helper's two checks.
const HOSTILE: [&str; 2] = ["assert_one_line", "assert_no_stored_text"];

/// Text-returning functions that need no hostile test, each with the reason.
const EXEMPT: &[(&str, &str)] = &[
    ("day_label", "formats a date only"),
    ("date_label", "formats a date only"),
    ("state_words", "formats an enum and a number only"),
    ("promises_heading", "formats dates only"),
    ("marks", "formats a number only"),
    ("days_ago", "formats two instants only"),
    ("warnings", "formats enums only"),
    (
        "not_a_blocker",
        "formats enums and keys only (a key is grammar the client checked); it is given no stored text",
    ),
    (
        "with_urgency",
        "joins text its callers already rendered; their hostile tests cover what it prints",
    ),
    (
        "continued",
        "lays out one long text given as a string, not a DTO; `continued_lays_long_text_out_under_its_header` \
         pins it, and its renderers' hostile tests and `a_servers_error_message_continues_indented` cover its callers",
    ),
    (
        "import_report",
        "its Report is the CLI's own type, not a DTO the hostile helper can deserialise; \
         its test `a_jira_value_cannot_start_a_row_of_its_own` in commands/import.rs is hand-rolled",
    ),
];

/// The tokens of `src` as identifiers and punctuation, lines dropped.
fn toks(src: &str) -> Vec<Tok> {
    tokens(src).into_iter().map(|(_, t)| t).collect()
}

fn ident(t: Option<&Tok>, w: &str) -> bool {
    matches!(t, Some(Tok::Ident(x)) if x == w)
}

fn punct(t: Option<&Tok>, c: char) -> bool {
    t == Some(&Tok::Punct(c))
}

/// The index just past the group that opens at `i` (`(`, `[`, `{` or `<`), counting only its own kind of bracket.
/// For `<`, a `->` inside is not a closing `>`.
fn past_group(t: &[Tok], i: usize) -> usize {
    let (open, close) = match &t[i] {
        Tok::Punct('(') => ('(', ')'),
        Tok::Punct('[') => ('[', ']'),
        Tok::Punct('{') => ('{', '}'),
        _ => ('<', '>'),
    };
    let mut depth = 0;
    let mut j = i;
    while j < t.len() {
        if t[j] == Tok::Punct(open) {
            depth += 1;
        } else if t[j] == Tok::Punct(close) && !(close == '>' && j > 0 && punct(t.get(j - 1), '-'))
        {
            depth -= 1;
            if depth == 0 {
                return j + 1;
            }
        }
        j += 1;
    }
    j
}

/// Each `pub` or `pub(crate)` function in `src` with its return type, spelled from its tokens (`String`,
/// `Option<String>`, `&str`: a lifetime leaves no token).
fn public_fns(src: &str) -> Vec<(String, String)> {
    let t = toks(src);
    let mut found = vec![];
    for i in 0..t.len() {
        if !ident(t.get(i), "pub") {
            continue;
        }
        let mut j = i + 1;
        if punct(t.get(j), '(') {
            j = past_group(&t, j);
        }
        if !ident(t.get(j), "fn") {
            continue;
        }
        let Some(Tok::Ident(name)) = t.get(j + 1) else {
            continue;
        };
        let mut k = j + 2;
        if punct(t.get(k), '<') {
            k = past_group(&t, k);
        }
        if !punct(t.get(k), '(') {
            continue;
        }
        k = past_group(&t, k);
        let mut ret = String::new();
        if punct(t.get(k), '-') && punct(t.get(k + 1), '>') {
            k += 2;
            while k < t.len() && !punct(t.get(k), '{') && !ident(t.get(k), "where") {
                match &t[k] {
                    Tok::Ident(w) => ret.push_str(w),
                    Tok::Punct(c) => ret.push(*c),
                }
                k += 1;
            }
        }
        found.push((name.clone(), ret));
    }
    found
}

/// Every identifier inside the body of a function that calls one of `HOSTILE`.
fn named_in_hostile_tests(src: &str) -> BTreeSet<String> {
    let t = toks(src);
    let mut named = BTreeSet::new();
    let mut i = 0;
    while i < t.len() {
        if !ident(t.get(i), "fn") {
            i += 1;
            continue;
        }
        // The body is the first `{` after the name; a function without one (a declaration) ends at `;`.
        let mut b = i + 1;
        while b < t.len() && !punct(t.get(b), '{') && !punct(t.get(b), ';') {
            b += 1;
        }
        if !punct(t.get(b), '{') {
            i = b;
            continue;
        }
        let end = past_group(&t, b);
        let body = &t[b..end];
        if body
            .iter()
            .any(|x| matches!(x, Tok::Ident(w) if HOSTILE.contains(&w.as_str())))
        {
            for x in body {
                if let Tok::Ident(w) = x {
                    named.insert(w.clone());
                }
            }
        }
        // Nested functions are visited too: step into the body, not past it.
        i += 1;
    }
    named
}

const TEXT: [&str; 3] = ["String", "Option<String>", "Vec<String>"];

/// The renderers in `renderers` that no hostile test names and `EXEMPT` does not list, given every source file's text.
fn untested(renderers: &[(String, String)], all: &[String]) -> Vec<String> {
    let mut named = BTreeSet::new();
    for src in all {
        named.extend(named_in_hostile_tests(src));
    }
    let mut missing: Vec<String> = renderers
        .iter()
        .filter(|(_, ret)| TEXT.contains(&ret.as_str()))
        .filter(|(name, _)| !named.contains(name) && !EXEMPT.iter().any(|(e, _)| e == name))
        .map(|(name, _)| name.clone())
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

fn crate_src() -> (Vec<(String, String)>, Vec<String>) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![];
    rust_files(&src, &mut files);
    let mut renderers = vec![];
    let mut all = vec![];
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let rel = file.strip_prefix(&src).unwrap();
        if RENDERERS.iter().any(|r| rel == Path::new(r)) {
            renderers.extend(public_fns(&text));
        }
        all.push(text);
    }
    (renderers, all)
}

#[test]
fn every_renderer_has_a_hostile_test() {
    let (renderers, all) = crate_src();
    // The walk found the renderers at all (a moved file would make this pass vacuously).
    assert!(
        renderers
            .iter()
            .any(|(n, r)| n == "story_row" && r == "String"),
        "{renderers:?}"
    );
    let missing = untested(&renderers, &all);
    assert!(
        missing.is_empty(),
        "name each in a test that calls hostile::assert_one_line or assert_no_stored_text, or add it to EXEMPT with \
         the reason it needs none:\n{}",
        missing.join("\n")
    );
}

/// Every exemption still names a text-returning renderer, so the list cannot outlive what it excuses.
#[test]
fn every_exemption_names_a_renderer() {
    let (renderers, _) = crate_src();
    for (name, _) in EXEMPT {
        assert!(
            renderers
                .iter()
                .any(|(n, r)| n == name && TEXT.contains(&r.as_str())),
            "{name} is exempt but is no text-returning renderer"
        );
    }
}

/// The check itself: it reads a signature however it is spread, and counts a renderer tested only when a function
/// that calls the hostile helper names it.
#[test]
fn the_check_reads_signatures_and_hostile_tests() {
    let src = "
        pub fn plain(s: &StoryDto) -> String { x }
        pub(crate) fn lines<'a>(
            xs: impl Iterator<Item = &'a WarRoomEntryDto>,
            now: DateTime<Utc>,
        ) -> Vec<String> { y }
        pub fn maybe(l: &[WebLinkDto]) -> Option<String> where T: X { z }
        pub fn word(s: DeliveryState) -> &'static str { w }
        fn private(s: &StoryDto) -> String { v }
        pub fn tested(s: &StoryDto) -> String { u }
    ";
    assert_eq!(
        public_fns(src),
        [
            ("plain".to_string(), "String".to_string()),
            ("lines".into(), "Vec<String>".into()),
            ("maybe".into(), "Option<String>".into()),
            ("word".into(), "&str".into()),
            ("tested".into(), "String".into()),
        ]
    );
    let tests = "
        #[test]
        fn a() { hostile::assert_one_line(&s, Paths::none(), |s| tested(s)); }
        #[test]
        fn b() { let s = plain(&x); assert!(s.len() > 0); }
        fn helper() { fn inner() { assert_no_stored_text(&v, |v| lines(v)) } }
    "
    .to_string();
    assert_eq!(untested(&public_fns(src), &[tests]), ["maybe", "plain"]);
}
