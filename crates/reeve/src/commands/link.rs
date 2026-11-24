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
            let note = match &out {
                Outcome::Ok(links) if !json && name == ItemLinkType::Follows => {
                    not_a_blocker(c, &key, &other, links).await
                }
                _ => None,
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

/// What the lookup behind the informational line may take. The line is optional text after a write that already
/// succeeded, so it does not get the client's 30 seconds a request.
const NOTICE_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// The informational line for a `follows` that was just added, when both items of the link that was written are
/// stories, named by their native keys. The two items are taken from the server's own answers, never from a
/// resolution of the CLI's: `links` is the add's answer (the item links of `key`, so the item it now follows is
/// in it with its level), and one read of the other item's links, through the same route family and so the same
/// resolution as the write (`/items/{other}/links`), names the item that follows it. A typed lookup such as
/// `/stories/{key}` would not do: it can match a story's external reference where the write linked an epic with
/// that native key.
///
/// Best effort: the read happens only when the item followed is a story, within `NOTICE_BUDGET`, and anything
/// else in that time (another level, a failed request, a server that does not answer, an end that cannot be told
/// from the answers) gives no line and no error.
async fn not_a_blocker(
    c: &Client,
    key: &KeyRef,
    other: &KeyRef,
    links: &[ItemLinkDto],
) -> Option<String> {
    let to = followed_end(links, ItemLinkDirection::Out, other)?;
    if to.level != ItemType::Story {
        return None;
    }
    let path = format!("/items/{other}/links");
    let read = c.get::<LinkListingDto>(&path);
    let Ok(Ok(Outcome::Ok(listing))) = tokio::time::timeout(NOTICE_BUDGET, read).await else {
        return None;
    };
    let from = followed_end(&listing.links, ItemLinkDirection::In, key)?;
    (from.level == ItemType::Story).then(|| crate::relations::not_a_blocker(&from.key, &to.key))
}

/// The item at the other end of the `follows` link that was just written, among one item's links read from
/// `direction`'s side; `named` is how the command line named that other item. It is the one whose native key is
/// `named` (a native key always wins over an external reference when the server resolves, so that is the item the
/// write used), or else the only candidate (the link was just written, so it is among them). With several
/// candidates and a name that is none of their keys (an external reference), nothing tells them apart: None.
fn followed_end<'a>(
    links: &'a [ItemLinkDto],
    direction: ItemLinkDirection,
    named: &KeyRef,
) -> Option<&'a LinkedItemDto> {
    let ends: Vec<&LinkedItemDto> = links
        .iter()
        .filter(|l| l.link_type == ItemLinkType::Follows && l.direction == direction)
        .map(|l| &l.item)
        .collect();
    match ends.iter().find(|i| i.key.to_string() == named.as_str()) {
        Some(end) => Some(end),
        None if ends.len() == 1 => Some(ends[0]),
        None => None,
    }
}
