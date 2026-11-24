//! `reeve link`: item links (kb/ADR-0031), informational connections between two items of any level. A blocker
//! is not an item link: `reeve story block` holds a story back, nothing here does.
use super::finish;
use crate::client::{Client, Outcome};
use crate::relations::parse_link_name;
use clap::Subcommand;
use reeve_protocol::*;

#[derive(Subcommand)]
pub enum Link {
    /// Say that an item follows, duplicates or relates to another. Informational: it never holds work back (to
    /// hold a story back, use `reeve story block <KEY> --by <OTHER>`). Adding a link that exists succeeds and
    /// changes nothing.
    Add {
        /// The item that follows, duplicates or relates: an initiative, an epic or a story.
        key: KeyRef,
        /// follows, duplicates or relates-to.
        #[arg(value_parser = parse_link_name, value_name = "follows|duplicates|relates-to")]
        name: ItemLinkType,
        /// The other item, of any level.
        other: KeyRef,
    },
    /// Remove an item link, named as it was added: <KEY> is the item that follows, duplicates or relates. A line
    /// printed as "is followed by ABC-12" is removed with `reeve link rm ABC-12 follows <KEY>`. Removing a link
    /// that does not exist succeeds and changes nothing.
    Rm {
        /// The item that follows, duplicates or relates.
        key: KeyRef,
        /// follows, duplicates or relates-to.
        #[arg(value_parser = parse_link_name, value_name = "follows|duplicates|relates-to")]
        name: ItemLinkType,
        /// The other item.
        other: KeyRef,
    },
    /// List an item's item links, each read from that item's side.
    Ls {
        /// An initiative, an epic or a story.
        key: KeyRef,
    },
}

pub async fn run(c: &Client, cmd: Link, json: bool) -> anyhow::Result<i32> {
    Ok(match cmd {
        Link::Add { key, name, other } => {
            let out = c
                .post::<Vec<ItemLinkDto>>(
                    &format!("/items/{key}/links"),
                    serde_json::json!({"type": name, "to": other}),
                )
                .await?;
            let note = if !json && name == ItemLinkType::Follows && matches!(out, Outcome::Ok(_)) {
                not_a_blocker(c, &key, &other).await
            } else {
                None
            };
            finish(out, json, |_| {
                let line = format!("{key} {} {other}", name.label(ItemLinkDirection::Out));
                match &note {
                    Some(note) => format!("{line}\n{note}"),
                    None => line,
                }
            })
        }
        Link::Rm { key, name, other } => finish(
            c.delete::<Vec<ItemLinkDto>>(&format!("/items/{key}/links/{}/{other}", name.wire()))
                .await?,
            json,
            |_| {
                format!(
                    "removed: {key} {} {other}",
                    name.label(ItemLinkDirection::Out)
                )
            },
        ),
        Link::Ls { key } => finish(
            c.get::<LinkListingDto>(&format!("/items/{key}/links"))
                .await?,
            json,
            crate::relations::listing,
        ),
    })
}

/// What the two lookups behind the informational line may take together. The line is optional text after a write
/// that already succeeded, so it does not get the client's 30 seconds a request.
const NOTICE_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// The informational line for a `follows` that was just added, when both items are stories. Best effort: two
/// reads within `NOTICE_BUDGET` in total, and anything but two stories in that time (another level answers 404, a
/// failed request, a server that does not answer) gives no line and no error.
async fn not_a_blocker(c: &Client, key: &KeyRef, other: &KeyRef) -> Option<String> {
    let both = async {
        let story = story_key(c, key).await?;
        let other = story_key(c, other).await?;
        Some(crate::relations::not_a_blocker(&story, &other))
    };
    tokio::time::timeout(NOTICE_BUDGET, both)
        .await
        .ok()
        .flatten()
}

/// The native key of the story `r` names; None when it names no story.
async fn story_key(c: &Client, r: &KeyRef) -> Option<Key> {
    match c.get::<StoryDto>(&format!("/stories/{r}")).await {
        Ok(Outcome::Ok(story)) => Some(story.key),
        _ => None,
    }
}
