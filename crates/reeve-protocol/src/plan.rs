//! `POST /plans` (`reeve apply`): a plan of new items, required decisions, blockers, item links and web links, sent
//! as one request.
use crate::{enums::*, ids::Key, links::ItemLinkType};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The most item links one plan may hold (`ApplyPlan::links`); one more fails the validation.
pub const MAX_PLAN_ITEM_LINKS: usize = 500;
/// The most web links one plan may hold (`ApplyPlan::web_links`); one more fails the validation.
pub const MAX_PLAN_WEB_LINKS: usize = 500;

/// A plan holds items (initiatives, epics and stories), required decisions, blockers, item links and web links.
/// Items refer to each other by their local `name`; a reference that is no name in the plan is an existing item's key
/// or external reference. The plan is validated whole before anything is written. Applying the same plan (`id`)
/// again creates nothing: items and required decisions are keyed by the plan id and their name, while blockers,
/// item links and web links are idempotent by their content. Each write commits on its own, so a plan is not
/// atomic: a failed apply is finished by applying the same id again once the cause is corrected. A name keeps its
/// level across applies of one id. Unknown fields are refused, so a typo is an error, not a silently dropped field.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyPlan {
    /// Names this plan: the same id and names are the same items on a second apply.
    pub id: String,
    #[serde(default)]
    pub initiatives: Vec<PlanInitiative>,
    #[serde(default)]
    pub epics: Vec<PlanEpic>,
    #[serde(default)]
    pub stories: Vec<PlanStory>,
    #[serde(default)]
    pub decisions: Vec<PlanDecision>,
    #[serde(default)]
    pub blocks: Vec<PlanBlock>,
    /// Item links (kb/ADR-0031) between items of any level. Informational: a link never holds work back; `blocks`
    /// does that. At most 500 in one plan; more is a validation error.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<PlanLink>,
    /// Web links (kb/ADR-0031): titled URLs on items of any level. At most 500 in one plan; more is a validation
    /// error.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub web_links: Vec<PlanWebLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanInitiative {
    pub name: String,
    pub title: String,
    pub horizon: String,
    #[serde(default)]
    pub brief: Option<String>,
    #[serde(default)]
    pub target_month: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanEpic {
    pub name: String,
    pub title: String,
    /// A plan initiative's name, or an existing initiative's key.
    #[serde(default)]
    pub initiative: Option<String>,
    #[serde(default)]
    pub brief: Option<String>,
    #[serde(default)]
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanStory {
    pub name: String,
    pub title: String,
    /// story when absent.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// A plan epic's name, or an existing epic's key.
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub criteria: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub external_ref: Option<String>,
    /// The state of the migrated ticket: todo when absent. Any state needs `external_ref`; one other than
    /// todo is created in that state and logged `imported`, as the importer does, never `override`. Done is the
    /// importer's alone (it needs a resolution).
    #[serde(default)]
    pub delivery: Option<DeliveryState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanDecision {
    pub name: String,
    /// A plan item's name, or an existing item's key.
    pub on: String,
    pub title: String,
    pub decider: String,
    #[serde(default)]
    pub due: Option<chrono::NaiveDate>,
    /// One-way or two-way; absent leaves it unset (kb/ADR-0013 R1).
    #[serde(default)]
    pub reversibility: Option<Reversibility>,
}

/// `blocked_by` blocks `story`; each is a plan story's name or an existing story's key.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanBlock {
    pub story: String,
    pub blocked_by: String,
}

/// `from` follows, duplicates or relates to `to`: always the forward form. Each end is a plan item's name (an
/// initiative, an epic or a story) or an existing item's key or external reference. `type` is written as on the
/// wire (`relates_to`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanLink {
    pub from: String,
    #[serde(rename = "type")]
    pub link_type: ItemLinkType,
    pub to: String,
}

/// A titled URL on `on`, which is a plan item's name or an existing item's key or external reference. The title
/// and the URL are under the rule of `AddWebLink`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanWebLink {
    pub on: String,
    pub title: String,
    pub url: String,
}

