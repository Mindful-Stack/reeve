//! `reeve link`: item links and web links (kb/ADR-0031). An item link is an informational connection between two
//! items of any level; a web link is a titled URL on one item (`--url`, `--title`). Neither is a blocker:
//! `reeve story block` holds a story back, nothing here does.
use super::finish;
use crate::client::{Client, Outcome};
use crate::relations::parse_link_name;
use clap::Subcommand;
use reeve_protocol::*;

#[derive(Subcommand)]
pub enum Link {
    /// Say that an item follows, duplicates or relates to another, or, with --url and --title, attach a web link
    /// to it. Both are informational: neither holds work back (to hold a story back, use `reeve story block <KEY>
    /// --by <OTHER>`). Adding what is already there succeeds and changes nothing; a URL the item already has under
    /// another title is refused (remove it first).
    Add {
        /// The item: for an item link, the one that follows, duplicates or relates. An initiative, an epic or a
        /// story.
        key: KeyRef,
        /// follows, duplicates or relates-to.
        #[arg(
            value_parser = parse_link_name,
            value_name = "follows|duplicates|relates-to",
            required_unless_present = "url"
        )]
        name: Option<ItemLinkType>,
        /// The other item, of any level.
        #[arg(required_unless_present = "url")]
        other: Option<KeyRef>,
        /// Attach a web link instead: an http or https URL.
        #[arg(long, conflicts_with_all = ["name", "other"], requires = "title")]
        url: Option<String>,
        /// The web link's title, 1 to 200 characters.
        #[arg(long, requires = "url")]
        title: Option<String>,
    },
    /// Remove an item link, named as it was added: <KEY> is the item that follows, duplicates or relates. A line
    /// printed as "is followed by ABC-12" is removed with `reeve link rm ABC-12 follows <KEY>`. With --url, remove
    /// the item's web link to that URL. Removing what is not there succeeds and changes nothing.
    Rm {
        /// The item: for an item link, the one that follows, duplicates or relates.
        key: KeyRef,
        /// follows, duplicates or relates-to.
        #[arg(
            value_parser = parse_link_name,
            value_name = "follows|duplicates|relates-to",
            required_unless_present = "url"
        )]
        name: Option<ItemLinkType>,
        /// The other item.
        #[arg(required_unless_present = "url")]
        other: Option<KeyRef>,
        /// Remove the web link with this URL instead.
        #[arg(long, conflicts_with_all = ["name", "other"])]
        url: Option<String>,
    },
    /// List an item's item links, each read from that item's side, and its web links.
    Ls {
        /// An initiative, an epic or a story.
        key: KeyRef,
    },
}

/// The item's web links as text, for the answers of `add --url` and `rm --url`.
#[allow(clippy::ptr_arg)] // `finish` hands its renderer exactly `&Vec<_>`
fn web_links_text(links: &Vec<WebLinkDto>) -> String {
    crate::relations::web_links(links).unwrap_or_else(|| "no web links".into())
}

pub async fn run(c: &Client, cmd: Link, json: bool) -> anyhow::Result<i32> {
    Ok(match cmd {
        Link::Add {
            key,
            name: Some(name),
            other: Some(other),
            ..
        } => {
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
        Link::Add {
            key,
            url: Some(url),
            title: Some(title),
            ..
        } => finish(
            c.post::<Vec<WebLinkDto>>(
                &format!("/items/{key}/web-links"),
                serde_json::json!({"title": title, "url": url}),
            )
            .await?,
            json,
            web_links_text,
        ),
        Link::Rm {
            key,
            name: Some(name),
            other: Some(other),
            ..
        } => finish(
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
        Link::Rm {
            key,
            url: Some(url),
            ..
        } => {
            // The web link is removed by id; its id is found by its URL in the item's listing.
            let listed = c
                .get::<LinkListingDto>(&format!("/items/{key}/links"))
                .await?;
            match listed {
                Outcome::Ok(v) => {
                    let found = v
                        .web_links
                        .iter()
                        .find(|w| crate::relations::same_url(&url, &w.url))
                        .map(|w| w.id);
                    match found {
                        Some(id) => finish(
                            c.delete::<Vec<WebLinkDto>>(&format!("/items/{key}/web-links/{id}"))
                                .await?,
                            json,
                            |left| format!("web link removed\n{}", web_links_text(left)),
                        ),
                        // Removing one that does not exist succeeds and changes nothing.
                        None => finish(Outcome::Ok(v.web_links), json, |_| {
                            "no web link with that URL: nothing removed".to_string()
                        }),
                    }
                }
                other => finish(other, json, |_| String::new()),
            }
        }
        // Clap's rules (each form complete, the two exclusive) leave no other shape.
        Link::Add { .. } | Link::Rm { .. } => {
            anyhow::bail!("give <NAME> <OTHER>, or --url (with --title to add)")
        }
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
/// stories, both named by their native keys. The two items are taken from the server's own answers, never from a
/// resolution of the CLI's: `links` is the add's answer (the item links of `key`, so the item it now follows is
/// in it with its level), and one read of the other item's links, through the same route family and so the same
/// resolution as the write (`/items/{other}/links`), names the item that follows it. A typed lookup such as
/// `/stories/{key}` would not do: it can match a story's external reference where the write linked an epic with
/// that native key.
///
/// Best effort: the read happens only when the item followed is a story, within `NOTICE_BUDGET`, and anything
/// else in that time (another level, a failed request, a server that does not answer, an end not named by its own
/// key in the answers) gives no line and no error.
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
/// `direction`'s side; `named` is how the command line named that other item. Only the entry whose key is exactly
/// `named` (a native key always wins over an external reference when the server resolves, so that is the item the
/// write used); anything else is a guess and gives None. Not "the only candidate": the read after the write is a
/// separate request, so another caller may have removed the link in between and left an unrelated item as the only
/// one. So a link added by naming an item through its external reference gets no line, which is accepted: the line
/// is optional, and a wrong `story block` suggestion is worse than none.
fn followed_end<'a>(
    links: &'a [ItemLinkDto],
    direction: ItemLinkDirection,
    named: &KeyRef,
) -> Option<&'a LinkedItemDto> {
    links
        .iter()
        .filter(|l| l.link_type == ItemLinkType::Follows && l.direction == direction)
        .map(|l| &l.item)
        .find(|i| i.key.to_string() == named.as_str())
}
