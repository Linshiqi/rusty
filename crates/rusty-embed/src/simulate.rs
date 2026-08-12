//! Wokwi-style simulation, on Espressif's QEMU.
//!
//! Local-first deliberately: Espressif ships QEMU builds with ESP machine
//! models (`-M esp32c3` and friends), which boot the very image `espflash`
//! would put on a real board — ROM, second-stage bootloader, partition
//! table, app — and speak UART on stdio. No account, no cloud, no token.
//!
//! The loop is three commands, each inspectable in the panel before it runs:
//!
//! 1. `cargo build --release` — the project's own toolchain does the work.
//! 2. `espflash save-image --merge` — a bootable 4MB flash image, the same
//!    bytes a device would hold.
//! 3. `qemu-system-<arch> -M <chip> -nographic` — serial streams back into
//!    the dock until stopped.
//!
//! Refusals name what is missing and how to get it. A chip QEMU has no
//! machine model for is refused with the list of ones it has — a plausible
//! "it might work" would cost someone an afternoon.

use std::path::{Path, PathBuf};

use crate::config;
use crate::model::{CommandPlan, EmbeddedProject, SimBoard, SimLed, SimPlan, SimTool};

/// Chips Espressif's QEMU actually models, with the system emulator each
/// needs. Kept small and honest — c6/h2/p4 have no machine model yet.
const MACHINES: &[(&str, &str)] = &[
    ("esp32c3", "qemu-system-riscv32"),
    ("esp32", "qemu-system-xtensa"),
    ("esp32s3", "qemu-system-xtensa"),
];

/// Everything needed to simulate `project`, or exactly why not.
pub fn plan(project: &EmbeddedProject) -> SimPlan {
    let Some(chip) = project.chip.as_deref() else {
        return SimPlan {
            supported: false,
            reason: Some(
                "no chip could be detected for this project, and a simulator needs to know \
                 which machine to model — set the target in .cargo/config.toml"
                    .to_string(),
            ),
            missing: Vec::new(),
            steps: Vec::new(),
            board: None,
        };
    };

    let Some((_, emulator)) = MACHINES.iter().find(|(name, _)| *name == chip) else {
        let known: Vec<&str> = MACHINES.iter().map(|(name, _)| *name).collect();
        return SimPlan {
            supported: false,
            reason: Some(format!(
                "QEMU has no machine model for {chip}; it can model {}",
                known.join(", "),
            )),
            missing: Vec::new(),
            steps: Vec::new(),
            board: None,
        };
    };

    let mut missing = Vec::new();
    let espflash = match find_espflash() {
        Some(path) => path,
        None => {
            missing.push(SimTool {
                name: "espflash".to_string(),
                install: "cargo install espflash".to_string(),
            });
            PathBuf::from("espflash")
        }
    };
    let qemu = match find_qemu(emulator) {
        Some(path) => path,
        None => {
            missing.push(SimTool {
                name: emulator.to_string(),
                install: format!(
                    "download the {emulator} build from \
                     https://github.com/espressif/qemu/releases and unpack it into the data \
                     directory's tools/qemu/"
                ),
            });
            PathBuf::from(emulator)
        }
    };

    let Some(target) = project.configured_target.as_deref() else {
        return SimPlan {
            supported: false,
            reason: Some(
                "no build target in .cargo/config.toml — the simulator cannot guess where \
                 the ELF will land"
                    .to_string(),
            ),
            missing,
            steps: Vec::new(),
            board: None,
        };
    };
    let binary = package_name(Path::new(&project.root)).unwrap_or_else(|| "app".to_string());
    let elf = format!("target/{target}/release/{binary}");
    let image = "target/rusty-sim/flash.bin".to_string();

    let build = CommandPlan {
        program: "cargo".to_string(),
        args: vec!["build".to_string(), "--release".to_string()],
        display: "cargo build --release".to_string(),
        rationale: "the project's own toolchain builds the exact firmware a device would get"
            .to_string(),
    };
    let mut image_args = vec![
        "save-image".to_string(),
        "--chip".to_string(),
        chip.to_string(),
        "--merge".to_string(),
        elf.clone(),
        image.clone(),
    ];
    let image_step = CommandPlan {
        display: format!("espflash {}", image_args.join(" ")),
        program: espflash.to_string_lossy().into_owned(),
        args: std::mem::take(&mut image_args),
        rationale: "merges bootloader, partition table and app into the bootable flash image \
                    QEMU maps as the SPI flash"
            .to_string(),
    };
    let qemu_args = vec![
        "-M".to_string(),
        chip.to_string(),
        "-nographic".to_string(),
        "-drive".to_string(),
        format!("file={image},if=mtd,format=raw"),
        "-serial".to_string(),
        "mon:stdio".to_string(),
    ];
    let run = CommandPlan {
        display: format!("{emulator} {}", qemu_args.join(" ")),
        program: qemu.to_string_lossy().into_owned(),
        args: qemu_args,
        rationale: "boots the image in Espressif's QEMU; the serial console streams here \
                    until stopped"
            .to_string(),
    };

    SimPlan {
        supported: true,
        reason: None,
        missing,
        steps: vec![build, image_step, run],
        board: board_view(Path::new(&project.root)),
    }
}

