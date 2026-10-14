//! A key as a chord — `Ctrl+Shift+P` — the way the bindings spell it.

use slint::platform::Key;

/// The chord a key event names, or `None` for a modifier pressed alone.
pub fn chord(text: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Option<String> {
    let key = name(text)?;
    let mut chord = String::new();
    if ctrl || meta {
        chord.push_str("Ctrl+");
    }
    if alt {
        chord.push_str("Alt+");
    }
    if shift {
        chord.push_str("Shift+");
    }
    chord.push_str(&key);
    Some(chord)
}

/// A key's name in a chord: letters upper case, the named keys by name.
/// Slint sends the letter for Ctrl+P, not a control character, and keeps
/// the control range for keys of its own — 0x10 is Shift.
fn name(text: &str) -> Option<String> {
    let ch = text.chars().next()?;
    let named = [
        (Key::Escape, "Escape"),
        (Key::Return, "Enter"),
        (Key::Tab, "Tab"),
        (Key::Backtab, "Tab"),
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
    if let Some((_, name)) = named.iter().find(|(key, _)| char::from(*key) == ch) {
        return Some((*name).to_string());
    }
    let modifiers = [
        Key::Control,
        Key::ControlR,
        Key::Shift,
        Key::ShiftR,
        Key::Alt,
        Key::AltGr,
        Key::Meta,
        Key::MetaR,
    ];
    if modifiers.iter().any(|key| char::from(*key) == ch) {
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
}
