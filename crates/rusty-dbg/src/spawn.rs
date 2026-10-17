//! The one way this crate starts a debugger.
//!
//! The same two rules `rusty_embed::process::command` applies, kept here
//! because rusty-embed is a dev-dependency of this crate and nothing more
//! (`Cargo.toml` says why): no console window on Windows, where every
//! probe would otherwise blink a black rectangle over the window, and no
//! inherited `RUSTUP_TOOLCHAIN` — the rustup shim exports it for rusty's
//! own build, and gdb has no business inheriting it: the same leak that
//! made a spawned cargo compile an esp project with stable. gdb's and the
//! adapter's both went through their own copies of these lines once, and a
//! rule written twice is a rule one of them drifts from.

use std::ffi::OsStr;
use std::process::Command;

/// A `Command` for `program` under the rules above.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.env_remove("RUSTUP_TOOLCHAIN");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
