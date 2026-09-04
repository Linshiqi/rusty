//! Fetching the tools rusty drives but does not ship.
//!
//! QEMU, the two esp-gdb builds and the RISC-V C toolchain are archives on
//! somebody else's release page. Installing one is the same three steps every
//! time — pick the asset for this platform, fetch it down a ladder of
//! mirrors, unpack it into the data directory — so it is one module rather
//! than three copies inside the panels that happen to need them.
//!
//! **Versions are pinned, never discovered.** An installer that fetches
//! "latest" breaks the day upstream changes its asset layout, and it breaks
//! for everyone at once, with an error about a 404 rather than about a
//! version. Each constant below names the build this pipeline is proven
//! against; bumping one is a deliberate act with a test run behind it.
//!
//! Two hosts for every asset, because the first is unreachable from some
//! networks entirely: GitHub, then Espressif's own mirror
//! (`dl.espressif.com`), which exists precisely for that and serves identical
//! bytes. [`crate::net`] adds the proxy ladder underneath.

use std::path::{Path, PathBuf};

use crate::{
    error::{Error, Result},
    model::{CommandPlan, REPO},
    net, tools,
};

/// The esp-gdb release the installer pulls.
pub(crate) const GDB_RELEASE: &str = "esp-gdb-v14.2_20240403";
const GDB_VERSION: &str = "14.2_20240403";

/// The crosstool-NG release the RISC-V C toolchain comes from.
const GCC_RELEASE: &str = "esp-16.1.0_20260609";
const GCC_VERSION: &str = "16.1.0_20260609";

/// The QEMU release every install pulls.
const QEMU_RELEASE: &str = "esp-develop-9.2.2-20260417";
const QEMU_VERSION: &str = "esp_develop_9.2.2_20260417";

/// rusty's own build of the release above, carrying the GPIO device model in
/// `qemu/`. Where Espressif's emulator discards every GPIO write, this one
/// keeps the registers, so the board view can show what a pin *is* rather
/// than what the firmware said it set.
///
/// The tag is pinned for the reason in this module's header; the repository
/// it hangs off is [`REPO`], named once for the whole crate.
const RUSTY_QEMU_TAG: &str = "qemu-v1";

/// An archive to fetch: where to put it, the URLs to try in order, and the
/// extraction step. One shape for QEMU, the debuggers and the C toolchain —
/// the name says so, where `QemuDownload` used to claim only the first.
pub struct ToolDownload {
    pub archive: PathBuf,
    pub urls: Vec<String>,
    pub extract: CommandPlan,
}

/// How to install a tool the plan reported missing, as inspectable steps —
/// one click in the panel, the dock shows every line, and only a failure
/// sends anyone to the manual instructions.
///
/// The public entry: the recipe table in `toolchain` covers every cargo- and
/// rustup-installed tool, and this adds the one refusal that table cannot
/// know about — QEMU is an archive, not a recipe.
pub fn install_steps(tool: &str) -> Result<Vec<CommandPlan>> {
    if tool.starts_with("qemu-system-") {
        return Err(Error::refused(format!(
            "qemu installs through its own download path — this is a bug if it surfaces; \
             manual fallback: https://github.com/espressif/qemu/releases/tag/{QEMU_RELEASE} \
             into the data directory's tools/"
        )));
    }
    crate::toolchain::recipe(tool)
}

/// Where every archive unpacks — resolved, not created. A plan is made
/// against this path and the directory is made only once there is a plan:
/// asking about a tool rusty does not know must not leave a directory behind,
/// and nor must a test.
fn data_tools_dir() -> Result<PathBuf> {
    tools::data_tools_dir().ok_or_else(|| Error::Config {
        detail: "the data directory could not be resolved".to_string(),
    })
}

