//! Tool inputs and results of the MCP endpoint (kb/ADR-0029 R12). A tool's arguments deserialize into its `…Input`
//! type, and its `inputSchema` is generated from that type; so an input type is flat (or uses `#[schema(inline)]`,
//! never a reference to another schema) and refuses unknown fields.
use crate::context::StoryContextDto;
use crate::ids::Key;
use crate::items::{EpicDto, SearchHit};
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
    /// How many more hits were found than `hits` lists, left out to keep the answer small; 0 when all are listed.
    pub omitted: usize,
}

/// `get_epic`: read one epic by key (`reeve epic show <KEY>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEpicInput {
    /// The epic's key.
    pub key: String,
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

/// `get_initiative`: read one initiative by key (`reeve initiative show <KEY>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInitiativeInput {
    /// The initiative's key.
    pub key: String,
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
}
