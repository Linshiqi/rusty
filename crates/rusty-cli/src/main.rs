//! `rusty` — the workbench without the window.
//!
//! Everything the desktop app shows is computed here too. Keeping a real CLI
//! from day one is what makes CI and team integration a wiring job rather than
//! a rewrite — and `rusty check --json` is the thing to paste into a bug report
//! when someone's board will not build.

mod check;
mod disk;
mod hardware;
mod sim;
mod workspace;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "rusty",
    version,
    about = "Embedded Rust workbench: projects, toolchains, boards, and binary size"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Why this project will or will not build: chip, toolchain, and every
    /// mismatch between them.
    ///
    /// The first thing to run when something is wrong, and the thing to paste
    /// into a bug report. Exits non-zero if anything blocking was found, so it
    /// drops straight into CI.
    Check {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Parts and boards rusty knows about, including any the project adds.
    Catalog {
        /// Show boards instead of chips.
        #[arg(long)]
        boards: bool,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Serial ports and debug probes currently attached.
    Devices {
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Where a built firmware's bytes went, by section and by crate.
    Size {
        /// The linked ELF, e.g. target/riscv32imc-unknown-none-elf/release/blinky
        /// — or a project directory, whose newest firmware for its configured
        /// target is analysed, found where the firmware is built (the
        /// excluded firmware crate, in a host workspace) the way the desktop
        /// app finds it.
        elf: PathBuf,
        /// The project the chip is read from, when `elf` is a file.
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Where the project's builds went on disk, and what of it is stale:
    /// artifacts of dependency versions the lockfile no longer resolves, of
    /// packages no longer in the graph, and incremental caches that are idle
    /// or superseded by a crate's newer ones.
    Disk {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// An incremental cache untouched for this many days counts as idle.
        #[arg(long, default_value_t = 7)]
        idle_days: u32,
        /// Keep this many of each crate's newest incremental caches; older
        /// ones are superseded.
        #[arg(long, default_value_t = 4)]
        keep_variants: u32,
        #[arg(long)]
        json: bool,
    },

    /// Remove the stale artifacts `disk` lists. Prints what would go and
    /// stops there unless `--apply` is given; nothing a build holds the lock
    /// on is touched either way.
    Sweep {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 7)]
        idle_days: u32,
        /// Keep this many of each crate's newest incremental caches; older
        /// ones are superseded.
        #[arg(long, default_value_t = 4)]
        keep_variants: u32,
        /// Actually remove. Without it this is a dry run.
        #[arg(long)]
        apply: bool,
    },

    /// A schematic symbol for an LCSC part number, imported from EasyEDA and
    /// kept in the data directory's symbol library for the board editor.
    ///
    /// The proof that a machine can reach the service, and what the symbol
    /// came out as: every pin with its position, and every record the
    /// reader had to skip.
    Symbol {
        /// The part's number on lcsc.com, e.g. C2286.
        number: String,
        #[arg(long)]
        json: bool,
    },

    /// Serve rusty's analyses to another assistant — Claude Code, Cursor —
    /// over the Model Context Protocol on stdin and stdout: the tools the
    /// built-in assistant calls, answering about the project at `path`.
    ///
    /// Registered with the client rather than run by hand, e.g.
    /// `claude mcp add rusty -- rusty-cli mcp /path/to/project`. Every tool
    /// reads; none writes.
    Mcp {
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Run the firmware in rusty's emulator without the window: build, image,
    /// boot, and watch the serial line and the pins.
    ///
    /// What the firmware prints goes to stdout as it arrives; the build and
    /// rusty's own notes go to stderr. The run passes once every `--expect`
    /// text has appeared and every step of `--scenario` is done, and fails on
    /// the first `--fail` text, a step whose check does not hold, or a
    /// timeout with something still expected. With nothing expected it runs
    /// for the whole timeout. Exit code 0 passed, 1 failed or timed out, 2
    /// could not run at all (no chip, a missing tool, a failed build).
    Sim {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Seconds the firmware may run, from the emulator starting.
        #[arg(long)]
        timeout: Option<f64>,
        /// Text the firmware must print. Repeatable: all of them.
        #[arg(long)]
        expect: Vec<String>,
        /// Text that fails the run the moment it appears. Repeatable.
        #[arg(long)]
        fail: Vec<String>,
        /// A TOML file of steps, taken in order while the firmware runs:
        /// wait-serial, write-serial, press, release, delay, expect-pin, set.
        #[arg(long)]
        scenario: Option<PathBuf>,
        /// Write every pin transition to this file as a Value Change Dump.
        #[arg(long)]
        vcd: Option<PathBuf>,
        /// Print nothing but the verdict.
        #[arg(long, short)]
        quiet: bool,
    },

    /// Cargo dependency health: duplicates, direct vs transitive, build scripts.
    Deps {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// What a feature selection costs, relative to the package's defaults.
    Features {
        package: String,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long, value_delimiter = ',')]
        features: Vec<String>,
        #[arg(long)]
        no_default_features: bool,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Check { path, json } => check::check(&path, json),
        Command::Catalog { boards, path, json } => hardware::catalog(&path, boards, json),
        Command::Devices { path, json } => hardware::devices(&path, json),
        Command::Size { elf, path, json } => check::size(elf, path, json),
        Command::Disk {
            path,
            idle_days,
            keep_variants,
            json,
        } => disk::disk(&path, idle_days, keep_variants, json),
        Command::Sweep {
            path,
            idle_days,
            keep_variants,
            apply,
        } => disk::sweep(&path, idle_days, keep_variants, apply),
        Command::Mcp { path } => mcp(&path),
        Command::Sim {
            path,
            timeout,
            expect,
            fail,
            scenario,
            vcd,
            quiet,
        } => sim::sim(&path, timeout, expect, fail, scenario, vcd, quiet),
        Command::Deps { path, json } => workspace::deps(&path, json),
        Command::Symbol { number, json } => hardware::symbol(&number, json),
        Command::Features {
            package,
            path,
            features,
            no_default_features,
            json,
        } => workspace::features(package, &path, features, no_default_features, json),
    }
}

/// `rusty mcp`: rusty's analyses served over the Model Context Protocol on
/// stdin and stdout.
fn mcp(path: &Path) -> Result<()> {
    // Absolute, not canonical: a canonical path on Windows is a
    // verbatim `\\?\` one, which a tool that appends to it cannot use.
    let root =
        std::path::absolute(path).with_context(|| format!("resolving {}", path.display()))?;
    if !root.is_dir() {
        anyhow::bail!("{} is not a directory", root.display());
    }
    rusty_ai::mcp::serve(root, std::io::stdin().lock(), std::io::stdout().lock())?;
    Ok(())
}

fn emit<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn human(bytes: u64) -> String {
    const KB: u64 = 1024;
    match bytes {
        b if b >= KB * KB * KB => format!("{:.1} GB", b as f64 / (KB * KB * KB) as f64),
        b if b >= KB * KB => format!("{:.1} MB", b as f64 / (KB * KB) as f64),
        b if b >= KB => format!("{:.1} KB", b as f64 / KB as f64),
        b => format!("{b} B"),
    }
}
