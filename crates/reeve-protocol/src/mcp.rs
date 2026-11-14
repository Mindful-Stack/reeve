//! Tool inputs and results of the MCP endpoint (kb/ADR-0029 R12). A tool's arguments deserialize into its `…Input`
//! type, and its `inputSchema` is generated from that type; so an input type is flat (or uses `#[schema(inline)]`,
//! never a reference to another schema) and refuses unknown fields.
use crate::activity::StoryWithReadiness;
use crate::context::StoryContextDto;
use crate::enums::{DeliveryState, Kind, Resolution};
use crate::ids::Key;
use crate::items::{EpicDto, InitiativeDto, RequiredDecisionDto, SearchHit, StoryDto};
use crate::promise::PromisedFilter;
use crate::readiness::Reason;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `get_story`: read one story by key (`reeve story context <KEY>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StoryContextInput {
    /// The story's key.
    pub key: String,
}

/// What `get_story` answers: the API's story context with its comments bounded to the most recent ones and its
/// long text (the description, each comment's body) cut.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryContextResult {
    #[serde(flatten)]
    pub context: StoryContextDto,
    /// How many older comments were left out; 0 when every comment is sent.
    pub comments_omitted: usize,
}

/// `search_items`: find items by text (`reeve search <text>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchItemsInput {
    /// The text to find, 1 to 200 characters: a word or phrase from a title, brief or description, or a key.
    pub text: String,
}

/// What `search_items` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchItemsResult {
    /// At most 50 hits, best match first.
    pub hits: Vec<SearchHit>,
    /// How many of the at most 50 hits were left out to keep the answer small.
    pub omitted: usize,
}

/// `get_epic`: read one epic by key (`reeve epic show <KEY>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEpicInput {
    /// The epic's key.
    pub key: String,
}

/// One story in a list: what a person scans a backlog for, without the description, the criteria's text, the
/// pull requests or the nested marks. It carries the key, to read the story whole with `get_story`, and the
/// version, so a list is enough to prepare a move.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StorySummary {
    pub key: Key,
    pub title: String,
    pub kind: Kind,
    pub epic: Option<Key>,
    pub owner: Option<String>,
    pub delivery: DeliveryState,
    pub resolution: Option<Resolution>,
    pub labels: Vec<String>,
    pub version: i64,
    pub blocked_by: Vec<Key>,
    /// How many acceptance criteria are checked.
    pub criteria_done: usize,
    /// How many acceptance criteria there are.
    pub criteria_total: usize,
    pub ready: bool,
    /// Why the story is not ready; empty when it is.
    pub reasons: Vec<Reason>,
    /// Whether the story is shown as urgent, through its own mark or its epic's or initiative's.
    pub urgent: bool,
    /// The date the story is promised by (its own promise or the one it inherits from its epic), whatever the
    /// promise's state: kept, late and dropped ones included. The `promised` filter selects the open or overdue.
    pub promised_on: Option<NaiveDate>,
}

impl From<StoryWithReadiness> for StorySummary {
    fn from(row: StoryWithReadiness) -> Self {
        let StoryWithReadiness { story, readiness } = row;
        Self {
            criteria_done: story.criteria.iter().filter(|c| c.done).count(),
            criteria_total: story.criteria.len(),
            urgent: !story.urgent_sources.is_empty(),
            promised_on: story.promise.as_ref().map(|promise| promise.on),
            key: story.key,
            title: story.title,
            kind: story.kind,
            epic: story.epic,
            owner: story.owner,
            delivery: story.delivery,
            resolution: story.resolution,
            labels: story.labels,
            version: story.version,
            blocked_by: story.blocked_by,
            ready: readiness.ready,
            reasons: readiness.reasons,
        }
    }
}

