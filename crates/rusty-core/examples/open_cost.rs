//! How long the Cargo analysis takes for a project — the two steps the
//! Crates panel waits on, and the reason neither is on the path of opening
//! a project: a millisecond there is time the window spends showing the
//! last one. Run it against a real project and against a freshly generated
//! one: the second is the case the user reported, and the one where nothing
//! cargo needs is warm yet.
//!
//! ```text
//! cargo run -p rusty-core --example open_cost -- <project>
//! ```
fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let root = std::path::Path::new(&root);

    let at = std::time::Instant::now();
    let workspace = rusty_core::Workspace::load(root);
    println!(
        "Workspace::load   {:>7.0} ms   {}",
        at.elapsed().as_secs_f64() * 1000.0,
        if workspace.is_ok() { "ok" } else { "failed" }
    );

    if let Ok(workspace) = workspace {
        let at = std::time::Instant::now();
        let report = workspace.report();
        println!(
            "report            {:>7.0} ms   {}",
            at.elapsed().as_secs_f64() * 1000.0,
            if report.is_ok() { "ok" } else { "failed" }
        );
    }
}
