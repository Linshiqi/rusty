//! Finding the binaries the workbench drives.
//!
//! Its own module because four callers need it and none of them is about
//! simulation: the toolchain panel probes with it, `process::command` puts these
//! directories on a child's PATH, the installer checks whether its work
//! landed, and the simulator looks for QEMU. It lived in `simulate.rs`, which
//! meant `toolchain.rs` importing `simulate::find_tool` to answer "is espflash
//! installed" — a dependency that says nothing true about either module.
//!
//! **One ladder, in one order, for every binary** — [`find`]:
//!
//! 1. rusty's own `tools/` in the data directory: every `<family>/bin/` under
//!    it, and `<family>/` itself for the archives that unpack with no `bin/`.
//!    First, because a tool rusty downloaded on request has to be found or the
//!    panel keeps offering to install it again.
//! 2. `$CARGO_HOME/bin` (else `~/.cargo/bin`), where `cargo install` puts
//!    espflash and friends. Usually on PATH too — but not in a window opened
//!    before rustup ran, which is exactly the first-run machine.
//! 3. PATH.
//! 4. The tools the installer shipped beside the app — `bundled/` in Tauri's
//!    resource directory, the same `<family>/bin/` shape — which is what makes
//!    a fresh install able to simulate, debug and flash with no download at
//!    all. **Here, and not earlier**: everything in it is a program the user
//!    may have installed themselves, and a copy they chose on purpose is the
//!    one they meant. The bundle is a floor under a machine with nothing, not
//!    a preference. Only when its `PLATFORM` file names this machine, since a
//!    universal macOS bundle carries one architecture's binaries.
//!    - The exception is rusty's own QEMU ([`bundle_wins`]), which is searched
//!      at step 1: a stock `qemu-system-riscv32` wears the same name and has
//!      none of the peripherals, so one on PATH winning would quietly take the
//!      board view apart on a machine that had the right one in the bundle.
//! 5. The directories espup exports (`esp_env.rs`): the Xtensa GCC and clang
//!    it installed under the `esp` toolchain. Last, because that is where
//!    `process::command` puts them on a child's PATH — so "is the linker
//!    there" is answered here exactly as the build will answer it.
//!
//! Three finders used to each have their own order and two of them disagreed:
//! QEMU was PATH first and gdb was the data directory first, and espflash's
//! cargo fallback ignored `CARGO_HOME` while the pin map honoured it. A caller
//! that checks for a tool under one rule and runs it under another reports it
//! installed and then fails to start it.
//!
//! The child's PATH is a different question with the opposite answer:
//! `process::command` *appends* these directories, because what a child
//! resolves by name is `cc` asking for a compiler, and the one the user put on
//! PATH themselves is the one they meant. This ladder answers "which binary
//! does rusty run", and there the copy rusty installed wins.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config;

/// Where the installer put the tools it ships — set once by the app from
/// Tauri's resource directory. The CLI and the tests never set it and the
/// ladder simply has one root fewer.
static BUNDLED: OnceLock<PathBuf> = OnceLock::new();

/// Name the directory the installer shipped tools in (`<resources>/bundled`).
/// Idempotent; the first caller wins.
///
/// Spelled plainly first: Tauri's resource directory arrives as a Windows
/// verbatim path (`\\?\E:\…`) under `cargo tauri dev`, and QEMU joins its
/// `-L` directory to `esp32c3-rom.bin` with a forward slash, which the
/// verbatim prefix forbids — so it answered "ROM code binary not found" for
/// a file sitting exactly where the bundle had put it.
pub fn set_bundled_dir(dir: PathBuf) {
    let _ = BUNDLED.set(plain(&dir));
}

/// A path without Windows' verbatim prefix: `\\?\E:\x` is `E:\x`,
/// `\\?\UNC\host\share` is `\\host\share`, and anything else is itself.
/// Tools that take a path on their command line and go on to append to it
/// (QEMU, tar) cannot be handed the verbatim form.
pub fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

/// The bundled tools root, when the app set one and its binaries were built
/// for this machine. `bundled/PLATFORM` is written by
/// `scripts/bundle-tools.sh` with the target they are for; a bundle that names
/// another architecture — Intel macOS opening a universal app built with the
/// arm64 tools — is ignored rather than tried and blamed on the firmware.
///
/// The older layout wrote the file under `qemu/`, when QEMU was all the
/// bundle held. Both are read, so an app built before the bundle grew is
/// still understood by a newer library.
pub(crate) fn bundled_dir() -> Option<PathBuf> {
    let dir = BUNDLED.get()?;
    platform_matches(dir).then(|| dir.clone())
}