// `list_stories` (`reeve story ls`). There is no `sort`: the answer is in backlog order. The doc comment is the
// schema's root description.
/// One page of the story list, filtered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListStoriesInput {
    /// Only stories in this epic: the epic's key. A key that names nothing lists nothing.
    pub epic: Option<String>,
    /// Only stories in this initiative's epics: the initiative's key. A key that names nothing lists nothing.
    pub initiative: Option<String>,
    /// Only stories owned by this person: an email address. Not together with `mine`.
    pub owner: Option<String>,
    /// Only stories owned by the person asking. With `open`, this is their plate.
    #[serde(default)]
    pub mine: bool,
    /// Only stories in this delivery state.
    #[schema(inline)]
    pub delivery: Option<DeliveryState>,
    /// Only items of this kind.
    #[schema(inline)]
    pub kind: Option<Kind>,
    /// Only stories with this label.
    pub label: Option<String>,
    /// Only stories whose title, description or external reference contains this text (any case, 1 to 200 characters); a key also finds that story.
    pub text: Option<String>,
    /// Only items with no epic that are not done: work nobody has placed yet.
    #[serde(default)]
    pub untriaged: bool,
    /// Only work that can be picked up: ready stories still in todo.
    #[serde(default)]
    pub ready: bool,
    /// Only stories that are not done.
    #[serde(default)]
    pub open: bool,
    /// Only stories shown as urgent.
    #[serde(default)]
    pub urgent: bool,
    /// Only stories serving the active sprint's goals: a goal id, `any` (some goal) or `none` (upkeep).
    pub goal: Option<String>,
    /// Only stories with a promise in this state: `this_sprint` (overdue, or due this sprint), `next_sprint`, `overdue`, or `any` (open or overdue). Kept, late and dropped promises never match.
    #[schema(inline)]
    pub promised: Option<PromisedFilter>,
    /// At most this many stories, 1 to 50; 25 when left out.
    #[schema(minimum = 1, maximum = 50)]
    pub limit: Option<u32>,
    /// Stories to skip first: the `next_offset` of the page before; 0 when left out.
    pub offset: Option<u32>,
}

/// What `list_stories` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListStoriesResult {
    /// One page, in backlog order: by delivery state, then rank.
    pub stories: Vec<StorySummary>,
    /// Where the next page starts when there may be more (this page is full, or rows were left out to keep the answer small); null on the last page.
    pub next_offset: Option<u32>,
}

/// One epic in a list: what tells epics apart, without the brief or the nested marks. It carries the key, to read
/// the epic whole with `get_epic`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EpicSummary {
    pub key: Key,
    pub title: String,
    pub initiative: Option<Key>,
    pub version: i64,
    /// Whether the epic's discovery is marked complete.
    pub discovery_complete: bool,
    /// The date the epic is promised by, when it is.
    pub promised_on: Option<NaiveDate>,
}

impl From<EpicDto> for EpicSummary {
    fn from(epic: EpicDto) -> Self {
        Self {
            key: epic.key,
            title: epic.title,
            initiative: epic.initiative,
            version: epic.version,
            discovery_complete: epic.discovery_complete.is_some(),
            promised_on: epic.promise.map(|promise| promise.fact.on),
        }
    }
}

/// `list_epics`: the epics, of one initiative when given (`reeve epic ls`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEpicsInput {
    /// Only the epics of this initiative: its key. A key that names nothing lists nothing.
    pub initiative: Option<String>,
}

/// What `list_epics` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListEpicsResult {
    /// At most 200 epics, oldest first.
    pub epics: Vec<EpicSummary>,
    /// How many more epics there are than `epics` lists, left out to keep the answer small; 0 when all are listed.
    pub omitted: usize,
}

/// One initiative in a list: what tells initiatives apart, without the brief or the nested marks. It carries the
/// key, to read the initiative whole with `get_initiative`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InitiativeSummary {
    pub key: Key,
    pub title: String,
    pub horizon: String,
    pub target_month: Option<String>,
    pub owner: Option<String>,
    pub version: i64,
    /// Whether the initiative's discovery is marked complete.
    pub discovery_complete: bool,
    /// The share of its stories completed, of those not dropped, 0 to 1; none when nothing counts yet.
    pub progress: Option<f32>,
}

impl From<InitiativeDto> for InitiativeSummary {
    fn from(initiative: InitiativeDto) -> Self {
        Self {
            key: initiative.key,
            title: initiative.title,
            horizon: initiative.horizon,
            target_month: initiative.target_month,
            owner: initiative.owner,
            version: initiative.version,
            discovery_complete: initiative.discovery_complete.is_some(),
            progress: initiative.progress,
        }
    }
}

/// `list_initiatives`: every initiative (`reeve initiative ls`). It takes no arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListInitiativesInput {}

/// What `list_initiatives` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListInitiativesResult {
    /// At most 200 initiatives, oldest first.
    pub initiatives: Vec<InitiativeSummary>,
    /// How many more initiatives there are than `initiatives` lists, left out to keep the answer small; 0 when all are listed.
    pub omitted: usize,
}

