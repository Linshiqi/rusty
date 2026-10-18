//! What a Mac does differently, in one place.
//!
//! Slint hands the window one set of modifier flags, and on macOS it maps
//! the keyboard's two onto them the other way round from what the names
//! say: `modifiers.control` is **Cmd** and `modifiers.meta` is the
//! **physical Ctrl** key (i-slint-core's `input.rs` says so). The window's
//! bindings read `Ctrl+S` as Cmd+S there, which is what a Mac expects —
//! and the terminal read Cmd+C as the shell's interrupt and a physical
//! Ctrl+C as a plain `c`, which is not. [`Modifiers`] is the one reading of
//! the two flags, so every key handler asks the same question.
//!
//! The rest of the platform's shape is here beside it: the window keeps its
//! own frame on Windows and Linux and takes the system's on a Mac, so the
//! traffic lights and native fullscreen are there ([`select_backend`],
//! [`mark`]); a chord is shown with the Mac's own symbols ([`pretty`]);
//! and a second window is started through `open`, so the Dock shows one
//! rusty ([`launcher`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use slint::ComponentHandle;

use crate::{AppWindow, Theme, TornWindow};

/// Whether this is a Mac. A function rather than `cfg!` at every site, so
/// the pure rules below take it as a parameter and are tested both ways on
/// every platform — CLAUDE.md's rule for `shell_choices`.
pub fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

/// Slint's two modifier flags, read as what the keyboard means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modifiers {
    /// The chord key: Ctrl on Windows and Linux, Cmd on a Mac. What the
    /// window's bindings and the editor's VS Code keys answer to.
    pub command: bool,
    /// The physical Ctrl key, which on a Mac is not the chord key: it is
    /// what a terminal turns into a control code.
    pub control: bool,
}

impl Modifiers {
    /// From Slint's `control` and `meta` flags on `macos` or not.
    pub fn read(control: bool, meta: bool, macos: bool) -> Modifiers {
        if macos {
            Modifiers {
                command: control,
                control: meta,
            }
        } else {
            // Windows and Linux: the Windows key means nothing to the
            // editor, and Ctrl is both the chord key and the control key.
            Modifiers {
                command: control,
                control,
            }
        }
    }

    /// From a Slint key event's two flags on this platform.
    pub fn here(control: bool, meta: bool) -> Modifiers {
        Modifiers::read(control, meta, is_macos())
    }
}