/// Whether a bundle's `PLATFORM` names this machine — the pure half, so a
/// test can put a directory anywhere and ask.
fn platform_matches(dir: &Path) -> bool {
    let Ok(platform) = std::fs::read_to_string(dir.join("PLATFORM"))
        .or_else(|_| std::fs::read_to_string(dir.join("qemu").join("PLATFORM")))
    else {
        return false;
    };
    host_platform().is_some_and(|here| platform.trim() == here)
}

/// Whether the bundle outranks whatever the machine already has.
///
/// **It does for exactly one thing.** Everything else the installer ships —
/// the debuggers, the flasher, the LLDB adapter — is the same program the
/// user may have installed themselves, and a copy they chose on purpose is
/// the one they meant; the bundle is a floor under a fresh machine, not a
/// preference. rusty's QEMU is different in kind: a stock `qemu-system-riscv32`
/// wears the same name and has no pin state, no GPIO interrupt, no converter
/// and neither bus, so letting one on PATH win would silently take the board
/// view apart on a machine that had everything it needed sitting in the
/// bundle.
fn bundle_wins(name: &str) -> bool {
    name.starts_with("qemu-system-")
}

/// A binary's file name on this platform.
pub(crate) fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Where a binary is, by the ladder in the module header, or `None` when this
/// machine has none.
pub(crate) fn find(name: &str) -> Option<PathBuf> {
    let bundle = bundled_dir();
    let (early, late) = match bundle_wins(name) {
        true => (bundle, None),
        false => (None, bundle),
    };
    let roots: Vec<PathBuf> = [data_tools_dir(), early].into_iter().flatten().collect();
    find_in_roots(name, &roots)
        .or_else(|| late.and_then(|dir| in_roots(name, std::slice::from_ref(&dir))))
        .or_else(|| in_dirs(name, &crate::esp_env::esp_env().path_dirs))
}

