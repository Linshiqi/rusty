//! rusty's installer for Windows: one executable that is the installer, the
//! updater's second half and, without its payload, the uninstaller.
//!
//! It replaced the Tauri bundler's NSIS installer when the window moved to
//! Slint, and it answers that installer's command line, because a copy of
//! rusty from before runs whatever its updater downloaded as
//! `setup.exe /P /UPDATE /R /ARGS …`: `/P` passive (no questions), `/S`
//! silent (nothing shown), `/R` open rusty afterwards with the arguments
//! after `/ARGS`. It installs where that installer did
//! (`%LOCALAPPDATA%\rusty`, or wherever the uninstall entry says), takes
//! over its entry and its shortcuts, and leaves none of its files behind.
//!
//! [`payload`] is the format, [`files`] what happens in the directory —
//! both portable, so the release workflow packs on any runner and the
//! tests run anywhere — and `shell` the registry and the shortcuts.

pub mod files;
pub mod payload;
#[cfg(windows)]
pub mod shell;

/// What the command line asked for.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Flags {
    /// `/P`: no questions, nothing but a failure shown.
    pub passive: bool,
    /// `/S`: nothing shown at all.
    pub silent: bool,
    /// `/R`: rusty opened when it is installed.
    pub restart: bool,
    /// What rusty is opened with — everything after `/ARGS`.
    pub args: Vec<String>,
    /// `--wait <pid>`: the rusty that asked for this, to be gone first.
    pub wait: Option<u32>,
    /// `--dir <path>`: where to install, over where the entry says.
    pub dir: Option<std::path::PathBuf>,
}

impl Flags {
    pub fn parse(args: &[String]) -> Flags {
        let mut flags = Flags::default();
        let mut at = 0;
        while let Some(arg) = args.get(at) {
            match arg.to_ascii_uppercase().as_str() {
                "/P" => flags.passive = true,
                "/S" => flags.silent = true,
                "/R" => flags.restart = true,
                // The Tauri installer's; this one installs the same way
                // whether it is an update or not.
                "/UPDATE" => flags.passive = true,
                "/ARGS" => {
                    flags.args = args[at + 1..].to_vec();
                    break;
                }
                "--WAIT" => {
                    flags.wait = args.get(at + 1).and_then(|pid| pid.parse().ok());
                    at += 1;
                }
                "--DIR" => {
                    flags.dir = args.get(at + 1).map(std::path::PathBuf::from);
                    at += 1;
                }
                _ => {}
            }
            at += 1;
        }
        flags
    }

    /// Whether anybody is asked anything.
    pub fn quiet(&self) -> bool {
        self.passive || self.silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Flags {
        let args: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        Flags::parse(&args)
    }

    /// What the Tauri updater runs a downloaded installer with.
    #[test]
    fn the_tauri_updaters_command_line_reads() {
        let flags = parse("/P /UPDATE /R /ARGS --new-window E:\\proj");
        assert!(flags.passive && flags.restart && !flags.silent);
        assert_eq!(flags.args, ["--new-window", "E:\\proj"]);
        assert!(flags.quiet());
    }

    #[test]
    fn rustys_own_command_line_reads() {
        let flags = parse("/S /R --wait 4242 --dir D:\\apps\\rusty");
        assert!(flags.silent && flags.restart);
        assert_eq!(flags.wait, Some(4242));
        assert_eq!(
            flags.dir.as_deref(),
            Some(std::path::Path::new("D:\\apps\\rusty"))
        );
        assert!(!parse("").quiet(), "double-clicked: it asks");
    }
}