/// Make the directory an archive is about to land in.
fn prepare(plan: &ToolDownload) -> Result<()> {
    if let Some(parent) = plan.archive.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

/// The unpack step for an archive already downloaded.
///
/// `program` is the absolute Windows tar where one is needed: it is bsdtar,
/// which reads zip, whereas a bare `tar` can resolve to MSYS GNU tar on PATH,
/// which does not.
fn extract_step(archive: &Path, tools: &Path, rationale: &str, absolute_tar: bool) -> CommandPlan {
    let archive_text = archive.to_string_lossy().into_owned();
    let tools_text = tools.to_string_lossy().into_owned();
    CommandPlan {
        program: if absolute_tar {
            "C:\\Windows\\System32\\tar.exe".to_string()
        } else {
            "tar".to_string()
        },
        args: vec![
            "-xf".to_string(),
            archive_text.clone(),
            "-C".to_string(),
            tools_text.clone(),
        ],
        display: format!("tar -xf {archive_text} -C {tools_text}"),
        rationale: rationale.to_string(),
        warning: None,
    }
}

/// Two mirrors for one Espressif asset: GitHub, then their own asset host.
fn espressif_urls(project: &str, release: &str, asset: &str) -> Vec<String> {
    vec![
        format!("https://github.com/espressif/{project}/releases/download/{release}/{asset}"),
        format!(
            "https://dl.espressif.com/github_assets/espressif/{project}/releases/download/{release}/{asset}"
        ),
    ]
}

/// The archive for a gdb family, mirror ladder included.
pub fn gdb_download(tool: &str) -> Result<ToolDownload> {
    let plan = gdb_download_into(tool, &data_tools_dir()?)?;
    prepare(&plan)?;
    Ok(plan)
}

/// [`gdb_download`] against a tools directory the caller names — the pure
/// half, which is what the tests exercise so they never touch the real data
/// directory.
fn gdb_download_into(tool: &str, tools: &Path) -> Result<ToolDownload> {
    if tool != "xtensa-esp-elf-gdb" && tool != "riscv32-esp-elf-gdb" {
        return Err(Error::refused(format!(
            "{tool} is not a gdb this installer knows"
        )));
    }
    if !cfg!(windows) {
        return Err(Error::refused(format!(
            "one-click install only knows the Windows build so far — download {tool} from \
             https://github.com/espressif/binutils-gdb/releases/tag/{GDB_RELEASE} and unpack \
             it into the data directory's tools/"
        )));
    }
    let asset = format!("{tool}-{GDB_VERSION}-x86_64-w64-mingw32.zip");
    let archive = tools.join(format!("{tool}.zip"));
    Ok(ToolDownload {
        urls: espressif_urls("binutils-gdb", GDB_RELEASE, &asset),
        extract: extract_step(
            &archive,
            tools,
            "unpacks the gdb bundle into the data directory's tools/",
            true,
        ),
        archive,
    })
}

/// The C cross compiler for a RISC-V ESP part, from Espressif's crosstool-NG
/// releases — the same archive-and-unpack shape as QEMU and the debuggers.
///
/// espup installs the Xtensa compiler as part of the toolchain it manages and
/// installs nothing for RISC-V, because rustc needs no help emitting RISC-V.
/// So this is the one C toolchain a user has to fetch on purpose, and making
/// them do it by hand is the half-answer this workbench exists to avoid.
///
/// The asset name was checked rather than assumed: `riscv32-esp-elf-gcc` and
/// `-{tag}-` spellings both 404, and only this one answers 200.
pub fn gcc_download(tool: &str) -> Result<ToolDownload> {
    let plan = gcc_download_into(tool, &data_tools_dir()?)?;
    prepare(&plan)?;
    Ok(plan)
}

fn gcc_download_into(tool: &str, tools: &Path) -> Result<ToolDownload> {
    if tool != "riscv32-esp-elf-gcc" {
        return Err(Error::refused(format!(
            "{tool} is not a C toolchain this installer knows"
        )));
    }
    if !cfg!(windows) {
        return Err(Error::refused(format!(
            "one-click install only knows the Windows build so far — download \
             riscv32-esp-elf from \
             https://github.com/espressif/crosstool-NG/releases/tag/{GCC_RELEASE} and put \
             its bin/ on PATH"
        )));
    }
    let asset = format!("riscv32-esp-elf-{GCC_VERSION}-x86_64-w64-mingw32.zip");
    let archive = tools.join("riscv32-esp-elf.zip");
    Ok(ToolDownload {
        urls: espressif_urls("crosstool-NG", GCC_RELEASE, &asset),
        extract: extract_step(
            &archive,
            tools,
            "unpacks the RISC-V C toolchain into the data directory's tools/, which moves \
             with it when the directory is relocated",
            true,
        ),
        archive,
    })
}

/// Which platforms rusty's own QEMU release actually carries.
///
/// Espressif additionally ships `aarch64-linux-gnu` and `x86_64-apple-darwin`
/// and rusty does not, so those users fall through to the stock emulator and
/// everything works as it always has — minus real pin state. Listing what we
/// build rather than letting the URL 404 keeps "we do not build that" separate
/// from "the download failed", which are different things to tell somebody.
///
/// Intel macOS is absent for a dull reason worth writing down: GitHub retired
/// the `macos-13` runner, so that job queued for 103 minutes while the other
/// three finished in four to fifteen and would never have been picked up.
/// Adding it back needs a runner label somebody has watched work — guessing
/// one costs another hour of queue to disprove.
fn rusty_qemu_asset(platform: &str) -> Option<String> {
    matches!(
        platform,
        "x86_64-linux-gnu" | "aarch64-apple-darwin" | "x86_64-w64-mingw32"
    )
    .then(|| {
        let version = RUSTY_QEMU_TAG.trim_start_matches("qemu-");
        format!("qemu-rusty-{version}-{platform}.tar.xz")
    })
}

pub fn qemu_download(tool: &str) -> Result<ToolDownload> {
    let plan = qemu_download_into(tool, &data_tools_dir()?)?;
    prepare(&plan)?;
    Ok(plan)
}

fn qemu_download_into(tool: &str, tools: &Path) -> Result<ToolDownload> {
    let Some(arch) = tool.strip_prefix("qemu-system-") else {
        return Err(Error::refused(format!(
            "{tool} is not a qemu emulator name"
        )));
    };
    let Some(platform) = tools::host_platform() else {
        return Err(Error::refused(format!(
            "no {tool} build is published for {}-{} — build QEMU from \
             https://github.com/espressif/qemu/releases/tag/{QEMU_RELEASE} and unpack it \
             into the data directory's tools/qemu/",
            std::env::consts::OS,
            std::env::consts::ARCH,
        )));
    };

    let asset = format!("qemu-{arch}-softmmu-{QEMU_VERSION}-{platform}.tar.xz");
    let archive = tools.join(format!("qemu-{arch}.tar.xz"));
    // rusty's build first, Espressif's behind it. Both unpack to the same
    // `qemu/` directory, so whichever answers, the rest of the install is
    // identical — and a failure to reach ours degrades to the emulator this
    // workbench has always used rather than to no emulator at all.
    //
    // Ours carries both emulators in one package, so a request for either
    // arch is satisfied by the same file.
    let mut urls = Vec::new();
    if let Some(ours) = rusty_qemu_asset(platform) {
        urls.push(format!("{REPO}/releases/download/{RUSTY_QEMU_TAG}/{ours}"));
    }
    urls.extend(espressif_urls("qemu", QEMU_RELEASE, &asset));

    Ok(ToolDownload {
        urls,
        extract: extract_step(
            &archive,
            tools,
            "unpacks into the data directory's tools/qemu — bsdtar handles .tar.xz and \
             ships with Windows",
            false,
        ),
        archive,
    })
}

/// CodeLLDB, pinned like every other archive here.
///
/// The debug adapter that makes host debugging work on Windows: Rust's
/// default target there emits a PDB, gdb reads DWARF, and LLDB is what reads
/// the difference. CodeLLDB bundles its own LLDB, so this is one download and
/// no system dependency — which is the point, because the `lldb-dap` that
/// LLVM ships for Windows has been observed to start and then answer nothing
/// at all.
///
/// Fetched rather than bundled: it is tens of megabytes of debugger per
/// platform, and only the people who debug host code on Windows need it.
const CODELLDB_RELEASE: &str = "v1.12.3";

/// The asset for a platform, in CodeLLDB's own naming.
///
/// Transcribed from the release's actual asset list rather than assembled
/// from `env::consts`, for the reason [`tools::host_platform`] gives: a name
/// that is nearly right 404s exactly like a network problem. `armhf` and the
/// bootstrap package are deliberately absent — rusty has no 32-bit ARM host
/// to offer them to.
fn codelldb_asset(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("windows", "x86_64") => "codelldb-win32-x64.vsix",
        ("macos", "aarch64") => "codelldb-darwin-arm64.vsix",
        ("macos", "x86_64") => "codelldb-darwin-x64.vsix",
        ("linux", "x86_64") => "codelldb-linux-x64.vsix",
        ("linux", "aarch64") => "codelldb-linux-arm64.vsix",
        _ => return None,
    })
}

