//! rusty's installer for Windows: one executable that is the installer, the
//! updater's second half and, without its payload, the uninstaller.
//!
//! Per user, with no elevation: `%LOCALAPPDATA%\Programs\rusty`, an entry
//! in Settings ▸ Apps, a Start menu shortcut and, on a first install, one
//! on the desktop. Double-clicked it asks before it installs; the updater
//! runs it as `--quiet --run --wait <pid>`.
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
    /// `--quiet`: no questions, nothing but a failure shown.
    pub quiet: bool,
    /// `--silent`: nothing shown at all.
    pub silent: bool,
    /// `--run`: rusty opened when it is installed.
    pub run: bool,
    /// What rusty is opened with — everything after `--`.
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
            match arg.as_str() {
                "--quiet" => flags.quiet = true,
                "--silent" => flags.silent = true,
                "--run" => flags.run = true,
                "--" => {
                    flags.args = args[at + 1..].to_vec();
                    break;
                }
                "--wait" => {
                    flags.wait = args.get(at + 1).and_then(|pid| pid.parse().ok());
                    at += 1;
                }
                "--dir" => {
                    flags.dir = args.get(at + 1).map(std::path::PathBuf::from);
                    at += 1;
                }
                _ => {}
            }
            at += 1;
        }
        flags
    }

    /// Whether nobody is asked anything.
    pub fn asks_nothing(&self) -> bool {
        self.quiet || self.silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Flags {
        let args: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        Flags::parse(&args)
    }

    /// What the updater runs a downloaded installer with.
    #[test]
    fn the_updaters_command_line_reads() {
        let flags = parse("--quiet --run --wait 4242");
        assert!(flags.quiet && flags.run && !flags.silent);
        assert_eq!(flags.wait, Some(4242));
        assert!(flags.asks_nothing());
    }

    #[test]
    fn a_directory_and_rustys_own_arguments_read() {
        let flags = parse("--silent --dir D:\\apps\\rusty -- --new-window E:\\proj");
        assert!(flags.silent && !flags.run);
        assert_eq!(
            flags.dir.as_deref(),
            Some(std::path::Path::new("D:\\apps\\rusty"))
        );
        assert_eq!(flags.args, ["--new-window", "E:\\proj"]);
        assert!(!parse("").asks_nothing(), "double-clicked: it asks");
    }
}
