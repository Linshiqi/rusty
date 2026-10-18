//! A key as a chord — `Ctrl+Shift+P` — the way the bindings spell it.
//!
//! One press is a *stroke*; a chord is one stroke, or two with a space
//! between, as VS Code's `Ctrl+K Ctrl+S`. A stroke names its modifiers in
//! one order, Ctrl, Alt, Shift, and its key upper case or by name, so two
//! spellings of one key are one string to compare.

use slint::platform::Key;

/// The stroke a key event names, or `None` for a modifier pressed alone.
pub fn chord(text: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Option<String> {
    let key = name(text)?;
    Some(
        Stroke {
            ctrl: ctrl || meta,
            alt,
            shift,
            key,
        }
        .to_string(),
    )
}

/// One press: its modifiers and its key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stroke {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Upper case for a character, the name for a named key (`F5`, `Enter`).
    pub key: String,
}

impl std::fmt::Display for Stroke {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(&self.key)
    }
}

impl Stroke {
    /// A stroke as somebody wrote it — `ctrl+shift+p`, `Shift+Ctrl+P`,
    /// `Cmd+K`, `Ctrl++` — or `None` for what names no key. The modifiers
    /// may come in any order and any case; the key must be one character or
    /// a key this window names.
    pub fn parse(text: &str) -> Option<Stroke> {
        let text = text.trim();
        // `+` is a key too, and `Ctrl++` ends in the separator and the key.
        let (modifiers, key) = if text == "+" {
            ("", "+")
        } else if let Some(rest) = text.strip_suffix("++") {
            (rest, "+")
        } else {
            match text.rsplit_once('+') {
                Some((modifiers, key)) => (modifiers, key),
                None => ("", text),
            }
        };
        let mut stroke = Stroke {
            ctrl: false,
            alt: false,
            shift: false,
            key: key_name(key)?,
        };
        for modifier in modifiers.split('+').filter(|m| !m.is_empty()) {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "meta" | "super" => stroke.ctrl = true,
                "alt" | "option" => stroke.alt = true,
                "shift" => stroke.shift = true,
                _ => return None,
            }
        }
        Some(stroke)
    }

    /// Whether the key is one of F1 to F12, which nothing types.
    pub fn is_function(&self) -> bool {
        self.key.len() >= 2
            && self.key.starts_with('F')
            && self.key[1..].chars().all(|c| c.is_ascii_digit())
    }

    /// Whether the key is a character rather than a named key.
    pub fn is_character(&self) -> bool {
        self.key.chars().count() == 1
    }

    /// What Slint sends for this key: the letter for a letter, lower case,
    /// the key's own code for a named key.
    pub fn text(&self) -> String {
        match NAMED.iter().find(|(_, name)| *name == self.key) {
            Some((key, _)) => char::from(*key).to_string(),
            None => self.key.to_lowercase(),
        }
    }
}

/// A key's name as a chord spells it, from however it was written.
fn key_name(key: &str) -> Option<String> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (None, _) => None,
        (Some(ch), None) => Some(ch.to_uppercase().collect()),
        _ => {
            let alias = match key.to_ascii_lowercase().as_str() {
                "esc" => "Escape",
                "return" => "Enter",
                "del" => "Delete",
                "up" => "ArrowUp",
                "down" => "ArrowDown",
                "left" => "ArrowLeft",
                "right" => "ArrowRight",
                _ => "",
            };
            let wanted = if alias.is_empty() { key } else { alias };
            NAMED
                .iter()
                .map(|(_, name)| *name)
                .find(|name| name.eq_ignore_ascii_case(wanted))
                .map(str::to_string)
        }
    }
}

/// The keys a chord spells by name, beside the code Slint sends for each.
const NAMED: [(Key, &str); 26] = [
    (Key::Escape, "Escape"),
    (Key::Return, "Enter"),
    (Key::Tab, "Tab"),
    (Key::Backspace, "Backspace"),
    (Key::Delete, "Delete"),
    (Key::Home, "Home"),
    (Key::End, "End"),
    (Key::PageUp, "PageUp"),
    (Key::PageDown, "PageDown"),
    (Key::UpArrow, "ArrowUp"),
    (Key::DownArrow, "ArrowDown"),
    (Key::LeftArrow, "ArrowLeft"),
    (Key::RightArrow, "ArrowRight"),
    (Key::Space, "Space"),
    (Key::F1, "F1"),
    (Key::F2, "F2"),
    (Key::F3, "F3"),
    (Key::F4, "F4"),
    (Key::F5, "F5"),
    (Key::F6, "F6"),
    (Key::F7, "F7"),
    (Key::F8, "F8"),
    (Key::F9, "F9"),
    (Key::F10, "F10"),
    (Key::F11, "F11"),
    (Key::F12, "F12"),
];