/// Where the adapter ends up, relative to `tools/`.
///
/// A `.vsix` is a zip whose payload sits under `extension/`, so it unpacks
/// into a directory of its own rather than over `tools/`: the adapter has to
/// keep the `lldb/` beside it that it loads its debugger from, and dropping
/// a bare `extension/` into the shared directory would name nothing.
pub(crate) const CODELLDB_DIR: &str = "codelldb";

/// Whether this platform has a published adapter to fetch.
///
/// The panel asks before drawing an Install button: CodeLLDB publishes no
/// ARM64 Windows build, and a button that always fails is worse than the
/// link it sits on top of — the same rule the chip picker follows for a part
/// behind another HAL.
pub fn codelldb_available() -> bool {
    codelldb_asset(std::env::consts::OS, std::env::consts::ARCH).is_some()
}

/// Where to get it by hand, for the platforms rusty cannot fetch for.
pub fn codelldb_page() -> String {
    format!("https://github.com/vadimcn/codelldb/releases/tag/{CODELLDB_RELEASE}")
}

pub fn codelldb_download() -> Result<ToolDownload> {
    let plan = codelldb_download_into(&data_tools_dir()?)?;
    prepare(&plan)?;
    // `tar -C` will not make its destination, and this one is a directory of
    // rusty's own choosing rather than something the archive brings.
    if let Some(into) = plan.extract.args.last() {
        std::fs::create_dir_all(into).map_err(|source| Error::Write {
            path: into.clone(),
            source,
        })?;
    }
    Ok(plan)
}

