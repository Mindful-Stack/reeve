//! The glyph beside every key. kb/ADR-0002 R5 keeps one key sequence per instance, so a key alone does not say
//! whether it names an initiative, an epic or a story; its remedy is to show the level beside the key. One set, defined
//! here, for the CLI and the TUI; the web reads the same set from `web/src/lib/glyphs.json`, which a test regenerates
//! from this table (`UPDATE_SNAPSHOTS=1 cargo test -p reeve --test glyphs_snapshot`) and fails on drift.
use reeve_protocol::{ItemType, Kind};
use std::sync::atomic::{AtomicBool, Ordering};

/// What a key names: an initiative, an epic, or a story of one of the three kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Initiative,
    Epic,
    Story,
    Bug,
    Chore,
}

impl Level {
    pub const ALL: [Level; 5] = [
        Level::Initiative,
        Level::Epic,
        Level::Story,
        Level::Bug,
        Level::Chore,
    ];

    pub fn of_kind(k: Kind) -> Self {
        match k {
            Kind::Story => Level::Story,
            Kind::Bug => Level::Bug,
            Kind::Chore => Level::Chore,
        }
    }
    /// A story whose kind is unknown here (a blocker, a decision's target) reads as a story.
    pub fn of_item(t: ItemType) -> Self {
        match t {
            ItemType::Initiative => Level::Initiative,
            ItemType::Epic => Level::Epic,
            ItemType::Story => Level::Story,
        }
    }
    /// The level's name, the accessible text and the JSON key.
    pub fn name(self) -> &'static str {
        match self {
            Level::Initiative => "initiative",
            Level::Epic => "epic",
            Level::Story => "story",
            Level::Bug => "bug",
            Level::Chore => "chore",
        }
    }
    /// One Unicode geometric shape, a single column wide in a standard terminal font.
    pub fn glyph(self) -> &'static str {
        match self {
            Level::Initiative => "◆",
            Level::Epic => "■",
            Level::Story => "●",
            Level::Bug => "▲",
            Level::Chore => "○",
        }
    }
    /// The plain-text fallback for a terminal that cannot show the glyph.
    pub fn ascii(self) -> &'static str {
        match self {
            Level::Initiative => "[I]",
            Level::Epic => "[E]",
            Level::Story => "[S]",
            Level::Bug => "[B]",
            Level::Chore => "[C]",
        }
    }
}

static ASCII: AtomicBool = AtomicBool::new(false);

/// Text output uses the ASCII fallback from now on (`--ascii`, or `REEVE_ASCII` set in the environment).
pub fn set_ascii(on: bool) {
    ASCII.store(on, Ordering::Relaxed);
}

/// The key with its level in front: `◆ ABC-1`, or `[I] ABC-1` with the fallback on.
pub fn mark(level: Level, key: impl std::fmt::Display) -> String {
    mark_with(level, key, ASCII.load(Ordering::Relaxed))
}

pub fn mark_with(level: Level, key: impl std::fmt::Display, ascii: bool) -> String {
    let g = if ascii { level.ascii() } else { level.glyph() };
    format!("{g} {key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_has_a_distinct_single_column_glyph_and_fallback() {
        let glyphs: Vec<_> = Level::ALL.iter().map(|l| l.glyph()).collect();
        assert_eq!(glyphs, ["◆", "■", "●", "▲", "○"]);
        let ascii: Vec<_> = Level::ALL.iter().map(|l| l.ascii()).collect();
        assert_eq!(ascii, ["[I]", "[E]", "[S]", "[B]", "[C]"]);
        for l in Level::ALL {
            assert_eq!(l.glyph().chars().count(), 1, "{}", l.name());
            assert!(l.ascii().is_ascii());
        }
        assert_eq!(Level::of_kind(Kind::Bug), Level::Bug);
        assert_eq!(Level::of_item(ItemType::Epic), Level::Epic);
        assert_eq!(Level::of_item(ItemType::Story), Level::Story);
    }

    #[test]
    fn mark_puts_the_glyph_or_its_fallback_before_the_key() {
        assert_eq!(mark_with(Level::Initiative, "ABC-1", false), "◆ ABC-1");
        assert_eq!(mark_with(Level::Chore, "ABC-9", true), "[C] ABC-9");
    }
}
