//! Item links and web links (kb/ADR-0031): the three item link names with their spellings and their wording for
//! each side, as one table, and the wire types that carry an item link and a web link. The server, the CLI and
//! (through `label` on the wire) the web client all take the names from here.
use crate::{
    enums::ItemType,
    ids::{Key, KeyRef},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The name of an item link (kb/ADR-0031 R1), stored and sent as the forward word. None of them holds work back
/// (R2); a name is added only by an observation on that record. Declared in display order: `follows`,
/// `duplicates`, `relates_to`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ItemLinkType {
    Follows,
    Duplicates,
    RelatesTo,
}

/// The side an item link is read from. `out`: the item shown is the one that follows, duplicates or relates.
/// `in`: it is the other one. `relates_to` has no direction and is always `out`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ItemLinkDirection {
    Out,
    In,
}

/// One row of the name table.
struct Name {
    link_type: ItemLinkType,
    /// On the wire and in the database.
    wire: &'static str,
    /// On the command line.
    cli: &'static str,
    /// Read from the `from` item.
    out: &'static str,
    /// Read from the `to` item.
    inward: &'static str,
    /// Whether the link has a direction: `from` and `to` are different roles. Stated here, never derived from the
    /// wording: it decides how a link is stored (an undirected one once, lower key first), and the wording may
    /// change without that changing.
    directed: bool,
}

/// The one table of names, in `ItemLinkType`'s declaration order: `ItemLinkType::name` indexes it, and a test holds the
/// two in step.
const NAMES: [Name; 3] = [
    Name {
        link_type: ItemLinkType::Follows,
        wire: "follows",
        cli: "follows",
        out: "follows",
        inward: "is followed by",
        directed: true,
    },
    Name {
        link_type: ItemLinkType::Duplicates,
        wire: "duplicates",
        cli: "duplicates",
        out: "duplicates",
        inward: "is duplicated by",
        directed: true,
    },
    Name {
        link_type: ItemLinkType::RelatesTo,
        wire: "relates_to",
        cli: "relates-to",
        out: "relates to",
        inward: "relates to",
        directed: false,
    },
];

impl ItemLinkType {
    pub const ALL: [ItemLinkType; 3] = [
        ItemLinkType::Follows,
        ItemLinkType::Duplicates,
        ItemLinkType::RelatesTo,
    ];

    fn name(self) -> &'static Name {
        &NAMES[self as usize]
    }
    /// The spelling on the wire and in the database (`relates_to`).
    pub fn wire(self) -> &'static str {
        self.name().wire
    }
    /// The spelling on the command line (`relates-to`).
    pub fn cli(self) -> &'static str {
        self.name().cli
    }
    /// Display text for one side ("is followed by"). Behaviour depends on the type and the direction, never on
    /// this text.
    pub fn label(self, direction: ItemLinkDirection) -> &'static str {
        match direction {
            ItemLinkDirection::Out => self.name().out,
            ItemLinkDirection::In => self.name().inward,
        }
    }
    pub fn from_wire(s: &str) -> Option<Self> {
        NAMES.iter().find(|n| n.wire == s).map(|n| n.link_type)
    }
    pub fn from_cli(s: &str) -> Option<Self> {
        NAMES.iter().find(|n| n.cli == s).map(|n| n.link_type)
    }
    /// Whether the link has a direction, from the name table's own column. `relates_to` has none: it is stored
    /// once, whichever end named it, and the table's check holds the same rule (a test in `reeve-planning` holds
    /// the two in step).
    pub fn directed(self) -> bool {
        self.name().directed
    }
}