/// A chord as the platform writes it: `Ctrl+Shift+P` as stored, or on a Mac
/// `⌘⇧P` — the symbols the menus use, Cmd for the control flag, since that
/// is the key the binding fires on there. Named keys keep their names.
pub fn pretty(chord: &str, macos: bool) -> String {
    if !macos {
        return chord.to_string();
    }
    chord
        .split_whitespace()
        .map(|stroke| match crate::keys::Stroke::parse(stroke) {
            Some(stroke) => {
                let mut out = String::new();
                if stroke.ctrl {
                    out.push('⌘');
                }
                if stroke.alt {
                    out.push('⌥');
                }
                if stroke.shift {
                    out.push('⇧');
                }
                out.push_str(&stroke.key);
                out
            }
            None => stroke.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Choose Slint's backend with what a Mac's window needs, before any window
/// exists. On a Mac the window keeps the system's frame (`no-frame` is off
/// there, see `app.slint`), drawn transparent with its title hidden and the
/// content under it, so the traffic lights sit over rusty's own title bar
/// and the green one gives native fullscreen. And winit's default menu bar
/// is left out: its Quit ends the process behind the window's back — no
/// unsaved-changes question, no hot exit — and its Hide takes Cmd+H from
/// Replace. Quit is the window's own binding (`app.exit`, Cmd+Q there),
/// which goes through `closing()`.
pub fn select_backend() -> Result<(), slint::PlatformError> {
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        use slint::winit_030::winit::platform::macos::{
            EventLoopBuilderExtMacOS, WindowAttributesExtMacOS,
        };
        let mut builder: slint::winit_030::EventLoopBuilder =
            slint::winit_030::winit::event_loop::EventLoop::with_user_event();
        builder.with_default_menu(false);
        slint::BackendSelector::new()
            .with_winit_event_loop_builder(builder)
            .with_winit_window_attributes_hook(|attributes| {
                attributes
                    .with_titlebar_transparent(true)
                    .with_title_hidden(true)
                    .with_fullsize_content_view(true)
            })
            .select()
    }
}

/// Tell the window which platform it is on, before it is shown: the title
/// bar leaves room for the traffic lights and drops its own buttons.
pub fn mark(window: &AppWindow) {
    window.global::<Theme>().set_macos(is_macos());
}

/// The same for a window torn off later, which has a Theme of its own.
pub fn mark_torn(window: &TornWindow) {
    window.global::<Theme>().set_macos(is_macos());
}

/// How to start this program again: the program and the arguments that go
/// before the caller's own. Inside a Mac app bundle — `…/rusty.app/Contents/
/// MacOS/rusty` — it is `open -n <bundle> --args`, so the new window is a
/// second instance of the *app* and the Dock shows one icon rather than a
/// second, nameless one for a bare binary; anywhere else it is the
/// executable itself.
pub fn launcher(exe: &Path) -> (PathBuf, Vec<OsString>) {
    if let Some(bundle) = bundle_of(exe) {
        return (
            PathBuf::from("/usr/bin/open"),
            vec![
                OsString::from("-n"),
                bundle.into_os_string(),
                OsString::from("--args"),
            ],
        );
    }
    (exe.to_path_buf(), Vec::new())
}

/// The `.app` an executable lives in, when it is a bundle's main binary:
/// the path ends `<name>.app/Contents/MacOS/<exe>`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a Mac the chord key is Cmd and the control key is Ctrl; elsewhere
    /// Ctrl is both and the Windows key is nothing.
    #[test]
    fn the_two_flags_read_as_the_keyboard_means_them() {
        assert_eq!(
            Modifiers::read(true, false, true),
            Modifiers {
                command: true,
                control: false
            },
            "Cmd alone on a Mac"
        );
        assert_eq!(
            Modifiers::read(false, true, true),
            Modifiers {
                command: false,
                control: true
            },
            "physical Ctrl alone on a Mac"
        );
        assert_eq!(
            Modifiers::read(true, false, false),
            Modifiers {
                command: true,
                control: true
            },
            "Ctrl on Windows"
        );
        assert_eq!(
            Modifiers::read(false, true, false),
            Modifiers {
                command: false,
                control: false
            },
            "the Windows key alone"
        );
    }

    /// A Mac shows its symbols for the modifiers and keeps a named key's
    /// name; the other platforms show the chord as it is stored.
    #[test]
    fn a_chord_is_shown_in_the_platforms_own_symbols() {
        assert_eq!(pretty("Ctrl+Shift+P", true), "⌘⇧P");
        assert_eq!(pretty("Ctrl+Alt+ArrowUp", true), "⌘⌥ArrowUp");
        assert_eq!(pretty("Ctrl+K Ctrl+S", true), "⌘K ⌘S");
        assert_eq!(pretty("F5", true), "F5");
        assert_eq!(pretty("Ctrl+Shift+P", false), "Ctrl+Shift+P");
        assert_eq!(pretty("", true), "");
    }

    /// A bundle's binary is started through `open`, so the Dock shows one
    /// app; a bare binary — a dev build, an installed Linux one — is itself.
    #[test]
    fn a_bundles_binary_is_started_through_open_and_a_bare_one_as_itself() {
        let (program, args) = launcher(Path::new("/Applications/rusty.app/Contents/MacOS/rusty"));
        assert_eq!(program, PathBuf::from("/usr/bin/open"));
        assert_eq!(
            args,
            [
                OsString::from("-n"),
                OsString::from("/Applications/rusty.app"),
                OsString::from("--args")
            ]
        );
        let (program, args) = launcher(Path::new("/home/me/rusty/target/debug/rusty"));
        assert_eq!(program, PathBuf::from("/home/me/rusty/target/debug/rusty"));
        assert!(args.is_empty());
        // Nearly a bundle is not one.
        assert_eq!(bundle_of(Path::new("/x/rusty.app/MacOS/rusty")), None);
        assert_eq!(bundle_of(Path::new("/x/rusty/Contents/MacOS/rusty")), None);
    }
}
