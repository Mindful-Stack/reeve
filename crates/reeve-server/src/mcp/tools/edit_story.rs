//! `edit_story`: `reeve story edit <KEY>` as a tool. Changes the planner.
use super::{Annotations, Tool, ToolError, bound};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{CriterionDto, EditItem, EditStoryInput, StoryReceipt};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<EditStoryInput>();
    input_schema["properties"]["key"]["description"] =
        json!(format!("The story's key, for example {key_prefix}-596."));
    Tool {
        name: "edit_story",
        title: "Edit a story",
        description: format!(
            "Change one story's title, kind, epic, description, acceptance criteria, owner or labels. Only the fields you send are changed; what a field held before is replaced. {} Read the story first (get_story) and send its `version`: if someone changed the story since, the edit is refused and you are told the current version. get_story shortens a long text and ends it with `[cut: N more characters]`. Such a text cannot be edited here: never send it back; leave that field out and tell the person to edit it in the CLI or the web client. To move a story between states use move_story, not this. Answers a receipt: the story's key, title, delivery state and new version.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It replaces what a field held (destructive). The same arguments again carry a version that is no longer
        // current and are refused, so a repeat changes nothing more (idempotent). Nothing outside this planner.
        annotations: Annotations {
            read_only: false,
            destructive: true,
            idempotent: true,
            open_world: false,
        },
        run,
    }
}

/// Criteria are replaced as a list, but a criterion whose text is unchanged at the same position keeps its done
/// flag. Must stay equal to the CLI's `replace_criteria` (`crates/reeve/src/commands/story.rs`): the CLI depends
/// on `reeve-protocol` only, so the two cannot share one function without moving behaviour into the protocol
/// crate.
fn merge_criteria(current: &[CriterionDto], texts: &[String]) -> Vec<CriterionDto> {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| CriterionDto {
            text: t.clone(),
            done: current
                .get(i)
                .is_some_and(|c| c.done && c.text.trim() == t.trim()),
        })
        .collect()
}

/// The story's labels in their order without the removed ones, then each added label it does not have yet; a
/// label both added and removed is removed. Must stay equal to the CLI's `edit_labels`
/// (`crates/reeve/src/commands/story.rs`), for the reason `merge_criteria` gives.
fn merge_labels(current: Vec<String>, add: &[String], remove: &[String]) -> Vec<String> {
    let mut out: Vec<String> = current
        .into_iter()
        .filter(|l| !remove.contains(l))
        .collect();
    for l in add {
        if !out.contains(l) && !remove.contains(l) {
            out.push(l.clone());
        }
    }
    out
}

