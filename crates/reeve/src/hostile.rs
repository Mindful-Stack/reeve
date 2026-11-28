//! Test support for every text renderer (GRA-882): a fixture with every string made hostile, and what the rendered
//! output must then show. A field added to a DTO later is made hostile too, and checked as one-line text unless the
//! call names its path as long text. A fixture must be full: a `null`, an empty list or an empty object in its JSON
//! fails the check unless the call names that path as absent, with the reason, so a field the fixture leaves out is a
//! choice the test states. A field skipped when serialised (`skip_serializing_if`) leaves no trace in the JSON and
//! cannot be seen; the fixtures set those.
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// What a call says about its fixture's paths, each a JSON pointer from the fixture's root, `*` for any array index or
/// tuple member: which hold long text (`terminal::block`'s, the guard's, not the renderer's) and which are
/// deliberately absent (`null`, `[]` or `{}`), each with its reason. A path is matched whole, so a one-line field
/// named like a long one under another parent is checked as one-line. Built per fixture type, and moved under a tuple
/// member or a field with `under` when the fixture is nested in another.
#[derive(Clone, Debug, Default)]
pub(crate) struct Paths {
    long: Vec<String>,
    absent: Vec<(String, &'static str)>,
}

impl Paths {
    /// No long text and nothing absent.
    pub(crate) fn none() -> Self {
        Self::default()
    }

    /// These paths hold long text.
    pub(crate) fn long(mut self, paths: &[&str]) -> Self {
        self.long.extend(paths.iter().map(|p| (*p).to_string()));
        self
    }

    /// The fixture leaves `path` empty on purpose, for `why`.
    pub(crate) fn absent(mut self, path: &str, why: &'static str) -> Self {
        assert!(!why.trim().is_empty(), "{path} is absent without a reason");
        self.absent.push((path.to_string(), why));
        self
    }

    /// The same paths under `prefix` (a JSON pointer such as `/0` or `/story`), for a fixture nested in another.
    pub(crate) fn under(self, prefix: &str) -> Self {
        Self {
            long: self
                .long
                .into_iter()
                .map(|p| format!("{prefix}{p}"))
                .collect(),
            absent: self
                .absent
                .into_iter()
                .map(|(p, why)| (format!("{prefix}{p}"), why))
                .collect(),
        }
    }

