//! Which brief a new initiative or epic starts with. The one place the rule is written: the create actions
//! (`create_initiative`, `create_epic`) call `starting_brief` for the brief they hand the planning handler, so
//! every caller of `POST /initiatives` and `POST /epics`, and both MCP create tools, get the same answer. A
//! template is applied only when the caller asks, by name or with `default_template`; a caller that says nothing
//! gets the brief it sent. Plan apply and the import call the planning handlers themselves and never come through
//! here.
use crate::{error::AppError, state::AppState};
use reeve_protocol::BriefTemplateLevel;

/// Where the brief comes from.
#[derive(Debug, PartialEq, Eq)]
enum BriefSource<'a> {
    /// What the caller sent, as sent: its brief, or nothing. What every create got before templates existed.
    AsSent(&'a str),
    /// The level's template with this name.
    Named(&'a str),
    /// The level's default template, or nothing when none is marked.
    LevelDefault,
}

/// The rule, without the database. A blank brief counts as none given. A brief that is not blank together with
/// either request for a template, and the two requests together, are refused: the server never silently drops
/// something the caller named.
fn brief_source<'a>(
    brief: Option<&'a str>,
    template: Option<&'a str>,
    default_template: bool,
) -> Result<BriefSource<'a>, AppError> {
    let written = brief.filter(|b| !b.trim().is_empty());
    match (written, template, default_template) {
        (Some(_), Some(_), _) => Err(AppError::Validation(
            "give a brief or a template, not both: a template only fills a brief that was not given".into(),
        )),
        (Some(_), None, true) => Err(AppError::Validation(
            "give a brief or ask for the default template, not both: a template only fills a brief that was not given"
                .into(),
        )),
        (None, Some(_), true) => Err(AppError::Validation(
            "template and default_template exclude each other: name a template or ask for the level's default"
                .into(),
        )),
        (None, Some(name), false) => Ok(BriefSource::Named(name)),
        (None, None, true) => Ok(BriefSource::LevelDefault),
        // Nothing asked for: the brief as it was sent, blank or not, exactly what the action passed on before.
        (_, None, false) => Ok(BriefSource::AsSent(brief.unwrap_or(""))),
    }
}

/// The brief a new item of `level` is created with, from what its create sent. Nothing about the template is kept:
/// the answer is plain text, and the item is created exactly as if the caller had typed it. With neither `template`
/// nor `default_template` it reads nothing and answers `brief.unwrap_or("")`.
pub(super) async fn starting_brief(
    state: &AppState,
    level: BriefTemplateLevel,
    brief: Option<&str>,
    template: Option<&str>,
    default_template: bool,
) -> Result<String, AppError> {
    match brief_source(brief, template, default_template)? {
        BriefSource::AsSent(b) => Ok(b.to_string()),
        BriefSource::Named(name) => state
            .handlers
            .brief_template_body(level, Some(name))
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "no {level} brief template is named {:?}: `reeve template ls --level {level}` lists them",
                    crate::text::echo(name),
                    level = level.wire()
                ))
            }),
        BriefSource::LevelDefault => Ok(state
            .handlers
            .brief_template_body(level, None)
            .await?
            .unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_applied_only_when_the_caller_asks() {
        use BriefSource::*;
        let ok = |b, t, d| brief_source(b, t, d).unwrap();
        // Nothing asked for: what was sent, as sent, also when it is blank or absent.
        assert_eq!(ok(Some("Mine"), None, false), AsSent("Mine"));
        assert_eq!(ok(Some("  Mine\n"), None, false), AsSent("  Mine\n"));
        assert_eq!(ok(Some(" \n"), None, false), AsSent(" \n"));
        assert_eq!(ok(Some(""), None, false), AsSent(""));
        assert_eq!(ok(None, None, false), AsSent(""));
        // Asked for, with no brief said in each way.
        for none in [None, Some(""), Some(" \n\t")] {
            assert_eq!(ok(none, None, true), LevelDefault, "{none:?}");
            assert_eq!(
                ok(none, Some("Scenario"), false),
                Named("Scenario"),
                "{none:?}"
            );
        }
        // The three contradictions.
        for (b, t, d, says) in [
            (
                Some("Mine"),
                Some("Scenario"),
                false,
                "a brief or a template, not both",
            ),
            (
                Some("Mine"),
                Some("Scenario"),
                true,
                "a brief or a template, not both",
            ),
            (
                Some("Mine"),
                None,
                true,
                "ask for the default template, not both",
            ),
            (None, Some("Scenario"), true, "exclude each other"),
            (Some(" "), Some("Scenario"), true, "exclude each other"),
        ] {
            match brief_source(b, t, d) {
                Err(AppError::Validation(m)) => assert!(m.contains(says), "{b:?} {t:?} {d}: {m}"),
                other => panic!("{b:?} {t:?} {d}: {other:?}"),
            }
        }
    }
}
