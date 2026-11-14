//! `move_story`: `reeve story mv <KEY>` as a tool. Changes the planner. It can never override the readiness gate
//! (kb/ADR-0029 R10): its input has no override reason, and it always passes none.
use super::{Annotations, Tool, ToolError, bound};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{MoveStory, MoveStoryInput, Reason, StoryReceipt};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<MoveStoryInput>();
    input_schema["properties"]["key"]["description"] =
        json!(format!("The story's key, for example {key_prefix}-596."));
    Tool {
        name: "move_story",
        title: "Move a story",
        description: format!(
            "Move one story to another delivery state: todo, in_progress, in_review, testing or done. {} Starting work (from todo or done into in_progress, in_review or testing) needs the story to be ready; if it is not, the move is refused, you are told why, and this tool cannot override that: a person can, in the CLI or the web client. Tell the person the reasons; do not look for another way to move it; fixing what the reasons name, when the person asks, is fine. Moving a story with no owner into work makes the person you act for its owner. Done needs a `resolution`. Send the story's `version` as you last read it. Answers a receipt: the story's key, title, new delivery state, resolution and version.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It changes the story's state and may set its owner (destructive: not only an addition). The same
        // arguments again carry a stale version, or name the state the story is in, and change nothing
        // (idempotent). Nothing outside this planner.
        annotations: Annotations {
            read_only: false,
            destructive: true,
            idempotent: true,
            open_world: false,
        },
        run,
    }
}

/// What the model is told when the planner refused a move: its reasons as `get_story` lists them, whether a
/// person could override (only when every reason is a readiness fact), and that nothing changed. The reasons are
/// one JSON list with a space after each comma between them, so `tools::bounded` sees each reason as a word of its
/// own and never cuts the list in its middle; a long list is cut here, leaving room for the sentences after it, so
/// the whole refusal is under `bound::MAX_OWN_TEXT` and always ends with them.
fn refused(key: &str, reasons: &[Reason]) -> String {
    let each: Vec<String> = reasons
        .iter()
        .map(|reason| serde_json::to_string(reason).unwrap_or_default())
        .collect();
    let mut listed = format!("[{}]", each.join(", "));
    bound::cut(&mut listed, bound::MAX_OWN_TEXT - 500);
    let not_ready = !reasons.is_empty() && reasons.iter().all(Reason::overridable);
    let (why, who_can) = if not_ready {
        (
            ": it is not ready",
            " A person can override this in the CLI or the web client; this tool cannot.",
        )
    } else {
        ("", "")
    };
    format!(
        "{} was not moved{why}. Reasons, as get_story lists them: {listed}.{who_can} Nothing changed.",
        bound::echo(key)
    )
}

fn run<'a>(
    state: &'a AppState,
    who: &'a Acting<'a>,
    args: Value,
) -> BoxFuture<'a, Result<Value, ToolError>> {
    Box::pin(async move {
        // `deny_unknown_fields`: a call that sends an override reason is refused here, before anything runs.
        let input: MoveStoryInput = super::arguments(args)?;
        // The route's body, every field written out. The override reason is always none: kb/ADR-0029 R10. (The
        // action refuses one from a caller that came through MCP, should an adapter ever pass it.)
        let body = MoveStory {
            version: input.version,
            to: input.to,
            resolution: input.resolution,
            override_reason: None,
        };
        let story = actions::move_story(state, who, &input.key, body)
            .await
            .map_err(|e| match e {
                AppError::Refused(reasons) => ToolError::Told(refused(&input.key, &reasons)),
                other => super::write_failed(other, &input.key, &state.config.key_prefix),
            })?;
        super::sent(StoryReceipt::from(story))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_says_a_person_can_override_only_when_every_reason_is_a_readiness_fact() {
        let key = "TST-5".parse().unwrap();
        assert_eq!(
            refused(
                "TST-5",
                &[
                    Reason::Untriaged,
                    Reason::AncestorDiscoveryIncomplete { item: key }
                ]
            ),
            "TST-5 was not moved: it is not ready. Reasons, as get_story lists them: [{\"reason\":\"untriaged\"}, {\"reason\":\"ancestor_discovery_incomplete\",\"item\":\"TST-5\"}]. A person can override this in the CLI or the web client; this tool cannot. Nothing changed."
        );
        for rule in [
            Reason::ResolutionRequired,
            Reason::CompletedNeedsActiveOrigin,
        ] {
            let said = refused("TST-5", std::slice::from_ref(&rule));
            assert!(
                !said.contains("override") && said.ends_with("Nothing changed."),
                "{said}"
            );
        }
        // A very long list of reasons is cut, not sent whole.
        let many: Vec<Reason> = (0..2_000)
            .map(|n| Reason::DecisionNoRecord {
                decision_id: n,
                title: "Which queue".repeat(5),
            })
            .collect();
        assert!(refused("TST-5", &many).chars().count() < bound::MAX_OWN_TEXT + 300);
        // Bounded as `call_tool` sends it, the refusal still ends with what the model must tell the person.
        let sent = super::super::bounded(refused("TST-5", &many));
        assert!(
            sent.ends_with("A person can override this in the CLI or the web client; this tool cannot. Nothing changed."),
            "{}",
            &sent[sent.len() - 300..]
        );
        assert!(
            sent.chars().count() <= bound::MAX_OWN_TEXT,
            "{}",
            sent.chars().count()
        );
    }

    /// Thirty blockers with an owner each make a list of reasons longer than the other-text bound, and no reason
    /// has a title to put a space in. Bounded as `call_tool` sends it, every blocker's key is still there.
    #[test]
    fn a_refusal_keeps_every_reason_of_a_long_list_without_titles() {
        let blockers: Vec<Reason> = (100..130)
            .map(|n| Reason::BlockedBy {
                story: format!("TST-{n}").parse().unwrap(),
                delivery: reeve_protocol::DeliveryState::InProgress,
                owner: Some("someone.with.a.long.name@example.com".into()),
            })
            .collect();
        let sent = super::super::bounded(refused("TST-5", &blockers));
        assert!(
            sent.chars().count() > bound::MAX_OTHER_TEXT,
            "the case is a long refusal"
        );
        for n in 100..130 {
            assert!(sent.contains(&format!("\"TST-{n}\"")), "TST-{n}: {sent}");
        }
        assert!(sent.ends_with("Nothing changed."), "{sent}");
    }
}
