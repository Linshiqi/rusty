//! Switch a project's chip, printing the plan before carrying it out.
//!
//! The headless half of what the status bar's chip popover does — the same
//! `migrate::plan` and `migrate::apply`, so what this proves is what the
//! window does.
//!
//! ```text
//! cargo run -p rusty-embed --example switch_chip -- <project-dir> <chip> [--apply]
//! ```

use rusty_embed::{catalog::Catalog, migrate};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(chip)) = (args.next(), args.next()) else {
        eprintln!("usage: switch_chip <project-dir> <chip-id> [--apply]");
        std::process::exit(2);
    };
    let apply = args.any(|arg| arg == "--apply");
    let root = std::path::PathBuf::from(&dir);

    let catalog = Catalog::load(Some(&root));
    let project = match rusty_embed::project::detect(&root) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("could not read {dir}: {error}");
            std::process::exit(1);
        }
    };
    let Some(current) = project.chip.as_deref() else {
        eprintln!("no chip detected for {dir}");
        std::process::exit(1);
    };
    let find = |id: &str| catalog.chips().iter().find(|c| c.id == id).cloned();
    let (Some(from), Some(to)) = (find(current), find(&chip)) else {
        eprintln!("{current} or {chip} is not in the catalogue");
        std::process::exit(1);
    };

    let plan = migrate::plan(&root, &from, &to);
    println!("{} → {}", plan.from, plan.to);
    if let Some(blocker) = &plan.blocker {
        println!("refused: {blocker}");
        std::process::exit(1);
    }
    for file in &plan.files {
        println!("  {}", file.path);
        for edit in &file.edits {
            let show = |text: &str| {
                if text.is_empty() {
                    "(nothing)".to_string()
                } else {
                    text.replace('\n', "⏎")
                }
            };
            println!("    {} → {}", show(&edit.before), show(&edit.after));
        }
    }
    for note in &plan.notes {
        println!("  — {note}");
    }

    if apply {
        match migrate::apply(&root, &plan) {
            Ok(written) => println!("wrote {}", written.join(", ")),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
}