/// What a client says after an item link was added, when there is something to say: for a `follows` between two
/// stories, that it is informational (kb/ADR-0031, Consequences: "the clients say so when a `follows` is added
/// between two stories"); for every other name and every other pair of levels, nothing. `from_level` and
/// `to_level` are the levels of the two items as the server resolved them, and `story` is the key of the item that
/// follows, which the sentence names. The condition and the wording live here once, for the CLI and the MCP tool;
/// each adds after the sentence what its own reader can do next. The web client does not call this: it keeps its
/// own copy of the sentence in `web/src/lib/components/Relations.svelte` and of the condition in `followsNote`
/// (`web/src/lib/relations.ts`), which that component calls; a change here is made there too.
pub fn follows_is_informational(
    link_type: ItemLinkType,
    from_level: ItemType,
    to_level: ItemType,
    story: &Key,
) -> Option<String> {
    (link_type == ItemLinkType::Follows
        && from_level == ItemType::Story
        && to_level == ItemType::Story)
        .then(|| format!("Informational: this does not hold {story} back."))
}

/// The words people try for blocking where an item link name goes (kb/ADR-0031 R2), in the command line's spelling
/// and the wire's. The one list: the CLI's name parser of `reeve link`, `reeve apply`'s reading of a plan file and
/// the two MCP item link tools all ask here, and each answers one of these words with where blocking lives for its
/// own reader. No item link name is among them, and a test holds that.
pub fn is_blocking_word(s: &str) -> bool {
    matches!(
        s,
        "blocks" | "blocked-by" | "blocked_by" | "depends-on" | "depends_on"
    )
}

/// The item at the other end of an item link. No delivery state on purpose: a state beside "follows" would invite
/// reading the link as a gate (kb/ADR-0031 R3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LinkedItemDto {
    pub key: Key,
    pub level: ItemType,
    pub title: String,
}

/// An item link as one of its two items reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ItemLinkDto {
    #[serde(rename = "type")]
    pub link_type: ItemLinkType,
    pub direction: ItemLinkDirection,
    /// Display text for this side ("is followed by"), from the name table.
    pub label: String,
    pub item: LinkedItemDto,
}

/// `POST /items/{key}/links`: the item in the path follows, duplicates or relates to `to`. Always the forward
/// form; there is no way to write the reverse wording. Any other field is refused rather than ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddItemLink {
    #[serde(rename = "type")]
    pub link_type: ItemLinkType,
    /// A key or an external reference of an item of any level.
    pub to: KeyRef,
}

/// A web link (kb/ADR-0031 R5): a titled URL on one item. `id` names it for removal; an item's web links come
/// oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WebLinkDto {
    pub id: i64,
    pub title: String,
    /// As stored: the URL parser's serialisation of what was sent (`HTTP://Example.com` is `http://example.com/`).
    pub url: String,
}

/// `POST /items/{key}/web-links`. The title is 1 to 200 characters after trimming, on one line, with no tab or
/// other control character; the URL is `http` or `https` and at most 1 000 characters as stored. Any other field is refused
/// rather than ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddWebLink {
    pub title: String,
    pub url: String,
}

