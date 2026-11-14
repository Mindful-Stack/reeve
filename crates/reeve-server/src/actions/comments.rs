use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_protocol::{AddComment, ItemType, Key};

/// What adding a comment answers, beside the thread its route sends: what a tool's receipt needs.
pub(crate) struct Commented {
    /// The comment's id: the new one, or the one its idempotency key already made (which may be on another item:
    /// a key names one comment across the instance).
    pub(crate) id: i64,
    /// Whether this call inserted it.
    pub(crate) created: bool,
}

/// `reeve comment add --item <KEY>`. Shared by `POST /api/v1/items/{key}/comments` and the MCP tool
/// `add_comment`. `key` and `b` are what the route takes: its path parameter (any item's key) and its body.
/// Answers the item as resolved (its type and key) and the comment; the route reads the thread from the first,
/// the tool builds its receipt from both.
pub(crate) async fn add_item_comment(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: AddComment,
) -> Result<((ItemType, Key), Commented), AppError> {
    // The key's space is the caller's to decide, never the body's.
    let idem = b
        .idempotency_key
        .as_deref()
        .map(|key| who.write_key(key))
        .transpose()?;
    let (item, key) = state.handlers.resolve_any(key).await?;
    let (id, created) = state
        .handlers
        .comment_item_attaching(
            &who.ctx,
            item,
            &key,
            &b.body,
            idem.as_deref(),
            &b.attachments,
        )
        .await?;
    Ok(((item, key), Commented { id, created }))
}

/// `reeve comment add --decision <ID>`. Shared by `POST /api/v1/decisions/{id}/comments` and the MCP tool
/// `add_comment`. `decision` and `b` are what the route takes: its path parameter and its body.
pub(crate) async fn add_decision_comment(
    state: &AppState,
    who: &Acting<'_>,
    decision: i64,
    b: AddComment,
) -> Result<Commented, AppError> {
    let idem = b
        .idempotency_key
        .as_deref()
        .map(|key| who.write_key(key))
        .transpose()?;
    let (id, created) = state
        .handlers
        .comment_decision_attaching(&who.ctx, decision, &b.body, idem.as_deref(), &b.attachments)
        .await?;
    Ok(Commented { id, created })
}
