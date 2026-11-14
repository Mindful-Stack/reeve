//! Tool inputs and results of the MCP endpoint (kb/ADR-0029 R12). A tool's arguments deserialize into its `…Input`
//! type, and its `inputSchema` is generated from that type; so an input type is flat (or uses `#[schema(inline)]`,
//! never a reference to another schema) and refuses unknown fields.
use crate::context::StoryContextDto;
use crate::items::SearchHit;
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
}