/// `GET /items/{key}/links`: what `reeve link ls` shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LinkListingDto {
    pub links: Vec<ItemLinkDto>,
    /// The item's web links, oldest first. Servers always send it; JSON without it (an older server) reads as
    /// none.
    #[serde(default)]
    #[schema(required = true)]
    pub web_links: Vec<WebLinkDto>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ItemLinkDirection::{In, Out};
    use ItemLinkType::{Duplicates, Follows, RelatesTo};
    use serde_json::json;

    /// The five words, each spelling written out, and nothing near them: the list is exact, as the command line has
    /// always read it. No item link name is a blocking word, in either spelling.
    #[test]
    fn the_blocking_words_are_these_five_and_no_item_link_name() {
        for word in [
            "blocks",
            "blocked-by",
            "blocked_by",
            "depends-on",
            "depends_on",
        ] {
            assert!(is_blocking_word(word), "{word}");
        }
        for other in ["Blocks", "block", "depends on", "precedes", " blocks", ""] {
            assert!(!is_blocking_word(other), "{other:?}");
        }
        for name in ItemLinkType::ALL {
            assert!(!is_blocking_word(name.wire()), "{}", name.wire());
            assert!(!is_blocking_word(name.cli()), "{}", name.cli());
        }
    }

    /// The whole table written out by hand, not read back through `NAMES`.
    #[test]
    fn every_name_has_its_spellings_and_its_wording_for_each_side() {
        let table: Vec<_> = ItemLinkType::ALL
            .iter()
            .map(|t| {
                (
                    *t,
                    t.wire(),
                    t.cli(),
                    t.label(Out),
                    t.label(In),
                    t.directed(),
                )
            })
            .collect();
        assert_eq!(
            table,
            vec![
                (
                    Follows,
                    "follows",
                    "follows",
                    "follows",
                    "is followed by",
                    true
                ),
                (
                    Duplicates,
                    "duplicates",
                    "duplicates",
                    "duplicates",
                    "is duplicated by",
                    true
                ),
                (
                    RelatesTo,
                    "relates_to",
                    "relates-to",
                    "relates to",
                    "relates to",
                    false
                ),
            ]
        );
    }

    #[test]
    fn the_wire_spelling_is_what_serde_writes_and_the_set_is_closed() {
        for t in ItemLinkType::ALL {
            assert_eq!(serde_json::to_value(t).unwrap(), json!(t.wire()));
            assert_eq!(ItemLinkType::from_wire(t.wire()), Some(t));
            assert_eq!(ItemLinkType::from_cli(t.cli()), Some(t));
        }
        for unknown in [
            "blocks",
            "blocked_by",
            "depends_on",
            "relates-to",
            "Follows",
            "",
        ] {
            assert_eq!(ItemLinkType::from_wire(unknown), None, "{unknown}");
            assert!(
                serde_json::from_value::<ItemLinkType>(json!(unknown)).is_err(),
                "{unknown}"
            );
        }
        // The command line takes the hyphen only.
        assert_eq!(ItemLinkType::from_cli("relates_to"), None);
        assert_eq!(ItemLinkType::from_cli("blocks"), None);
    }

    #[test]
    fn an_item_link_travels_as_type_direction_label_and_the_other_item() {
        let dto = ItemLinkDto {
            link_type: Follows,
            direction: In,
            label: Follows.label(In).to_string(),
            item: LinkedItemDto {
                key: "ABC-7".parse().unwrap(),
                level: ItemType::Epic,
                title: "Billing".into(),
            },
        };
        let wire = json!({
            "type": "follows", "direction": "in", "label": "is followed by",
            "item": {"key": "ABC-7", "level": "epic", "title": "Billing"}
        });
        assert_eq!(serde_json::to_value(&dto).unwrap(), wire);
        assert_eq!(serde_json::from_value::<ItemLinkDto>(wire).unwrap(), dto);

        let ok = json!({"type": "relates_to", "to": "SCRUM-7"});
        assert!(serde_json::from_value::<AddItemLink>(ok).is_ok());
        for bad in [
            json!({"type": "blocks", "to": "ABC-7"}),
            json!({"type": "follows", "to": "ABC-7", "direction": "in"}),
            json!({"type": "follows"}),
        ] {
            assert!(
                serde_json::from_value::<AddItemLink>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    /// The sentence the CLI and the MCP tool both say after a `follows` between two stories, and the one place
    /// that says when: the name is `follows` and both ends are stories. Every other name and every other pair of
    /// levels gets nothing.
    #[test]
    fn the_informational_sentence_is_said_for_a_follows_between_two_stories_only() {
        use ItemType::{Epic, Initiative, Story};
        let story: Key = "GRA-12".parse().unwrap();
        assert_eq!(
            follows_is_informational(ItemLinkType::Follows, Story, Story, &story).as_deref(),
            Some("Informational: this does not hold GRA-12 back.")
        );
        for link_type in ItemLinkType::ALL {
            for from in [Initiative, Epic, Story] {
                for to in [Initiative, Epic, Story] {
                    let said = follows_is_informational(link_type, from, to, &story);
                    assert_eq!(
                        said.is_some(),
                        link_type == ItemLinkType::Follows && from == Story && to == Story,
                        "{link_type:?} {from:?} {to:?}"
                    );
                }
            }
        }
    }
}
