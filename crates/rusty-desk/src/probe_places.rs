//! `RUSTY_PROBE_PLACES`, from step 8100: every tab comes back where it was
//! read. Two Markdown chapters read as pages and a code file, each scrolled
//! a different distance by the wheel, then each brought back to the front
//! in turn: the page's offset and the code's must be what they were left
//! at. The places go to `places.txt` in the probe's directory and the
//! window leaves through File > Exit, which writes the strip; run again on
//! the same data directory with `RUSTY_PROBE_PLACES=again`, and the project
//! opens with each file back where it was read.
//!
//! On a project with `03-vectors.md` and `04-rotations.md` at its root and
//! a `src/main.rs` long enough to scroll — chapters of a real book, opened
//! on `src/main.rs`.

use std::cell::Cell;

use slint::ComponentHandle;

use crate::{Code, Shell, probe_surfaces::Run, work};

const FIRST: &str = "03-vectors.md";
const SECOND: &str = "04-rotations.md";
const CODE: &str = "src/main.rs";

thread_local! {
    /// What each was left at: the first page, the second, the code.
    static LEFT: Cell<[f32; 3]> = const { Cell::new([0.0; 3]) };
}

fn left() -> [f32; 3] {
    LEFT.with(Cell::get)
}

fn keep(at: usize, value: f32) {
    LEFT.with(|left| {
        let mut now = left.get();
        now[at] = value;
        left.set(now);
    });
}

pub fn places(n: u32, run: &Run) {
    let app = work::app();
    let shell = run.window.global::<Shell>();
    let code = run.window.global::<Code>();
    let (_, height) = run.size();
    let mid = ((run.editor_left() + run.editor_right()) / 2.0, height * 0.5);
    let front = || app.editor().current_path().unwrap_or_default();
    let page_ready = |name: &str| front() == name && shell.get_md_page() && app.page_settled();
    let again = std::env::var("RUSTY_PROBE_PLACES").as_deref() == Ok("again");
    let say = |what: &str| {
        eprintln!(
            "places: {what}: front {:?}, page offset {:.1}, code offset {:.1}",
            front(),
            shell.get_md_scroll(),
            code.get_scroll_y()
        );
    };
    match n {
        8100 => {
            if !run.until(shell.get_active_kind() == "file", 750) {
                return;
            }
            shell.invoke_close_tab("page:welcome".into());
            if again {
                // What the first run left, read back from its file.
                if let Ok(text) = std::fs::read_to_string(run.dir.join("places.txt")) {
                    let values: Vec<f32> = text
                        .split_whitespace()
                        .filter_map(|v| v.parse().ok())
                        .collect();
                    for (at, value) in values.into_iter().take(3).enumerate() {
                        keep(at, value);
                    }
                }
                run.step.set(8150);
            }
        }
        8101 => app.open_file(FIRST.into()),
        8102 => {
            if !run.until(page_ready(FIRST), 1500) {
                return;
            }
            say("the first chapter opened");
            run.wheel(mid.0, mid.1, -1200.0, false);
        }
        8104 => run.wheel(mid.0, mid.1, -1200.0, false),
        8108 => {
            say("the first chapter scrolled");
            keep(0, shell.get_md_scroll());
            run.snapshot("places-first.png");
            app.open_file(SECOND.into());
        }
        8109 => {
            if !run.until(page_ready(SECOND), 1500) {
                return;
            }
            say("the second chapter opened, at its top");
            run.wheel(mid.0, mid.1, -3000.0, false);
        }
        8114 => {
            say("the second chapter scrolled");
            keep(1, shell.get_md_scroll());
            app.open_file(CODE.into());
        }
        8115 => {
            if !run.until(front() == CODE && !shell.get_md_page(), 500) {
                return;
            }
            run.wheel(mid.0, mid.1, -900.0, false);
        }
        8118 => {
            say("the code scrolled");
            keep(2, code.get_scroll_y());
            app.open_file(FIRST.into());
        }
        // Back to each in turn; a page's place goes back once its blocks
        // are laid out again, a few turns after it is settled.
        8119 => {
            if !run.until(
                page_ready(FIRST) && (shell.get_md_scroll() - left()[0]).abs() < 2.0,
                400,
            ) {
                return;
            }
            say("the first chapter again");
            eprintln!(
                "places: the first chapter left at {:.1}, back at {:.1}",
                left()[0],
                shell.get_md_scroll()
            );
            run.snapshot("places-first-again.png");
            app.open_file(SECOND.into());
        }
        8120 => {
            if !run.until(
                page_ready(SECOND) && (shell.get_md_scroll() - left()[1]).abs() < 2.0,
                400,
            ) {
                return;
            }
            eprintln!(
                "places: the second chapter left at {:.1}, back at {:.1}",
                left()[1],
                shell.get_md_scroll()
            );
            app.open_file(CODE.into());
        }
        8121 => {
            if !run.until(front() == CODE && !shell.get_md_page(), 500) {
                return;
            }
            eprintln!(
                "places: the code left at {:.1}, back at {:.1}",
                left()[2],
                code.get_scroll_y()
            );
            let [a, b, c] = left();
            let _ = std::fs::write(run.dir.join("places.txt"), format!("{a} {b} {c}\n"));
            // The first chapter in front, so the next run opens on a page.
            app.open_file(FIRST.into());
        }
        8124 => app.act("app.exit"),
        // The next run, opened on the code by its command line: each file
        // where the last run left it.
        8150 => {
            if !run.until(front() == CODE && !shell.get_md_page(), 750) {
                return;
            }
            eprintln!(
                "places: after a restart, the code left at {:.1}, opened at {:.1}",
                left()[2],
                code.get_scroll_y()
            );
            app.open_file(FIRST.into());
        }
        8151 => {
            if !run.until(
                page_ready(FIRST) && (shell.get_md_scroll() - left()[0]).abs() < 2.0,
                600,
            ) {
                return;
            }
            eprintln!(
                "places: after a restart, the first chapter left at {:.1}, opened at {:.1}",
                left()[0],
                shell.get_md_scroll()
            );
            run.snapshot("places-restart.png");
            app.open_file(SECOND.into());
        }
        8152 => {
            if !run.until(
                page_ready(SECOND) && (shell.get_md_scroll() - left()[1]).abs() < 2.0,
                600,
            ) {
                return;
            }
            eprintln!(
                "places: after a restart, the second chapter left at {:.1}, opened at {:.1}",
                left()[1],
                shell.get_md_scroll()
            );
        }
        8154 => app.act("app.exit"),
        _ => {}
    }
}
