//! The environment espup sets up, read back for the processes rusty starts.
//!
//! `espup install` puts the Xtensa GCC — the linker every `xtensa-esp32-*`
//! build needs — and its clang under the `esp` rustup toolchain, then makes
//! them reachable in one of two ways: on Windows it writes the user's PATH and
//! `LIBCLANG_PATH` into the registry, elsewhere it writes `~/export-esp.sh`
//! for the user to source. Neither reaches a process that is already running,
//! and rusty is that process: its setup sheet runs espup and then spawns
//! `cargo build` with the environment rusty was started with, and the build
//! dies with `linker xtensa-esp32-elf-gcc not found` — on the first run, on
//! the fresh machine, right after a setup that said the machine was ready.
//! Seen on a second machine the day v0.6.3 shipped.
//!
//! So rusty sources espup's environment itself, for its children only. The
//! export file is espup's own statement of what it installed and where, and
//! espup writes one on every platform (`export-esp.ps1` on Windows, beside
//! the registry write), so it is read first; the known layout under
//! `RUSTUP_HOME` is the fallback for a file that was deleted or written
//! somewhere else. Only directories that exist are used, so a stale file adds
//! nothing. The user's own environment is never written — that is espup's
//! job and the user's decision.

use std::path::{Path, PathBuf};

/// What espup asks a shell to export.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct EspEnv {
    /// Directories for a child's PATH, in the order the export file names them.
    pub path_dirs: Vec<PathBuf>,
    /// `LIBCLANG_PATH` as espup set it: the DLL itself on Windows, the `lib/`
    /// directory elsewhere. Passed through as-is; bindgen understands both.
    pub libclang: Option<PathBuf>,
}

/// espup's environment as it stands on this machine right now.
///
/// Not cached: the setup queue installs espup and runs a build in the same
/// process lifetime, which is exactly the moment the answer changes.
pub(crate) fn esp_env() -> EspEnv {
    let from_file = export_file()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .map(|text| parse_export(&text))
        .filter(|env| !env.path_dirs.is_empty());
    existing(from_file.unwrap_or_else(|| layout(rustup_home().as_deref())))
}

/// Where espup wrote its export file: `ESPUP_EXPORT_FILE` when set, as espup
/// itself honours it, else the home directory's `export-esp.ps1` on Windows
/// and `export-esp.sh` elsewhere.
fn export_file() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("ESPUP_EXPORT_FILE") {
        return Some(PathBuf::from(configured));
    }
    let name = if cfg!(windows) {
        "export-esp.ps1"
    } else {
        "export-esp.sh"
    };
    crate::tools::home_dir().map(|home| home.join(name))
}

/// The two shapes espup writes, read without a shell:
///
/// ```text
/// $Env:LIBCLANG_PATH = "C:\Users\me\.rustup\...\esp-clang\bin\libclang.dll"
/// $Env:PATH = "C:\Users\me\.rustup\toolchains\esp\xtensa-esp-elf\bin;" + $Env:PATH
/// export LIBCLANG_PATH="/home/me/.rustup/.../esp-clang/lib"
/// export PATH="/home/me/.rustup/toolchains/esp/xtensa-esp-elf/bin:$PATH"
/// ```
///
/// The quoted value is what matters; the reference to the existing PATH,
/// outside the quotes or inside them, is dropped. The list separator follows
/// the syntax — `;` for PowerShell, `:` for sh — because a Windows path has a
/// `:` of its own after the drive letter. A value still carrying a `$` is a
/// variable rusty did not expand, and is not offered as a path.
pub(crate) fn parse_export(text: &str) -> EspEnv {
    let mut env = EspEnv::default();
    for line in text.lines() {
        let line = line.trim();
        let (assignment, separator) = if let Some(rest) = strip_prefix_ignore_case(line, "$env:") {
            (rest, ';')
        } else if let Some(rest) = line.strip_prefix("export ") {
            (rest, ':')
        } else {
            continue;
        };
        let Some((name, value)) = assignment.split_once('=') else {
            continue;
        };
        // The first quoted run; everything after it is the shell's business.
        let Some(quoted) = value.split('"').nth(1) else {
            continue;
        };
        let mut parts = quoted
            .split(separator)
            .map(str::trim)
            .filter(|part| !part.is_empty() && !part.contains('$'))
            .map(PathBuf::from);
        let name = name.trim();
        if name.eq_ignore_ascii_case("PATH") {
            env.path_dirs.extend(parts);
        } else if name.eq_ignore_ascii_case("LIBCLANG_PATH") {
            env.libclang = parts.next();
        }
    }
    env
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// espup's layout when there is no export file to read: the toolchains it
/// installs under the `esp` rustup toolchain. `libclang` stays unset here —
/// the file it names differs by platform, and rusty does not guess at it.
fn layout(rustup_home: Option<&Path>) -> EspEnv {
    let Some(esp) = rustup_home.map(|home| home.join("toolchains").join("esp")) else {
        return EspEnv::default();
    };
    EspEnv {
        path_dirs: vec![
            esp.join("xtensa-esp-elf").join("bin"),
            esp.join("riscv32-esp-elf").join("bin"),
            esp.join("xtensa-esp32-elf-clang")
                .join("esp-clang")
                .join("bin"),
        ],
        libclang: None,
    }
}

/// `$RUSTUP_HOME`, else `~/.rustup` — rustup's own rule.
fn rustup_home() -> Option<PathBuf> {
    std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| crate::tools::home_dir().map(|home| home.join(".rustup")))
}

