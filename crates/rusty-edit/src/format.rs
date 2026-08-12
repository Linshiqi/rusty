//! rustfmt over stdin, for format-on-save.
//!
//! The binary is resolved the same wary way rust-analyzer's is: `rustup`'s
//! `rustfmt` proxy dispatches by the project's pinned toolchain, and an ESP
//! project pins `esp`, whose component set is not stable's. Asking rustup for
//! stable's rustfmt explicitly works for every project; stable formats any
//! edition it is told.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::model::Formatted;

/// Run the text through rustfmt.
///
/// `rel_path` is the file's project-relative path, used only to find the
/// nearest manifest for the edition — the text itself never touches disk, so
/// an unsaved buffer formats exactly as it reads in the editor.
pub fn format_rust(root: &Path, rel_path: &str, text: &str) -> Result<Formatted> {
    let edition = edition_of(root, rel_path);

    let mut command = Command::new(rustfmt_binary());
    command
        .arg("--edition")
        .arg(&edition)
        .arg("--emit")
        .arg("stdout")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut command);

    let mut child = command.spawn().map_err(|source| Error::Format {
        message: if source.kind() == std::io::ErrorKind::NotFound {
            "rustfmt is not installed — run `rustup component add rustfmt`".to_string()
        } else {
            format!("rustfmt could not start: {source}")
        },
    })?;

    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(text.as_bytes())
            .map_err(|source| Error::Format {
                message: format!("rustfmt stopped reading: {source}"),
            })?;
        // Dropped here: rustfmt reads to end-of-input before it answers.
    }

    let output = child.wait_with_output().map_err(|source| Error::Format {
        message: format!("rustfmt did not finish: {source}"),
    })?;

    if !output.status.success() {
        // Usually a parse error, which is normal mid-edit. The first real
        // line is the one that names it; the rest is context nobody reads
        // in a status bar.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("rustfmt failed")
            .to_string();
        return Err(Error::Format { message: reason });
    }

    let formatted = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(Formatted {
        changed: formatted != text,
        text: formatted,
    })
}

/// The rustfmt to run: stable's own binary when rustup can name it, else
/// whatever `rustfmt` resolves to on PATH.
fn rustfmt_binary() -> String {
    let mut command = Command::new("rustup");
    command.args(["which", "--toolchain", "stable", "rustfmt"]);
    no_window(&mut command);
    if let Ok(output) = command.output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }
    "rustfmt".to_string()
}

/// The edition of the crate `rel_path` belongs to.
///
/// Walks up from the file to the nearest `Cargo.toml` with an `edition` key;
/// `edition.workspace = true` sends the search to the root manifest's
/// `[workspace.package]`. A scan, not a TOML parse: the two spellings a
/// manifest actually uses are `edition = "..."` and `edition.workspace`, and
/// a parser dependency for one key is not worth its build time.
fn edition_of(root: &Path, rel_path: &str) -> String {
    let mut dir = Path::new(rel_path).parent();
    while let Some(current) = dir {
        let manifest = root.join(current).join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            match edition_in(&text) {
                EditionKey::Version(edition) => return edition,
                EditionKey::Workspace => break,
                EditionKey::Absent => {}
            }
        }
        dir = current.parent();
    }
    // The workspace root, or nothing found on the way up.
    if let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml"))
        && let EditionKey::Version(edition) = edition_in(&text)
    {
        return edition;
    }
    "2021".to_string()
}

enum EditionKey {
    Version(String),
    Workspace,
    Absent,
}

fn edition_in(manifest: &str) -> EditionKey {
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("edition") {
            let rest = rest.trim_start();
            if rest.starts_with(".workspace") {
                return EditionKey::Workspace;
            }
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.trim().trim_matches('"');
                if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
                    return EditionKey::Version(value.to_string());
                }
            }
        }
    }
    EditionKey::Absent
}

fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real rustfmt, because the failure modes worth catching — a missing
    // component, an edition mismatch, stdin handling — are all in the binary,
    // not in our plumbing.
    #[test]
    fn formats_and_reports_change() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-fmt")
            .tempdir()
            .expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nedition = \"2021\"\n",
        )
        .expect("manifest");

        let messy = "fn main(){let x=1;println!(\"{x}\");}\n";
        let result = format_rust(dir.path(), "src/main.rs", messy).expect("format");
        assert!(result.changed);
        assert!(result.text.contains("let x = 1;"), "got: {}", result.text);

        let again = format_rust(dir.path(), "src/main.rs", &result.text).expect("format");
        assert!(!again.changed, "formatting twice must be a fixpoint");
    }

    #[test]
    fn a_parse_error_names_the_problem() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-fmt")
            .tempdir()
            .expect("tempdir");
        let error = format_rust(dir.path(), "src/main.rs", "fn main( {").unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("error") || message.contains("expected"),
            "the caller needs rustfmt's reason, got: {message}"
        );
    }

    #[test]
    fn workspace_edition_is_followed_to_the_root() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-fmt")
            .tempdir()
            .expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\n[workspace.package]\nedition = \"2024\"\n",
        )
        .expect("root");
        std::fs::create_dir_all(dir.path().join("app")).expect("dir");
        std::fs::write(
            dir.path().join("app/Cargo.toml"),
            "[package]\nname = \"app\"\nedition.workspace = true\n",
        )
        .expect("member");

        assert_eq!(edition_of(dir.path(), "app/src/main.rs"), "2024");
    }
}
