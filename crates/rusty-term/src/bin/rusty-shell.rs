//! The built-in shell as a program of its own.
//!
//! Everywhere but Windows the app runs the shell by re-entering itself
//! (`rusty --builtin-shell`), and in a debug build on Windows too. A release
//! build on Windows cannot: it is a GUI-subsystem executable — so that no
//! console window opens behind the app — and Windows connects a
//! pseudoconsole to console programs only. Started as the terminal's shell,
//! the app had no standard handles at all: it read end-of-input at once and
//! exited 0 having printed nothing, and `CONIN$`, `CONOUT$` and
//! `AllocConsole` do not reach the pseudoconsole from there either. So the
//! Tauri window's installer carried this, a console program that is
//! nothing but the shell (`rusty_term::builtin`), and ran it where it was
//! found. The Slint window starts the system's shell
//! (`rusty_term::default_shell`) and does not offer this one yet; when it
//! does, the same holds for its release build, which is a window program
//! too.

fn main() {
    rusty_term::builtin::run();
}