/// Whether a key is a modifier pressed on its own — Ctrl, Shift, Alt,
/// AltGr or the Windows key — which is no chord and no key to put a card
/// away for.
pub fn is_modifier(text: &str) -> bool {
    const MODIFIERS: [Key; 8] = [
        Key::Control,
        Key::ControlR,
        Key::Shift,
        Key::ShiftR,
        Key::Alt,
        Key::AltGr,
        Key::Meta,
        Key::MetaR,
    ];
    text.chars()
        .next()
        .is_some_and(|ch| MODIFIERS.iter().any(|key| char::from(*key) == ch))
}

/// A key's name in a chord: letters upper case, the named keys by name.
/// Slint sends the letter for Ctrl+P, not a control character, and keeps
/// the control range for keys of its own — 0x10 is Shift.
fn name(text: &str) -> Option<String> {
    let ch = text.chars().next()?;
    // Shift+Tab arrives as a key of its own; it is Tab with Shift held.
    if ch == char::from(Key::Backtab) {
        return Some("Tab".to_string());
    }
    if let Some((_, name)) = NAMED.iter().find(|(key, _)| char::from(*key) == ch) {
        return Some((*name).to_string());
    }
    if is_modifier(text) {
        return None;
    }
    if ch.is_control() {
        return None;
    }
    Some(ch.to_uppercase().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_names_its_modifiers_in_one_order_and_its_key_upper_case() {
        assert_eq!(
            chord("p", true, false, true, false).as_deref(),
            Some("Ctrl+Shift+P")
        );
        assert_eq!(
            chord("P", true, false, true, false).as_deref(),
            Some("Ctrl+Shift+P")
        );
        assert_eq!(
            chord(",", true, false, false, false).as_deref(),
            Some("Ctrl+,")
        );
        let f5: slint::SharedString = Key::F5.into();
        assert_eq!(
            chord(&f5, false, false, false, false).as_deref(),
            Some("F5")
        );
    }

    #[test]
    fn a_modifier_alone_is_no_chord() {
        let ctrl: slint::SharedString = Key::Control.into();
        assert_eq!(chord(&ctrl, true, false, false, false), None);
    }

    /// The old window wrote Shift before Alt, and a hand-edited file may
    /// write anything: every spelling of one stroke reads as the one this
    /// window compares.
    #[test]
    fn a_stroke_reads_however_it_was_written() {
        let read = |text: &str| Stroke::parse(text).map(|s| s.to_string());
        assert_eq!(read("ctrl+shift+p").as_deref(), Some("Ctrl+Shift+P"));
        assert_eq!(
            read("Ctrl+Shift+Alt+X").as_deref(),
            Some("Ctrl+Alt+Shift+X")
        );
        assert_eq!(read("Cmd+K").as_deref(), Some("Ctrl+K"));
        assert_eq!(read("Ctrl++").as_deref(), Some("Ctrl++"));
        assert_eq!(read("Ctrl+=").as_deref(), Some("Ctrl+="));
        assert_eq!(read("alt+left").as_deref(), Some("Alt+ArrowLeft"));
        assert_eq!(read("shift+f12").as_deref(), Some("Shift+F12"));
        assert_eq!(read("Ctrl+\\").as_deref(), Some("Ctrl+\\"));
        assert_eq!(read("Hyper+K"), None, "no such modifier");
        assert_eq!(read("Ctrl+Banana"), None, "no such key");
        assert_eq!(read(""), None);
    }

    /// A stroke read back is the stroke the key makes: what the shortcuts
    /// page compares and what a press produces are one string.
    #[test]
    fn a_stroke_and_the_key_it_names_agree() {
        for written in [
            "Ctrl+S",
            "Ctrl+Shift+N",
            "F5",
            "Shift+F11",
            "Alt+ArrowLeft",
            "Ctrl+0",
        ] {
            let stroke = Stroke::parse(written).unwrap();
            assert_eq!(
                chord(&stroke.text(), stroke.ctrl, stroke.alt, stroke.shift, false).as_deref(),
                Some(written)
            );
        }
        assert!(Stroke::parse("F12").unwrap().is_function());
        assert!(!Stroke::parse("F").unwrap().is_function());
        assert!(Stroke::parse("Ctrl+K").unwrap().is_character());
        assert!(!Stroke::parse("Ctrl+Enter").unwrap().is_character());
    }
}
