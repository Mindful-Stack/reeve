use super::finish;
use crate::client::Client;
use clap::Subcommand;
use reeve_protocol::*;

#[derive(Subcommand)]
pub enum Comment {
    /// Add a comment to an initiative, an epic or a story (--item, its key says which) or a required decision (--decision);
    /// prints the thread. A repeated --idempotency-key adds nothing.
    Add {
        /// Caller-chosen key that makes a retried comment safe.
        #[arg(long)]
        idempotency_key: String,
        /// Initiative, epic or story to comment on (`--story` is an alias).
        #[arg(
            long,
            alias = "story",
            conflicts_with = "decision",
            required_unless_present = "decision"
        )]
        item: Option<KeyRef>,
        /// Decision id to comment on (decisions are numbered, not keyed).
        #[arg(long)]
        decision: Option<i64>,
        /// The comment text.
        #[arg(long, allow_hyphen_values = true)]
        body: String,
    },
    /// List the comments on an initiative, an epic or a story (--item) or a required decision (--decision).
    Ls {
        /// Initiative, epic or story to list comments of (`--story` is an alias).
        #[arg(
            long,
            alias = "story",
            conflicts_with = "decision",
            required_unless_present = "decision"
        )]
        item: Option<KeyRef>,
        /// Decision id to list comments of.
        #[arg(long)]
        decision: Option<i64>,
    },
}

fn base(item: Option<KeyRef>, decision: Option<i64>) -> String {
    match (item, decision) {
        (Some(k), _) => format!("/items/{k}/comments"),
        (None, Some(id)) => format!("/decisions/{id}/comments"),
        (None, None) => unreachable!("clap requires one of --item, --decision"),
    }
}

fn render_all(cs: &[CommentDto]) -> String {
    cs.iter()
        .map(crate::render::comment)
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn run(c: &Client, cmd: Comment, json: bool) -> anyhow::Result<i32> {
    Ok(match cmd {
        Comment::Add {
            idempotency_key,
            item,
            decision,
            body,
        } => finish(
            c.post::<Vec<CommentDto>>(
                &base(item, decision),
                serde_json::to_value(AddComment {
                    body,
                    idempotency_key: Some(idempotency_key),
                })?,
            )
            .await?,
            json,
            |cs| render_all(cs),
        ),
        Comment::Ls { item, decision } => finish(
            c.get::<Vec<CommentDto>>(&base(item, decision)).await?,
            json,
            |cs| render_all(cs),
        ),
    })
}