fn codelldb_download_into(tools: &Path) -> Result<ToolDownload> {
    let asset = codelldb_asset(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        Error::refused(format!(
            "CodeLLDB publishes no build for {}/{}, so rusty cannot fetch one. Manual \
             fallback: https://github.com/vadimcn/codelldb/releases/tag/{CODELLDB_RELEASE}",
            std::env::consts::OS,
            std::env::consts::ARCH,
        ))
    })?;
    let archive = tools.join("codelldb.vsix");
    let into = tools.join(CODELLDB_DIR);
    Ok(ToolDownload {
        urls: vec![format!(
            "https://github.com/vadimcn/codelldb/releases/download/{CODELLDB_RELEASE}/{asset}"
        )],
        extract: extract_step(
            &archive,
            &into,
            "unpacks the debug adapter into the data directory's tools/codelldb — a .vsix is \
             a zip, which bsdtar reads",
            cfg!(windows),
        ),
        archive,
    })
}

/// Fetch `urls` in order until one delivers, streaming progress through
/// `progress`. In-process on rustls: the OS TLS stack (schannel) aborts on
/// some CDNs with "server closed abruptly", and a spawned curl cannot be
/// relied on to exist unbroken everywhere.
pub fn download(urls: &[String], dest: &Path, mut progress: impl FnMut(String)) -> Result<()> {
    use std::io::{Read, Write};
    use std::time::Duration;

    // Every mirror, over every route — see `net` for why one proxy URL is
    // not one route. Each attempt is named as it is tried.
    let routes = net::proxy_candidates(net::effective_proxy());
    let deadlines = || net::Deadlines {
        connect: Duration::from_secs(15),
        // Headers must arrive promptly or this route is declared dead and
        // the next one gets its turn — a blackholed route must not hang the
        // panel at "downloading" forever.
        headers: Some(Duration::from_secs(30)),
        // And nothing runs unbounded: a stalled body eventually dies.
        total: Duration::from_secs(15 * 60),
    };
    let write_failed = |error: std::io::Error| Error::Write {
        path: dest.display().to_string(),
        source: error,
    };

    let mut last_error = String::new();
    let mut attempts: Vec<(String, Option<String>)> = Vec::new();
    for url in urls {
        for route in &routes {
            attempts.push((url.clone(), route.clone()));
        }
    }

    for (url, route) in attempts {
        match &route {
            Some(proxy) => progress(format!("downloading {url}\n  via {proxy}")),
            None => progress(format!("downloading {url}\n  direct")),
        }
        let agent = net::agent(route.as_deref(), deadlines());
        let response = match agent.get(&url).call() {
            Ok(response) => response,
            Err(error) => {
                let chain = net::error_chain(&error);
                last_error = format!("{url}: {chain}");
                progress(format!("  failed: {chain} — trying the next route"));
                continue;
            }
        };
        let total = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        match total {
            Some(total) => progress(format!("  connected — {:.1} MB", total as f64 / 1e6)),
            None => progress("  connected".to_string()),
        }

        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(dest).map_err(write_failed)?;
        let mut buffer = [0u8; 64 * 1024];
        let mut done: u64 = 0;
        let mut last_mark: u64 = 0;
        let mut interrupted = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    file.write_all(&buffer[..n]).map_err(write_failed)?;
                    done += n as u64;
                    if done - last_mark >= 4 * 1024 * 1024 {
                        last_mark = done;
                        match total {
                            Some(total) => progress(format!(
                                "  {:.1} / {:.1} MB",
                                done as f64 / 1e6,
                                total as f64 / 1e6,
                            )),
                            None => progress(format!("  {:.1} MB", done as f64 / 1e6)),
                        }
                    }
                }
                Err(error) => {
                    last_error = format!("{url}: interrupted mid-body: {error}");
                    progress(format!("  {last_error} — trying the next route"));
                    interrupted = true;
                    break;
                }
            }
        }
        let complete = !interrupted && done > 0 && total.is_none_or(|total| done == total);
        if complete {
            progress(format!("  done, {:.1} MB", done as f64 / 1e6));
            return Ok(());
        }
    }
    Err(Error::Download {
        detail: format!("every route failed; last error: {last_error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asset_is_offered_by_github_and_the_mirror() {
        let urls = espressif_urls("qemu", "rel", "a.tar.xz");
        assert_eq!(urls.len(), 2);
        assert!(urls[0].starts_with("https://github.com/espressif/qemu/"));
        assert!(urls[1].starts_with("https://dl.espressif.com/"));
        assert!(urls.iter().all(|url| url.ends_with("/a.tar.xz")));
    }

    #[test]
    fn install_steps_know_their_tools_and_refuse_strangers() {
        let espflash = install_steps("espflash").expect("espflash installs");
        assert_eq!(espflash.len(), 1);
        assert!(espflash[0].display.contains("cargo install espflash"));
        // The shared table serves the rest of the workbench's tools too.
        let espup = install_steps("espup").expect("espup installs");
        assert_eq!(espup.len(), 2, "install espup, then espup install");
        assert!(espup[1].display.contains("espup install"));
        let ra = install_steps("rust-analyzer").expect("rust-analyzer installs");
        assert!(ra[0].display.contains("rustup component add"));
        assert!(
            install_steps("rustup").is_err(),
            "the installer itself cannot be one-clicked, and says so",
        );

        assert!(
            install_steps("qemu-system-xtensa").is_err(),
            "qemu goes through its own download path",
        );
        let probers = install_steps("probe-rs").expect("probe-rs installs now");
        assert!(probers[0].display.contains("probe-rs-tools"));
        assert!(
            install_steps("some-imaginary-tool").is_err(),
            "unknown tools are named, not guessed",
        );
    }

    /// Planned against a directory the test names, so nothing here creates
    /// or reads the real data directory — a test that did left `tools/`
    /// behind on every developer machine it ran on.
    #[test]
    fn qemu_download_prefers_rustys_build_then_falls_back_to_espressif() {
        let Some(platform) = tools::host_platform() else {
            return; // a platform neither project publishes; qemu_download says so
        };
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        let plan = qemu_download_into("qemu-system-xtensa", &tools).expect("a plan for this host");
        assert!(
            !tools.exists(),
            "planning creates nothing; only a download about to land does",
        );
        assert_eq!(plan.archive, tools.join("qemu-xtensa.tar.xz"));

        // Espressif's two are always last and always in that order: GitHub is
        // unreachable from some networks entirely, and dl.espressif.com exists
        // precisely for that and serves identical bytes.
        let espressif: Vec<_> = plan
            .urls
            .iter()
            .filter(|u| u.contains("espressif/qemu"))
            .collect();
        assert_eq!(espressif.len(), 2, "{:?}", plan.urls);
        assert!(espressif[0].contains("github.com/"), "{}", espressif[0]);
        assert!(
            espressif[1].contains("dl.espressif.com/github_assets"),
            "{}",
            espressif[1],
        );
        assert!(
            espressif.iter().all(|u| u.contains("qemu-xtensa-softmmu")),
            "{espressif:?}",
        );

        // Ours goes first where it exists, because it is the one with pin
        // state. Asserting on the property rather than a count: an ARM Linux
        // host has no rusty build and must still get a working plan.
        match rusty_qemu_asset(platform) {
            Some(asset) => {
                assert!(plan.urls[0].starts_with(REPO), "{}", plan.urls[0]);
                assert!(plan.urls[0].ends_with(&asset), "{}", plan.urls[0]);
            }
            None => assert!(
                plan.urls.iter().all(|u| u.contains("espressif/qemu")),
                "a platform rusty does not build must fall through cleanly: {:?}",
                plan.urls,
            ),
        }
        assert!(plan.extract.display.starts_with("tar -xf"));
    }

    /// Every platform rusty runs on has an adapter to offer, and the names
    /// are the release's own. A name that is nearly right 404s exactly like
    /// a network problem, which is the whole reason this is a table.
    #[test]
    fn codelldb_publishes_a_build_for_every_platform_rusty_runs_on() {
        for (os, arch, expected) in [
            ("windows", "x86_64", "codelldb-win32-x64.vsix"),
            ("macos", "aarch64", "codelldb-darwin-arm64.vsix"),
            ("macos", "x86_64", "codelldb-darwin-x64.vsix"),
            ("linux", "x86_64", "codelldb-linux-x64.vsix"),
            ("linux", "aarch64", "codelldb-linux-arm64.vsix"),
        ] {
            assert_eq!(codelldb_asset(os, arch), Some(expected), "{os}/{arch}");
        }
        assert_eq!(
            codelldb_asset("linux", "powerpc64"),
            None,
            "a platform with no published build is refused rather than guessed at",
        );
    }

    /// The adapter unpacks into a directory of its own, and the plan says so
    /// before anything is downloaded.
    #[test]
    fn the_adapter_unpacks_into_its_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plan = codelldb_download_into(dir.path()).expect("this platform has an asset");
        assert!(
            plan.urls[0].ends_with(".vsix"),
            "the asset is a vsix: {}",
            plan.urls[0],
        );
        assert!(
            plan.urls[0].contains(CODELLDB_RELEASE),
            "pinned, not latest"
        );
        let into = plan.extract.args.last().expect("a destination");
        assert!(
            into.ends_with(CODELLDB_DIR),
            "unpacks into tools/{CODELLDB_DIR}, not over tools/: {into}",
        );
        assert!(
            !dir.path().join(CODELLDB_DIR).exists(),
            "planning creates nothing; only `codelldb_download` does",
        );
    }

    #[test]
    fn every_platform_rusty_builds_is_one_espressif_names() {
        // The two asset ladders must agree about how a platform is spelled,
        // or ours resolves and theirs 404s on the same machine — a fallback
        // that only works where it is not needed. These strings were read off
        // the release's own asset list.
        for platform in [
            "x86_64-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-w64-mingw32",
        ] {
            assert!(
                rusty_qemu_asset(platform).is_some(),
                "{platform} is published by rusty",
            );
        }
        // The two rusty does not build. Claiming either would 404 for every
        // user on it, and a 404 in this ladder is indistinguishable from a
        // network that is simply down.
        for absent in ["aarch64-linux-gnu", "x86_64-apple-darwin"] {
            assert!(
                rusty_qemu_asset(absent).is_none(),
                "{absent} is Espressif's to serve",
            );
        }
    }

    /// The installer must refuse a name it does not know rather than compose
    /// a plausible URL for it — and refuse before touching the disk.
    #[test]
    fn an_unknown_tool_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        assert!(matches!(
            gdb_download_into("gdb", &tools),
            Err(Error::Refused { .. })
        ));
        assert!(matches!(
            gcc_download_into("arm-none-eabi-gcc", &tools),
            Err(Error::Refused { .. })
        ));
        assert!(matches!(
            qemu_download_into("qemu", &tools),
            Err(Error::Refused { .. })
        ));
        assert!(!tools.exists(), "a refusal leaves nothing behind");
    }
}
