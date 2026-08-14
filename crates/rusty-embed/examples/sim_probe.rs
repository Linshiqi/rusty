//! Prove the simulation pipeline end to end, on a real project.
//!
//! ```text
//! cargo run -p rusty-embed --example sim_probe -- <project-dir> [seconds]
//! ```
//!
//! Detects the project, prints the plan, runs build → image → QEMU through
//! the same [`rusty_embed::process`] spawn the app uses, and lets the boot
//! run for `seconds` (default 8) before stopping it. Exit code 0 only if
//! every step behaved and the firmware printed something.

use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("usage: sim_probe <project> [secs]"));
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);

    let project = rusty_embed::project::detect(&root).expect("detect");
    let plan = rusty_embed::simulate::plan(&project, false);
    println!("supported: {}", plan.supported);
    if let Some(reason) = &plan.reason {
        println!("reason: {reason}");
    }
    for tool in &plan.missing {
        println!("missing: {} — {}", tool.name, tool.install);
    }
    if !plan.supported || !plan.missing.is_empty() {
        std::process::exit(2);
    }

    rusty_embed::simulate::prepare(&root).expect("prepare image dir");

    let total = plan.steps.len();
    let mut boot_lines = 0usize;
    for (index, step) in plan.steps.into_iter().enumerate() {
        println!("\n$ {}", step.display);
        let session = rusty_embed::process::spawn(&step, Some(&root)).expect("spawn");
        let is_boot = index + 1 == total;

        if is_boot {
            let stopper = session.stopper();
            let deadline = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < deadline {
                match session.recv() {
                    Some(line) => {
                        println!("{}", line.text);
                        boot_lines += 1;
                    }
                    None => break,
                }
            }
            stopper.stop();
            println!("— stopped after {seconds}s, {boot_lines} lines");
        } else {
            while let Some(line) = session.recv() {
                println!("{}", line.text);
            }
            let code = session.wait();
            if code != Some(0) {
                eprintln!("step {} failed with {code:?}", index + 1);
                std::process::exit(1);
            }
        }
    }

    if boot_lines == 0 {
        eprintln!("QEMU booted but the firmware never printed — the loop is not proven");
        std::process::exit(1);
    }
    println!("\nsimulation pipeline proven: {boot_lines} serial lines");
}
