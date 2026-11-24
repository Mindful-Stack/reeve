//! Text for an item's relations (kb/ADR-0031): the item link name as the command line spells it, the line that
//! tells a `follows` from a blocker, and the `relations:` and `web links:` blocks of `show`, `story context` and
//! `link ls`.
use crate::glyphs::{Level, mark};
use reeve_protocol::*;
use std::fmt::Write;

/// Clap's parser for an item link name. Blocking is not an item link (kb/ADR-0031 R2), so the words people try
/// for it are answered with where blocking lives.
pub fn parse_link_name(s: &str) -> Result<ItemLinkType, String> {
    if matches!(
        s,
        "blocks" | "blocked-by" | "blocked_by" | "depends-on" | "depends_on"
    ) {
        return Err(
            "blocking is not an item link: use `reeve story block <KEY> --by <OTHER>`".into(),
        );
    }
    ItemLinkType::from_cli(s).ok_or_else(|| {
        let [first, second, last] = ItemLinkType::ALL.map(ItemLinkType::cli);
        format!("`{s}` is not an item link name: use {first}, {second} or {last}")
    })
}

/// What `link add` says after a `follows` between two stories: the link is informational, and how to hold the
/// story back if that was meant.
pub fn not_a_blocker(story: &Key, other: &Key) -> String {
    format!(
        "Informational: this does not hold {story} back. To hold it back: `reeve story block {story} --by {other}`."
    )
}

/// A story's blocker as the block prints it: its key, and the story itself when the caller has it (`story
/// context` does; `story show` has only the keys of `blocked_by`).
pub struct BlockerLine<'a> {
    pub key: &'a Key,
    pub detail: Option<&'a StoryDto>,
}

const BLOCKED_BY: &str = "blocked by";

