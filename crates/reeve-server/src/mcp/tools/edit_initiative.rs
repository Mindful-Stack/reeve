//! `edit_initiative`: `reeve initiative edit <KEY>` as a tool. Changes the planner. It cannot reach the discovery
//! mark: the input has no such field, and the edit the action runs carries the stored mark over.
use super::{Annotations, Tool, ToolError};
use crate::{
    actions::{self, Acting, Edited},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_planning::Initiative;
use reeve_protocol::{EditInitiativeInput, EditItem, InitiativeDto, InitiativeReceipt};
use serde_json::{Value, json};

/// The receipt of an edit: the initiative as this call wrote it (its title and version), never as the read after
/// the commit shows it, which another writer may have changed since: a chat that sent that later version with the
/// brief it read before would overwrite a change it never saw.
fn receipt(edited: Edited<InitiativeDto, Initiative>) -> InitiativeReceipt {
    InitiativeReceipt::from(InitiativeDto::from(&edited.written))
}

pub(super) fn tool(key_prefix: &str, horizons: &[String]) -> Tool {
    let mut input_schema = super::input_schema::<EditInitiativeInput>();
    input_schema["properties"]["key"]["description"] = json!(format!(
        "The initiative's key, for example {key_prefix}-12."
    ));
    input_schema["properties"]["horizon"]["description"] = json!(if horizons.is_empty() {
        "A new horizon (how far off the team plans this initiative). This planner has no horizons configured."
            .to_string()
    } else {
        format!(
            "A new horizon (how far off the team plans this initiative), exactly one of: {}.",
            horizons.join(", ")
        )
    });
    Tool {
        name: "edit_initiative",
        title: "Edit an initiative",
        description: format!(
            "Change one initiative's title, brief, horizon, target month or owner. Only the fields you send are changed; what a field held before is replaced. {} Read the initiative first (get_initiative) and send its `version`: if someone changed the initiative since, the edit is refused and you are told the current version. get_initiative shortens a long brief and ends it with `[cut: N more characters]`. Such a brief cannot be edited here: never send it back; leave it out and tell the person to edit it in the CLI or the web client. This tool cannot mark an initiative's discovery complete or reopen it: a person does that, in the CLI or the web client. Answers a receipt: the initiative's key, title and new version. To see the rest, use get_initiative.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It replaces what a field held (destructive). The same arguments again carry a stale version and are
        // refused (idempotent). Nothing outside this planner.
        annotations: Annotations {
            read_only: false,
            destructive: true,
            idempotent: true,
            open_world: false,
        },
        run,
    }
}

fn run<'a>(
    state: &'a AppState,
    who: &'a Acting<'a>,
    args: Value,
) -> BoxFuture<'a, Result<Value, ToolError>> {
    Box::pin(async move {
        let input: EditInitiativeInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        let told = |message: &str| Err(ToolError::Told(message.to_string()));
        if input.clear_owner && input.owner.is_some() {
            return told("clear_owner and owner cannot be sent together.");
        }
        if input.clear_target_month && input.target_month.is_some() {
            return told("clear_target_month and target_month cannot be sent together.");
        }
        if input.brief.as_deref().is_some_and(super::looks_cut) {
            return told(
                "brief ends with the marker of a text get_initiative cut. The whole text cannot be read here, so it cannot be replaced here: tell the person to edit it in the CLI or the web client.",
            );
        }
        let changes = input.title.is_some()
            || input.brief.is_some()
            || input.horizon.is_some()
            || input.target_month.is_some()
            || input.clear_target_month
            || input.owner.is_some()
            || input.clear_owner;
        if !changes {
            return told("Nothing to change: send at least one field besides key and version.");
        }
        // `..EditItem::default()` on purpose, as in `edit_story`: what a chat may not send stays untouched.
        let body = EditItem {
            version: input.version,
            title: input.title,
            brief: input.brief,
            horizon: input.horizon,
            target_month: if input.clear_target_month {
                Some(None)
            } else {
                input.target_month.map(Some)
            },
            owner: if input.clear_owner {
                Some(None)
            } else {
                input.owner.map(Some)
            },
            ..EditItem::default()
        };
        let edited = actions::edit_initiative(state, who, &input.key, body)
            .await
            .map_err(|e| {
                super::item_write_failed(e, "initiative", "get_initiative", &input.key, prefix)
            })?;
        super::sent(receipt(edited))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reeve_protocol::Key;

    /// Another caller retitles the initiative between this edit's commit and the read after it: the read shows their
    /// title and version. The receipt must report this edit's ("Mine", version 2). Catches a receipt built from the
    /// read after the commit.
    #[test]
    fn the_receipt_reports_the_initiative_this_edit_wrote_not_a_later_read() {
        let at = |title: &str, version| {
            Initiative::from_persisted(
                "TST-1".parse::<Key>().unwrap(),
                title.into(),
                "brief".into(),
                "Now".into(),
                None,
                None,
                None,
                None,
                version,
            )
        };
        let edited = Edited {
            read: InitiativeDto::from(&at("Theirs", 3)),
            written: at("Mine", 2),
        };
        let r = receipt(edited);
        assert_eq!((r.title.as_str(), r.version), ("Mine", 2));
    }
}