/// Only what is actually on disk: a toolchain removed since the file was
/// written must not put a dead directory on a child's PATH.
fn existing(env: EspEnv) -> EspEnv {
    EspEnv {
        path_dirs: env
            .path_dirs
            .into_iter()
            .filter(|dir| dir.is_dir())
            .collect(),
        libclang: env.libclang.filter(|path| path.exists()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file espup writes on Windows, as found on a machine it set up. The
    /// list separator is `;`, so the drive letter's colon survives.
    #[test]
    fn the_windows_export_file_is_read_back() {
        let text = r#"$Env:LIBCLANG_PATH = "C:\Users\me\.rustup\toolchains\esp\xtensa-esp32-elf-clang\esp-clang\bin\libclang.dll"
$Env:PATH = "C:\Users\me\.rustup\toolchains\esp\xtensa-esp32-elf-clang\esp-clang\bin;" + $Env:PATH
$Env:PATH = "C:\Users\me\.rustup\toolchains\esp\xtensa-esp-elf\bin;" + $Env:PATH
"#;
        let env = parse_export(text);
        assert_eq!(
            env.path_dirs,
            vec![
                PathBuf::from(
                    r"C:\Users\me\.rustup\toolchains\esp\xtensa-esp32-elf-clang\esp-clang\bin"
                ),
                PathBuf::from(r"C:\Users\me\.rustup\toolchains\esp\xtensa-esp-elf\bin"),
            ]
        );
        assert_eq!(
            env.libclang,
            Some(PathBuf::from(
                r"C:\Users\me\.rustup\toolchains\esp\xtensa-esp32-elf-clang\esp-clang\bin\libclang.dll"
            ))
        );
    }

    /// The file espup writes everywhere else. `$PATH` inside the quotes is the
    /// existing PATH, not a directory.
    #[test]
    fn the_unix_export_file_is_read_back() {
        let text = r#"export LIBCLANG_PATH="/home/me/.rustup/toolchains/esp/xtensa-esp32-elf-clang/esp-clang/lib"
export PATH="/home/me/.rustup/toolchains/esp/xtensa-esp-elf/bin:$PATH"
"#;
        let env = parse_export(text);
        assert_eq!(
            env.path_dirs,
            vec![PathBuf::from(
                "/home/me/.rustup/toolchains/esp/xtensa-esp-elf/bin"
            )]
        );
        assert_eq!(
            env.libclang,
            Some(PathBuf::from(
                "/home/me/.rustup/toolchains/esp/xtensa-esp32-elf-clang/esp-clang/lib"
            ))
        );
    }

    /// Two directories in one assignment, and a Windows list is split on `;`
    /// only — splitting on `:` would cut every drive letter off its path.
    #[test]
    fn a_powershell_list_keeps_its_drive_letters() {
        let env = parse_export(r#"$Env:PATH = "C:\a\bin;D:\b\bin;" + $Env:PATH"#);
        assert_eq!(
            env.path_dirs,
            vec![PathBuf::from(r"C:\a\bin"), PathBuf::from(r"D:\b\bin")]
        );
    }

    /// A value rusty would have to expand is not a path rusty knows.
    #[test]
    fn an_unexpanded_variable_is_not_offered_as_a_path() {
        let env =
            parse_export("export LIBCLANG_PATH=\"$HOME/lib\"\nexport PATH=\"$HOME/bin:$PATH\"\n");
        assert_eq!(env, EspEnv::default());
        assert_eq!(parse_export("# nothing here\n"), EspEnv::default());
    }

    #[test]
    fn directories_that_are_gone_are_not_offered() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present").join("bin");
        std::fs::create_dir_all(&present).unwrap();
        let gone = dir.path().join("gone").join("bin");
        let libclang = dir.path().join("libclang.dll");
        std::fs::write(&libclang, b"").unwrap();

        let env = existing(EspEnv {
            path_dirs: vec![gone.clone(), present.clone()],
            libclang: Some(libclang.clone()),
        });
        assert_eq!(env.path_dirs, vec![present]);
        assert_eq!(env.libclang, Some(libclang));

        let env = existing(EspEnv {
            path_dirs: vec![gone],
            libclang: Some(dir.path().join("missing.dll")),
        });
        assert_eq!(env, EspEnv::default());
    }

    /// Without an export file the toolchain's own layout is the answer, and
    /// it names no libclang — the file differs by platform.
    #[test]
    fn the_layout_under_rustup_home_is_the_fallback() {
        let home = Path::new("/rh");
        let env = layout(Some(home));
        assert_eq!(
            env.path_dirs[0],
            home.join("toolchains")
                .join("esp")
                .join("xtensa-esp-elf")
                .join("bin")
        );
        assert_eq!(env.libclang, None);
        assert_eq!(layout(None), EspEnv::default());
    }
}