/// Whether `text` ends with what `bound::cut` appends, `[cut: <n> more characters]`: a text this server
/// shortened when it was read. Written back, it would replace the real text with the cut one.
fn looks_cut(text: &str) -> bool {
    text.trim_end()
        .strip_suffix(" more characters]")
        .and_then(|rest| rest.rsplit_once("[cut: "))
        .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn run<'a>(
    state: &'a AppState,
    who: &'a Acting<'a>,
    args: Value,
) -> BoxFuture<'a, Result<Value, ToolError>> {
    Box::pin(async move {
        let input: EditStoryInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        let told = |message: &str| Err(ToolError::Told(message.to_string()));
        const CUT: &str = "ends with the marker of a text get_story cut. The whole text cannot be read here, so it cannot be replaced here: tell the person to edit it in the CLI or the web client.";
        if input.detach_epic && input.epic.is_some() {
            return told("detach_epic and epic cannot be sent together.");
        }
        if input.clear_owner && input.owner.is_some() {
            return told("clear_owner and owner cannot be sent together.");
        }
        if input.criteria.as_ref().is_some_and(Vec::is_empty) {
            return told(
                "criteria cannot be emptied here: send every criterion the story should have.",
            );
        }
        if input.description.as_deref().is_some_and(looks_cut) {
            return told(&format!("description {CUT}"));
        }
        if input.criteria.iter().flatten().any(|text| looks_cut(text)) {
            return told(&format!("A criterion {CUT}"));
        }
        let merges = input.criteria.is_some()
            || !input.add_labels.is_empty()
            || !input.remove_labels.is_empty();
        let changes = merges
            || input.title.is_some()
            || input.kind.is_some()
            || input.epic.is_some()
            || input.detach_epic
            || input.description.is_some()
            || input.owner.is_some()
            || input.clear_owner;
        if !changes {
            return told("Nothing to change: send at least one field besides key and version.");
        }
        let failed = |e: AppError| super::write_failed(e, &input.key, prefix);
        // The story as it is now, read only to merge into. The edit below still carries the caller's version, and
        // the handler checks it in its transaction: a change since the caller read the story is a conflict
        // whatever was read here.
        let (criteria, labels) = if merges {
            let current = actions::read_story(state, who, &input.key)
                .await
                .map_err(failed)?;
            (
                input
                    .criteria
                    .as_deref()
                    .map(|texts| merge_criteria(&current.criteria, texts)),
                (!input.add_labels.is_empty() || !input.remove_labels.is_empty())
                    .then(|| merge_labels(current.labels, &input.add_labels, &input.remove_labels)),
            )
        } else {
            (None, None)
        };
        let named_epic = input.epic.clone();
        let epic = super::key_ref("epic", input.epic, prefix)?;
        // `..EditItem::default()` on purpose, where `create_story` writes every field out: in an edit `None` means
        // "leave it alone", so what a chat may not send stays untouched by the default (the rank, the external
        // reference, attachments, and every field that is an epic's or an initiative's), and a field added to
        // `EditItem` is withheld until someone adds it here.
        let body = EditItem {
            version: input.version,
            title: input.title,
            description: input.description,
            kind: input.kind,
            epic: if input.detach_epic {
                Some(None)
            } else {
                epic.map(Some)
            },
            owner: if input.clear_owner {
                Some(None)
            } else {
                input.owner.map(Some)
            },
            criteria,
            labels,
            ..EditItem::default()
        };
        let story = actions::edit_story(state, who, &input.key, body)
            .await
            .map_err(|e| match (e, &named_epic) {
                // With an epic named, a missing item may be either.
                (AppError::NotFound, Some(epic)) => ToolError::Told(format!(
                    "No story with key {:?}, or no epic with key {:?}. Keys look like {prefix}-123.",
                    bound::echo(&input.key),
                    bound::echo(epic)
                )),
                (other, _) => failed(other),
            })?;
        super::sent(StoryReceipt::from(story))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn criterion(text: &str, done: bool) -> CriterionDto {
        CriterionDto {
            text: text.into(),
            done,
        }
    }

    /// The CLI's rule (`replace_criteria`): a criterion keeps its done flag when the same trimmed text stays at
    /// the same position, and only then.
    #[test]
    fn a_criterion_keeps_its_done_flag_only_at_the_same_position_with_the_same_text() {
        let current = [
            criterion("a slot can be picked", true),
            criterion("it is held", true),
            criterion("gone", true),
        ];
        let texts = [
            "  a slot can be picked ".to_string(),
            "it is held for ten minutes".to_string(),
        ];
        assert_eq!(
            merge_criteria(&current, &texts),
            vec![
                criterion("  a slot can be picked ", true),
                criterion("it is held for ten minutes", false)
            ]
        );
        // Moved to another position: not the same criterion any more.
        let swapped = ["it is held".to_string(), "a slot can be picked".to_string()];
        assert!(merge_criteria(&current, &swapped).iter().all(|c| !c.done));
        // A criterion that was not done stays not done.
        assert!(!merge_criteria(&[criterion("a", false)], &["a".to_string()])[0].done);
    }

    /// The CLI's rule (`edit_labels`): the current labels in their order without the removed ones, then each
    /// added label the story does not have; a label both added and removed is removed.
    #[test]
    fn labels_are_the_current_ones_less_the_removed_plus_the_new() {
        let current = vec!["web".to_string(), "api".to_string(), "old".to_string()];
        let s = |labels: &[&str]| labels.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        assert_eq!(
            merge_labels(current.clone(), &s(&["api", "new"]), &s(&["old"])),
            s(&["web", "api", "new"])
        );
        assert_eq!(
            merge_labels(current.clone(), &s(&["x"]), &s(&["x", "web"])),
            s(&["api", "old"])
        );
        assert_eq!(merge_labels(current.clone(), &[], &[]), current);
    }

    #[test]
    fn a_text_that_ends_with_the_cut_marker_is_recognised() {
        let mut cut = "å".repeat(bound::MAX_OTHER_TEXT + 5);
        bound::cut(&mut cut, bound::MAX_OTHER_TEXT);
        assert!(
            looks_cut(&cut),
            "what `bound::cut` really appends: {}",
            &cut[cut.len() - 30..]
        );
        assert!(looks_cut("text[cut: 1 more characters]  "));
        for whole in [
            "",
            "a description",
            "see [cut: 5 more characters] above",
            "[cut: many more characters]",
        ] {
            assert!(!looks_cut(whole), "{whole}");
        }
    }
}
