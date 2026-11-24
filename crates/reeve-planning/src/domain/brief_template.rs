//! A brief template: a level, a name and a Markdown body, from which a new brief can be prefilled. Pure. It names no
//! item and no item names it; it is no readiness fact, and nothing here reads the clock.
use reeve_protocol::BriefTemplateLevel;

/// A name is at most this many characters, after trimming.
pub const MAX_NAME: usize = 100;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BriefTemplateError {
    #[error("a template's name must be 1 to 100 characters on one line")]
    Name,
    #[error("a template's body must not be blank and must hold no NUL character")]
    Body,
}

/// How names are compared: trimmed and lowercased (Unicode, `str::to_lowercase`). Computed here, never by the
/// database, so "the same name" does not depend on a database's locale. A typed name and a stored one meet through
/// this one function.
pub fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// A level, a name and a body, all as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefTemplate {
    level: BriefTemplateLevel,
    name: String,
    body: String,
}

impl BriefTemplate {
    /// The name trimmed, 1 to `MAX_NAME` characters on one line: none of them a control character (Unicode category
    /// Cc: NUL, ESC, a line break, a tab and the rest) or a line or paragraph separator (U+2028, U+2029), the rule
    /// of a web link's title. Such a character is refused, not stripped. The body is kept exactly as written; it
    /// must not be blank, and it must hold no NUL, which PostgreSQL does not take in text. It has no length rule
    /// of its own, as a brief has none.
    pub fn new(
        level: BriefTemplateLevel,
        name: &str,
        body: &str,
    ) -> Result<Self, BriefTemplateError> {
        let name = name.trim();
        let off_the_line = |c: char| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}');
        if !(1..=MAX_NAME).contains(&name.chars().count()) || name.chars().any(off_the_line) {
            return Err(BriefTemplateError::Name);
        }
        if body.trim().is_empty() || body.contains('\0') {
            return Err(BriefTemplateError::Body);
        }
        Ok(Self {
            level,
            name: name.to_string(),
            body: body.to_string(),
        })
    }
    /// A stored row. It does not validate.
    pub fn from_persisted(level: BriefTemplateLevel, name: String, body: String) -> Self {
        Self { level, name, body }
    }
    pub fn level(&self) -> BriefTemplateLevel {
        self.level
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    /// What this template's name is unique by, at its level.
    pub fn name_key(&self) -> String {
        name_key(&self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reeve_protocol::BriefTemplateLevel::{Epic, Initiative};

    #[test]
    fn a_name_is_stored_trimmed_and_is_one_line_of_1_to_100_characters() {
        let t = BriefTemplate::new(Initiative, "  User scenario \n", "body").unwrap();
        assert_eq!(t.name(), "User scenario");
        assert!(
            BriefTemplate::new(Initiative, &"å".repeat(100), "b").is_ok(),
            "counted in characters, not bytes"
        );
        let long = "a".repeat(101);
        for bad in [
            "",
            "   ",
            long.as_str(),
            "two\nlines",
            "tab\there",
            "esc\u{1b}[2J",
            "sep\u{2028}x",
        ] {
            assert_eq!(
                BriefTemplate::new(Initiative, bad, "b"),
                Err(BriefTemplateError::Name),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn names_compare_trimmed_and_lowercased() {
        let key = |n: &str| BriefTemplate::new(Epic, n, "b").unwrap().name_key();
        assert_eq!(key("Scenario"), key("  sCENARIO "));
        assert_eq!(key("Ärende"), key("ärende"), "beyond ASCII too");
        assert_ne!(key("Scenario"), key("Scenarios"));
        assert_eq!(
            name_key(" Tech Debt "),
            "tech debt",
            "a typed name gets the key a stored one has"
        );
    }

    #[test]
    fn a_body_is_kept_as_written_and_must_say_something() {
        let body = "\n# Scenario\n\n| Step | Who |\n|---|---|\n\n";
        assert_eq!(
            BriefTemplate::new(Initiative, "S", body).unwrap().body(),
            body,
            "not trimmed: a brief is stored as written, and so is what prefills it"
        );
        for bad in ["", " \n\t", "a\0b"] {
            assert_eq!(
                BriefTemplate::new(Initiative, "S", bad),
                Err(BriefTemplateError::Body),
                "{bad:?}"
            );
        }
    }
}