/// The board `.rusty/sim.toml` describes, if the project carries one.
///
/// The file format is its own struct, converted to the wire model — the
/// same file/wire split every other user-authored TOML here gets, so a
/// panel refactor cannot silently break the files people wrote.
fn board_view(root: &Path) -> Option<SimBoard> {
    #[derive(serde::Deserialize)]
    struct File {
        board: Option<FileBoard>,
        #[serde(default)]
        led: Vec<FileLed>,
    }
    #[derive(serde::Deserialize)]
    struct FileBoard {
        chip: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct FileLed {
        pin: u8,
        color: Option<String>,
        label: Option<String>,
        x: Option<f64>,
        y: Option<f64>,
    }

    let text = std::fs::read_to_string(root.join(".rusty/sim.toml")).ok()?;
    let parsed: File = toml::from_str(&text).ok()?;
    if parsed.led.is_empty() {
        return None;
    }
    Some(SimBoard {
        chip: parsed
            .board
            .and_then(|b| b.chip)
            .unwrap_or_else(|| "esp32".to_string()),
        leds: parsed
            .led
            .into_iter()
            .map(|led| SimLed {
                label: led.label.unwrap_or_else(|| format!("GPIO{}", led.pin)),
                color: led.color.unwrap_or_else(|| "green".to_string()),
                pin: led.pin,
                x: led.x,
                y: led.y,
            })
            .collect(),
    })
}



/// The QEMU release every install pulls — the version this pipeline is
/// proven against. Bumped deliberately, not discovered at run time: an
/// installer that fetches "latest" breaks the day upstream changes layout.
const QEMU_RELEASE: &str = "esp-develop-9.2.2-20260417";
const QEMU_VERSION: &str = "esp_develop_9.2.2_20260417";

/// How to install a tool the plan reported missing, as inspectable steps —
/// one click in the panel, the dock shows every line, and only a failure
/// sends anyone to the manual instructions.
pub fn install_steps(tool: &str) -> std::result::Result<Vec<CommandPlan>, String> {
    if tool == "espflash" {
        return Ok(vec![CommandPlan {
            program: "cargo".to_string(),
            args: vec![
                "install".to_string(),
                "espflash".to_string(),
                "--locked".to_string(),
            ],
            display: "cargo install espflash --locked".to_string(),
            rationale: "builds espflash into ~/.cargo/bin, where the simulator looks"
                .to_string(),
        }]);
    }

    if tool.starts_with("qemu-system-") {
        return Err(format!(
            "qemu installs through its own download path — this is a bug if it surfaces; \
             manual fallback: https://github.com/espressif/qemu/releases/tag/{QEMU_RELEASE} \
             into the data directory's tools/qemu/"
        ));
    }

    Err(format!("no installer for {tool}"))
}

/// The QEMU archive for one architecture: where to put it, the URLs to try
/// in order, and the extraction step.
///
/// Two hosts, because the first is unreachable from some networks entirely:
/// GitHub itself, then Espressif's own asset mirror (`dl.espressif.com`),
/// which exists precisely for this situation and serves identical bytes.
pub struct QemuDownload {
    pub archive: std::path::PathBuf,
    pub urls: Vec<String>,
    pub extract: CommandPlan,
}

pub fn qemu_download(tool: &str) -> std::result::Result<QemuDownload, String> {
    let Some(arch) = tool.strip_prefix("qemu-system-") else {
        return Err(format!("{tool} is not a qemu emulator name"));
    };
    if !cfg!(windows) {
        return Err(format!(
            "one-click install only knows the Windows build so far — download the {tool} \
             build from https://github.com/espressif/qemu/releases/tag/{QEMU_RELEASE} and \
             unpack it into the data directory's tools/qemu/"
        ));
    }
    let Some(tools) = config::data_dir().map(|d| d.join("tools")) else {
        return Err("the data directory could not be resolved".to_string());
    };
    std::fs::create_dir_all(&tools)
        .map_err(|e| format!("could not create {}: {e}", tools.display()))?;

    let asset = format!("qemu-{arch}-softmmu-{QEMU_VERSION}-x86_64-w64-mingw32.tar.xz");
    let archive = tools.join(format!("qemu-{arch}.tar.xz"));
    let urls = vec![
        format!("https://github.com/espressif/qemu/releases/download/{QEMU_RELEASE}/{asset}"),
        format!(
            "https://dl.espressif.com/github_assets/espressif/qemu/releases/download/{QEMU_RELEASE}/{asset}"
        ),
    ];
    let archive_text = archive.to_string_lossy().into_owned();
    let tools_text = tools.to_string_lossy().into_owned();
    let extract = CommandPlan {
        program: "tar".to_string(),
        args: vec![
            "-xf".to_string(),
            archive_text.clone(),
            "-C".to_string(),
            tools_text.clone(),
        ],
        display: format!("tar -xf {archive_text} -C {tools_text}"),
        rationale: "unpacks into the data directory's tools/qemu — bsdtar handles .tar.xz \
                    and ships with Windows"
            .to_string(),
    };
    Ok(QemuDownload {
        archive,
        urls,
        extract,
    })
}

/// Fetch `urls` in order until one delivers, streaming progress through
/// `progress`. In-process on rustls: the OS TLS stack (schannel) aborts on
/// some CDNs with "server closed abruptly", and a spawned curl cannot be
/// relied on to exist unbroken everywhere.
pub fn download(
    urls: &[String],
    dest: &Path,
    mut progress: impl FnMut(String),
) -> std::result::Result<(), String> {
    use std::io::{Read, Write};

    // One proxy URL is not one route. A local proxy's HTTP CONNECT can be
    // incompatible with this client while its SOCKS listener on the same
    // port works (mixed-port proxies answer both), and a mirror may be
    // reachable with no proxy at all. Every route is tried, and named.
    let routes = proxy_candidates(effective_proxy());

    let agent_for = |route: &Option<String>| -> ureq::Agent {
        let mut builder = ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(15)))
            // Headers must arrive promptly or this route is declared dead
            // and the next one gets its turn — a blackholed route must not
            // hang the panel at "downloading" forever.
            .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
            // And nothing runs unbounded: a stalled body eventually dies.
            .timeout_global(Some(std::time::Duration::from_secs(15 * 60)));
        if let Some(url) = route
            && let Ok(proxy) = ureq::Proxy::new(url)
        {
            builder = builder.proxy(Some(proxy));
        }
        builder.build().into()
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
        let agent = agent_for(&route);
        let response = match agent.get(&url).call() {
            Ok(response) => response,
            Err(error) => {
                let chain = error_chain(&error);
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
        let mut file = match std::fs::File::create(dest) {
            Ok(file) => file,
            Err(error) => return Err(format!("could not create {}: {error}", dest.display())),
        };
        let mut buffer = [0u8; 64 * 1024];
        let mut done: u64 = 0;
        let mut last_mark: u64 = 0;
        let mut interrupted = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(error) = file.write_all(&buffer[..n]) {
                        return Err(format!("could not write {}: {error}", dest.display()));
                    }
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
        let complete = !interrupted
            && done > 0
            && total.is_none_or(|total| done == total);
        if complete {
            progress(format!("  done, {:.1} MB", done as f64 / 1e6));
            return Ok(());
        }
    }
    Err(format!("every route failed; last error: {last_error}"))
}

/// The transport ladder for one configured proxy: as given, then the SOCKS5
/// spelling of the same address (mixed-port proxies answer both), then no
/// proxy at all. Deduplicated, order kept.
fn proxy_candidates(configured: Option<String>) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = Vec::new();
    if let Some(url) = configured {
        out.push(Some(url.clone()));
        if let Some(rest) = url.strip_prefix("http://") {
            let socks = format!("socks5://{rest}");
            if !out.contains(&Some(socks.clone())) {
                out.push(Some(socks));
            }
        }
    }
    out.push(None);
    out
}