/// The `relations:` block: blockers first (stories only), under their own `blocked by` label, then the item links
/// in the order the server sent them. The block lists; it makes no claim about readiness. A blocker in testing or
/// done, or one of the same owner, is listed and holds nothing back (kb/ADR-0027 R5), and whether one currently
/// holds the story back is what the readiness reasons say (`story context`'s first lines), never this block: the
/// CLI derives no readiness. Labels are padded to the longest one shown. None when there is nothing to show. Every
/// key carries its level's glyph, like every key in text output.
pub fn relations(blockers: &[BlockerLine<'_>], links: &[ItemLinkDto]) -> Option<String> {
    if blockers.is_empty() && links.is_empty() {
        return None;
    }
    let width = links
        .iter()
        .map(|l| l.label.chars().count())
        .chain(blockers.iter().map(|_| BLOCKED_BY.len()))
        .max()
        .unwrap_or(0);
    let mut out = String::from("relations:");
    for b in blockers {
        let what = match b.detail {
            Some(s) => format!(
                "{} {} ({}, {})",
                mark(Level::of_kind(s.kind), b.key),
                s.title,
                crate::render::state_phrase(s.delivery),
                s.owner.as_deref().unwrap_or("no owner")
            ),
            None => mark(Level::Story, b.key),
        };
        let _ = write!(out, "\n  {BLOCKED_BY:<width$}  {what}");
    }
    for l in links {
        let _ = write!(
            out,
            "\n  {:<width$}  {} {}",
            l.label,
            mark(Level::of_item(l.item.level), &l.item.key),
            l.item.title
        );
    }
    Some(out)
}

/// What `reeve link ls` prints: the relations block, then the web links block.
pub fn listing(v: &LinkListingDto) -> String {
    let blocks: Vec<String> = [relations(&[], &v.links), web_links(&v.web_links)]
        .into_iter()
        .flatten()
        .collect();
    if blocks.is_empty() {
        "no item links or web links".into()
    } else {
        blocks.join("\n")
    }
}

/// The `web links:` block: one line per web link, title then URL, oldest first as the server sent them. None when
/// there are none.
pub fn web_links(links: &[WebLinkDto]) -> Option<String> {
    if links.is_empty() {
        return None;
    }
    let mut out = String::from("web links:");
    for l in links {
        let _ = write!(out, "\n  {}  {}", l.title, l.url);
    }
    Some(out)
}

/// Whether a URL as a person typed it names a stored web link. The stored form is the server's URL parser's
/// serialisation, so the typed one is parsed the same way first; text the parser refuses is compared as typed.
pub fn same_url(typed: &str, stored: &str) -> bool {
    let typed = typed.trim();
    typed == stored || url::Url::parse(typed).is_ok_and(|u| u.as_str() == stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web(id: i64, title: &str, url: &str) -> WebLinkDto {
        WebLinkDto {
            id,
            title: title.into(),
            url: url.into(),
        }
    }

    #[test]
    fn web_links_print_title_then_url_and_nothing_when_there_are_none() {
        assert_eq!(web_links(&[]), None);
        assert_eq!(
            web_links(&[
                web(1, "Scenario page", "https://example.com/wiki/scenario"),
                web(2, "Board", "http://example.com/board"),
            ])
            .as_deref(),
            Some(
                "web links:\n  Scenario page  https://example.com/wiki/scenario\n  Board  http://example.com/board"
            )
        );
    }

    #[test]
    fn a_typed_url_finds_its_stored_form() {
        let stored = "http://example.com/";
        for typed in [
            "http://example.com/",
            "HTTP://Example.COM",
            " http://example.com ",
            "http://example.com:80",
        ] {
            assert!(same_url(typed, stored), "{typed}");
        }
        for other in [
            "https://example.com/",
            "http://example.com/a",
            "example.com",
            "",
        ] {
            assert!(!same_url(other, stored), "{other}");
        }
    }

    #[test]
    fn the_listing_prints_both_blocks_or_says_there_is_nothing() {
        let follows = ItemLinkDto {
            link_type: ItemLinkType::Follows,
            direction: ItemLinkDirection::Out,
            label: "follows".into(),
            item: LinkedItemDto {
                key: "ABC-5".parse().unwrap(),
                level: ItemType::Epic,
                title: "Billing".into(),
            },
        };
        let both = LinkListingDto {
            links: vec![follows],
            web_links: vec![web(1, "Board", "http://example.com/board")],
        };
        assert_eq!(
            listing(&both),
            "relations:\n  follows  ■ ABC-5 Billing\nweb links:\n  Board  http://example.com/board"
        );
        let only_web = LinkListingDto {
            links: vec![],
            web_links: both.web_links.clone(),
        };
        assert_eq!(
            listing(&only_web),
            "web links:\n  Board  http://example.com/board"
        );
        let none = LinkListingDto {
            links: vec![],
            web_links: vec![],
        };
        assert_eq!(listing(&none), "no item links or web links");
    }

    #[test]
    fn a_name_is_spelled_with_a_hyphen_and_blocking_words_point_at_story_block() {
        assert_eq!(parse_link_name("follows"), Ok(ItemLinkType::Follows));
        assert_eq!(parse_link_name("duplicates"), Ok(ItemLinkType::Duplicates));
        assert_eq!(parse_link_name("relates-to"), Ok(ItemLinkType::RelatesTo));
        for blocking in [
            "blocks",
            "blocked-by",
            "blocked_by",
            "depends-on",
            "depends_on",
        ] {
            assert_eq!(
                parse_link_name(blocking),
                Err(
                    "blocking is not an item link: use `reeve story block <KEY> --by <OTHER>`"
                        .to_string()
                ),
                "{blocking}"
            );
        }
        for unknown in ["relates_to", "Follows", "precedes", ""] {
            assert_eq!(
                parse_link_name(unknown),
                Err(format!(
                    "`{unknown}` is not an item link name: use follows, duplicates or relates-to"
                )),
                "{unknown}"
            );
        }
    }

    #[test]
    fn the_informational_line_names_the_story_and_the_command_that_would_hold_it_back() {
        let (story, other): (Key, Key) = ("GRA-12".parse().unwrap(), "GRA-7".parse().unwrap());
        assert_eq!(
            not_a_blocker(&story, &other),
            "Informational: this does not hold GRA-12 back. To hold it back: `reeve story block GRA-12 --by GRA-7`."
        );
    }
}
