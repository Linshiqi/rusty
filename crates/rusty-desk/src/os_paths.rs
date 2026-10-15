//! A project's file as the operating system names it: the path Copy Path
//! puts on the clipboard, and the file shown selected in the system's file
//! manager. One door for every surface that offers them — a tab's menu, the
//! tree's.
//!
//! The tree and the tabs name a file relative to the project with `/`, and
//! joined onto the root on Windows that is `E:\proj\core/src/lib.rs`: every
//! file API takes it, a person pasting it does not expect it, and Explorer
//! reads the slashes as switches. So what leaves the window is spelled the
//! system's way.

use std::path::{Path, PathBuf};

/// `path`, relative to the project at `root` and spelled with `/`, as an
/// absolute path in the system's own spelling — no verbatim prefix, the
/// system's separator throughout. What Copy Path copies.
pub fn absolute(root: &Path, path: &str) -> PathBuf {
    let mut full = rusty_embed::tools::plain(root);
    for part in path.split('/').filter(|part| !part.is_empty()) {
        full.push(part);
    }
    full
}

/// `path` relative to the project, in the system's spelling: what Copy
/// Relative Path copies, VS Code's `auto` separator.
pub fn relative(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

/// Show `path`, relative to the project at `root`, selected in the file
/// manager: Explorer on Windows, Finder on macOS, and elsewhere the folder
/// holding it, since no file manager there takes a selection portably.
/// Refuses what is not on disk, so a reveal never opens a folder with
/// nothing selected.
pub fn reveal(root: &Path, path: &str) -> Result<(), String> {
    let target = rusty_edit::absolute(root, path).map_err(|error| error.to_string())?;
    let target = rusty_embed::tools::plain(&target);
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = rusty_embed::process::command("explorer");
        command.raw_arg(explorer_select(&target));
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = rusty_embed::process::command("open");
        command.arg("-R").arg(&target);
        command
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut command = {
        let mut command = rusty_embed::process::command("xdg-open");
        command.arg(target.parent().unwrap_or(&target));
        command
    };
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    // Waited for, so no exited child is left behind; Explorer's own exit
    // status is 1 whether it found the file or not, so it says nothing.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Explorer's argument for selecting `target`, in the form it documents:
/// `/select,"<path>"` — the switch and the path in one argument with no
/// space after the comma (a space made Explorer open the home folder), the
/// path quoted so a space in it does not end it, and written with
/// backslashes: a forward slash is a switch to Explorer, which then finds
/// nothing to select and opens its default folder — the Desktop, with
/// Documents selected, measured through the Shell's window list. The
/// verbatim prefix (`\\?\`) it cannot read at all. Handed over as one raw
/// argument, since the standard library's quoting would quote it again.
#[cfg(any(windows, test))]
fn explorer_select(target: &Path) -> String {
    let path = rusty_embed::tools::plain(target)
        .to_string_lossy()
        .replace('/', "\\");
    format!("/select,\"{path}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `root.join("core/src/lib.rs")` is on Windows, and what a
    /// canonical path is: Explorer is handed neither, but the same file in
    /// the one form it reads, quoted so a space in a folder name holds.
    #[test]
    fn explorer_is_handed_a_path_it_reads() {
        assert_eq!(
            explorer_select(Path::new(r"E:\proj\core/src/lib.rs")),
            r#"/select,"E:\proj\core\src\lib.rs""#
        );
        assert_eq!(
            explorer_select(Path::new(r"\\?\E:\My Projects\src\main.rs")),
            r#"/select,"E:\My Projects\src\main.rs""#
        );
    }

    /// A tab's path joined onto its project reads as one path in the
    /// system's spelling, whatever the root came as.
    #[test]
    fn a_copied_path_is_spelled_the_systems_way() {
        let sep = std::path::MAIN_SEPARATOR;
        let root = Path::new("proj");
        assert_eq!(
            absolute(root, "core/src/lib.rs").to_string_lossy(),
            format!("proj{sep}core{sep}src{sep}lib.rs")
        );
        assert_eq!(
            relative("core/src/lib.rs"),
            format!("core{sep}src{sep}lib.rs")
        );
        assert_eq!(relative("Cargo.toml"), "Cargo.toml");
    }

    #[cfg(windows)]
    #[test]
    fn a_verbatim_root_is_copied_without_its_prefix() {
        assert_eq!(
            absolute(Path::new(r"\\?\E:\work\blinky"), "src/main.rs"),
            Path::new(r"E:\work\blinky\src\main.rs")
        );
    }
}