/// Every layer of an error, because "unexpected end of file" alone points
/// nowhere — whether the proxy or the TLS or the socket said it is the
/// entire diagnosis.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(inner) = source {
        parts.push(inner.to_string());
        source = inner.source();
    }
    parts.dedup();
    parts.join(" ← ")
}

/// Create the directory the image step writes into. espflash does not make
/// parent directories, and "os error 3" from a missing folder reads like a
/// broken tool rather than a missing mkdir.
pub fn prepare(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join("target/rusty-sim"))
}

/// espflash: PATH first, then the copy the workbench keeps in its data
/// directory, then cargo's bin dir.
fn find_espflash() -> Option<PathBuf> {
    if let Some(path) = on_path("espflash") {
        return Some(path);
    }
    if let Some(tools) = config::data_dir().map(|d| d.join("tools/espflash").join(exe("espflash")))
        && tools.is_file()
    {
        return Some(tools);
    }
    let cargo = home_dir()?.join(".cargo/bin").join(exe("espflash"));
    cargo.is_file().then_some(cargo)
}

/// QEMU: PATH first, then the data directory's tools/qemu/bin.
fn find_qemu(emulator: &str) -> Option<PathBuf> {
    if let Some(path) = on_path(emulator) {
        return Some(path);
    }
    let tools = config::data_dir()?.join("tools/qemu/bin").join(exe(emulator));
    tools.is_file().then_some(tools)
}