    /// These paths and `other`'s.
    pub(crate) fn and(mut self, other: Paths) -> Self {
        self.long.extend(other.long);
        self.absent.extend(other.absent);
        self
    }
}

/// What a leaf that refused the hostile text is tried with: a word only free text takes.
const PLAIN: &str = "plainword";

/// One-line leaf `n`: a marker, a line break and a tab, an escape sequence, an override and a closing marker. As
/// `terminal::line` prints it, the marker is followed by the four characters `\n\t`; `block` keeps the tab, so the
/// long-text rule with its line breaks replaced does not pass for `line`.
fn one_line_text(n: usize) -> String {
    format!("q{n}z\n\t\u{1b}[2J\u{202e}e{n}q")
}

/// Long leaf `n`: an escape sequence and an override, and no line break, so a renderer that splits long text into
/// indented lines leaves it whole and the check can take it out.
fn long_text(n: usize) -> String {
    format!("l{n}z\u{1b}[2J\u{202e}e{n}l")
}

/// Whether `c` could drive a terminal or reorder the text after it. A line break and a tab are the CLI's own.
fn drives(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Whether JSON pointer `at` matches `pattern` whole, `*` standing for an array index or tuple member only.
fn path_matches(pattern: &str, at: &str) -> bool {
    let (mut want, mut got) = (pattern.split('/'), at.split('/'));
    loop {
        match (want.next(), got.next()) {
            (None, None) => return true,
            (Some(w), Some(g)) if w == g || (w == "*" && g.parse::<usize>().is_ok()) => {}
            _ => return false,
        }
    }
}

/// Whether the leaf at JSON pointer `at` is long text: its whole path matches one of `long`.
fn is_long(at: &str, long: &[String]) -> bool {
    long.iter().any(|p| path_matches(p, at))
}

/// Every string leaf under `v` as a JSON pointer, and every empty value (`null`, `[]`, `{}`).
fn leaves(v: &Value, at: &str, strings: &mut Vec<String>, empty: &mut Vec<String>) {
    match v {
        Value::String(_) => strings.push(at.to_string()),
        Value::Null => empty.push(at.to_string()),
        Value::Array(xs) if xs.is_empty() => empty.push(at.to_string()),
        Value::Object(m) if m.is_empty() => empty.push(at.to_string()),
        Value::Array(xs) => {
            for (i, x) in xs.iter().enumerate() {
                leaves(x, &format!("{at}/{i}"), strings, empty);
            }
        }
        Value::Object(m) => {
            for (k, x) in m {
                let token = k.replace('~', "~0").replace('/', "~1");
                leaves(x, &format!("{at}/{token}"), strings, empty);
            }
        }
        _ => {}
    }
}

/// A leaf the helper left as the fixture had it.
#[derive(Debug)]
struct Kept {
    /// Its JSON pointer.
    at: String,
    /// The fixture's value.
    value: String,
    /// Whether it took `PLAIN` though it refused the hostile text: free text the helper could not check.
    takes_plain_text: bool,
    /// Whether it took its own value with one letter or digit changed (`None`: the value has neither): a validated
    /// type that takes words like the fixture's is free text too, unless the value is a key, a date or a uuid.
    takes_a_near_value: Option<bool>,
}

/// What `hostile` made of a fixture.
struct Hostile<T> {
    value: T,
    /// The one-line leaves made hostile: pointer, text.
    one_line: Vec<(String, String)>,
    /// The long texts.
    long: Vec<String>,
    /// The leaves left as they were.
    kept: Vec<Kept>,
    /// The empty values (`null`, `[]`, `{}`) the call did not name as absent.
    empty: Vec<String>,
    /// The paths named as absent that match no empty value.
    not_absent: Vec<String>,
}

/// Whether `v` deserialises as `T` and serialises back with `text` at `at`.
fn carries<T: Serialize + DeserializeOwned>(v: &Value, at: &str, text: &str) -> bool {
    serde_json::from_value::<T>(v.clone())
        .ok()
        .and_then(|t| serde_json::to_value(t).ok())
        .is_some_and(|back| back.pointer(at) == Some(&Value::String(text.to_string())))
}

/// `s` with its last ASCII letter or digit changed to another of its kind, or `None` when it has neither.
fn near(s: &str) -> Option<String> {
    let (i, c) = s
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_ascii_alphanumeric())?;
    let other = match c {
        'z' => 'y',
        'Z' => 'Y',
        '9' => '8',
        c if c.is_ascii_digit() => '9',
        c if c.is_ascii_uppercase() => 'Z',
        _ => 'z',
    };
    Some(format!("{}{other}{}", &s[..i], &s[i + 1..]))
}

/// `fixture` with each string leaf replaced, one at a time, by a hostile text wherever the result still deserialises
/// as `T` and serialises back with that text in place; the rest keep the fixture's value (a key, a date, a uuid, an
/// enum's word, and an enum's `#[serde(other)]` fallback, which would read the text as `Unknown`) and are returned as
/// kept. Long text is the leaves `paths` names long.
fn hostile<T: Serialize + DeserializeOwned>(fixture: &T, paths: &Paths) -> Hostile<T> {
    let mut v = serde_json::to_value(fixture).expect("a fixture serialises");
    let (mut found, mut empty) = (vec![], vec![]);
    leaves(&v, "", &mut found, &mut empty);
    let not_absent = paths
        .absent
        .iter()
        .filter(|(p, _)| !empty.iter().any(|e| path_matches(p, e)))
        .map(|(p, _)| p.clone())
        .collect();
    empty.retain(|e| !paths.absent.iter().any(|(p, _)| path_matches(p, e)));
    let (mut one_line, mut long, mut kept) = (vec![], vec![], vec![]);
    for (n, at) in found.into_iter().enumerate() {
        let is_long = is_long(&at, &paths.long);
        let text = if is_long {
            long_text(n)
        } else {
            one_line_text(n)
        };
        let was = std::mem::replace(
            v.pointer_mut(&at).expect("the leaf is there"),
            Value::String(text.clone()),
        );
        if carries::<T>(&v, &at, &text) {
            if is_long {
                long.push(text);
            } else {
                one_line.push((at, text));
            }
            continue;
        }
        let value = was.as_str().expect("a string leaf").to_string();
        let mut takes = |probe: &str| {
            *v.pointer_mut(&at).expect("the leaf is there") = Value::String(probe.into());
            carries::<T>(&v, &at, probe)
        };
        let takes_plain_text = takes(PLAIN);
        let takes_a_near_value = near(&value).map(|n| takes(&n));
        *v.pointer_mut(&at).expect("the leaf is there") = was;
        kept.push(Kept {
            at,
            value,
            takes_plain_text,
            takes_a_near_value,
        });
    }
    let value = serde_json::from_value(v).expect("every kept replacement deserialised");
    Hostile {
        value,
        one_line,
        long,
        kept,
        empty,
        not_absent,
    }
}

/// Whether a kept value carries no text: a key, a date, a timestamp or a uuid, or a word its type takes alone (an
/// enum's), which refuses the same word with one letter changed.
fn fixed_word(k: &Kept) -> bool {
    let s = k.value.as_str();
    s.parse::<reeve_protocol::Key>().is_ok()
        || s.parse::<chrono::NaiveDate>().is_ok()
        || s.parse::<chrono::DateTime<chrono::Utc>>().is_ok()
        || s.parse::<uuid::Uuid>().is_ok()
        || k.takes_a_near_value == Some(false)
}

/// The check behind `assert_one_line` and `assert_no_stored_text`, as a result, so the helper's own tests can see
/// which part failed. `some_shown`: fail when no one-line field reached the output.
fn check<T: Serialize + DeserializeOwned>(
    fixture: &T,
    paths: &Paths,
    render: impl Fn(&T) -> String,
    some_shown: bool,
) -> Result<Vec<String>, String> {
    let Hostile {
        value,
        one_line,
        long,
        kept,
        empty,
        not_absent,
    } = hostile(fixture, paths);
    if !empty.is_empty() {
        return Err(format!(
            "the fixture leaves {empty:?} empty: fill them, or name each as absent with its reason"
        ));
    }
    if !not_absent.is_empty() {
        return Err(format!(
            "{not_absent:?} named as absent but the fixture holds a value there: drop the name"
        ));
    }
    for k in &kept {
        if k.takes_plain_text || !fixed_word(k) {
            return Err(format!(
                "{} kept its value {:?}: it refused the hostile text but is not a key, a date, a uuid or an enum's \
                 word{}; the helper cannot check it",
                k.at,
                k.value,
                if k.takes_plain_text {
                    ", and it takes a plain word"
                } else if k.takes_a_near_value == Some(true) {
                    ", and it takes its value with one letter changed"
                } else {
                    ""
                }
            ));
        }
    }
    let out = render(&value);
    let mut rest = out.clone();
    for l in &long {
        rest = rest.replace(l.as_str(), "");
    }
    if let Some(c) = rest.chars().find(|&c| drives(c)) {
        // The leaf it came from: the one whose text, from a character that drives a terminal to its closing marker,
        // is in the output as it is.
        let from = one_line
            .iter()
            .find(|(_, text)| {
                text.char_indices()
                    .any(|(i, d)| drives(d) && rest.contains(&text[i..]))
            })
            .map_or("no hostile leaf the helper can name", |(at, _)| at.as_str());
        return Err(format!(
            "{c:?} reached the output outside long text, from {from}:\n{out:?}"
        ));
    }
    let mut shown = vec![];
    for (at, text) in &one_line {
        let marker = &text[..text.find('\n').expect("a one-line text holds a line break")];
        let mut seen = false;
        for (i, _) in out.match_indices(marker) {
            if !out[i + marker.len()..].starts_with("\\n\\t") {
                return Err(format!("{at} is not printed as one line:\n{out}"));
            }
            seen = true;
        }
        if seen {
            shown.push(at.clone());
        }
    }
    if some_shown && shown.is_empty() {
        return Err(format!(
            "no one-line field reached the output; the fixture or the renderer is wrong:\n{out}"
        ));
    }
    Ok(shown)
}

/// Renders `fixture` with every string leaf hostile and asserts that the fixture is full (every empty value named in
/// `paths` as absent), that the renderer's own output, before any guard, holds nothing that drives a terminal or
/// reorders a line outside the long text `paths` names, that every one-line leaf it prints is printed escaped, its
/// line break and tab as `\n\t`, and that every leaf left as it was is a key, a date, a uuid or an enum's word.
/// Returns the JSON pointers of the one-line leaves the output showed: a test names those it expects with
/// `assert_shown` or compares the list whole; dropped, the call proves only that one field was printed.
#[must_use = "name the fields the renderer prints with `assert_shown`, or compare the list"]
pub(crate) fn assert_one_line<T: Serialize + DeserializeOwned>(
    fixture: &T,
    paths: &Paths,
    render: impl Fn(&T) -> String,
) -> Vec<String> {
    check(fixture, paths, render, true).unwrap_or_else(|e| panic!("{e}"))
}

/// For a renderer that prints no stored text (keys, dates and its own words only): the same checks, and no one-line
/// value is shown at all. A change that starts printing a field fails here, raw or escaped; an escaped one then
/// belongs in `assert_one_line`.
pub(crate) fn assert_no_stored_text<T: Serialize + DeserializeOwned>(
    fixture: &T,
    paths: &Paths,
    render: impl Fn(&T) -> String,
) {
    let shown = check(fixture, paths, render, false).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        shown.is_empty(),
        "the renderer prints {shown:?}: assert it with `assert_one_line`"
    );
}

