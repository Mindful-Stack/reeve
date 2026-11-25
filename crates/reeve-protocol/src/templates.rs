//! Brief templates: a named Markdown text an admin keeps, for one level, from which a new brief can be prefilled.
//! Prefill only: no item refers to its template, and nothing is validated against one.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The levels that have a brief. A type of its own, so a story (which has a description, not a brief) cannot be
/// named. Declared in list order: initiative templates come before epic templates.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BriefTemplateLevel {
    Initiative,
    Epic,
}

impl BriefTemplateLevel {
    pub const ALL: [Self; 2] = [Self::Initiative, Self::Epic];

    /// The name on the wire, in the database and on the command line.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Initiative => "initiative",
            Self::Epic => "epic",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.wire() == s)
    }
}

/// A brief template as every member reads it. `version` is what an edit must name; the default mark changes none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct BriefTemplateDto {
    pub id: i64,
    pub level: BriefTemplateLevel,
    /// Unique per level, compared trimmed and lowercased.
    pub name: String,
    /// Markdown, as it was written.
    pub body: String,
    /// At most one template per level is the default: what a new brief starts from when its create asks for the
    /// level's default (`default_template`). The web create form preselects it and puts its text in the brief, which
    /// it then sends as the brief.
    pub is_default: bool,
    pub version: i64,
}

/// `POST /brief-templates` (admins only). Creating one that exists (same level, same name, same body) answers it and
/// changes nothing; the same name with another body is a validation error.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBriefTemplate {
    pub level: BriefTemplateLevel,
    /// 1 to 100 characters on one line, with no tab or other control character.
    pub name: String,
    /// Markdown; not blank.
    pub body: String,
}

/// `PATCH /brief-templates/{id}` (admins only): the fields given change; `version` is the one last read. The level
/// of a template never changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EditBriefTemplate {
    pub version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_level_has_one_name_everywhere_and_story_is_not_one() {
        assert_eq!(
            BriefTemplateLevel::ALL.map(BriefTemplateLevel::wire),
            ["initiative", "epic"]
        );
        for level in BriefTemplateLevel::ALL {
            assert_eq!(BriefTemplateLevel::from_wire(level.wire()), Some(level));
            assert_eq!(serde_json::to_value(level).unwrap(), json!(level.wire()));
        }
        assert_eq!(BriefTemplateLevel::from_wire("story"), None);
        assert_eq!(BriefTemplateLevel::from_wire("Initiative"), None);
        assert!(serde_json::from_value::<BriefTemplateLevel>(json!("story")).is_err());
        assert!(BriefTemplateLevel::Initiative < BriefTemplateLevel::Epic);
    }

    #[test]
    fn the_template_requests_refuse_a_field_they_do_not_know() {
        let create = json!({"level": "epic", "name": "N", "body": "B", "is_default": true});
        assert!(serde_json::from_value::<CreateBriefTemplate>(create).is_err());
        let edit = json!({"version": 1, "level": "epic"});
        assert!(serde_json::from_value::<EditBriefTemplate>(edit).is_err());
        let edit: EditBriefTemplate = serde_json::from_value(json!({"version": 3})).unwrap();
        assert_eq!((edit.version, edit.name, edit.body), (3, None, None));
    }

    #[test]
    fn a_create_asks_for_a_template_only_when_it_says_so() {
        use crate::requests::{CreateEpic, CreateInitiative};
        // A body as every client sent it before templates existed.
        let plain: CreateInitiative = serde_json::from_value(json!({
            "idempotency_key": "k", "title": "T", "brief": null, "horizon": "Now",
            "target_month": null, "owner": null
        }))
        .unwrap();
        assert_eq!(
            (plain.template.as_deref(), plain.default_template),
            (None, false)
        );
        let out = serde_json::to_value(&plain).unwrap();
        assert!(
            out.get("template").is_none() && out.get("default_template").is_none(),
            "{out}"
        );

        let asked: CreateEpic = serde_json::from_value(json!({
            "idempotency_key": "k", "title": "T", "initiative": null, "brief": null,
            "template": "Scenario", "default_template": true
        }))
        .unwrap();
        assert_eq!(
            (asked.template.as_deref(), asked.default_template),
            (Some("Scenario"), true)
        );
        let out = serde_json::to_value(&asked).unwrap();
        assert_eq!(
            (&out["template"], &out["default_template"]),
            (&json!("Scenario"), &json!(true))
        );
    }
}