/// The proxy the rest of this machine uses, if any.
///
/// Environment variables first (the cross-platform convention), then the
/// Windows system proxy from the registry — which is what the browser and
/// every GUI proxy tool (Clash and friends) configure. A tool that ignores
/// it downloads into a wall on exactly the machines that need a proxy.
/// What downloads and index queries should actually use: the stored setting
/// first (an explicit URL, or "none" for forced direct), then detection.
pub fn effective_proxy() -> Option<String> {
    if let Some(configured) = crate::config::workbench().proxy {
        let configured = configured.trim().to_string();
        if configured.eq_ignore_ascii_case("none") || configured.is_empty() {
            return None;
        }
        if !configured.eq_ignore_ascii_case("auto") {
            return Some(configured);
        }
    }
    system_proxy()
}

pub fn system_proxy() -> Option<String> {
    for key in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY"] {
        if let Ok(value) = std::env::var(key)
            && !value.trim().is_empty()
        {
            return Some(value.trim().to_string());
        }
    }
    if cfg!(windows) {
        return windows_system_proxy();
    }
    None
}

fn windows_system_proxy() -> Option<String> {
    let query = |value: &str| -> Option<String> {
        let mut command = std::process::Command::new("reg");
        command.args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            value,
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let out = command.output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines()
            .find(|line| line.trim_start().starts_with(value))
            .and_then(|line| line.split_whitespace().last())
            .map(str::to_string)
    };

    let enabled = query("ProxyEnable")?;
    if !enabled.ends_with('1') {
        return None;
    }
    parse_proxy_server(&query("ProxyServer")?)
}