/// `get_initiative`: read one initiative by key (`reeve initiative show <KEY>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInitiativeInput {
    /// The initiative's key.
    pub key: String,
}

// `list_decisions` (`reeve decision ls`, without `--queue`). The doc comment is the schema's root description.
/// Required decisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDecisionsInput {
    /// Only the decisions on this item: the key of a story, an epic or an initiative. Closed ones are listed too. A key that names nothing lists nothing. Not together with `initiative`.
    pub on: Option<String>,
    /// Every decision under this initiative (on it, on its epics and on their stories): the initiative's key. Closed ones are listed too. A key that names nothing lists nothing. Not together with `on`.
    pub initiative: Option<String>,
    /// Only the decisions the person asking is the decider of.
    #[serde(default)]
    pub mine: bool,
}

/// What `list_decisions` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListDecisionsResult {
    /// At most 50 decisions, each whole except that a closed one's reason is cut when long.
    pub decisions: Vec<RequiredDecisionDto>,
    /// How many more decisions there are than `decisions` lists, left out to keep the answer small; 0 when all are listed.
    pub omitted: usize,
}

/// `get_sprint`: read one sprint, the current one when no id is given (`reeve sprint show [id]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSprintInput {
    /// The sprint's id. Left out: the current sprint, which is the active one, or the draft being planned when none is active. No tool lists earlier sprints: ask the person for the id.
    pub id: Option<i64>,
}

// `create_story` (`reeve story create`). The doc comment is the schema's root description.
/// A new story, bug or chore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateStoryInput {
    /// A key you make up for this one create, 1 to 100 characters, for example `slot-story-7f3k9`: end it with a few random characters so it never equals a key from an earlier conversation. Send the same key again only to retry the same create: a repeat writes nothing and answers the story the key already made.
    #[schema(min_length = 1, max_length = 100)]
    pub idempotency_key: String,
    /// A short title, at most 200 characters.
    pub title: String,
    /// The kind of work; a story when left out.
    #[schema(inline)]
    pub kind: Option<Kind>,
    /// The epic to place it in: the epic's key. Left out, the item is untriaged until someone places it.
    pub epic: Option<String>,
    /// A longer description.
    pub description: Option<String>,
    /// The acceptance criteria, one text each.
    #[serde(default)]
    pub criteria: Vec<String>,
    /// Labels.
    #[serde(default)]
    pub labels: Vec<String>,
    /// The owner's email address. Set it only when the person names an owner, and use the exact address.
    pub owner: Option<String>,
}

// `edit_story` (`reeve story edit`). The doc comment is the schema's root description.
/// A change to one story: only the fields sent are changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EditStoryInput {
    /// The story's key.
    pub key: String,
    /// The story's `version` as you last read it (get_story or list_stories). If the story changed since, the edit is refused and nothing is overwritten.
    pub version: i64,
    /// A new title.
    pub title: Option<String>,
    /// A new kind.
    #[schema(inline)]
    pub kind: Option<Kind>,
    /// Move the story to this epic: the epic's key. Not together with `detach_epic`.
    pub epic: Option<String>,
    /// True takes the story out of its epic, which makes it untriaged. Not together with `epic`.
    #[serde(default)]
    pub detach_epic: bool,
    /// A new description, replacing the whole text. Never send a text that ends with `[cut: N more characters]`.
    pub description: Option<String>,
    /// The story's acceptance criteria, replacing the list: send every criterion the story should have, not only a new one. A criterion keeps its done mark when its text and position are unchanged. Cannot be empty.
    pub criteria: Option<Vec<String>>,
    /// A new owner's email address. Set it only when the person names an owner. Not together with `clear_owner`.
    pub owner: Option<String>,
    /// True removes the owner. Not together with `owner`.
    #[serde(default)]
    pub clear_owner: bool,
    /// Labels to add to the ones the story has.
    #[serde(default)]
    pub add_labels: Vec<String>,
    /// Labels to remove from the ones the story has.
    #[serde(default)]
    pub remove_labels: Vec<String>,
}

