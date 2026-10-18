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
    /// Ctrl+Shift+[ and ]: the block the caret is in collapsed, or opened.
    Fold(bool),
    /// Cmd+Backspace and Cmd+Delete on a Mac: the line to the caret, or
    /// from it; at the edge already, the line break.
    Kill {
        before: bool,
    },
}

/// The command a key names, or `None` for the window's bindings to have it.
///
/// `ctrl` is the chord key — Ctrl, or Cmd on a Mac, where Slint reports Cmd
/// as `control` — and `macos` adds the Mac's own keys over VS Code's table:
/// Cmd with an arrow goes to the line's or the document's edge and Option
/// with one moves by word, Option+Backspace deletes a word and
/// Cmd+Backspace the line before the caret. Passed in rather than read off
/// `cfg!`, so both tables are tested on every platform.
pub fn command(
    text: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    page: isize,
    macos: bool,
) -> Option<Command> {
    use Command::*;
    let ch = text.chars().next()?;
    let is = |key: Key| ch == char::from(key);
    let letter = (!ch.is_control()).then(|| ch.to_ascii_lowercase());
    if macos {
        let own = match (ctrl, alt) {
            (true, false) => match () {
                _ if is(Key::LeftArrow) => Some(Move(Motion::Home, shift)),
                _ if is(Key::RightArrow) => Some(Move(Motion::End, shift)),
                _ if is(Key::UpArrow) => Some(Edge {
                    end: false,
                    extend: shift,
                }),
                _ if is(Key::DownArrow) => Some(Edge {
                    end: true,
                    extend: shift,
                }),
                _ if is(Key::Backspace) => Some(Kill { before: true }),
                _ if is(Key::Delete) => Some(Kill { before: false }),
                _ => None,
            },
            (false, true) => match () {
                _ if is(Key::LeftArrow) => Some(Move(Motion::WordLeft, shift)),
                _ if is(Key::RightArrow) => Some(Move(Motion::WordRight, shift)),
                _ if is(Key::Backspace) => Some(Backspace { word: true }),
                _ if is(Key::Delete) => Some(Delete { word: true }),
                _ => None,
            },
            _ => None,
        };
        if own.is_some() {
            return own;
        }
    }
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
                '[' | '{' if shift => Fold(true),
                ']' | '}' if shift => Fold(false),
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
            command("d", true, false, false, 30, false),
            Some(Command::NextMatch)
        );
        assert_eq!(
            command("L", true, false, true, 30, false),
            Some(Command::AllMatches)
        );
        assert_eq!(
            command("K", true, false, true, 30, false),
            Some(Command::DeleteLines)
        );
        assert_eq!(
            command("/", true, false, false, 30, false),
            Some(Command::ToggleComment)
        );
        assert_eq!(
            command(&key(Key::UpArrow), false, true, false, 30, false),
            Some(Command::MoveLines { up: true })
        );
        assert_eq!(
            command(&key(Key::DownArrow), false, true, true, 30, false),
            Some(Command::CopyLines { up: false })
        );
        assert_eq!(
            command(&key(Key::DownArrow), true, true, false, 30, false),
            Some(Command::AddCursor { up: false })
        );
        assert_eq!(
            command(&key(Key::PageDown), false, false, true, 30, false),
            Some(Command::Vertical(30, true))
        );
        // Shift turns a bracket into a brace on most layouts.
        assert_eq!(
            command("{", true, false, true, 30, false),
            Some(Command::Fold(true))
        );
        assert_eq!(
            command("]", true, false, true, 30, false),
            Some(Command::Fold(false))
        );
        assert_eq!(
            command("[", true, false, false, 30, false),
            Some(Command::Outdent)
        );
    }

    /// What the editor does not take reaches the window: Ctrl+S saves,
    /// Ctrl+P finds a file.
    #[test]
    fn the_windows_chords_are_left_alone() {
        assert_eq!(command("s", true, false, false, 30, false), None);
        assert_eq!(command("p", true, false, false, 30, false), None);
        assert_eq!(command("P", true, false, true, 30, false), None);
        assert_eq!(command("s", true, false, false, 30, true), None);
    }

    /// A Mac's own keys: Cmd with an arrow is the line's or the document's
    /// edge, Option with one is a word, and the deletes follow — where the
    /// other platforms read Ctrl+Left as a word and Alt+Left as nothing.
    #[test]
    fn a_mac_has_its_own_arrows_and_deletes_over_the_shared_table() {
        let left = key(Key::LeftArrow);
        let up = key(Key::UpArrow);
        let back = key(Key::Backspace);
        assert_eq!(
            command(&left, true, false, false, 30, true),
            Some(Command::Move(Motion::Home, false))
        );
        assert_eq!(
            command(&left, true, false, true, 30, true),
            Some(Command::Move(Motion::Home, true))
        );
        assert_eq!(
            command(&up, true, false, false, 30, true),
            Some(Command::Edge {
                end: false,
                extend: false
            })
        );
        assert_eq!(
            command(&left, false, true, false, 30, true),
            Some(Command::Move(Motion::WordLeft, false))
        );
        assert_eq!(
            command(&back, true, false, false, 30, true),
            Some(Command::Kill { before: true })
        );
        assert_eq!(
            command(&back, false, true, false, 30, true),
            Some(Command::Backspace { word: true })
        );
        // Letters and the line-moving keys are the shared table's on a Mac.
        assert_eq!(
            command("d", true, false, false, 30, true),
            Some(Command::NextMatch)
        );
        assert_eq!(
            command(&up, false, true, false, 30, true),
            Some(Command::MoveLines { up: true })
        );
        assert_eq!(
            command(&up, true, true, false, 30, true),
            Some(Command::AddCursor { up: true })
        );
        // And the other platforms are what they were.
        assert_eq!(
            command(&left, true, false, false, 30, false),
            Some(Command::Move(Motion::WordLeft, false))
        );
        assert_eq!(command(&left, false, true, false, 30, false), None);
    }
}
