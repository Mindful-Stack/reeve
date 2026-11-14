//! Actions: everything between the caller check and the response, shared by an API route and its MCP tool, so the
//! two run the same code and the same validation (kb/ADR-0029 R1). Every action has one shape:
//!
//! ```text
//! async fn <name>(state: &AppState, who: &Acting<'_>, <the input its route takes>) -> Result<<Dto>, AppError>
//! ```
//!
//! It answers what its route sends. A create answers a pair, the DTO and whether this call created it
//! (`create_story`: `(StoryDto, bool)`): the route sends the DTO, and the tool's receipt needs the flag.
//!
//! The input is what the route takes (its path, its query struct or its body), not the tool's type: the route
//! passes what it parsed, and a tool is an adapter that turns its own narrower input into the route's and calls the
//! same action. A check that sits in a route function moves into the action when that route gets a tool.
mod decisions;
mod epics;
mod initiatives;
mod search;
mod sprints;
mod stories;

use crate::state::AppState;
use chrono::{DateTime, Utc};
pub(crate) use decisions::{
    DecisionQuery, decision_dto, decision_scope, list_decisions, resolve_on,
};
pub(crate) use epics::{EpicQuery, get_epic, list_epics};
pub(crate) use initiatives::{get_initiative, list_initiatives};
use reeve_identity::AccessPolicy;
use reeve_protocol::{RequiredDecisionDto, Via};
pub(crate) use search::{SearchQuery, search};
pub(crate) use sprints::{get_sprint, list_sprints};
pub(crate) use stories::{
    StoryQuery, create_story, edit_story, list_stories, move_story, read_story, story_context,
};

/// What an action may need to know about the caller: planning's context (actor, channel, the request's one
/// timestamp, and the client when the call came through MCP) and the access policy the request was authenticated with, which owner validation needs and which
/// cannot be recovered from an input.
///
/// `policy` is private to `actions`, so an `Acting` is built in one place, `Acting::new`.
pub(crate) struct Acting<'a> {
    pub(crate) ctx: reeve_planning::Ctx,
    policy: &'a AccessPolicy,
}

impl<'a> Acting<'a> {
    /// The one constructor: `Caller::acting` and `McpCaller::acting` both call it, so the two cannot differ.
    pub(crate) fn new(
        email: &str,
        via: Via,
        now: DateTime<Utc>,
        policy: &'a AccessPolicy,
        client: Option<String>,
    ) -> Self {
        Self {
            ctx: reeve_planning::Ctx {
                actor: email.to_string(),
                via,
                now,
                client,
            },
            policy,
        }
    }
}

/// Sets `browse_url` on an already-built decision DTO: `<repo url>/blob/HEAD/<path>`, nothing for a repository
/// without an `https://` URL.
pub(crate) fn browse(s: &AppState, dto: &mut RequiredDecisionDto) {
    if let Some(r) = dto.record.as_mut() {
        r.browse_url = s.repo_urls.get(&r.repo).map(|base| {
            let path = r
                .path
                .trim_start_matches('/')
                .split('/')
                .map(|seg| urlencoding::encode(seg).into_owned())
                .collect::<Vec<_>>()
                .join("/");
            format!("{base}/blob/HEAD/{path}")
        });
    }
}

/// An optional key input (a parent, a filter, `before`) resolved through `Handlers::resolve_ref`.
pub(crate) async fn resolve_opt(
    h: &reeve_planning::Handlers,
    r: Option<&reeve_protocol::KeyRef>,
    item: reeve_protocol::ItemType,
) -> Result<Option<reeve_protocol::Key>, reeve_planning::HandlerError> {
    match r {
        Some(r) => Ok(Some(h.resolve_ref(r.as_str(), item).await?)),
        None => Ok(None),
    }
}

/// An edit's parent: `None` leaves it alone, `Some(None)` clears it, `Some(Some(r))` resolves `r`.
pub(crate) async fn resolve_edit(
    h: &reeve_planning::Handlers,
    r: Option<Option<reeve_protocol::KeyRef>>,
    item: reeve_protocol::ItemType,
) -> Result<Option<Option<reeve_protocol::Key>>, reeve_planning::HandlerError> {
    match r {
        Some(r) => Ok(Some(resolve_opt(h, r.as_ref(), item).await?)),
        None => Ok(None),
    }
}

/// A list filter naming an item. A filter is a query, not an address: a reference that matches nothing (an external
/// reference not imported yet) selects nothing, exactly as an unknown instance key does, so it is `Nothing` (200, empty
/// list) and not `NotFound`. Path parameters and body fields keep 404, where naming a missing item is an error.
pub(crate) enum FilterRef {
    Unset,
    Is(reeve_protocol::Key),
    Nothing,
}

pub(crate) async fn resolve_filter(
    h: &reeve_planning::Handlers,
    r: Option<&reeve_protocol::KeyRef>,
    item: reeve_protocol::ItemType,
) -> Result<FilterRef, reeve_planning::HandlerError> {
    match resolve_opt(h, r, item).await {
        Ok(Some(k)) => Ok(FilterRef::Is(k)),
        Ok(None) => Ok(FilterRef::Unset),
        Err(reeve_planning::HandlerError::NotFound) => Ok(FilterRef::Nothing),
        Err(e) => Err(e),
    }
}
