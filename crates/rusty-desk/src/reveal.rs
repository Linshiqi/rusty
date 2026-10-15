//! Showing a file in the system's file manager, selected: the tree's
//! *Reveal in File Explorer* and a tab's. The one place the window hands a
//! path to Explorer, Finder or the desktop's file manager, so the rules
//! below are kept once.
//!
//! Explorer's `/select,"<path>"` goes as one raw argument — a space after
//! the comma makes Explorer open the home folder, and **so does a forward
//! slash in the path**: the tree and the tabs name a file relative to the
//! project with `/`, and joined onto the root that is
//! `E:\proj\core/src/lib.rs`, which every file API takes and Explorer reads
//! as switches. So the path goes over with backslashes, without a verbatim
//! prefix, quoted through `raw_arg` so a space in a folder name holds.
//! `open -R` on macOS; elsewhere, the containing folder through `xdg-open`,
//! since no file manager there takes a selection portably.

use std::path::Path;

/// The catalogue key of the menu row that reveals, named as the platform
/// names its file manager.
pub const fn label_key() -> &'static str {
    if cfg!(windows) {
        "context.tree-reveal-explorer"
    } else if cfg!(target_os = "macos") {
        "context.tree-reveal-finder"
    } else {
        "context.tree-reveal"
    }
}

/// Show `target` — an absolute path, a file or a folder — in the file
/// manager, selected where the platform can select. Started, not waited
/// for: the file manager is the user's from then on.
pub fn in_file_manager(target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = rusty_embed::process::command("explorer");
        command.raw_arg(explorer_select(target));
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = rusty_embed::process::command("open");
        command.arg("-R").arg(target);
        command
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut command = {
        let mut command = rusty_embed::process::command("xdg-open");
        command.arg(target.parent().unwrap_or(target));
        command
    };
    command.spawn().map(|_| ())
}

/// Explorer's argument for selecting `target`, in the form it documents:
/// `/select,"<path>"` — the switch and the path in one argument with no
/// space after the comma, the path quoted so a space in it does not end
/// it, and written the way Explorer reads one: backslashes, no `\\?\`.
#[cfg(any(windows, test))]
fn explorer_select(target: &Path) -> String {
    let path = rusty_embed::tools::plain(target)
        .to_string_lossy()
        .replace('/', "\\");
    format!("/select,\"{path}\"")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::explorer_select;

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
}
