//! Item links (kb/ADR-0031): a named, informational connection between two different items of any level. An edge
//! of its own, owned by neither item (R4). Pure: no item is read here, and nothing here is a readiness fact (R3).
use reeve_protocol::{Key, LinkDirection, LinkType};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ItemLinkError {
    #[error("an item cannot have an item link to itself")]
    SelfLink,
}

/// "`from` follows, duplicates or relates to `to`", as stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemLink {
    from: Key,
    link_type: LinkType,
    to: Key,
}

impl ItemLink {
    /// The link as it is stored. A link from an item to itself is refused (R1). `relates_to` has no direction, so
    /// it is stored with the lower key in `from`, whichever end the caller named (R4). "Lower" is byte order of
    /// the whole key text (`OLD-9` and `NEW-9` never tie, `ABC-10` is below `ABC-9`); the order means nothing
    /// beyond making the row unique, and the table's check holds the same order (`COLLATE "C"`).
    pub fn new(from: Key, link_type: LinkType, to: Key) -> Result<Self, ItemLinkError> {
        if from == to {
            return Err(ItemLinkError::SelfLink);
        }
        // `str`'s order is byte order, which is what the table's `COLLATE "C"` check compares.
        let swap = !link_type.directed() && to.to_string() < from.to_string();
        let (from, to) = if swap { (to, from) } else { (from, to) };
        Ok(Self {
            from,
            link_type,
            to,
        })
    }
    /// A stored row, as it is: no check.
    pub fn from_persisted(from: Key, link_type: LinkType, to: Key) -> Self {
        Self {
            from,
            link_type,
            to,
        }
    }
    pub fn from_key(&self) -> &Key {
        &self.from
    }
    pub fn link_type(&self) -> LinkType {
        self.link_type
    }
    pub fn to_key(&self) -> &Key {
        &self.to
    }
    /// How the link reads from `item`: the side, and the item at the other end. None when `item` is at neither
    /// end. Both ends of a `relates_to` read it as `out`.
    pub fn seen_from(&self, item: &Key) -> Option<(LinkDirection, &Key)> {
        if *item == self.from {
            Some((LinkDirection::Out, &self.to))
        } else if *item == self.to {
            let direction = if self.link_type.directed() {
                LinkDirection::In
            } else {
                LinkDirection::Out
            };
            Some((direction, &self.from))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LinkType::{Duplicates, Follows, RelatesTo};

    fn k(s: &str) -> Key {
        s.parse().unwrap()
    }

    #[test]
    fn a_link_from_an_item_to_itself_is_refused_for_every_name() {
        for t in LinkType::ALL {
            assert_eq!(
                ItemLink::new(k("ABC-1"), t, k("ABC-1")),
                Err(ItemLinkError::SelfLink),
                "{t:?}"
            );
        }
    }

    /// Each pair is (lower, higher) in byte order. `ABC-10` below `ABC-9` tells byte order from key-number order;
    /// `NEW-9` below `OLD-9` tells a comparison of the whole key from one of the number alone.
    #[test]
    fn relates_to_is_stored_with_the_lower_key_first_whichever_end_was_named() {
        for (lower, higher) in [("ABC-1", "ABC-2"), ("ABC-10", "ABC-9"), ("NEW-9", "OLD-9")] {
            assert!(
                lower.as_bytes() < higher.as_bytes(),
                "the control: {lower} < {higher}"
            );
            let one = ItemLink::new(k(lower), RelatesTo, k(higher)).unwrap();
            let other = ItemLink::new(k(higher), RelatesTo, k(lower)).unwrap();
            assert_eq!(one, other, "one link, whichever end named it");
            assert_eq!((one.from_key(), one.to_key()), (&k(lower), &k(higher)));
        }
    }

    #[test]
    fn a_directed_name_keeps_the_ends_as_named() {
        for t in [Follows, Duplicates] {
            let link = ItemLink::new(k("ABC-2"), t, k("ABC-1")).unwrap();
            assert_eq!((link.from_key(), link.to_key()), (&k("ABC-2"), &k("ABC-1")));
            assert_ne!(link, ItemLink::new(k("ABC-1"), t, k("ABC-2")).unwrap());
        }
    }

    #[test]
    fn each_end_reads_the_link_from_its_side() {
        let follows = ItemLink::new(k("ABC-2"), Follows, k("ABC-1")).unwrap();
        assert_eq!(
            follows.seen_from(&k("ABC-2")),
            Some((LinkDirection::Out, &k("ABC-1")))
        );
        assert_eq!(
            follows.seen_from(&k("ABC-1")),
            Some((LinkDirection::In, &k("ABC-2")))
        );
        assert_eq!(follows.seen_from(&k("ABC-3")), None);
        let relates = ItemLink::new(k("ABC-2"), RelatesTo, k("ABC-1")).unwrap();
        assert_eq!(
            relates.seen_from(&k("ABC-1")),
            Some((LinkDirection::Out, &k("ABC-2")))
        );
        assert_eq!(
            relates.seen_from(&k("ABC-2")),
            Some((LinkDirection::Out, &k("ABC-1")))
        );
    }
}
