//! Which files rusty will dim in a project, and every file it considered.
//!
//! The tree dims a `.rs` file that no `mod` declaration reaches, because
//! rust-analyzer offers nothing at all in such a file. `rusty_edit::modules`
//! decides that from the declarations, and it is arranged to claim nothing
//! wherever it cannot be sure — so the two questions worth asking about a
//! report of a *wrong* dim are "did the scan see the declaring file at all?"
//! and "what did it make of it". This answers both, without the window.
//!
//! ```text
//! cargo run -p rusty-edit --features backend --example unlinked_probe -- <project>
//! ```
fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let root = std::path::Path::new(&root);

    let mut seen = 0usize;
    for found in ignore::WalkBuilder::new(root).build().flatten() {
        let Ok(relative) = found.path().strip_prefix(root) else {
            continue;
        };
        let relative = relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if relative.ends_with(".rs") || relative.ends_with("Cargo.toml") {
            println!("  read {relative}");
            seen += 1;
        }
    }

    let unlinked = rusty_edit::scan_unlinked(root);
    println!("\n{seen} files read, {} dimmed", unlinked.len());
    for path in &unlinked {
        println!("  {path}");
    }
    // Nothing dimmed is the ordinary answer, and it is also what a refusal
    // looks like — a `#[path]` anywhere takes the whole claim away. Say so,
    // rather than letting silence read as "every file is fine".
    if unlinked.is_empty() {
        println!("  (nothing — every file is declared, or the scan refused)");
    }
}
