//! Running `git`, the same way for every question: one process in the
//! project, its answer read as bytes or text, and its failure named.
//!
//! `core.quotepath` is off per call so a path with a Chinese character in it
//! arrives as itself rather than as octal escapes, colour is off because this
//! is a machine reading it, and `GIT_OPTIONAL_LOCKS=0` stops a background
//! `git status` from taking `index.lock` to refresh the index — which made a
//! commit typed in a terminal at the same moment fail with "Unable to create
//! index.lock", the panel's read beating the user's write.

use std::path::Path;
use std::process::Command;

use super::{Error, Result};

/// Whether `root` is inside a working tree — asked only after a command has
/// failed, to tell "not a repository", the one refusal the panel answers
/// with an Initialize button, from every other failure. Not by reading
/// git's message: that is translated when the user's git is.
fn inside_work_tree(root: &Path) -> bool {
    let mut command = Command::new("git");
    command
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0");
    no_window(&mut command);
    command.output().ok().is_some_and(|out| {
        out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
    })
}

/// One `git` invocation, its stdout as text.
pub(super) fn run<S: AsRef<str>>(root: &Path, args: &[S]) -> Result<String> {
    run_allowing(root, args, &[0])
}

/// [`run`], treating any of `ok` as success — for the commands whose exit
/// code is an answer rather than a verdict.
pub(super) fn run_allowing<S: AsRef<str>>(root: &Path, args: &[S], ok: &[i32]) -> Result<String> {
    run_bytes(root, args, ok).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The invocation itself, stdout as bytes — what [`blob`](super::blob)
/// needs, and what every text reader lossily decodes.
pub(super) fn run_bytes<S: AsRef<str>>(root: &Path, args: &[S], ok: &[i32]) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .args(["-c", "core.quotepath=off", "-c", "color.ui=never"])
        .args(args.iter().map(AsRef::as_ref))
        .current_dir(root)
        .env_remove("RUSTUP_TOOLCHAIN");
    // The pager and the editor must never be consulted: this is a machine
    // asking, and a git that waited on either would hang the panel. And no
    // optional lock: see the module header.
    command
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    no_window(&mut command);
    let output = command.output().map_err(|source| Error::Spawn { source })?;
    let accepted = output.status.code().is_some_and(|code| ok.contains(&code));
    if !accepted {
        if !inside_work_tree(root) {
            return Err(Error::NotARepository {
                path: root.display().to_string(),
            });
        }
        return Err(Error::Git {
            command: args
                .iter()
                .map(AsRef::as_ref)
                .next()
                .unwrap_or("")
                .to_string(),
            detail: failure_detail(&String::from_utf8_lossy(&output.stderr)),
        });
    }
    Ok(output.stdout)
}

/// What a failed `git` said, as the lines that are the failure.
///
/// The first line of stderr was taken, and git writes its warnings first: on
/// a machine with `core.autocrlf=true`, staging a file with LF endings prints
/// `warning: in the working copy of '.gitignore', LF will be replaced by
/// CRLF…` before anything else, so a stage that failed on something real
/// was reported as that harmless warning — and the reason, three lines down,
/// was dropped. The `error:` and `fatal:` lines are the failure, in git's
/// order; with none, whatever is not a warning or a hint; with nothing else,
/// all of it rather than nothing.
fn failure_detail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let failures: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("error:") || line.starts_with("fatal:"))
        .collect();
    let chosen = if !failures.is_empty() {
        failures
    } else {
        let plain: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|line| !line.starts_with("warning:") && !line.starts_with("hint:"))
            .collect();
        if plain.is_empty() { lines } else { plain }
    };
    if chosen.is_empty() {
        return "no message".to_string();
    }
    chosen.into_iter().take(6).collect::<Vec<_>>().join("\n")
}

/// No console window for a child of the GUI on Windows.
fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::failure_detail;

    /// git's own words from the reproduction, in git's order: two warnings,
    /// the error, the fatal line. The warnings are not the failure.
    #[test]
    fn the_failure_is_the_error_lines_not_the_warnings_before_them() {
        let stderr = "warning: in the working copy of '.gitignore', LF will be replaced by CRLF the next time Git touches it\n\
                      warning: in the working copy of 'README.md', LF will be replaced by CRLF the next time Git touches it\n\
                      error: 'firmware/' does not have a commit checked out\n\
                      fatal: adding files failed\n";
        assert_eq!(
            failure_detail(stderr),
            "error: 'firmware/' does not have a commit checked out\nfatal: adding files failed"
        );
    }

    /// A refusal git words without a prefix keeps its words; hints go; and a
    /// stderr of nothing but warnings is still said rather than lost.
    #[test]
    fn unprefixed_words_stay_hints_go_and_warnings_alone_are_kept() {
        let ignored = "The following paths are ignored by one of your .gitignore files:\n\
                       target\n\
                       hint: Use -f if you really want to add them.\n";
        assert_eq!(
            failure_detail(ignored),
            "The following paths are ignored by one of your .gitignore files:\ntarget"
        );
        assert_eq!(
            failure_detail("warning: something odd\n"),
            "warning: something odd"
        );
        assert_eq!(failure_detail("  \n"), "no message");
    }
}
