//! The binding table for the board and the drawer; `?` renders `help_lines()`, `Space` lists `MENU` from it. Prompts,
//! pickers and the search line read keys themselves (text there is text, `q` included).
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Left,
    Right,
    Up,
    Down,
    First,
    Last,
    Open,
    Menu,
    Back,
    MoveEarlier,
    MoveLater,
    MoveTo,
    Take,
    SetEpic,
    Yank,
    OpenWeb,
    OpenPr,
    Filter,
    Search,
    Refresh,
    ScrollDown,
    ScrollUp,
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pat {
    /// A printable key; its case is the binding (`H` is not `h`), Shift is how the terminal reports the case.
    Char(char),
    /// A named key with no modifier.
    Code(KeyCode),
    /// A named key with Shift.
    Shift(KeyCode),
}

pub struct Binding {
    pub pats: &'static [Pat],
    pub action: Action,
    pub keys: &'static str,
    pub help: &'static str,
}

use Action as A;
use Pat::{Char as C, Code, Shift};

pub const BINDINGS: &[Binding] = &[
    Binding {
        pats: &[C('h'), Code(KeyCode::Left)],
        action: A::Left,
        keys: "h ←",
        help: "focus the column to the left (list: previous group)",
    },
    Binding {
        pats: &[C('l'), Code(KeyCode::Right)],
        action: A::Right,
        keys: "l →",
        help: "focus the column to the right (list: next group)",
    },
    Binding {
        pats: &[C('k'), Code(KeyCode::Up)],
        action: A::Up,
        keys: "k ↑",
        help: "select the story above",
    },
    Binding {
        pats: &[C('j'), Code(KeyCode::Down)],
        action: A::Down,
        keys: "j ↓",
        help: "select the story below",
    },
    Binding {
        pats: &[C('g')],
        action: A::First,
        keys: "g",
        help: "first story",
    },
    Binding {
        pats: &[C('G')],
        action: A::Last,
        keys: "G",
        help: "last story",
    },
    Binding {
        pats: &[Code(KeyCode::Enter)],
        action: A::Open,
        keys: "Enter",
        help: "open the drawer",
    },
    Binding {
        pats: &[C(' ')],
        action: A::Menu,
        keys: "Space",
        help: "actions for the story",
    },
    Binding {
        pats: &[Code(KeyCode::Esc)],
        action: A::Back,
        keys: "Esc",
        help: "close one layer (drawer, then search)",
    },
    Binding {
        pats: &[C('H'), Shift(KeyCode::Left)],
        action: A::MoveEarlier,
        keys: "H ⇧←",
        help: "move the story to the earlier state",
    },
    Binding {
        pats: &[C('L'), Shift(KeyCode::Right)],
        action: A::MoveLater,
        keys: "L ⇧→",
        help: "move the story to the later state",
    },
    Binding {
        pats: &[C('m')],
        action: A::MoveTo,
        keys: "m",
        help: "move to…",
    },
    Binding {
        pats: &[C('t')],
        action: A::Take,
        keys: "t",
        help: "take (owner = me)",
    },
    Binding {
        pats: &[C('e')],
        action: A::SetEpic,
        keys: "e",
        help: "set epic",
    },
    Binding {
        pats: &[C('y')],
        action: A::Yank,
        keys: "y",
        help: "copy the key",
    },
    Binding {
        pats: &[C('o')],
        action: A::OpenWeb,
        keys: "o",
        help: "open in the web board",
    },
    Binding {
        pats: &[C('O')],
        action: A::OpenPr,
        keys: "O",
        help: "open the first pull request",
    },
    Binding {
        pats: &[C('f')],
        action: A::Filter,
        keys: "f",
        help: "filter (mine / all, ready only, urgent only)",
    },
    Binding {
        pats: &[C('/')],
        action: A::Search,
        keys: "/",
        help: "search by key or title",
    },
    Binding {
        pats: &[C('r')],
        action: A::Refresh,
        keys: "r",
        help: "refresh now",
    },
    Binding {
        pats: &[C('J'), Code(KeyCode::PageDown)],
        action: A::ScrollDown,
        keys: "J PgDn",
        help: "scroll the drawer down",
    },
    Binding {
        pats: &[C('K'), Code(KeyCode::PageUp)],
        action: A::ScrollUp,
        keys: "K PgUp",
        help: "scroll the drawer up",
    },
    Binding {
        pats: &[C('?')],
        action: A::Help,
        keys: "?",
        help: "help",
    },
    Binding {
        pats: &[C('q')],
        action: A::Quit,
        keys: "q",
        help: "quit",
    },
];