/// What a tool that changes a story answers: enough to see that the change happened and to make the next one
/// (the key and the new version), and nothing whose size a person controls except the title, which the planner
/// limits to 200 characters. So an answer is small whatever was written, and reading the story whole is
/// `get_story`'s.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryReceipt {
    pub key: Key,
    pub title: String,
    pub delivery: DeliveryState,
    pub resolution: Option<Resolution>,
    pub version: i64,
    /// `create_story` only: whether this call created the story. False: its idempotency key had already made
    /// this story, and nothing was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<bool>,
    /// What the model should know about this answer, when there is something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl From<StoryDto> for StoryReceipt {
    fn from(story: StoryDto) -> Self {
        Self {
            key: story.key,
            title: story.title,
            delivery: story.delivery,
            resolution: story.resolution,
            version: story.version,
            created: None,
            note: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn story_context_input_takes_exactly_one_string_key() {
        let ok: StoryContextInput = serde_json::from_value(json!({"key": "TST-1"})).unwrap();
        assert_eq!(ok.key, "TST-1");
        for (bad, names) in [
            (json!({}), "key"),
            (json!({"key": "TST-1", "extra": 1}), "extra"),
            (json!({"key": 5}), "string"),
        ] {
            let e = serde_json::from_value::<StoryContextInput>(bad)
                .unwrap_err()
                .to_string();
            assert!(e.contains(names), "{e}");
        }
    }

    /// The summary's field names and derivations, pinned as literal JSON: a renamed field or a wrong derivation
    /// changes what every chat reads.
    #[test]
    fn an_epic_summary_is_these_six_fields() {
        let epic: EpicDto = serde_json::from_value(json!({
            "key": "TST-2", "title": "Booking flow", "initiative": "TST-1", "brief": "A long brief.",
            "discovery_complete": {"at": "2026-11-01T10:00:00Z", "by": "dan@example.com"},
            "external_ref": "JPD-7", "version": 3, "urgent_mark": null, "urgent_sources": [],
            "promise": {
                "fact": {"on": "2026-12-24", "by": "dan@example.com", "at": "2026-11-02T09:00:00Z", "to": null},
                "due": "later", "past": false
            }
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(EpicSummary::from(epic)).unwrap(),
            json!({
                "key": "TST-2", "title": "Booking flow", "initiative": "TST-1", "version": 3,
                "discovery_complete": true, "promised_on": "2026-12-24"
            })
        );
        // Neither derived field is a constant: no mark and no promise read as false and null.
        let bare: EpicDto = serde_json::from_value(json!({
            "key": "TST-3", "title": "Loose ends", "initiative": null, "brief": "",
            "discovery_complete": null, "external_ref": null, "version": 1
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(EpicSummary::from(bare)).unwrap(),
            json!({
                "key": "TST-3", "title": "Loose ends", "initiative": null, "version": 1,
                "discovery_complete": false, "promised_on": null
            })
        );
    }

    #[test]
    fn an_initiative_summary_is_these_eight_fields() {
        let initiative: InitiativeDto = serde_json::from_value(json!({
            "key": "TST-1", "title": "Bookings", "brief": "A long brief.", "horizon": "Now",
            "target_month": "2027-01", "owner": "dan@example.com", "discovery_complete": null,
            "progress": 0.5, "external_ref": "JPD-30", "version": 2
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(InitiativeSummary::from(initiative)).unwrap(),
            json!({
                "key": "TST-1", "title": "Bookings", "horizon": "Now", "target_month": "2027-01",
                "owner": "dan@example.com", "version": 2, "discovery_complete": false, "progress": 0.5
            })
        );
        // The derived field is not a constant, and absent values stay null.
        let marked: InitiativeDto = serde_json::from_value(json!({
            "key": "TST-9", "title": "Later", "brief": "", "horizon": "Later", "target_month": null,
            "owner": null, "discovery_complete": {"at": "2026-11-01T10:00:00Z", "by": "dan@example.com"},
            "progress": null, "external_ref": null, "version": 4
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(InitiativeSummary::from(marked)).unwrap(),
            json!({
                "key": "TST-9", "title": "Later", "horizon": "Later", "target_month": null,
                "owner": null, "version": 4, "discovery_complete": true, "progress": null
            })
        );
    }

    /// The one summary the design asks to be pinned as literal JSON: comparing a tool with its route through
    /// `StorySummary::from` on both sides would prove nothing about the conversion.
    #[test]
    fn a_story_summary_is_these_sixteen_fields() {
        let row: StoryWithReadiness = serde_json::from_value(json!({
            "story": {
                "key": "TST-7", "title": "Pick a slot", "kind": "bug", "epic": "TST-2",
                "description": "A long description.",
                "criteria": [
                    {"text": "a", "done": true}, {"text": "b", "done": false}, {"text": "c", "done": true}
                ],
                "owner": "bob@example.com", "labels": ["web"], "rank": 4, "blocked_by": ["TST-5"],
                "delivery": "in_review", "resolution": null, "external_ref": "SCRUM-9", "version": 6,
                "prs": [], "goals": [], "urgent_mark": null,
                "urgent_sources": [{"key": "TST-2", "item": "epic"}],
                "promise": {
                    "own": null, "epic": null, "on": "2026-12-01", "holder": "own", "state": "open",
                    "due": "this_sprint", "days": null
                }
            },
            "readiness": {"ready": false, "reasons": [{"reason": "no_criteria"}]}
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(StorySummary::from(row)).unwrap(),
            json!({
                "key": "TST-7", "title": "Pick a slot", "kind": "bug", "epic": "TST-2",
                "owner": "bob@example.com", "delivery": "in_review", "resolution": null,
                "labels": ["web"], "version": 6, "blocked_by": ["TST-5"],
                "criteria_done": 2, "criteria_total": 3,
                "ready": false, "reasons": [{"reason": "no_criteria"}],
                "urgent": true, "promised_on": "2026-12-01"
            })
        );
        // No derived field is a constant: a done, ready story with no criteria, no urgent source and no promise.
        let plain: StoryWithReadiness = serde_json::from_value(json!({
            "story": {
                "key": "TST-8", "title": "Tidy the footer", "kind": "chore", "epic": null,
                "description": "", "criteria": [], "owner": null, "labels": [], "rank": 0,
                "blocked_by": [], "delivery": "done", "resolution": "dropped", "external_ref": null,
                "version": 1, "prs": []
            },
            "readiness": {"ready": true, "reasons": []}
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(StorySummary::from(plain)).unwrap(),
            json!({
                "key": "TST-8", "title": "Tidy the footer", "kind": "chore", "epic": null,
                "owner": null, "delivery": "done", "resolution": "dropped",
                "labels": [], "version": 1, "blocked_by": [],
                "criteria_done": 0, "criteria_total": 0,
                "ready": true, "reasons": [],
                "urgent": false, "promised_on": null
            })
        );
    }

    /// A receipt's field names, pinned as literal JSON: `created` and `note` are sent only when set.
    #[test]
    fn a_story_receipt_is_these_fields() {
        let story: crate::items::StoryDto = serde_json::from_value(json!({
            "key": "TST-7", "title": "Pick a slot", "kind": "story", "epic": "TST-2",
            "description": "A long description.", "criteria": [{"text": "a slot can be picked", "done": false}],
            "owner": "bob@example.com", "labels": ["web"], "rank": 3, "blocked_by": [], "delivery": "in_progress",
            "resolution": null, "external_ref": null, "version": 4, "prs": [], "goals": [],
            "urgent_mark": null, "urgent_sources": []
        }))
        .unwrap();
        let mut receipt = StoryReceipt::from(story);
        assert_eq!(
            serde_json::to_value(&receipt).unwrap(),
            json!({"key": "TST-7", "title": "Pick a slot", "delivery": "in_progress", "resolution": null, "version": 4})
        );
        receipt.created = Some(false);
        receipt.note = Some("Nothing was written.".into());
        assert_eq!(
            serde_json::to_value(&receipt).unwrap(),
            json!({"key": "TST-7", "title": "Pick a slot", "delivery": "in_progress", "resolution": null,
                   "version": 4, "created": false, "note": "Nothing was written."})
        );
    }

    #[test]
    fn create_story_input_refuses_what_the_tool_withholds() {
        let ok: CreateStoryInput =
            serde_json::from_value(json!({"idempotency_key": "k1", "title": "Pick a slot"}))
                .unwrap();
        assert_eq!((ok.kind, ok.criteria.len(), ok.labels.len()), (None, 0, 0));
        for withheld in ["external_ref", "attachments", "rank", "delivery"] {
            let mut call = json!({"idempotency_key": "k1", "title": "Pick a slot"});
            call[withheld] = json!("x");
            let e = serde_json::from_value::<CreateStoryInput>(call)
                .unwrap_err()
                .to_string();
            assert!(e.contains(withheld), "{e}");
        }
    }
}