/// Every field in `want` (a JSON pointer) was among those a hostile render showed.
pub(crate) fn assert_shown(shown: &[String], want: &[&str]) {
    for f in want {
        assert!(
            shown.iter().any(|s| s == f),
            "{f} was not printed; printed: {shown:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{block, line};
    use reeve_protocol::{Key, Kind, PromiseHolder};
    use serde::Deserialize;

    #[derive(Serialize, Deserialize)]
    struct Card {
        key: Key,
        kind: Kind,
        holder: PromiseHolder,
        title: String,
        owner: Option<String>,
        labels: Vec<String>,
        description: String,
    }

    fn card() -> Card {
        Card {
            key: "ABC-1".parse().unwrap(),
            kind: Kind::Story,
            holder: PromiseHolder::Own,
            title: "Booking".into(),
            owner: Some("dan@example.com".into()),
            labels: vec!["web".into()],
            description: "Long text".into(),
        }
    }

    fn card_paths() -> Paths {
        Paths::none().long(&["/description"])
    }

    /// Escapes every one-line field and prints the long one raw, on a line of its own.
    fn escaped(c: &Card) -> String {
        format!(
            "{} {:?} {}\n  owner: {}  labels: {}\n{}",
            c.key,
            c.kind,
            line(&c.title),
            line(c.owner.as_deref().unwrap_or_default()),
            c.labels
                .iter()
                .map(|l| line(l))
                .collect::<Vec<_>>()
                .join(","),
            c.description
        )
    }

    /// A renderer that escapes its one-line fields passes, its long field printed raw included, and the helper names
    /// what it printed.
    #[test]
    fn a_renderer_that_escapes_its_one_line_fields_passes() {
        let mut shown = check(&card(), &card_paths(), escaped, true).unwrap();
        shown.sort();
        assert_eq!(shown, ["/labels/0", "/owner", "/title"]);
        // A renderer of keys and words alone shows nothing, which `assert_no_stored_text` accepts.
        assert!(
            check(&card(), &card_paths(), |c| c.key.to_string(), false)
                .unwrap()
                .is_empty()
        );
    }

    /// A key, an enum's word and an enum with a fallback (`PromiseHolder` reads any word as `Unknown`) cannot carry
    /// hostile text: they keep the fixture's value, and the helper reports them as kept, with that value and what
    /// they took instead.
    #[test]
    fn what_cannot_carry_the_text_keeps_its_value() {
        let h = hostile(&card(), &card_paths());
        assert_eq!(h.value.key.to_string(), "ABC-1");
        assert_eq!(h.value.kind, Kind::Story);
        assert_eq!(h.value.holder, PromiseHolder::Own);
        let mut made: Vec<&str> = h.one_line.iter().map(|(at, _)| at.as_str()).collect();
        made.sort();
        assert_eq!(made, ["/labels/0", "/owner", "/title"]);
        assert_eq!(h.long.len(), 1, "the description");
        let mut kept: Vec<(&str, &str, bool, Option<bool>)> = h
            .kept
            .iter()
            .map(|k| {
                (
                    k.at.as_str(),
                    k.value.as_str(),
                    k.takes_plain_text,
                    k.takes_a_near_value,
                )
            })
            .collect();
        kept.sort();
        // The key takes `ABC-9`, another key: it is kept because it parses as a key, not as a word.
        assert_eq!(
            kept,
            [
                ("/holder", "own", false, Some(false)),
                ("/key", "ABC-1", false, Some(true)),
                ("/kind", "story", false, Some(false))
            ]
        );
    }

    /// Each way of getting it wrong fails, at the check meant to catch it: a field printed raw (even one no test
    /// names, and the failure names it), a field printed through the guard alone (which keeps its line break), the
    /// long-text rule with its line breaks replaced (which keeps the tab), and a render that shows no field at all.
    #[test]
    fn a_renderer_that_prints_a_field_raw_fails() {
        let raw_owner = check(
            &card(),
            &card_paths(),
            |c| {
                format!(
                    "{} {}",
                    line(&c.title),
                    c.owner.as_deref().unwrap_or_default()
                )
            },
            true,
        )
        .unwrap_err();
        assert!(
            raw_owner.starts_with("'\\u{1b}' reached the output outside long text, from /owner:"),
            "{raw_owner}"
        );
        let raw_labels = check(
            &card(),
            &card_paths(),
            |c| format!("{} [{}]", line(&c.title), c.labels.join(",")),
            true,
        )
        .unwrap_err();
        assert!(
            raw_labels.contains("reached the output outside long text, from /labels/0:"),
            "{raw_labels}"
        );
        let guard_only = check(
            &card(),
            &card_paths(),
            |c| block(&c.title).into_owned(),
            true,
        );
        assert!(
            guard_only
                .unwrap_err()
                .starts_with("/title is not printed as one line")
        );
        let long_rule_on_one_line = check(
            &card(),
            &card_paths(),
            |c| block(&c.title).replace('\n', "\\n"),
            true,
        );
        assert!(
            long_rule_on_one_line
                .unwrap_err()
                .starts_with("/title is not printed as one line")
        );
        let nothing = check(&card(), &card_paths(), |_| "nothing".to_string(), true);
        assert!(
            nothing
                .unwrap_err()
                .starts_with("no one-line field reached the output")
        );
    }

    /// A long path is the call's and matches whole: `notes` is long under the parent named and one-line anywhere
    /// else, and `*` stands for an index only.
    #[test]
    fn a_long_path_matches_whole_and_only_where_named() {
        let long = ["/goals/*/notes".to_string()];
        assert!(is_long("/goals/0/notes", &long));
        assert!(is_long("/goals/12/notes", &long));
        for one_line in [
            "/notes",
            "/goals/notes",
            "/goals/x/notes",
            "/sprint/goals/0/notes",
            "/goals/0/notes/0",
            "/promises/0/notes",
        ] {
            assert!(!is_long(one_line, &long), "{one_line}");
        }
        // Under another parent a field called `notes` is one-line, and printed raw it fails.
        #[derive(Serialize, Deserialize)]
        struct Note {
            notes: String,
        }
        #[derive(Serialize, Deserialize)]
        struct Sprint {
            goals: Vec<Note>,
            promises: Vec<Note>,
        }
        let s = Sprint {
            goals: vec![Note {
                notes: "Long text".into(),
            }],
            promises: vec![Note {
                notes: "Call Anna".into(),
            }],
        };
        let paths = Paths::none().long(&["/goals/*/notes"]);
        let e = check(
            &s,
            &paths,
            |s| format!("{}\n{}", s.goals[0].notes, s.promises[0].notes),
            true,
        )
        .unwrap_err();
        assert!(e.contains("from /promises/0/notes:"), "{e}");
        let printed = |s: &Sprint| format!("{}\n{}", s.goals[0].notes, line(&s.promises[0].notes));
        assert_eq!(
            check(&s, &paths, printed, true).unwrap(),
            ["/promises/0/notes"]
        );
        // Nested under a tuple member, the paths move with it.
        assert_eq!(
            check(&(1, s), &paths.under("/1"), |(_, s)| printed(s), true).unwrap(),
            ["/1/promises/0/notes"]
        );
    }

    /// A fixture that leaves a field `null` or a list or an object empty carries nothing there to make hostile: the
    /// check fails unless the call names the path as absent, and a path named absent that holds a value fails too.
    #[test]
    fn an_empty_fixture_fails_unless_its_absence_is_named() {
        let bare = Card {
            owner: None,
            labels: vec![],
            ..card()
        };
        let e = check(&bare, &card_paths(), escaped, true).unwrap_err();
        assert!(
            e.starts_with("the fixture leaves [\"/labels\", \"/owner\"] empty"),
            "{e}"
        );
        let named = card_paths()
            .absent("/owner", "an unowned card")
            .absent("/labels", "an unlabelled card");
        assert_eq!(check(&bare, &named, escaped, true).unwrap(), ["/title"]);
        let e = check(&card(), &named, escaped, true).unwrap_err();
        assert!(
            e.starts_with(
                "[\"/owner\", \"/labels\"] named as absent but the fixture holds a value"
            ),
            "{e}"
        );
        let e = check(
            &serde_json::json!({"title": "a", "cli": {}}),
            &Paths::none(),
            |v| line(v["title"].as_str().unwrap()).into_owned(),
            true,
        )
        .unwrap_err();
        assert!(e.starts_with("the fixture leaves [\"/cli\"] empty"), "{e}");
    }

    /// A renderer that drops one field while it prints the others passes the checks, and `assert_shown` names the
    /// field it dropped.
    #[test]
    #[should_panic(expected = "/owner was not printed")]
    fn a_renderer_that_drops_a_field_fails_where_the_test_names_it() {
        let shown = assert_one_line(&card(), &card_paths(), |c| line(&c.title).into_owned());
        assert_eq!(shown, ["/title"]);
        assert_shown(&shown, &["/title", "/owner"]);
    }

    /// A type that refuses control characters and text longer than `MAX` bytes: free text all the same.
    #[derive(Clone, Serialize, Deserialize)]
    #[serde(try_from = "String", into = "String")]
    struct Clean<const MAX: usize>(String);

    impl<const MAX: usize> TryFrom<String> for Clean<MAX> {
        type Error = &'static str;
        fn try_from(s: String) -> Result<Self, Self::Error> {
            if s.chars().any(char::is_control) || s.len() > MAX {
                Err("a control character, or too long")
            } else {
                Ok(Clean(s))
            }
        }
    }

    impl<const MAX: usize> From<Clean<MAX>> for String {
        fn from(c: Clean<MAX>) -> String {
            c.0
        }
    }

    #[derive(Serialize, Deserialize)]
    struct Noted<const MAX: usize> {
        title: String,
        note: Clean<MAX>,
    }

    /// Free text that refuses the hostile text would go unchecked; the helper fails on it instead of keeping it
    /// quietly: when it takes a plain word, and when it refuses that too (here eight bytes at most) but takes its own
    /// value with a letter changed (a lowercase word, which looks like an enum's).
    #[test]
    fn free_text_the_helper_cannot_replace_fails() {
        let sentence = Noted::<80> {
            title: "Booking".into(),
            note: Clean("Call Anna".into()),
        };
        let e = check(
            &sentence,
            &Paths::none(),
            |n| line(&n.title).into_owned(),
            true,
        )
        .unwrap_err();
        assert!(e.starts_with("/note kept its value \"Call Anna\""), "{e}");
        assert!(e.contains("it takes a plain word"), "{e}");
        let word = Noted::<8> {
            title: "Booking".into(),
            note: Clean("urgent".into()),
        };
        let e = check(&word, &Paths::none(), |n| line(&n.title).into_owned(), true).unwrap_err();
        assert!(e.starts_with("/note kept its value \"urgent\""), "{e}");
        assert!(
            e.contains("it takes its value with one letter changed"),
            "{e}"
        );
    }

    #[test]
    fn a_near_value_changes_the_last_letter_or_digit() {
        assert_eq!(near("story").as_deref(), Some("storz"));
        assert_eq!(near("fizz").as_deref(), Some("fizy"));
        assert_eq!(near("ABC-1").as_deref(), Some("ABC-9"));
        assert_eq!(near("in_progress_").as_deref(), Some("in_progresz_"));
        assert_eq!(near("--"), None);
    }
}
