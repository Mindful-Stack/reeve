//! Text for an item's relations (kb/ADR-0031): the item link name as the command line spells it, the line that
//! tells a `follows` from a blocker, and the `relations:` block of `show`, `story context` and `link ls`.
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

/// What `reeve link ls` prints.
pub fn listing(v: &LinkListingDto) -> String {
    relations(&[], &v.links).unwrap_or_else(|| "no item links".into())
}

#[cfg(test)]
mod tests {
    use super::*;

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
