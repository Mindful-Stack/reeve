//! Item links (kb/ADR-0031): the three names with their spellings and their wording for each side, as one table,
//! and the wire types that carry an item link. The server, the CLI and (through `label` on the wire) the web
//! client all take the names from here.
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
pub enum LinkType {
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
pub enum LinkDirection {
    Out,
    In,
}

/// One row of the name table.
struct Name {
    link_type: LinkType,
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

/// The one table of names, in `LinkType`'s declaration order: `LinkType::name` indexes it, and a test holds the
/// two in step.
const NAMES: [Name; 3] = [
    Name {
        link_type: LinkType::Follows,
        wire: "follows",
        cli: "follows",
        out: "follows",
        inward: "is followed by",
        directed: true,
    },
    Name {
        link_type: LinkType::Duplicates,
        wire: "duplicates",
        cli: "duplicates",
        out: "duplicates",
        inward: "is duplicated by",
        directed: true,
    },
    Name {
        link_type: LinkType::RelatesTo,
        wire: "relates_to",
        cli: "relates-to",
        out: "relates to",
        inward: "relates to",
        directed: false,
    },
];

impl LinkType {
    pub const ALL: [LinkType; 3] = [LinkType::Follows, LinkType::Duplicates, LinkType::RelatesTo];

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
    pub fn label(self, direction: LinkDirection) -> &'static str {
        match direction {
            LinkDirection::Out => self.name().out,
            LinkDirection::In => self.name().inward,
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
    pub link_type: LinkType,
    pub direction: LinkDirection,
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
    pub link_type: LinkType,
    /// A key or an external reference of an item of any level.
    pub to: KeyRef,
}

/// `GET /items/{key}/links`: what `reeve link ls` shows. Named for the listing, not for item links alone: it gains
/// the item's web links when they exist (GRA-878).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LinkListingDto {
    pub links: Vec<ItemLinkDto>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use LinkDirection::{In, Out};
    use LinkType::{Duplicates, Follows, RelatesTo};
    use serde_json::json;

    /// The whole table written out by hand, not read back through `NAMES`.
    #[test]
    fn every_name_has_its_spellings_and_its_wording_for_each_side() {
        let table: Vec<_> = LinkType::ALL
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
        for t in LinkType::ALL {
            assert_eq!(serde_json::to_value(t).unwrap(), json!(t.wire()));
            assert_eq!(LinkType::from_wire(t.wire()), Some(t));
            assert_eq!(LinkType::from_cli(t.cli()), Some(t));
        }
        for unknown in [
            "blocks",
            "blocked_by",
            "depends_on",
            "relates-to",
            "Follows",
            "",
        ] {
            assert_eq!(LinkType::from_wire(unknown), None, "{unknown}");
            assert!(
                serde_json::from_value::<LinkType>(json!(unknown)).is_err(),
                "{unknown}"
            );
        }
        // The command line takes the hyphen only.
        assert_eq!(LinkType::from_cli("relates_to"), None);
        assert_eq!(LinkType::from_cli("blocks"), None);
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
}
