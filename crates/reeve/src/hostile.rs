//! Test support for every text renderer (GRA-882): a fixture with every string made hostile, and what the rendered
//! output must then show. A field added to a DTO later is made hostile too, and checked as one-line text unless its
//! path is on `LONG`, provided the test's fixture serialises it with a string in it: a field the fixture leaves
//! `None` or empty, or that is skipped when serialised, carries nothing to replace and is not checked.
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// The paths of long text in the fixtures the renderer tests use, from the fixture's root, `*` for any array index or
/// tuple member: a story's `description`, an initiative's and an epic's `brief`, a comment's and a template's `body`,
/// a goal's `notes`, alone and where the tests nest them. Long text is the guard's (`terminal::block` in
/// `terminal::out` and `err`), not the renderer's. A path is matched whole, so a one-line field that is named `notes`
/// or `body` somewhere else is checked as one-line; a fixture holding long text at a path not listed here fails, and
/// the path is added.
pub(crate) const LONG: &[&str] = &[
    // `StoryDto` alone, first in a tuple or listed, listed second in a tuple; in `StoryPromiseCleared` and
    // `StoryContextDto`.
    "/description",
    "/*/description",
    "/*/*/description",
    "/story/description",
    "/blockers/*/description",
    // `InitiativeDto`, `EpicDto`; in `StoryContextDto`.
    "/brief",
    "/initiative/brief",
    "/epic/brief",
    // `CommentDto` and `BriefTemplateDto`, alone and listed; in `StoryContextDto`.
    "/body",
    "/*/body",
    "/comments/*/body",
    // `GoalDto`; in `SprintDto`, `SprintViewDto`, `SprintsDto` and the planning tuple `(SprintsDto, SprintPlanningDto)`.
    "/notes",
    "/goals/*/notes",
    "/sprint/goals/*/notes",
    "/active/sprint/goals/*/notes",
    "/draft/goals/*/notes",
    "/closed/*/goals/*/notes",
    "/*/active/sprint/goals/*/notes",
    "/*/draft/goals/*/notes",
    "/*/closed/*/goals/*/notes",
];

/// What a leaf that refused the hostile text is tried with: a word only free text takes.
const PLAIN: &str = "plainword";

