//! `sim`: the firmware run without the window.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusty_embed::project;
use rusty_embed::simulate::headless::{self, Scenario};

/// `rusty sim`: the firmware built, booted in rusty's emulator and
/// watched without the window. Exits 0 when it passed, 1 when it failed or
/// timed out, 2 when it could not run.
pub(crate) fn sim(
    path: &Path,
    timeout: Option<f64>,
    expect: Vec<String>,
    fail: Vec<String>,
    scenario: Option<PathBuf>,
    vcd: Option<PathBuf>,
    quiet: bool,
) -> Result<()> {
    let mut plan = match &scenario {
        Some(file) => {
            let text = std::fs::read_to_string(file)
                .with_context(|| format!("reading {}", file.display()))?;
            Scenario::from_toml(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?
        }
        None => Scenario::default(),
    };
    if timeout.is_some() {
        plan.timeout = timeout;
    }
    plan.expect.extend(expect);
    plan.fail.extend(fail);

    let root =
        std::path::absolute(path).with_context(|| format!("resolving {}", path.display()))?;
    let root = project::firmware_root(&root);
    let outcome = headless::run(&root, &plan, &mut |event| {
        if quiet {
            return;
        }
        match event {
            headless::Event::Command(line) => eprintln!("$ {line}"),
            headless::Event::Output(line) => eprintln!("{line}"),
            headless::Event::Serial(line) => println!("{line}"),
            headless::Event::Note(line) => eprintln!("rusty: {line}"),
        }
    });

    if let Some(file) = &vcd {
        std::fs::write(file, rusty_embed::to_vcd(&outcome.events))
            .with_context(|| format!("writing {}", file.display()))?;
        if !quiet {
            eprintln!(
                "rusty: {} pin changes written to {}",
                outcome.events.len(),
                file.display()
            );
        }
    }
    if !quiet {
        for (pin, level) in outcome.levels() {
            let changes = outcome.events.iter().filter(|(_, p, _)| *p == pin).count();
            eprintln!(
                "rusty: GPIO{pin} ended {}, {changes} report(s)",
                u8::from(level)
            );
        }
    }
    let code = match &outcome.verdict {
        headless::Verdict::Passed => {
            eprintln!("passed");
            0
        }
        headless::Verdict::Failed(why) => {
            eprintln!("failed: {why}");
            1
        }
        headless::Verdict::TimedOut(why) => {
            eprintln!("timed out: {why}");
            1
        }
        headless::Verdict::Unrunnable(why) => {
            eprintln!("could not run: {why}");
            2
        }
    };
    std::process::exit(code);
}
