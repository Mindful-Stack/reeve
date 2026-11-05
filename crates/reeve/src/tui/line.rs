//! A one-line editor with readline keys (Backspace removes one code point, not one grapheme): the override reason, the search query, the epic filter.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEvent {
    Edited,
    Submit,
    Cancel,
    Ignored,
}

/// Text only from plain or shifted keys, or AltGr (reported as Control+Alt on Windows); other chords are not text.
fn printable(m: KeyModifiers) -> bool {
    KeyModifiers::SHIFT.contains(m) || m.contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LineEditor {
    text: String,
    /// In characters, not bytes.
    cursor: usize,
}

impl LineEditor {
    pub fn with_text(s: &str) -> Self {
        Self {
            text: s.to_string(),
            cursor: s.chars().count(),
        }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    /// The text with a bar at the cursor, cut to at most `width` columns and scrolled so the cursor bar is always visible: a `…` marks each
    /// edge where text is cut, and a wide character is never split. Below three columns only the bar fits.
    pub fn window(&self, width: usize) -> String {
        use unicode_width::UnicodeWidthChar;
        const BAR: char = '▏';
        let mut items: Vec<char> = self.text.chars().collect();
        let c = self.cursor.min(items.len());
        items.insert(c, BAR);
        let w = |ch: char| {
            if ch == BAR {
                1
            } else {
                ch.width().unwrap_or(0)
            }
        };
        let n = items.len();
        let total: usize = items.iter().map(|&ch| w(ch)).sum();
        if total <= width {
            return items.into_iter().collect();
        }
        if width < 3 {
            return if width == 0 {
                String::new()
            } else {
                BAR.to_string()
            };
        }
        let (mut s, mut e) = (c, c + 1);
        let mut used = 1;
        let cost = |s: usize, e: usize, used: usize| used + usize::from(s > 0) + usize::from(e < n);
        loop {
            let mut grew = false;
            if e < n && cost(s, e + 1, used + w(items[e])) <= width {
                used += w(items[e]);
                e += 1;
                grew = true;
            }
            if s > 0 && cost(s - 1, e, used + w(items[s - 1])) <= width {
                used += w(items[s - 1]);
                s -= 1;
                grew = true;
            }
            if !grew {
                break;
            }
        }
        let mut out = String::new();
        if s > 0 {
            out.push('…');
        }
        out.extend(&items[s..e]);
        if e < n {
            out.push('…');
        }
        out
    }
    fn byte(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(b, _)| b)
    }
    fn len(&self) -> usize {
        self.text.chars().count()
    }
    pub fn handle(&mut self, k: &KeyEvent) -> LineEvent {
        if k.kind == KeyEventKind::Release {
            return LineEvent::Ignored;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match (k.code, ctrl) {
            (KeyCode::Enter, _) => return LineEvent::Submit,
            (KeyCode::Esc, _) => return LineEvent::Cancel,
            (KeyCode::Char('a'), true) | (KeyCode::Home, _) => self.cursor = 0,
            (KeyCode::Char('e'), true) | (KeyCode::End, _) => self.cursor = self.len(),
            (KeyCode::Char('u'), true) => {
                let b = self.byte(self.cursor);
                self.text.replace_range(..b, "");
                self.cursor = 0;
            }
            (KeyCode::Char('k'), true) => {
                let b = self.byte(self.cursor);
                self.text.truncate(b);
            }
            (KeyCode::Char('w'), true) => {
                let chars: Vec<char> = self.text.chars().collect();
                let mut i = self.cursor;
                while i > 0 && chars[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && chars[i - 1] != ' ' {
                    i -= 1;
                }
                let (from, to) = (self.byte(i), self.byte(self.cursor));
                self.text.replace_range(from..to, "");
                self.cursor = i;
            }
            (KeyCode::Left, _) => self.cursor = self.cursor.saturating_sub(1),
            (KeyCode::Right, _) => self.cursor = (self.cursor + 1).min(self.len()),
            (KeyCode::Backspace, _) if self.cursor > 0 => {
                let (from, to) = (self.byte(self.cursor - 1), self.byte(self.cursor));
                self.text.replace_range(from..to, "");
                self.cursor -= 1;
            }
            (KeyCode::Delete, _) if self.cursor < self.len() => {
                let (from, to) = (self.byte(self.cursor), self.byte(self.cursor + 1));
                self.text.replace_range(from..to, "");
            }
            (KeyCode::Char(c), _) if printable(k.modifiers) => {
                let b = self.byte(self.cursor);
                self.text.insert(b, c);
                self.cursor += 1;
            }
            _ => return LineEvent::Ignored,
        }
        LineEvent::Edited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(e: &mut LineEditor, code: KeyCode, mods: KeyModifiers) -> LineEvent {
        e.handle(&KeyEvent::new(code, mods))
    }
    fn typed(s: &str) -> LineEditor {
        let mut e = LineEditor::default();
        for c in s.chars() {
            press(&mut e, KeyCode::Char(c), KeyModifiers::NONE);
        }
        e
    }

    fn at(text: &str, cursor: usize) -> LineEditor {
        let mut e = LineEditor::with_text(text);
        e.cursor = cursor;
        e
    }

    #[test]
    fn window_shows_everything_that_fits() {
        assert_eq!(at("abc", 3).window(4), "abc▏");
        assert_eq!(at("abc", 1).window(10), "a▏bc");
    }

    #[test]
    fn window_with_the_cursor_at_the_end_shows_the_tail() {
        let w = at("abcdefghij", 10).window(6);
        assert_eq!(w, "…ghij▏");
    }

    #[test]
    fn window_with_the_cursor_at_the_start_shows_the_head() {
        assert_eq!(at("abcdefghij", 0).window(6), "▏abcd…");
    }

    #[test]
    fn window_with_the_cursor_in_the_middle_cuts_both_edges() {
        let w = at("abcdefghij", 5).window(7);
        assert_eq!(w.chars().count(), 7);
        assert!(
            w.starts_with('…') && w.ends_with('…') && w.contains('▏'),
            "{w}"
        );
    }

    #[test]
    fn window_never_splits_a_wide_character_or_overflows() {
        use unicode_width::UnicodeWidthStr;
        let e = at("日本語のテキスト", 4);
        for width in 0..20 {
            let w = e.window(width);
            assert!(w.width() <= width, "{width}: {w}");
            if width >= 1 {
                assert!(w.contains('▏'), "{width}: {w}");
            }
        }
    }

    #[test]
    fn window_of_tiny_width_does_not_panic() {
        for width in 0..=2 {
            let _ = at("abcdef", 3).window(width);
            let _ = at("", 0).window(width);
        }
        assert_eq!(at("abcdef", 3).window(1), "▏");
    }

    #[test]
    fn q_and_shifted_letters_are_text() {
        let mut e = typed("q");
        assert_eq!(
            press(&mut e, KeyCode::Char('Q'), KeyModifiers::SHIFT),
            LineEvent::Edited
        );
        assert_eq!(e.text(), "qQ");
    }

    #[test]
    fn editing_respects_multibyte_characters() {
        let mut e = typed("Åsa ö");
        press(&mut e, KeyCode::Left, KeyModifiers::NONE);
        press(&mut e, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(e.text(), "Åsaö");
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn ctrl_w_deletes_the_word_before_the_cursor() {
        let mut e = typed("hot fix now ");
        press(&mut e, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(e.text(), "hot fix ");
    }

    #[test]
    fn enter_submits_esc_cancels_release_is_ignored() {
        let mut e = typed("x");
        assert_eq!(
            press(&mut e, KeyCode::Enter, KeyModifiers::NONE),
            LineEvent::Submit
        );
        assert_eq!(
            press(&mut e, KeyCode::Esc, KeyModifiers::NONE),
            LineEvent::Cancel
        );
        let mut rel = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        rel.kind = KeyEventKind::Release;
        assert_eq!(e.handle(&rel), LineEvent::Ignored);
        assert_eq!(e.text(), "x");
    }

    #[test]
    fn other_chords_are_not_text_but_altgr_is() {
        let mut e = typed("ab");
        assert_eq!(
            press(&mut e, KeyCode::Char('x'), KeyModifiers::ALT),
            LineEvent::Ignored
        );
        assert_eq!(
            press(&mut e, KeyCode::Char('x'), KeyModifiers::SUPER),
            LineEvent::Ignored
        );
        assert_eq!(
            press(&mut e, KeyCode::Char('x'), KeyModifiers::CONTROL),
            LineEvent::Ignored
        );
        assert_eq!(e.text(), "ab");
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(press(&mut e, KeyCode::Char('@'), altgr), LineEvent::Edited);
        assert_eq!(e.text(), "ab@");
    }

    #[test]
    fn delete_removes_the_character_under_the_cursor() {
        let mut e = typed("abc");
        press(&mut e, KeyCode::Home, KeyModifiers::NONE);
        press(&mut e, KeyCode::Right, KeyModifiers::NONE);
        press(&mut e, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(e.text(), "ac");
        press(&mut e, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(
            press(&mut e, KeyCode::Delete, KeyModifiers::NONE),
            LineEvent::Ignored
        );
        assert_eq!(e.text(), "ac");
    }

    #[test]
    fn ctrl_u_and_ctrl_k_cut_either_side_of_the_cursor() {
        let mut e = typed("Åsa ö");
        press(&mut e, KeyCode::Left, KeyModifiers::NONE);
        press(&mut e, KeyCode::Left, KeyModifiers::NONE);
        let mut k = e.clone();
        press(&mut e, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!((e.text(), e.cursor()), (" ö", 0));
        press(&mut k, KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!((k.text(), k.cursor()), ("Åsa", 3));
    }

    #[test]
    fn ctrl_w_over_a_multibyte_word() {
        let mut e = typed("hej Åsa");
        press(&mut e, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(e.text(), "hej ");
    }
}