/// What the plan came to: every item and decision by its local name, with `created` false when an earlier apply of
/// this plan had made it, and every blocker edge, item link and web link with `created` false when it was already there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlanApplied {
    pub items: Vec<PlannedItem>,
    pub decisions: Vec<PlannedDecision>,
    pub blocks: Vec<PlannedBlock>,
    /// One entry per `links` entry of the plan, in its order. Servers always send it; JSON without it (an older
    /// server) reads as none.
    #[serde(default)]
    #[schema(required = true)]
    pub links: Vec<PlannedLink>,
    /// One entry per `web_links` entry of the plan, in its order. Servers always send it; JSON without it (an older
    /// server) reads as none.
    #[serde(default)]
    #[schema(required = true)]
    pub web_links: Vec<PlannedWebLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlannedItem {
    pub name: String,
    pub item: ItemType,
    pub key: Key,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlannedDecision {
    pub name: String,
    pub id: i64,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlannedBlock {
    pub story: Key,
    pub blocked_by: Key,
    pub created: bool,
}

/// An item link the plan asked for, as stored: a `relates_to` has the lower key in `from`, whichever end the plan
/// named first. `created` is false when the link was already there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlannedLink {
    pub from: Key,
    pub from_level: ItemType,
    #[serde(rename = "type")]
    pub link_type: ItemLinkType,
    pub to: Key,
    pub to_level: ItemType,
    pub created: bool,
}

/// A web link the plan asked for, as stored: its `id` on the item (how one is removed), the title trimmed and the
/// URL in the URL parser's form. `created` is false when the item already had that URL under that title; the `id`
/// is then the one it had.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlannedWebLink {
    pub on: Key,
    pub level: ItemType,
    pub id: i64,
    pub title: String,
    pub url: String,
    pub created: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The CLI sends a plan by serialising this type. A plan that lists no item link must go out exactly as it did
    /// before the field existed, so a server one release older does not refuse it for an unknown field.
    #[test]
    fn a_plan_without_links_goes_out_as_before_and_a_link_travels_as_from_type_to() {
        let plain: ApplyPlan =
            serde_json::from_value(json!({"id": "p", "stories": [{"name": "s", "title": "S"}]}))
                .unwrap();
        let sent = serde_json::to_value(&plain).unwrap();
        assert!(
            sent.get("blocks").is_some(),
            "the control: an empty list that was always sent"
        );
        assert!(sent.get("links").is_none(), "{sent}");

        let linked: ApplyPlan = serde_json::from_value(json!({
            "id": "p",
            "links": [{"from": "a", "type": "relates_to", "to": "ABC-7"}]
        }))
        .unwrap();
        assert_eq!(linked.links[0].link_type, ItemLinkType::RelatesTo);
        assert_eq!(
            serde_json::to_value(&linked).unwrap()["links"],
            json!([{"from": "a", "type": "relates_to", "to": "ABC-7"}])
        );
        for bad in [
            json!({"id": "p", "links": [{"from": "a", "type": "blocks", "to": "b"}]}),
            json!({"id": "p", "links": [{"from": "a", "type": "relates-to", "to": "b"}]}),
            json!({"id": "p", "links": [{"from": "a", "kind": "follows", "to": "b"}]}),
            json!({"id": "p", "links": [{"from": "a", "type": "follows", "to": "b", "direction": "in"}]}),
            json!({"id": "p", "links": [{"from": "a", "type": "follows"}]}),
        ] {
            assert!(
                serde_json::from_value::<ApplyPlan>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_plan_without_web_links_goes_out_as_before_and_a_web_link_travels_as_on_title_url() {
        let plain: ApplyPlan =
            serde_json::from_value(json!({"id": "p", "stories": [{"name": "s", "title": "S"}]}))
                .unwrap();
        let sent = serde_json::to_value(&plain).unwrap();
        assert!(
            sent.get("blocks").is_some(),
            "the control: an empty list that was always sent"
        );
        assert!(sent.get("web_links").is_none(), "{sent}");

        let entry = json!({"on": "s", "title": "Scenario page", "url": "https://example.com/wiki"});
        let with: ApplyPlan =
            serde_json::from_value(json!({"id": "p", "web_links": [entry.clone()]})).unwrap();
        assert_eq!(
            serde_json::to_value(&with).unwrap()["web_links"],
            json!([entry])
        );
        for bad in [
            json!({"id": "p", "web_links": [{"on": "s", "title": "T", "url": "https://example.com/", "note": "x"}]}),
            json!({"id": "p", "web_links": [{"item": "s", "title": "T", "url": "https://example.com/"}]}),
            json!({"id": "p", "web_links": [{"on": "s", "url": "https://example.com/"}]}),
            json!({"id": "p", "weblinks": []}),
        ] {
            assert!(
                serde_json::from_value::<ApplyPlan>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }
}
