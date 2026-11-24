//! `get_epic`: `reeve epic show <KEY>` as a tool. Read-only.
use super::{
    Tool, ToolError,
    bound::{MAX_OWN_TEXT, bound_links, cut, emptied, sent_bytes},
};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{EpicDto, GetEpicInput, GetEpicResult};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<GetEpicInput>();
    input_schema["properties"]["key"]["description"] =
        json!(format!("The epic's key, for example {key_prefix}-20."));
    input_schema["properties"]["key"]["examples"] = json!([format!("{key_prefix}-20")]);
    Tool {
        name: "get_epic",
        title: "Read an epic",
        description: format!(
            "Read one epic by key (for example {key_prefix}-20): its title, brief, the initiative it belongs to, whether its discovery is complete (stories under an epic still in discovery are not ready), its promise if it has one, whether it is urgent, and its version. Use it when the person asks what an epic is about or why its stories are held. Its item links to other items are in `links`, each read from this epic's side with its wording in `label` (informational, they hold nothing back); at most 50 are sent, fewer when the epic itself is large, and `links_omitted` is the number left out. Its stories are not included: list them with list_stories and `epic`. {} Read-only. {}",
            super::CUT_TEXT,
            super::PEOPLES_TEXT
        ),
        input_schema,
        annotations: super::READ_ONLY,
        run,
    }
}

fn run<'a>(
    state: &'a AppState,
    who: &'a Acting<'a>,
    args: Value,
) -> BoxFuture<'a, Result<Value, ToolError>> {
    Box::pin(async move {
        let input: GetEpicInput = super::arguments(args)?;
        let epic = actions::get_epic(state, who, &input.key)
            .await
            .map_err(|e| super::not_found(e, "epic", &input.key, &state.config.key_prefix))?;
        super::sent(bounded(epic))
    })
}

/// The epic as the tool answers it. The epic is the item asked for, so its brief is its own text, cut at
/// `MAX_OWN_TEXT`. In a function of its own, like `get_story`'s, so the bound is tested on an epic built in
/// memory.
fn bounded(mut epic: EpicDto) -> GetEpicResult {
    cut(&mut epic.brief, MAX_OWN_TEXT);
    // Last: the answer with no link sent and all of them counted (its baseline), then the rows fitted into what
    // that leaves of one answer.
    let mut links = epic.links.take();
    let (no_links, all_links) = emptied(&links);
    epic.links = no_links;
    let mut result = GetEpicResult {
        epic,
        links_omitted: all_links,
    };
    result.links_omitted = bound_links(&mut links, sent_bytes(&result));
    result.epic.links = links;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property of Task 9 for `get_epic`: for any stored epic the answer is within the limit of one answer. A
    /// brief of the maximum length it is sent at, in four-byte characters (40 000 bytes), and 200 item links whose
    /// titles JSON escapes sixfold.
    #[test]
    fn an_epic_at_its_longest_with_200_item_links_fits_one_answer() {
        let escaped = "\u{1}".repeat(200);
        let links: Vec<Value> = (2..=201)
            .map(|n| {
                json!({"type": "relates_to", "direction": "out", "label": "relates to",
                       "item": {"key": format!("TST-{n}"), "level": "story", "title": escaped}})
            })
            .collect();
        let epic: EpicDto = serde_json::from_value(json!({
            "key": "TST-1", "title": "An epic", "initiative": null, "brief": "𝄞".repeat(MAX_OWN_TEXT),
            "discovery_complete": null, "external_ref": null, "version": 1, "links": links
        }))
        .unwrap();
        let limit = crate::mcp::tools::MAX_SENT;
        let sent = bounded(epic);
        assert_eq!(
            sent.epic.brief.len(),
            4 * MAX_OWN_TEXT,
            "the control: the brief is whole, 40 000 bytes"
        );
        let kept = sent.epic.links.as_ref().unwrap();
        assert!(!kept.is_empty() && kept.len() < 50, "{} sent", kept.len());
        assert_eq!(
            kept.len() + sent.links_omitted,
            200,
            "every link is either sent or counted"
        );
        assert_eq!(kept[0].item.key.to_string(), "TST-2", "the first are kept");
        let bytes = serde_json::to_value(&sent).unwrap().to_string().len();
        assert!(bytes <= limit, "{bytes} bytes");
        assert!(
            bytes > limit - 2_000,
            "and the room is used, to within one link: {bytes} bytes"
        );
    }
}