/// The registry's `ProxyServer` shapes: a bare `host:port` for everything,
/// or `http=h:p;https=h:p;ftp=…` per protocol. Https wins, then http; socks
/// entries are skipped — this client does not speak socks.
fn parse_proxy_server(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if !value.contains('=') {
        return Some(format!("http://{value}"));
    }
    let mut http = None;
    for part in value.split(';') {
        let Some((scheme, address)) = part.split_once('=') else {
            continue;
        };
        match scheme.trim() {
            "https" => return Some(format!("http://{}", address.trim())),
            "http" => http = Some(format!("http://{}", address.trim())),
            _ => {}
        }
    }
    http
}

/// Write the board back to `.rusty/sim.toml`, the file the editor edits.
///
/// Serialised through the file structs, not the wire ones — the file format
/// is a contract with people who write it by hand, and it stays stable when
/// the wire model grows.
pub fn save_board(root: &Path, board: &SimBoard) -> std::result::Result<(), String> {
    #[derive(serde::Serialize)]
    struct File<'a> {
        board: FileBoard<'a>,
        led: Vec<FileLed<'a>>,
    }
    #[derive(serde::Serialize)]
    struct FileBoard<'a> {
        chip: &'a str,
    }
    #[derive(serde::Serialize)]
    struct FileLed<'a> {
        pin: u8,
        color: &'a str,
        label: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
    }

    let file = File {
        board: FileBoard { chip: &board.chip },
        led: board
            .leds
            .iter()
            .map(|led| FileLed {
                pin: led.pin,
                color: &led.color,
                label: &led.label,
                x: led.x.map(|v| v.round()),
                y: led.y.map(|v| v.round()),
            })
            .collect(),
    };
    let text = toml::to_string_pretty(&file).map_err(|e| format!("could not encode: {e}"))?;
    let dir = root.join(".rusty");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create .rusty: {e}"))?;
    std::fs::write(dir.join("sim.toml"), text).map_err(|e| format!("could not write: {e}"))
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(exe(name));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The `[package] name` of the project's manifest — where cargo will put the
/// ELF. A scan, like the edition scan in rusty-edit, for the same reason.
fn package_name(root: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package
            && let Some(rest) = line.strip_prefix("name")
        {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(chip: Option<&str>, target: Option<&str>) -> EmbeddedProject {
        EmbeddedProject {
            root: ".".to_string(),
            chip: chip.map(str::to_string),
            chip_source: None,
            runtime: None,
            configured_target: target.map(str::to_string),
            configured_toolchain: None,
            frameworks: Vec::new(),
            uses_defmt: false,
            uses_embassy: false,
            evidence: Vec::new(),
            problems: Vec::new(),
        }
    }


    #[test]
    fn the_board_round_trips_through_save_and_load() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim-rt")
            .tempdir()
            .expect("tempdir");
        let board = SimBoard {
            chip: "esp32".to_string(),
            leds: vec![SimLed {
                pin: 26,
                color: "green".to_string(),
                label: "G".to_string(),
                x: Some(40.0),
                y: Some(60.0),
            }],
        };
        save_board(dir.path(), &board).expect("save");
        let loaded = board_view(dir.path()).expect("load");
        assert_eq!(loaded, board);
    }

    #[test]
    fn the_board_file_converts_to_the_wire_model() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim-board")
            .tempdir()
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".rusty")).expect("dirs");
        std::fs::write(
            dir.path().join(".rusty/sim.toml"),
            "[board]\nchip = \"esp32\"\n[[led]]\npin = 26\ncolor = \"green\"\n[[led]]\npin = 27\ncolor = \"blue\"\nlabel = \"BLUE\"\n",
        )
        .expect("write");
        let board = board_view(dir.path()).expect("board");
        assert_eq!(board.chip, "esp32");
        assert_eq!(board.leds.len(), 2);
        assert_eq!(board.leds[0].label, "GPIO26");
        assert_eq!(board.leds[1].label, "BLUE");
        assert!(board_view(Path::new("nowhere-at-all")).is_none());
    }

    #[test]
    fn install_steps_know_their_tools_and_refuse_strangers() {
        let espflash = install_steps("espflash").expect("espflash installs");
        assert_eq!(espflash.len(), 1);
        assert!(espflash[0].display.contains("cargo install espflash"));

        assert!(
            install_steps("qemu-system-xtensa").is_err(),
            "qemu goes through its own download path",
        );
        assert!(install_steps("probe-rs").is_err(), "unknown tools are named, not guessed");
    }

    #[test]
    fn the_transport_ladder_tries_http_then_socks_then_direct() {
        let routes = proxy_candidates(Some("http://127.0.0.1:7890".to_string()));
        assert_eq!(
            routes,
            vec![
                Some("http://127.0.0.1:7890".to_string()),
                Some("socks5://127.0.0.1:7890".to_string()),
                None,
            ],
        );
        assert_eq!(proxy_candidates(None), vec![None]);
        // An explicit socks proxy is not doubled.
        let socks = proxy_candidates(Some("socks5://1.2.3.4:1080".to_string()));
        assert_eq!(
            socks,
            vec![Some("socks5://1.2.3.4:1080".to_string()), None],
        );
    }

    #[test]
    fn proxy_server_shapes_parse_like_the_browser_reads_them() {
        assert_eq!(
            parse_proxy_server("127.0.0.1:7890").as_deref(),
            Some("http://127.0.0.1:7890"),
        );
        assert_eq!(
            parse_proxy_server("http=a:1;https=b:2;ftp=c:3").as_deref(),
            Some("http://b:2"),
            "https wins",
        );
        assert_eq!(
            parse_proxy_server("http=a:1;socks=s:9").as_deref(),
            Some("http://a:1"),
        );
        assert_eq!(parse_proxy_server("socks=s:9"), None, "socks is not spoken");
        assert_eq!(parse_proxy_server(""), None);
    }

    #[test]
    fn qemu_download_tries_github_then_espressifs_mirror() {
        if !cfg!(windows) {
            return;
        }
        let plan = qemu_download("qemu-system-xtensa").expect("windows plan");
        assert_eq!(plan.urls.len(), 2);
        assert!(plan.urls[0].contains("github.com/espressif/qemu"), "{}", plan.urls[0]);
        assert!(
            plan.urls[1].contains("dl.espressif.com/github_assets"),
            "{}",
            plan.urls[1],
        );
        assert!(plan.urls.iter().all(|u| u.contains("qemu-xtensa-softmmu")));
        assert!(plan.extract.display.starts_with("tar -xf"));
    }

    #[test]
    fn an_unmodelled_chip_is_refused_with_the_supported_list() {
        let plan = plan(&project(Some("esp32c6"), Some("riscv32imac-unknown-none-elf")));
        assert!(!plan.supported);
        let reason = plan.reason.expect("names the problem");
        assert!(reason.contains("esp32c6"), "{reason}");
        assert!(reason.contains("esp32c3"), "the alternatives are listed: {reason}");
    }

    #[test]
    fn a_supported_chip_plans_three_inspectable_steps() {
        let sim = plan(&project(Some("esp32c3"), Some("riscv32imc-unknown-none-elf")));
        assert!(sim.supported, "{:?}", sim.reason);
        assert_eq!(sim.steps.len(), 3);
        assert_eq!(sim.steps[0].display, "cargo build --release");
        assert!(sim.steps[1].display.contains("save-image"), "{}", sim.steps[1].display);
        assert!(sim.steps[1].display.contains("--merge"));
        assert!(sim.steps[2].display.contains("-M esp32c3"), "{}", sim.steps[2].display);
        assert!(sim.steps[2].display.contains("if=mtd"));
    }

    #[test]
    fn no_chip_and_no_target_refuse_rather_than_guess() {
        assert!(!plan(&project(None, None)).supported);
        let sim = plan(&project(Some("esp32c3"), None));
        assert!(!sim.supported);
        assert!(sim.reason.expect("says why").contains(".cargo/config.toml"));
    }
}
