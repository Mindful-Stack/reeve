use crate::{
    enums::*,
    ids::{Key, KeyRef},
    items::CriterionDto,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Every create request carries a caller-supplied idempotency key; a repeat returns the same item.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateInitiative {
    pub idempotency_key: String,
    pub title: String,
    pub brief: Option<String>,
    pub horizon: String,
    pub target_month: Option<String>,
    pub owner: Option<String>,
    /// The item's origin elsewhere, e.g. the Jira key it was migrated from (`JPD-30`). It names one item across all
    /// three types, and a later import of the same reference finds this item instead of creating another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateEpic {
    pub idempotency_key: String,
    pub title: String,
    pub initiative: Option<KeyRef>,
    pub brief: Option<String>,
    /// The item's origin elsewhere, e.g. the Jira key it was migrated from (`JPD-30`). It names one item across all
    /// three types, and a later import of the same reference finds this item instead of creating another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateStory {
    pub idempotency_key: String,
    pub title: String,
    pub kind: Kind,
    pub epic: Option<KeyRef>,
    pub description: Option<String>,
    pub criteria: Vec<String>,
    pub labels: Vec<String>,
    /// The item's origin elsewhere, e.g. the Jira key it was migrated from (`JPD-30`). It names one item across all
    /// three types, and a later import of the same reference finds this item instead of creating another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
    /// The story's owner (email); it must be an allowed user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EditItem {
    pub version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A story's kind (story, bug, chore); None leaves it alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<String>,
    /// Some(None) clears the target month; None leaves it alone.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_month: Option<Option<String>>,
    /// Some(None) clears the owner; None leaves it alone.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub owner: Option<Option<String>>,
    /// Some(None) clears the epic; None leaves it alone.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub epic: Option<Option<KeyRef>>,
    /// An epic's parent: Some(None) clears the initiative; None leaves it alone.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub initiative: Option<Option<KeyRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Vec<crate::items::CriterionDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<i32>,
    /// Records the item's origin elsewhere once (the Jira key a hand-made item was migrated from), under the same rules
    /// as on create; the reference it already has again is no change, and an existing one is never changed or cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MoveStory {
    pub version: i64,
    pub to: DeliveryState,
    pub resolution: Option<Resolution>,
    pub override_reason: Option<String>,
}

/// Places a todo story before `before` (another todo story), or at the end of the todo column when `before` is absent.
/// The server renumbers the column's ranks 0..n-1 in one write; only the moved story's version changes, because rank is
/// ordering metadata, not content an editor guards on.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankStory {
    pub version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<KeyRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RequireDecision {
    pub idempotency_key: String,
    pub on: KeyRef,
    pub title: String,
    pub decider: String,
    pub due: Option<chrono::NaiveDate>,
    /// One-way or two-way; absent leaves it unset (kb/ADR-0013 R1).
    #[serde(default)]
    pub reversibility: Option<Reversibility>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LinkRecord {
    pub repo: String,
    pub path: String,
    pub record_id: String,
    pub pr_url: Option<String>,
}

/// The check names the record it checked and the decision version it observed, so an out-of-order check never overwrites a relink; the server refuses a result for a reference that has since changed (kb/ADR-0004 R3).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RecordCheckResult {
    pub record_id: String,
    pub version: i64,
    pub check: CheckResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CloseDecision {
    /// `decided`: the reason says what was decided; `withdrawn`: why it is no longer needed (kb/ADR-0013 R3).
    pub kind: ClosureKind,
    pub reason: String,
}

/// Edits a required decision's planner-side fields (kb/ADR-0004 R2). The record reference is set with `link`, the check
/// result by the checker; neither is accepted here, and any field not listed is refused rather than ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EditDecision {
    pub version: i64,
    /// `null` leaves the title unchanged, like absent (as in `EditItem`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `null` leaves the decider unchanged, like absent (as in `EditItem`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decider: Option<String>,
    /// Some(None) clears the due date; None leaves it alone.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub due: Option<Option<chrono::NaiveDate>>,
    /// Some(None) clears the classification; None leaves it alone.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub reversibility: Option<Option<Reversibility>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AddComment {
    pub body: String,
    /// Caller-chosen key that makes a retried comment safe: a repeat returns the thread with the first comment only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DiscoveryMark {
    pub version: i64,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BlockEdge {
    pub blocker: KeyRef,
}

/// A Jira initiative imported in its current state. `external_ref` is the Jira key and the idempotency
/// key: importing the same reference again creates nothing.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportInitiative {
    pub external_ref: String,
    pub title: String,
    pub brief: Option<String>,
    pub horizon: String,
    pub target_month: Option<String>,
    pub owner: Option<String>,
    /// Marks discovery complete, by the importing actor at import time.
    pub discovery_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportEpic {
    pub external_ref: String,
    pub title: String,
    pub initiative: Option<Key>,
    pub brief: Option<String>,
    pub discovery_complete: bool,
}

/// A story created directly in its delivery state: no gate is consulted and no override is logged.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportStory {
    pub external_ref: String,
    pub title: String,
    pub kind: Kind,
    pub epic: Option<Key>,
    pub description: Option<String>,
    pub criteria: Vec<CriterionDto>,
    pub owner: Option<String>,
    pub labels: Vec<String>,
    pub delivery: DeliveryState,
    /// Required exactly when `delivery` is done.
    pub resolution: Option<Resolution>,
}

/// A Jira comment, kept with its original author and date.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportComment {
    pub external_ref: String,
    pub story: Key,
    pub author: String,
    pub body: String,
    pub at: DateTime<Utc>,
}

