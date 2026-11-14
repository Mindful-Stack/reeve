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

use crate::{error::AppError, state::AppState};
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
/// timestamp, and the client when the call came through MCP) and the access policy the request was authenticated
/// with, which owner validation needs and which cannot be recovered from an input.
///
/// `policy` is private to `actions`, so an `Acting` is built in one place, `Acting::new`. A caller with no client
/// is an API caller (the API, the CLI, the web client): `write_key` reads it so.
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

/// What the idempotency key of every MCP caller starts with. Reserved: `write_key` refuses it to an API caller.
const MCP_KEYS: &str = "mcp:";

impl Acting<'_> {
    /// The idempotency key a write is stored under. Keys are unique across the instance and blind to who sent
    /// them, and a chat model invents short ones (`create-story-1`), so an MCP caller's key is stored as
    /// `mcp:<the caller's email>:<key>`: one person's chats share a key space, and no other person's chat can get
    /// their item back. An API caller's key is stored as it is, and may not start with `mcp:`, so that space cannot
    /// be written from the API either. An MCP caller is the one with a client (`McpCaller::acting`); a caller with
    /// no client is an API caller.
    ///
    /// The stored key is unambiguous only while the email has no colon (`a@example.com` with key `x:k` would
    /// otherwise equal `a@example.com:x` with key `k`). Identity does not forbid one in a domain, so no key is
    /// built for such an address; the sign-in providers cannot produce one.
    pub(crate) fn write_key(&self, raw: &str) -> Result<String, AppError> {
        if self.ctx.client.is_none() {
            return if raw.starts_with(MCP_KEYS) {
                Err(AppError::Validation(
                    "an idempotency key must not start with mcp: (reserved for MCP clients)".into(),
                ))
            } else {
                Ok(raw.to_string())
            };
        }
        if !(1..=100).contains(&raw.chars().count()) || raw.chars().any(char::is_control) {
            return Err(AppError::Validation(
                "idempotency_key must be 1 to 100 characters, without control characters".into(),
            ));
        }
        let email = reeve_protocol::normalise_email(&self.ctx.actor);
        if email.contains(':') {
            return Err(AppError::Validation(
                "an idempotency key cannot be kept for an email address that contains a colon: it would not be this person's alone".into(),
            ));
        }
        Ok(format!("{MCP_KEYS}{email}:{raw}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> AccessPolicy {
        AccessPolicy::floor_only(&reeve_identity::IdentityConfig {
            allowed_domains: vec!["example.com".into()],
            allowed_emails: vec![],
            admins: vec![],
        })
    }

    /// The key `email` stores `raw` under, through `client` (none: an API caller).
    fn stored(email: &str, client: Option<&str>, raw: &str) -> Result<String, String> {
        let policy = policy();
        let who = Acting::new(
            email,
            Via::Interactive,
            Utc::now(),
            &policy,
            client.map(str::to_string),
        );
        who.write_key(raw).map_err(|e| match e {
            AppError::Validation(message) => message,
            other => panic!("not a validation error: {other:?}"),
        })
    }

    fn ok(key: &str) -> Result<String, String> {
        Ok(key.to_string())
    }

    fn refused(message: &str) -> Result<String, String> {
        Err(message.to_string())
    }

    #[test]
    fn an_mcp_callers_key_is_stored_under_the_callers_email_and_an_api_callers_as_it_is() {
        assert_eq!(
            stored("Dan@Example.com", Some("claude.ai"), "k1"),
            ok("mcp:dan@example.com:k1")
        );
        assert_eq!(stored("dan@example.com", None, "k1"), ok("k1"));
        // An API caller's key has no length rule of this function's, and may not be in the MCP callers' space.
        assert_eq!(
            stored("dan@example.com", None, ""),
            ok(""),
            "the handler's rule, not this one's"
        );
        assert_eq!(
            stored("dan@example.com", None, "mcp:bob@example.com:k1"),
            refused("an idempotency key must not start with mcp: (reserved for MCP clients)")
        );
        // An MCP caller may send any text of 1 to 100 characters, a leading `mcp:` included: it is stored inside
        // the caller's own space.
        assert_eq!(
            stored(
                "dan@example.com",
                Some("claude.ai"),
                "mcp:bob@example.com:k1"
            ),
            ok("mcp:dan@example.com:mcp:bob@example.com:k1")
        );
        let rule =
            refused("idempotency_key must be 1 to 100 characters, without control characters");
        for bad in [String::new(), "k".repeat(101), "k\n1".to_string()] {
            assert_eq!(stored("dan@example.com", Some("claude.ai"), &bad), rule);
        }
        assert!(stored("dan@example.com", Some("claude.ai"), &"å".repeat(100)).is_ok());
    }

    /// Two people can never produce one stored key. With a colon in an email the format would be ambiguous
    /// (`a@example.com` + `x:k` against `a@example.com:x` + `k`), so no key is built for such an address.
    #[test]
    fn no_key_is_built_for_an_email_with_a_colon_so_two_people_cannot_collide() {
        let one = stored("a@example.com", Some("claude.ai"), "x:k");
        assert_eq!(one, ok("mcp:a@example.com:x:k"));
        let other = stored("a@example.com:x", Some("claude.ai"), "k");
        assert_ne!(other, one, "the collision");
        assert_eq!(
            other,
            refused(
                "an idempotency key cannot be kept for an email address that contains a colon: it would not be this person's alone"
            )
        );
    }
}