/// The first of `dirs` holding the binary.
fn in_dirs(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = exe(name);
    dirs.iter()
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// The ladder over any number of tools roots, in order — the data directory
/// and then the bundle — before cargo's bin and PATH.
pub(crate) fn find_in_roots(name: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    in_roots(name, roots)
        .or_else(|| cargo_bin().and_then(|bin| in_dirs(name, &[bin])))
        .or_else(|| on_path(name))
}

/// The tools roots alone, in order, and nothing after them — the half of the
/// ladder that answers "did rusty put this here", which the bundle needs to
/// ask *after* PATH rather than before it.
fn in_roots(name: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let file = exe(name);
    for tools in roots {
        for family in tool_families(tools) {
            for candidate in [family.join("bin").join(&file), family.join(&file)] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// The first match for a binary on PATH, and nothing else — the question
/// [`find`] asks last.
pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(exe(name));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// `tools/` in the data directory — where every archive the installer fetches
/// unpacks.
pub(crate) fn data_tools_dir() -> Option<PathBuf> {
    config::data_dir().map(|d| d.join("tools"))
}

/// Every `bin/` under rusty's tools directory, for putting on a child's PATH.
///
/// `cc` invokes the cross compiler *by name*, so a compiler rusty unpacked
/// into its own directory is one cargo cannot find however correctly the
/// panel reports it. Handing the directories to the child is what closes
/// that gap without touching the user's environment.
pub(crate) fn tool_bin_dirs() -> Vec<PathBuf> {
    // The bundle too, and after the data directory for the reason the ladder
    // gives — these are *appended* to the child's PATH, so anything the user
    // put there themselves is still found first.
    [data_tools_dir(), bundled_dir()]
        .into_iter()
        .flatten()
        .flat_map(|root| tool_families(&root))
        .map(|family| family.join("bin"))
        .filter(|bin| bin.is_dir())
        .collect()
}

/// The directories directly under `tools/`, sorted by name.
///
/// Sorted so the answer does not depend on the filesystem's enumeration
/// order: two families carrying the same binary name would otherwise be found
/// in a different order on different disks.
fn tool_families(tools: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(tools) else {
        return Vec::new();
    };
    let mut families: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    families.sort();
    families
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// cargo's home — `$CARGO_HOME`, else `~/.cargo` — where `cargo install` puts
/// binaries and where the registry keeps its sources.
pub(crate) fn cargo_home() -> Option<PathBuf> {
    cargo_home_from(
        std::env::var_os("CARGO_HOME").map(PathBuf::from),
        home_dir(),
    )
}

/// `cargo_home`'s rule, over values a test can choose.
fn cargo_home_from(configured: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    configured.or_else(|| home.map(|home| home.join(".cargo")))
}

fn cargo_bin() -> Option<PathBuf> {
    cargo_home().map(|home| home.join("bin"))
}

/// The platform in Espressif's asset naming — which rusty's own packages
/// reuse, so one ladder of URLs covers both.
///
/// Transcribed from the release's actual asset list rather than assembled
/// from `env::consts`: `x86_64-w64-mingw32` is not a string any pair of those
/// constants spells, and an asset name that is *nearly* right 404s exactly
/// like a network problem.
pub(crate) fn host_platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-w64-mingw32",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-linux-gnu",
        ("linux", "aarch64") => "aarch64-linux-gnu",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_name_carries_the_platforms_extension() {
        let named = exe("espflash");
        if cfg!(windows) {
            assert_eq!(named, "espflash.exe");
        } else {
            assert_eq!(named, "espflash");
        }
    }

    /// Probing must never fail the caller: a machine with none of these tools
    /// installed is the normal state before the toolchain panel is read.
    /// The verbatim prefix goes; a plain path, a UNC path and a POSIX path
    /// come back as they were. QEMU's `-L` is what this exists for.
    #[test]
    fn a_verbatim_windows_path_is_spelled_plainly() {
        assert_eq!(
            plain(Path::new(r"\\?\E:\CodeBase\rusty\target\debug\bundled")),
            PathBuf::from(r"E:\CodeBase\rusty\target\debug\bundled")
        );
        assert_eq!(
            plain(Path::new(r"\\?\UNC\nas\share\tools")),
            PathBuf::from(r"\\nas\share\tools")
        );
        assert_eq!(
            plain(Path::new(r"E:\CodeBase\rusty")),
            PathBuf::from(r"E:\CodeBase\rusty")
        );
        assert_eq!(plain(Path::new("/opt/rusty")), PathBuf::from("/opt/rusty"));
        assert_eq!(
            plain(Path::new(r"\\?\pipe\rusty")),
            PathBuf::from(r"\\?\pipe\rusty"),
            "a device path is not a drive path"
        );
    }

    /// The bundle is a second root: searched after the data directory, so a
    /// copy the user installed wins, and before PATH.
    #[test]
    fn the_bundle_is_searched_after_the_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data-tools");
        let bundle = dir.path().join("bundled");
        let bundled_qemu = bundle.join("qemu/bin").join(exe("qemu-system-riscv32"));
        std::fs::create_dir_all(bundled_qemu.parent().unwrap()).unwrap();
        std::fs::write(&bundled_qemu, b"ours").unwrap();
        std::fs::create_dir_all(&data).unwrap();

        let roots = [data.clone(), bundle.clone()];
        assert_eq!(
            find_in_roots("qemu-system-riscv32", &roots),
            Some(bundled_qemu.clone()),
            "with nothing in the data directory the bundle answers",
        );

        let own = data.join("qemu/bin").join(exe("qemu-system-riscv32"));
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, b"theirs").unwrap();
        assert_eq!(
            find_in_roots("qemu-system-riscv32", &roots),
            Some(own),
            "a copy in the data directory outranks the bundle",
        );
    }

    #[test]
    fn probing_for_something_absent_answers_none() {
        assert!(on_path("a-binary-nobody-has-installed-xyzzy").is_none());
        assert!(find("a-binary-nobody-has-installed-xyzzy").is_none());
    }

    /// The bundle is a floor, not a preference — except for the one binary
    /// that is not the program its name says. A user who installed espflash
    /// themselves gets theirs; a user with a stock QEMU on PATH still gets
    /// ours, because a stock one has none of the peripherals and the board
    /// view would come apart with the right emulator sitting in the bundle.
    #[test]
    fn only_the_emulator_outranks_what_the_machine_already_has() {
        assert!(bundle_wins("qemu-system-riscv32"));
        assert!(bundle_wins("qemu-system-xtensa"));
        assert!(!bundle_wins("espflash"));
        assert!(!bundle_wins("riscv32-esp-elf-gdb"));
        assert!(!bundle_wins("codelldb"));
    }

    /// The bundle the installer ships answers for every tool it claims to.
    ///
    /// Checked against the real directory `scripts/bundle-tools.sh` fills,
    /// because every failure this has had was a *shape* failure and not a
    /// logic one: an archive that nests its payload one level down, or names
    /// its binary per chip — Espressif ships no plain `xtensa-esp-elf-gdb`,
    /// only `xtensa-esp32-elf-gdb` and its siblings — so a script that
    /// unpacked something and declared victory would ship an installer whose
    /// debugger is not where the ladder looks.
    ///
    /// Skipped, aloud, on a checkout that has not run the script; it is not
    /// a thing every `cargo test` should have to download.
    #[test]
    fn the_shipped_bundle_answers_for_every_tool_it_carries() {
        let bundle = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rusty-app/bundled");
        if !platform_matches(&bundle) {
            eprintln!(
                "skipping: no bundle for this machine at {} — run scripts/bundle-tools.sh",
                bundle.display()
            );
            return;
        }
        let roots = [bundle.clone()];
        // The emulator, the debugger for each architecture, and the flasher.
        // CodeLLDB is not here: it keeps a directory of its own that the
        // ladder deliberately does not reach into, and `host_adapters` knows
        // that shape instead.
        for tool in [
            "qemu-system-riscv32",
            "qemu-system-xtensa",
            "riscv32-esp-elf-gdb",
            "espflash",
        ] {
            assert!(
                in_roots(tool, &roots).is_some(),
                "the bundle carries no {tool} the ladder can find"
            );
        }
        // Xtensa's gdb is named per chip, which is the one the simulator asks
        // for; the family's own name is not a binary at all.
        assert!(
            in_roots("xtensa-esp32-elf-gdb", &roots).is_some(),
            "the Xtensa debugger is not where simulate::find_gdb looks"
        );
    }

    /// A bundle that names another architecture is not used at all: a
    /// universal macOS app carries one architecture's binaries, and trying
    /// the wrong ones reads as the tool being broken. Both spellings of the
    /// platform file are read, because an app built before the bundle grew
    /// past QEMU wrote it one level down.
    #[test]
    fn a_bundle_is_used_only_when_it_names_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundled");
        std::fs::create_dir_all(bundle.join("qemu")).unwrap();

        std::fs::write(bundle.join("PLATFORM"), "some-other-machine\n").unwrap();
        assert!(
            !platform_matches(&bundle),
            "a bundle for another architecture is ignored"
        );

        if let Some(here) = host_platform() {
            std::fs::write(bundle.join("PLATFORM"), format!("{here}\n")).unwrap();
            assert!(platform_matches(&bundle));

            std::fs::remove_file(bundle.join("PLATFORM")).unwrap();
            std::fs::write(bundle.join("qemu").join("PLATFORM"), format!("{here}\n")).unwrap();
            assert!(
                platform_matches(&bundle),
                "the older layout wrote it under qemu/"
            );
        }
    }

    /// Both layouts the installer produces are searched: QEMU and the
    /// debuggers unpack to `<family>/bin/`, espflash's archive to `<family>/`.
    /// Missing the second is how a downloaded espflash kept being offered for
    /// download.
    #[test]
    fn the_data_directory_is_searched_with_and_without_a_bin_level() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        let qemu = tools.join("qemu/bin").join(exe("qemu-system-riscv32"));
        let espflash = tools.join("espflash").join(exe("espflash"));
        for file in [&qemu, &espflash] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"").unwrap();
        }

        assert_eq!(
            find_in_roots("qemu-system-riscv32", std::slice::from_ref(&tools)),
            Some(qemu)
        );
        assert_eq!(
            find_in_roots("espflash", std::slice::from_ref(&tools)),
            Some(espflash)
        );
        assert_eq!(
            find_in_roots(
                "a-binary-nobody-has-installed-xyzzy",
                std::slice::from_ref(&tools)
            ),
            None,
        );
        // The directory itself is not a binary, and a name matching a family
        // must not be answered with a folder.
        assert_eq!(find_in_roots("qemu", std::slice::from_ref(&tools)), None);
    }

    /// The copy rusty installed wins over whatever else the machine has, or
    /// the panel offers to install a tool that is already there.
    #[test]
    fn the_data_directory_comes_before_the_rest_of_the_ladder() {
        // `cargo` is on every machine that can run these tests; a copy in the
        // tools directory has to be the one reported.
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        let ours = tools.join("cargo-shim/bin").join(exe("cargo"));
        std::fs::create_dir_all(ours.parent().unwrap()).unwrap();
        std::fs::write(&ours, b"").unwrap();

        assert_eq!(
            find_in_roots("cargo", std::slice::from_ref(&tools)),
            Some(ours)
        );
        assert!(
            find_in_roots("cargo", &[]).is_some(),
            "and without a tools directory the ladder still reaches PATH",
        );
    }

    /// `CARGO_HOME` outranks the home directory, as it does for cargo itself;
    /// the pin map honoured this and the espflash finder did not, so the two
    /// disagreed about where cargo's binaries were.
    #[test]
    fn cargo_home_is_the_variable_when_set_and_the_home_directory_otherwise() {
        assert_eq!(
            cargo_home_from(
                Some(PathBuf::from("/opt/cargo")),
                Some(PathBuf::from("/home/me"))
            ),
            Some(PathBuf::from("/opt/cargo")),
        );
        assert_eq!(
            cargo_home_from(None, Some(PathBuf::from("/home/me"))),
            Some(PathBuf::from("/home/me").join(".cargo")),
        );
        assert_eq!(cargo_home_from(None, None), None);
    }
}