/// `created` is false when the external reference already existed (create-only, re-runnable). Items carry `key`,
/// comments carry `id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Imported {
    pub key: Option<Key>,
    pub id: Option<i64>,
    pub created: bool,
}

mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer, T: Serialize>(
        v: &Option<Option<T>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match v {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_item_epic_three_states_roundtrip() {
        let omitted = EditItem {
            version: 1,
            ..Default::default()
        };
        let j = serde_json::to_value(&omitted).unwrap();
        assert!(j.get("epic").is_none(), "an unchanged epic is not sent");
        let titled = EditItem {
            version: 1,
            title: Some("x".into()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&titled).unwrap(),
            r#"{"version":1,"title":"x"}"#
        );
        let owner: EditItem = serde_json::from_str(r#"{"version":1,"owner":null}"#).unwrap();
        assert_eq!(owner.owner, Some(None));
        let back: EditItem = serde_json::from_value(j).unwrap();
        assert_eq!(back.epic, None);
        let cleared: EditItem = serde_json::from_str(r#"{"version":1,"epic":null}"#).unwrap();
        assert_eq!(cleared.epic, Some(None));
        assert_eq!(
            serde_json::to_value(&cleared).unwrap()["epic"],
            serde_json::Value::Null
        );
        let set: EditItem = serde_json::from_str(r#"{"version":1,"epic":"ABC-2"}"#).unwrap();
        assert_eq!(set.epic, Some(Some("ABC-2".parse().unwrap())));
        let ini: EditItem = serde_json::from_str(r#"{"version":1,"initiative":"ABC-1"}"#).unwrap();
        assert_eq!(ini.initiative, Some(Some("ABC-1".parse().unwrap())));
        assert_eq!(ini.epic, None, "an epic's parent is never read from `epic`");
        let no_ini: EditItem = serde_json::from_str(r#"{"version":1,"initiative":null}"#).unwrap();
        assert_eq!(no_ini.initiative, Some(None));
    }

    #[test]
    fn edit_decision_due_has_three_states_and_unknown_fields_are_refused() {
        let omitted: EditDecision = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!(omitted.due, None);
        assert_eq!(
            serde_json::to_string(&omitted).unwrap(),
            r#"{"version":1}"#,
            "an unchanged due date is not sent"
        );
        let cleared: EditDecision = serde_json::from_str(r#"{"version":1,"due":null}"#).unwrap();
        assert_eq!(cleared.due, Some(None));
        // The whole object, not `["due"]`: indexing a missing key also yields Null, so an omitted clear would pass.
        assert_eq!(
            serde_json::to_value(&cleared).unwrap(),
            serde_json::json!({"version": 1, "due": null})
        );
        let set: EditDecision =
            serde_json::from_str(r#"{"version":1,"due":"2026-11-30"}"#).unwrap();
        assert_eq!(set.due, Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 30)));
        let null_title: EditDecision =
            serde_json::from_str(r#"{"version":1,"title":null}"#).unwrap();
        assert_eq!(null_title.title, None);
        for field in ["record_id", "check", "pr_url", "checked_at", "closed", "on"] {
            assert!(
                serde_json::from_str::<EditDecision>(&format!(r#"{{"version":1,"{field}":"x"}}"#))
                    .is_err(),
                "{field} is record data and must be refused, not ignored"
            );
        }
    }
}
