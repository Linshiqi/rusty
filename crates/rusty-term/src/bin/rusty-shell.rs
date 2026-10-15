//! The built-in shell as a program of its own.
//!
//! A console program, because a release build of the window on Windows is a
//! GUI-subsystem executable — so that no console window opens behind it —
//! and Windows connects a pseudoconsole to console programs only. Started
//! as a terminal's shell, a window program has no standard handles at all:
//! it reads end-of-input at once and exits 0 having printed nothing, and
//! `CONIN$`, `CONOUT$` and `AllocConsole` do not reach the pseudoconsole
//! from there either. So this is nothing but the shell
//! (`rusty_term::builtin`). The window starts the system's shell
//! (`rusty_term::default_shell`) and does not run this one yet.

fn main() {
    rusty_term::builtin::run();
}