/// What the action menu lists for the selected story, in this order; its keys and wording come from `BINDINGS`.
pub const MENU: &[Action] = &[
    A::Open,
    A::Take,
    A::MoveEarlier,
    A::MoveLater,
    A::MoveTo,
    A::SetEpic,
    A::Yank,
    A::OpenWeb,
    A::OpenPr,
];

/// The binding of `a`; every action has exactly one (a test checks it).
pub fn binding(a: Action) -> &'static Binding {
    BINDINGS
        .iter()
        .find(|b| b.action == a)
        .expect("every action is bound")
}

fn matches(p: &Pat, k: &KeyEvent) -> bool {
    let chord = k
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    match *p {
        Pat::Char(c) => {
            // Kitty-protocol terminals report Shift+h as 'h' with SHIFT: that is 'H'.
            let reported = match k.code {
                KeyCode::Char(r)
                    if k.modifiers.contains(KeyModifiers::SHIFT) && r.is_ascii_lowercase() =>
                {
                    KeyCode::Char(r.to_ascii_uppercase())
                }
                other => other,
            };
            !chord && reported == KeyCode::Char(c)
        }
        Pat::Code(c) => !chord && !k.modifiers.contains(KeyModifiers::SHIFT) && k.code == c,
        Pat::Shift(c) => !chord && k.modifiers.contains(KeyModifiers::SHIFT) && k.code == c,
    }
}

pub fn action(k: &KeyEvent) -> Option<Action> {
    BINDINGS
        .iter()
        .find(|b| b.pats.iter().any(|p| matches(p, k)))
        .map(|b| b.action)
}

pub fn help_lines() -> Vec<String> {
    BINDINGS
        .iter()
        .map(|b| format!("{:<8} {}", b.keys, b.help))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn ev(p: &Pat) -> KeyEvent {
        match *p {
            Pat::Char(c) if c.is_ascii_uppercase() => {
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
            }
            Pat::Char(c) => KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            Pat::Code(c) => KeyEvent::new(c, KeyModifiers::NONE),
            Pat::Shift(c) => KeyEvent::new(c, KeyModifiers::SHIFT),
        }
    }

    #[test]
    fn every_pattern_resolves_to_its_own_binding() {
        for b in BINDINGS {
            for p in b.pats {
                assert_eq!(action(&ev(p)), Some(b.action), "{p:?}");
            }
        }
    }

    #[test]
    fn shift_arrows_move_and_plain_arrows_focus() {
        assert_eq!(
            action(&KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT)),
            Some(Action::MoveEarlier)
        );
        assert_eq!(
            action(&KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)),
            Some(Action::MoveLater)
        );
        assert_eq!(
            action(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            Some(Action::Left)
        );
        assert_eq!(
            action(&ev(&Pat::Char('H'))),
            action(&KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT))
        );
    }

    #[test]
    fn control_and_alt_chords_are_not_bindings() {
        assert_eq!(
            action(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            action(&KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT)),
            None
        );
    }

    #[test]
    fn no_pattern_is_bound_twice_and_help_lists_every_binding() {
        let all: Vec<String> = BINDINGS
            .iter()
            .flat_map(|b| b.pats.iter().map(|p| format!("{p:?}")))
            .collect();
        let mut dedup = all.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(all.len(), dedup.len());
        let help = help_lines();
        assert_eq!(help.len(), BINDINGS.len());
        assert!(help.iter().any(|l| l.contains("earlier state")));
    }

    #[test]
    fn every_menu_action_is_bound_once() {
        for a in MENU {
            assert_eq!(
                BINDINGS.iter().filter(|b| b.action == *a).count(),
                1,
                "{a:?}"
            );
        }
        assert_eq!(
            action(&KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(Action::Menu)
        );
    }

    #[test]
    fn a_lowercase_char_reported_with_shift_is_its_uppercase() {
        assert_eq!(
            action(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::SHIFT)),
            Some(Action::MoveEarlier)
        );
        assert_eq!(
            action(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
            Some(Action::Left)
        );
    }
}
