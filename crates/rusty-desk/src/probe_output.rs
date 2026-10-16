//! The probe's `RUSTY_PROBE_OUTPUT` run, from step 7500: what Output and
//! Problems show across runs, the filter over them, and Ctrl+`.
//!
//! On a copy of `examples/draw-vectors` opened at `src/lib.rs`, it breaks
//! the library, builds, mends it and builds again, and says whether the
//! first build's error is still in the dock or in Problems; then the same
//! with a test's lens and with an example's ▶ Run as the second run. Then
//! it filters the dock — typed text, a line arriving under the filter, the
//! errors switch, Ctrl+F from the dock and Escape — and Problems; then it
//! presses Ctrl+` from the code and from the terminal, as a hand does, and
//! checks the shell never got the backquote. Each state is photographed.
//! It edits the library and leaves it mended: point it at a copy.

use std::cell::{Cell, RefCell};

use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};

use crate::{Code, Shell, probe_surfaces::Run, work};

thread_local! {
    /// The library as it was, the file it is, and whether the run waited
    /// for has been seen going.
    static ORIGINAL: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
    static SEEN: Cell<bool> = const { Cell::new(false) };
    static SCREEN: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The line the probe breaks the library after, and what it puts there: a
/// type error rustc names with a place (`E0308`), which rust-analyzer's own
/// analysis does not report — so what Problems lists of it is the build's.
const AFTER: &str = "pub fn norm(self) -> f32 {";
const BROKEN: &str = "        let broken: u32 = \"text\";";

fn send(run: &Run, event: WindowEvent) {
    run.window.window().dispatch_event(event);
}

/// A key with modifiers held, pressed and let go as a hand does it.
fn chord(run: &Run, held: &[Key], text: &str) {
    for key in held {
        send(
            run,
            WindowEvent::KeyPressed {
                text: (*key).into(),
            },
        );
    }
    send(run, WindowEvent::KeyPressed { text: text.into() });
    send(run, WindowEvent::KeyReleased { text: text.into() });
    for key in held.iter().rev() {
        send(
            run,
            WindowEvent::KeyReleased {
                text: (*key).into(),
            },
        );
    }
}

fn snapshot(run: &Run, name: &str) {
    crate::probe::snapshot(run.window, &run.dir.join(name));
}

/// Stay at this step.
fn again(run: &Run) {
    run.step.set(run.step.get() - 1);
}

/// Wait at this step for the run just asked for to start and end: `true`
/// once it has. Two minutes at most for it to start.
fn ran(run: &Run) -> bool {
    let app = work::app();
    if app.runner().is_running() {
        SEEN.set(true);
        again(run);
        return false;
    }
    if !SEEN.get() && run.waited.get() < 3000 {
        run.waited.set(run.waited.get() + 1);
        again(run);
        return false;
    }
    SEEN.set(false);
    run.waited.set(0);
    true
}

/// The library broken, or as it was.
fn library(broken: bool) {
    let app = work::app();
    let editor = app.editor();
    let Some((path, original)) = ORIGINAL.with(|o| o.borrow().clone()) else {
        return;
    };
    let text = if broken {
        original.replacen(AFTER, &format!("{AFTER}\n{BROKEN}"), 1)
    } else {
        original
    };
    let version = editor.version_of(&path).unwrap_or(0);
    let done = editor.replace(&path, version, text);
    eprintln!(
        "output: {path} {} ({done})",
        if broken { "broken" } else { "mended" }
    );
}

/// What the dock and Problems hold, in a line each.
fn report(run: &Run, after: &str) {
    let shell = run.window.global::<Shell>();
    let output = shell.get_output();
    let stale = output
        .iter()
        .filter(|line| line.text.contains("E0308"))
        .count();
    let commands: Vec<String> = output
        .iter()
        .filter(|line| line.kind == crate::run::kind::COMMAND)
        .map(|line| line.text.to_string())
        .collect();
    let problems: Vec<String> = shell
        .get_problems()
        .iter()
        .map(|row| format!("{} {}", row.text, row.detail))
        .collect();
    eprintln!(
        "output: after {after}: the dock has {} lines, {stale} naming E0308, commands {commands:?}; \
         Problems lists {problems:?}",
        output.row_count()
    );
}

/// The code view's row whose lens is of `kind`: 1 a test, 2 an example.
fn lens_row(run: &Run, kind: i32) -> Option<usize> {
    run.window
        .global::<Code>()
        .get_rows()
        .iter()
        .position(|row| row.lens == kind)
}

/// Breaking, mending and running again, from step 7500 to 7529: the steps
/// that show what a second run leaves of the first.
pub fn reproduce(n: u32, run: &Run) {
    let shell = run.window.global::<Shell>();
    let app = work::app();
    match n {
        7500 => {
            if shell.get_active_kind() != "file" {
                again(run);
                return;
            }
            let editor = app.editor();
            let path = editor.current_path().unwrap_or_default();
            let text = editor
                .draft_of(&path)
                .map(|(text, ..)| text)
                .unwrap_or_default();
            if !text.contains(AFTER) {
                eprintln!("output: {path} has no `{AFTER}`: open src/lib.rs of draw-vectors");
                app.act("app.exit");
                return;
            }
            ORIGINAL.with(|o| *o.borrow_mut() = Some((path, text)));
            library(true);
        }
        7501 => app.act("run.build"),
        7502 => {
            if ran(run) {
                report(run, "a build of the broken library");
                shell.invoke_show_panel("bottom".into(), "output".into());
            }
        }
        7503 => {
            snapshot(run, "output-1-broken.png");
            shell.invoke_show_panel("bottom".into(), "problems".into());
        }
        7504 => {
            snapshot(run, "problems-1-broken.png");
            library(false);
        }
        7505 => app.act("run.build"),
        7506 => {
            if ran(run) {
                report(run, "the mended library built again");
                shell.invoke_show_panel("bottom".into(), "output".into());
            }
        }
        7507 => {
            snapshot(run, "output-2-mended.png");
            shell.invoke_show_panel("bottom".into(), "problems".into());
        }
        7508 => {
            snapshot(run, "problems-2-mended.png");
            library(true);
        }
        // Broken and built again, then mended and run from a test's lens.
        7509 => app.act("run.build"),
        7510 => {
            if ran(run) {
                report(run, "the broken library built again");
                library(false);
            }
        }
        7511 => match lens_row(run, 1) {
            Some(row) => {
                eprintln!("output: Run Test from the lens on row {row}");
                run.window
                    .global::<Code>()
                    .invoke_lens(row as i32, "run".into());
            }
            None => eprintln!("output: no test lens on screen"),
        },
        7512 => {
            if ran(run) {
                report(run, "the mended library's test run from its lens");
                shell.invoke_show_panel("bottom".into(), "output".into());
            }
        }
        7513 => {
            snapshot(run, "lens-output.png");
            shell.invoke_show_panel("bottom".into(), "problems".into());
        }
        7514 => {
            snapshot(run, "lens-problems.png");
            library(true);
        }
        // Broken and built again, then mended and the example run from
        // its ▶ Run.
        7515 => app.act("run.build"),
        7516 => {
            if ran(run) {
                report(run, "the broken library built once more");
                library(false);
                app.open_file("examples/cross.rs".to_string());
            }
        }
        7517 => {
            let front = app.editor().current_path().unwrap_or_default();
            if !front.ends_with("cross.rs") || lens_row(run, 2).is_none() {
                run.waited.set(run.waited.get() + 1);
                if run.waited.get() < 500 {
                    again(run);
                }
                return;
            }
            run.waited.set(0);
            let row = lens_row(run, 2).unwrap_or(0);
            eprintln!("output: ▶ Run from the example's lens on row {row}");
            run.window
                .global::<Code>()
                .invoke_lens(row as i32, "run".into());
        }
        7518 => {
            if ran(run) {
                report(run, "the example run from its lens");
                shell.invoke_show_panel("bottom".into(), "output".into());
            }
        }
        7519 => {
            snapshot(run, "example-output.png");
            shell.invoke_show_panel("bottom".into(), "problems".into());
        }
        7520 => snapshot(run, "example-problems.png"),
        _ => {}
    }
}

/// What the dock draws, each line as it reads, and its filter's count.
fn drawn(run: &Run) -> String {
    let shell = run.window.global::<Shell>();
    let lines: Vec<String> = shell
        .get_output()
        .iter()
        .map(|line| {
            let marked: Vec<String> = line
                .pieces
                .iter()
                .filter(|p| p.hit)
                .map(|p| p.text.to_string())
                .collect();
            if marked.is_empty() {
                line.text.to_string()
            } else {
                format!("{} [marked {marked:?}]", line.text)
            }
        })
        .collect();
    format!(
        "count {:?}, errors switch {}, {} lines drawn: {lines:#?}",
        shell.get_output_filter().count,
        shell.get_output_filter().errors,
        lines.len()
    )
}

/// The Output filter set as typing in its box sets it.
fn filter(run: &Run, text: &str) {
    let shell = run.window.global::<Shell>();
    shell.set_output_filter_text(text.into());
    shell.invoke_filter_edited("output".into(), text.into());
}

/// The filter and Ctrl+`, from step 7530.
pub fn filter_and_terminal(n: u32, run: &Run) {
    let shell = run.window.global::<Shell>();
    let app = work::app();
    let (_, height) = (run.window.get_laid_width(), run.window.get_laid_height());
    let left = if shell.get_left_open() {
        shell.get_left_size() + 1.0
    } else {
        0.0
    };
    // Under the bottom dock's strip of tabs, over the status bar.
    let panel_top = height - 24.0 - shell.get_bottom_size() + 35.0;
    match n {
        7530 => {
            shell.invoke_show_panel("bottom".into(), "output".into());
            filter(run, "VECTOR");
        }
        7531 => {
            eprintln!("filter: `VECTOR` typed: {}", drawn(run));
            snapshot(run, "filter-text.png");
            // Broken and built with the filter in force: every line is
            // judged as it arrives.
            library(true);
        }
        7532 => app.act("run.build"),
        7533 => {
            if ran(run) {
                eprintln!(
                    "filter: a build that failed, under `VECTOR`: {}",
                    drawn(run)
                );
                shell.invoke_show_panel("bottom".into(), "output".into());
            }
        }
        7534 => {
            snapshot(run, "filter-arrived.png");
            filter(run, "");
            shell.invoke_filter_level("output".into(), "errors".into());
        }
        7535 => {
            eprintln!("filter: the box emptied, errors only: {}", drawn(run));
            snapshot(run, "filter-errors.png");
            shell.invoke_filter_level("output".into(), "errors".into());
        }
        7536 => {
            let (shown, kept) = app.runner().counts();
            eprintln!("filter: the switch off again: {shown} of {kept} lines drawn");
            // One line drawn that names no place, for a click that gives
            // the dock the keys and opens nothing.
            filter(run, "Compiling");
        }
        7537 => {
            // The keys to the dock, by a click on its first line.
            let position = LogicalPosition::new(left + 300.0, panel_top + 9.0);
            send(run, WindowEvent::PointerMoved { position });
            send(
                run,
                WindowEvent::PointerPressed {
                    position,
                    button: PointerEventButton::Left,
                },
            );
            send(
                run,
                WindowEvent::PointerReleased {
                    position,
                    button: PointerEventButton::Left,
                },
            );
        }
        7538 => chord(run, &[Key::Control], "f"),
        7539 => {
            // Into the box, over what it held: Ctrl+F selects it all.
            for ch in "mismatched".chars() {
                chord(run, &[], &ch.to_string());
            }
        }
        7540 => {
            eprintln!(
                "filter: Ctrl+F in the dock, then `mismatched` typed: the box holds {:?}; {}",
                shell.get_output_filter_text(),
                drawn(run)
            );
            snapshot(run, "filter-ctrl-f.png");
            chord(run, &[], &slint::SharedString::from(Key::Escape));
        }
        7541 => {
            let (shown, kept) = app.runner().counts();
            eprintln!(
                "filter: Escape in the box: it holds {:?}, {shown} of {kept} lines drawn",
                shell.get_output_filter_text()
            );
            snapshot(run, "filter-escaped.png");
            shell.invoke_show_panel("bottom".into(), "problems".into());
            shell.set_problems_filter_text("mismatched".into());
            shell.invoke_filter_edited("problems".into(), "mismatched".into());
        }
        7542 => {
            let rows: Vec<String> = shell
                .get_problems()
                .iter()
                .map(|row| format!("{} {} ({})", row.text, row.detail, row.count))
                .collect();
            eprintln!(
                "filter: Problems under `mismatched`: count {:?}, {rows:?}",
                shell.get_problems_filter().count
            );
            snapshot(run, "problems-filter.png");
            shell.set_problems_filter_text("".into());
            shell.invoke_filter_edited("problems".into(), "".into());
            library(false);
        }
        // Ctrl+` from the code: the terminal forward with the keys.
        7545 => {
            shell.invoke_show_panel("bottom".into(), "output".into());
            run.window.invoke_focus_code();
        }
        7546 => chord(run, &[Key::Control], "`"),
        7547 => {
            eprintln!(
                "terminal: Ctrl+` from the code: bottom open {}, {:?} in front",
                shell.get_bottom_open(),
                shell.get_bottom_front()
            );
            snapshot(run, "term-1-from-code.png");
        }
        7548 => {
            // The shell up: something on its screen.
            if crate::probe::screen_text(&shell).trim().is_empty() {
                run.waited.set(run.waited.get() + 1);
                if run.waited.get() < 500 {
                    again(run);
                }
                return;
            }
            run.waited.set(0);
            SCREEN.with(|s| *s.borrow_mut() = crate::probe::screen_text(&shell));
            // Pressed in the terminal, which has the keys.
            chord(run, &[Key::Control], "`");
        }
        7550 => {
            let before = SCREEN.with(|s| s.borrow().clone());
            let after = crate::probe::screen_text(&shell);
            eprintln!(
                "terminal: Ctrl+` in the terminal: bottom open {}; backquotes on the shell's screen before {}, after {}",
                shell.get_bottom_open(),
                before.matches('`').count(),
                after.matches('`').count()
            );
            snapshot(run, "term-2-hidden.png");
            // From the code again, with the dock closed.
            run.window.invoke_focus_code();
            chord(run, &[Key::Control], "`");
        }
        7552 => {
            eprintln!(
                "terminal: Ctrl+` from the code, dock closed: bottom open {}, {:?} in front",
                shell.get_bottom_open(),
                shell.get_bottom_front()
            );
            snapshot(run, "term-3-reopened.png");
            // The keys back to the code with the terminal in front: Ctrl+`
            // takes them to the terminal rather than putting it away.
            run.window.invoke_focus_code();
            chord(run, &[Key::Control], "`");
        }
        7553 => {
            // The shell's prompt before typing at it: a PowerShell starting
            // on a busy machine prints its banner seconds before it reads,
            // and drops what was typed in between.
            // A PowerShell prompt may carry a conda `(base) ` before its
            // `PS ` and wrap a long path onto a second row.
            let screen = crate::probe::screen_text(&shell);
            let prompt = screen.contains("PS ")
                && screen.lines().any(|line| line.trim_end().ends_with('>'))
                || screen.lines().any(|line| line.trim_end().ends_with('$'));
            if !prompt {
                run.waited.set(run.waited.get() + 1);
                if run.waited.get() < 3000 {
                    again(run);
                }
                return;
            }
            eprintln!("terminal: the prompt after {} ticks", run.waited.get());
            run.waited.set(0);
        }
        7554 => {
            for ch in "echo rusty-$((6*7))".chars() {
                chord(run, &[], &ch.to_string());
            }
            chord(run, &[], &slint::SharedString::from(Key::Return));
        }
        7555 => {
            if !crate::probe::screen_text(&shell).contains("rusty-42") {
                run.waited.set(run.waited.get() + 1);
                if run.waited.get() < 250 {
                    again(run);
                    return;
                }
            }
            run.waited.set(0);
            eprintln!(
                "terminal: Ctrl+` from the code, terminal in front: bottom open {}, {:?} in front; typed into the terminal, it answered {}",
                shell.get_bottom_open(),
                shell.get_bottom_front(),
                crate::probe::screen_text(&shell).contains("rusty-42")
            );
            snapshot(run, "term-4-focused.png");
        }
        7556 => app.act("app.exit"),
        _ => {}
    }
}
