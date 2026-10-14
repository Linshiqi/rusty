//! The editor's own keys, VS Code's: what a key does in the code, before the
//! window's bindings (Ctrl+S, Ctrl+P, …) get what the editor leaves.

use slint::platform::Key;

use super::cursors::Motion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Move(Motion, bool),
    /// Up or Down by `lines` (a page is many), extending when `true`.
    Vertical(isize, bool),
    /// The document's start or end, extending when `true`.
    Edge {
        end: bool,
        extend: bool,
    },
    Backspace {
        word: bool,
    },
    Delete {
        word: bool,
    },
    Enter,
    Tab,
    Outdent,
    Indent,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    NextMatch,
    AllMatches,
    AddCursor {
        up: bool,
    },
    SingleCursor,
    MoveLines {
        up: bool,
    },
    CopyLines {
        up: bool,
    },
    DeleteLines,
    OpenLine {
        above: bool,
    },
    SelectLine,
    ToggleComment,
    /// Ctrl+Space: what the server offers at the caret.
    Suggest,
}

/// The command a key names, or `None` for the window's bindings to have it.
pub fn command(text: &str, ctrl: bool, alt: bool, shift: bool, page: isize) -> Option<Command> {
    use Command::*;
    let ch = text.chars().next()?;
    let is = |key: Key| ch == char::from(key);
    let letter = (!ch.is_control()).then(|| ch.to_ascii_lowercase());
    Some(match (ctrl, alt) {
        (false, false) => match () {
            _ if is(Key::LeftArrow) => Move(Motion::Left, shift),
            _ if is(Key::RightArrow) => Move(Motion::Right, shift),
            _ if is(Key::UpArrow) => Vertical(-1, shift),
            _ if is(Key::DownArrow) => Vertical(1, shift),
            _ if is(Key::Home) => Move(Motion::Home, shift),
            _ if is(Key::End) => Move(Motion::End, shift),
            _ if is(Key::PageUp) => Vertical(-page, shift),
            _ if is(Key::PageDown) => Vertical(page, shift),
            _ if is(Key::Backspace) => Backspace { word: false },
            _ if is(Key::Delete) => Delete { word: false },
            _ if is(Key::Return) => Enter,
            _ if is(Key::Tab) && !shift => Tab,
            _ if is(Key::Tab) || is(Key::Backtab) => Outdent,
            _ if is(Key::Escape) => SingleCursor,
            _ => return None,
        },
        (true, false) => match () {
            _ if is(Key::LeftArrow) => Move(Motion::WordLeft, shift),
            _ if is(Key::RightArrow) => Move(Motion::WordRight, shift),
            _ if is(Key::Home) => Edge {
                end: false,
                extend: shift,
            },
            _ if is(Key::End) => Edge {
                end: true,
                extend: shift,
            },
            _ if is(Key::Backspace) => Backspace { word: true },
            _ if is(Key::Delete) => Delete { word: true },
            _ if is(Key::Return) => OpenLine { above: shift },
            _ => match letter? {
                'a' if !shift => SelectAll,
                'c' if !shift => Copy,
                'x' if !shift => Cut,
                'v' if !shift => Paste,
                'z' if !shift => Undo,
                'z' => Redo,
                'y' if !shift => Redo,
                'd' if !shift => NextMatch,
                'l' if shift => AllMatches,
                'l' => SelectLine,
                'k' if shift => DeleteLines,
                '/' => ToggleComment,
                ']' => Indent,
                '[' => Outdent,
                ' ' => Suggest,
                _ => return None,
            },
        },
        (false, true) => match () {
            _ if is(Key::UpArrow) => {
                if shift {
                    CopyLines { up: true }
                } else {
                    MoveLines { up: true }
                }
            }
            _ if is(Key::DownArrow) => {
                if shift {
                    CopyLines { up: false }
                } else {
                    MoveLines { up: false }
                }
            }
            _ => return None,
        },
        (true, true) => match () {
            _ if is(Key::UpArrow) => AddCursor { up: true },
            _ if is(Key::DownArrow) => AddCursor { up: false },
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key) -> String {
        slint::SharedString::from(k).to_string()
    }

    #[test]
    fn the_keys_are_vs_codes() {
        assert_eq!(
            command("d", true, false, false, 30),
            Some(Command::NextMatch)
        );
        assert_eq!(
            command("L", true, false, true, 30),
            Some(Command::AllMatches)
        );
        assert_eq!(
            command("K", true, false, true, 30),
            Some(Command::DeleteLines)
        );
        assert_eq!(
            command("/", true, false, false, 30),
            Some(Command::ToggleComment)
        );
        assert_eq!(
            command(&key(Key::UpArrow), false, true, false, 30),
            Some(Command::MoveLines { up: true })
        );
        assert_eq!(
            command(&key(Key::DownArrow), false, true, true, 30),
            Some(Command::CopyLines { up: false })
        );
        assert_eq!(
            command(&key(Key::DownArrow), true, true, false, 30),
            Some(Command::AddCursor { up: false })
        );
        assert_eq!(
            command(&key(Key::PageDown), false, false, true, 30),
            Some(Command::Vertical(30, true))
        );
    }

    /// What the editor does not take reaches the window: Ctrl+S saves,
    /// Ctrl+P finds a file.
    #[test]
    fn the_windows_chords_are_left_alone() {
        assert_eq!(command("s", true, false, false, 30), None);
        assert_eq!(command("p", true, false, false, 30), None);
        assert_eq!(command("P", true, false, true, 30), None);
    }
}