/// One-line leaf `n`: a marker, a line break, an escape sequence, an override and a closing marker. As
/// `terminal::line` prints it, the marker is followed by the two characters `\n`.
fn one_line_text(n: usize) -> String {
    format!("q{n}z\n\u{1b}[2J\u{202e}e{n}q")
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

/// Whether the leaf at JSON pointer `at` is long text: its whole path matches a path on `LONG`.
fn is_long(at: &str) -> bool {
    LONG.iter().any(|p| {
        let (mut want, mut got) = (p.split('/'), at.split('/'));
        loop {
            match (want.next(), got.next()) {
                (None, None) => return true,
                (Some(w), Some(g)) if w == g || (w == "*" && g.parse::<usize>().is_ok()) => {}
                _ => return false,
            }
        }
    })
}

/// Every string leaf under `v` as a JSON pointer, and whether it is long text.
fn leaves(v: &Value, at: &str, into: &mut Vec<(String, bool)>) {
    match v {
        Value::String(_) => into.push((at.to_string(), is_long(at))),
        Value::Array(xs) => {
            for (i, x) in xs.iter().enumerate() {
                leaves(x, &format!("{at}/{i}"), into);
            }
        }
        Value::Object(m) => {
            for (k, x) in m {
                let token = k.replace('~', "~0").replace('/', "~1");
                leaves(x, &format!("{at}/{token}"), into);
            }
        }
        _ => {}
    }
}

/// A leaf the helper left as the fixture had it.
#[derive(Debug)]
pub(crate) struct Kept {
    /// Its JSON pointer.
    pub(crate) at: String,
    /// The fixture's value.
    pub(crate) value: String,
    /// Whether it took `PLAIN` though it refused the hostile text: free text the helper could not check.
    pub(crate) takes_plain_text: bool,
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
}

/// Whether `v` deserialises as `T` and serialises back with `text` at `at`.
fn carries<T: Serialize + DeserializeOwned>(v: &Value, at: &str, text: &str) -> bool {
    serde_json::from_value::<T>(v.clone())
        .ok()
        .and_then(|t| serde_json::to_value(t).ok())
        .is_some_and(|back| back.pointer(at) == Some(&Value::String(text.to_string())))
}

/// `fixture` with each string leaf replaced, one at a time, by a hostile text wherever the result still deserialises
/// as `T` and serialises back with that text in place; the rest keep the fixture's value (a key, a date, a uuid, an
/// enum's word, and an enum's `#[serde(other)]` fallback, which would read the text as `Unknown`) and are returned as
/// kept.
fn hostile<T: Serialize + DeserializeOwned>(fixture: &T) -> Hostile<T> {
    let mut v = serde_json::to_value(fixture).expect("a fixture serialises");
    let mut found = vec![];
    leaves(&v, "", &mut found);
    let (mut one_line, mut long, mut kept) = (vec![], vec![], vec![]);
    for (n, (at, is_long)) in found.into_iter().enumerate() {
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
        *v.pointer_mut(&at).expect("the leaf is there") = Value::String(PLAIN.into());
        let takes_plain_text = carries::<T>(&v, &at, PLAIN);
        let value = was.as_str().expect("a string leaf").to_string();
        *v.pointer_mut(&at).expect("the leaf is there") = was;
        kept.push(Kept {
            at,
            value,
            takes_plain_text,
        });
    }
    let value = serde_json::from_value(v).expect("every kept replacement deserialised");
    Hostile {
        value,
        one_line,
        long,
        kept,
    }
}

/// Whether a kept value is one that carries no text: a key, a date or a timestamp, a uuid, or an enum's word.
fn fixed_word(s: &str) -> bool {
    s.parse::<reeve_protocol::Key>().is_ok()
        || s.parse::<chrono::NaiveDate>().is_ok()
        || s.parse::<chrono::DateTime<chrono::Utc>>().is_ok()
        || s.parse::<uuid::Uuid>().is_ok()
        || (!s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
}

/// The check behind `assert_one_line` and `assert_no_stored_text`, as a result, so the helper's own tests can see
/// which part failed. `some_shown`: fail when no one-line field reached the output.
fn check<T: Serialize + DeserializeOwned>(
    fixture: &T,
    render: impl Fn(&T) -> String,
    some_shown: bool,
) -> Result<Vec<String>, String> {
    let Hostile {
        value,
        one_line,
        long,
        kept,
    } = hostile(fixture);
    for k in &kept {
        if k.takes_plain_text || !fixed_word(&k.value) {
            return Err(format!(
                "{} kept its value {:?}: it refused the hostile text but is not a key, a date, a uuid or an enum's \
                 word{}; the helper cannot check it",
                k.at,
                k.value,
                if k.takes_plain_text {
                    ", and it takes a plain word"
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
        return Err(format!(
            "{c:?} reached the output outside long text:\n{out:?}"
        ));
    }
    let mut shown = vec![];
    for (at, text) in &one_line {
        let marker = &text[..text.find('\n').expect("a one-line text holds a line break")];
        let mut seen = false;
        for (i, _) in out.match_indices(marker) {
            if !out[i + marker.len()..].starts_with("\\n") {
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

/// Renders `fixture` with every string leaf hostile and asserts that the renderer's own output, before any guard,
/// holds nothing that drives a terminal or reorders a line outside long text, that every one-line leaf it prints is
/// printed escaped, its line break as `\n`, and that every leaf left as it was is a key, a date, a uuid or an enum's
/// word. Returns the JSON pointers of the one-line leaves the output showed.
pub(crate) fn assert_one_line<T: Serialize + DeserializeOwned>(
    fixture: &T,
    render: impl Fn(&T) -> String,
) -> Vec<String> {
    check(fixture, render, true).unwrap_or_else(|e| panic!("{e}"))
}

/// For a renderer that prints no stored text (keys, dates and its own words only): the same checks, and no one-line
/// value is shown at all. A change that starts printing a field fails here, raw or escaped; an escaped one then
/// belongs in `assert_one_line`.
pub(crate) fn assert_no_stored_text<T: Serialize + DeserializeOwned>(
    fixture: &T,
    render: impl Fn(&T) -> String,
) {
    let shown = check(fixture, render, false).unwrap_or_else(|e| panic!("{e}"));
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
        let mut shown = check(&card(), escaped, true).unwrap();
        shown.sort();
        assert_eq!(shown, ["/labels/0", "/owner", "/title"]);
        // A renderer of keys and words alone shows nothing, which `assert_no_stored_text` accepts.
        assert!(
            check(&card(), |c| c.key.to_string(), false)
                .unwrap()
                .is_empty()
        );
    }

    /// A key, an enum's word and an enum with a fallback (`PromiseHolder` reads any word as `Unknown`) cannot carry
    /// hostile text: they keep the fixture's value, and the helper reports them as kept, with that value.
    #[test]
    fn what_cannot_carry_the_text_keeps_its_value() {
        let h = hostile(&card());
        assert_eq!(h.value.key.to_string(), "ABC-1");
        assert_eq!(h.value.kind, Kind::Story);
        assert_eq!(h.value.holder, PromiseHolder::Own);
        let mut made: Vec<&str> = h.one_line.iter().map(|(at, _)| at.as_str()).collect();
        made.sort();
        assert_eq!(made, ["/labels/0", "/owner", "/title"]);
        assert_eq!(h.long.len(), 1, "the description");
        let mut kept: Vec<(&str, &str, bool)> = h
            .kept
            .iter()
            .map(|k| (k.at.as_str(), k.value.as_str(), k.takes_plain_text))
            .collect();
        kept.sort();
        assert_eq!(
            kept,
            [
                ("/holder", "own", false),
                ("/key", "ABC-1", false),
                ("/kind", "story", false)
            ]
        );
    }

    /// Each way of getting it wrong fails, at the check meant to catch it: a field printed raw (even one no test
    /// names), a field printed through the guard alone (which keeps its line break), and a render that shows no
    /// field at all.
    #[test]
    fn a_renderer_that_prints_a_field_raw_fails() {
        let raw_owner = check(
            &card(),
            |c| {
                format!(
                    "{} {}",
                    line(&c.title),
                    c.owner.as_deref().unwrap_or_default()
                )
            },
            true,
        );
        assert!(
            raw_owner
                .unwrap_err()
                .contains("reached the output outside long text")
        );
        let raw_labels = check(
            &card(),
            |c| format!("{} [{}]", line(&c.title), c.labels.join(",")),
            true,
        );
        assert!(
            raw_labels
                .unwrap_err()
                .contains("reached the output outside long text")
        );
        let guard_only = check(&card(), |c| block(&c.title).into_owned(), true);
        assert!(
            guard_only
                .unwrap_err()
                .starts_with("/title is not printed as one line")
        );
        let nothing = check(&card(), |_| "nothing".to_string(), true);
        assert!(
            nothing
                .unwrap_err()
                .starts_with("no one-line field reached the output")
        );
    }

    /// A renderer that drops one field while it prints the others passes the checks, and `assert_shown` names the
    /// field it dropped.
    #[test]
    #[should_panic(expected = "/owner was not printed")]
    fn a_renderer_that_drops_a_field_fails_where_the_test_names_it() {
        let shown = assert_one_line(&card(), |c| line(&c.title).into_owned());
        assert_eq!(shown, ["/title"]);
        assert_shown(&shown, &["/title", "/owner"]);
    }

    /// Free text that refuses the hostile text (here a type that refuses control characters) would go unchecked; the
    /// helper fails on it instead of keeping it quietly.
    #[test]
    fn free_text_the_helper_cannot_replace_fails() {
        #[derive(Clone, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        struct Clean(String);
        impl TryFrom<String> for Clean {
            type Error = &'static str;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                if s.chars().any(char::is_control) {
                    Err("a control character")
                } else {
                    Ok(Clean(s))
                }
            }
        }
        impl From<Clean> for String {
            fn from(c: Clean) -> String {
                c.0
            }
        }
        #[derive(Serialize, Deserialize)]
        struct Noted {
            title: String,
            note: Clean,
        }
        let fixture = Noted {
            title: "Booking".into(),
            note: Clean("Call Anna".into()),
        };
        let e = check(&fixture, |n| line(&n.title).into_owned(), true).unwrap_err();
        assert!(e.starts_with("/note kept its value \"Call Anna\""), "{e}");
        assert!(e.contains("it takes a plain word"), "{e}");
    }
}
