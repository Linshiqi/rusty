//! The pin map, headless.
//!
//! ```text
//! cargo run -p rusty-embed --example pin_map -- <project-dir>
//! ```

use rusty_embed::pins;

fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: pin_map <project-dir>");
        std::process::exit(2);
    };
    let root = std::path::PathBuf::from(&dir);
    let project = match rusty_embed::project::detect(&root) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("could not read {dir}: {error}");
            std::process::exit(1);
        }
    };
    let Some(chip) = project.chip.as_deref() else {
        eprintln!("no chip detected");
        std::process::exit(1);
    };

    // The same split the window makes: scan where the chip is, report paths
    // the editor could open from the root.
    let firmware = rusty_embed::project::firmware_root(&root);
    let report = pins::report(&root, &firmware, chip);
    println!("{} — {} pins", report.chip, report.pins.len());
    if let Some(source) = &report.source {
        println!("capabilities: {source}");
    }
    if let Some(note) = &report.note {
        println!("note: {note}");
    }
    for pin in &report.pins {
        let mut marks = Vec::new();
        if pin.input_only {
            marks.push("input-only".to_string());
        }
        if let Some(reserved) = &pin.reserved {
            marks.push(reserved.clone());
        }
        if !pin.analog.is_empty() {
            marks.push(pin.analog.join("/"));
        }
        let used = pin
            .claims
            .iter()
            .map(|claim| format!("{}:{}", claim.file, claim.line + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let flag = if pin.claims.is_empty() { " " } else { "*" };
        println!("{flag} GPIO{:<3} {:<34} {used}", pin.gpio, marks.join(", "),);
    }
    for claim in &report.unknown {
        println!(
            "! GPIO{} does not exist on {} — {}:{}",
            claim.gpio,
            report.chip,
            claim.file,
            claim.line + 1,
        );
    }
}
