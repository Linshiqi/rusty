//! The probe's run for the finder's symbols, kept apart from `probe.rs`'s
//! long list of steps: with `RUSTY_PROBE_SYMBOLS`, on `draw-vectors`'
//! `src/lib.rs` — a copy, with a long tail of functions appended so a symbol
//! can be far from the view — it waits for the server, then presses
//! Ctrl+Shift+O and photographs the outline, types part of a name, moves the
//! selection and photographs the editor showing the symbol, takes it with
//! Enter and comes back with Alt+Left; shows a symbol far down and puts the
//! view back with Escape; groups the outline with `@:` and walks past its
//! twelfth row; then presses Ctrl+T, asks for `norm` until the project has
//! loaded, holds the server's answer to the next name back as a slow server
//! would, types
//! more of the name before it lands, and says which answer the rows showed
//! at every moment — the slow one must never be it — and takes a row with
//! Enter; last, `@` over a file no server reads. Every key is dispatched as
//! the platform dispatches it.

use std::{cell::Cell, time::Duration};

use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowEvent},
};

use crate::{Code, Shell, probe_surfaces::Run, work};

/// How long the probe holds the first workspace answer back: longer than a
/// pause and an answer to the name typed after it take together.
const HOLD: Duration = Duration::from_millis(2500);

thread_local! {
    /// The answers dropped before the held one was asked: an answer to a
    /// name asked while the project loaded may have been dropped already.
    static DROPPED: Cell<u32> = const { Cell::new(0) };
    /// The answer to the second name has been photographed.
    static SNAPPED: Cell<bool> = const { Cell::new(false) };
}

fn send(run: &Run, event: WindowEvent) {
    run.window.window().dispatch_event(event);
}

/// A key with modifiers held, each pressed first and let go after.
fn chord(run: &Run, held: &[Key], text: slint::SharedString) {
    for key in held {
        send(
            run,
            WindowEvent::KeyPressed {
                text: (*key).into(),
            },
        );
    }
    send(run, WindowEvent::KeyPressed { text: text.clone() });
    send(run, WindowEvent::KeyReleased { text });
    for key in held.iter().rev() {
        send(
            run,
            WindowEvent::KeyReleased {
                text: (*key).into(),
            },
        );
    }
}

fn key(run: &Run, text: slint::SharedString) {
    chord(run, &[], text);
}

/// Characters typed one key at a time, as a hand types them.
fn typed(run: &Run, text: &str) {
    for c in text.chars() {
        key(run, c.to_string().into());
    }
}

fn snapshot(run: &Run, name: &str) {
    crate::probe::snapshot(run.window, &run.dir.join(name));
}

/// Wait at this step until `ready`, at most `ticks` times.
fn until(run: &Run, ready: bool, ticks: u32) -> bool {
    if ready || run.waited.get() >= ticks {
        run.waited.set(0);
        return true;
    }
    run.waited.set(run.waited.get() + 1);
    run.step.set(run.step.get() - 1);
    false
}

/// The picker's rows as they read: the icon's number, the name, what it
/// sits in, the right-hand column and a group's heading.
fn rows(shell: &Shell) -> Vec<String> {
    shell
        .get_picker_rows()
        .iter()
        .map(|row| {
            let mut said = format!("[{}] {}", row.glyph, row.title);
            if !row.detail.is_empty() {
                said += &format!(" · {}", row.detail);
            }
            if !row.chord.is_empty() {
                said += &format!(" · {}", row.chord);
            }
            if !row.group.is_empty() {
                said += &format!(" ‹{}›", row.group);
            }
            said
        })
        .collect()
}

/// Whether the rows are symbols rather than a sentence on its way.
fn has_symbols(shell: &Shell) -> bool {
    shell.get_picker_rows().iter().any(|row| row.glyph > 0)
}

/// Where the view and the caret are, and the row lit.
fn view(code: &Code) -> String {
    let app = work::app();
    format!(
        "{:?} at {:?}, view at {:.0}, lit row {}",
        app.editor().current_path(),
        app.editor().status().map(|s| (s.0, s.1)),
        -code.get_scroll_y(),
        code.get_preview_row()
    )
}

pub fn symbols(n: u32, run: &Run) {
    let shell = run.window.global::<Shell>();
    let code = run.window.global::<Code>();
    let app = work::app();
    match n {
        // The server has the file once it has painted it.
        7200 => {
            let colours = app.editor().told().map_or(0, |told| told.0);
            if !until(run, shell.get_active_kind() == "file" && colours > 0, 3000) {
                return;
            }
            eprintln!("symbols: the server painted {colours} spans");
            eprintln!("symbols: before: {}", view(&code));
            snapshot(run, "symbols-start.png");
            chord(run, &[Key::Control, Key::Shift], "o".into());
        }
        7201 => {
            if !until(run, has_symbols(&shell), 750) {
                eprintln!("symbols: waiting on {:?}", rows(&shell));
                return;
            }
            eprintln!(
                "symbols: Ctrl+Shift+O opened the finder with {:?}: {} rows {:#?}",
                shell.get_picker_text(),
                shell.get_picker_rows().row_count(),
                rows(&shell)
            );
            eprintln!("symbols: the first row shown: {}", view(&code));
            snapshot(run, "symbols-outline.png");
            typed(run, "cr");
        }
        7203 => {
            eprintln!(
                "symbols: typed {:?}: {:?}",
                shell.get_picker_text(),
                rows(&shell)
            );
            eprintln!("symbols: the first shown: {}", view(&code));
            snapshot(run, "symbols-typed.png");
            key(run, Key::DownArrow.into());
        }
        7205 => {
            eprintln!(
                "symbols: Down to row {}: {}",
                shell.get_picker_selected(),
                view(&code)
            );
            snapshot(run, "symbols-preview.png");
            key(run, Key::Return.into());
        }
        7208 => {
            eprintln!(
                "symbols: Enter went to {} — finder open {}",
                view(&code),
                shell.get_picker_open()
            );
            snapshot(run, "symbols-jumped.png");
            chord(run, &[Key::Alt], Key::LeftArrow.into());
        }
        7210 => {
            eprintln!("symbols: Alt+Left came back to {}", view(&code));
            snapshot(run, "symbols-back.png");
            chord(run, &[Key::Control, Key::Shift], "o".into());
        }
        7211 => {
            if !until(run, has_symbols(&shell), 750) {
                return;
            }
            eprintln!("symbols: before the far symbol: {}", view(&code));
            typed(run, "far_away");
        }
        7213 => {
            eprintln!("symbols: typed far_away: {:?}", rows(&shell));
            eprintln!("symbols: shown far down: {}", view(&code));
            snapshot(run, "symbols-far.png");
            key(run, Key::Escape.into());
        }
        7215 => {
            eprintln!(
                "symbols: Escape put the view back: {} — finder open {}",
                view(&code),
                shell.get_picker_open()
            );
            snapshot(run, "symbols-escaped.png");
            chord(run, &[Key::Control, Key::Shift], "o".into());
        }
        7216 => {
            if !until(run, has_symbols(&shell), 750) {
                return;
            }
            typed(run, ":");
        }
        7218 => {
            eprintln!("symbols: @: grouped: {:#?}", rows(&shell));
            snapshot(run, "symbols-grouped.png");
            // Past the twelfth row: the list scrolls to keep it in view.
            for _ in 0..15 {
                key(run, Key::DownArrow.into());
            }
        }
        7220 => {
            eprintln!(
                "symbols: Down fifteen times to row {}: {}",
                shell.get_picker_selected(),
                view(&code)
            );
            snapshot(run, "symbols-grouped-down.png");
            key(run, Key::Escape.into());
        }
        7221 => chord(run, &[Key::Control], "t".into()),
        7222 => {
            eprintln!(
                "symbols: Ctrl+T opened the finder with {:?}: {:?}",
                shell.get_picker_text(),
                rows(&shell)
            );
            snapshot(run, "workspace-empty.png");
            // A name asked until the project has loaded: before then the
            // server answers with what it has, which may be nothing.
            typed(run, "norm");
        }
        7223 => {
            let waited = run.waited.get();
            if waited.is_multiple_of(25) {
                eprintln!("symbols: tick {waited}: #norm {:?}", rows(&shell));
            }
            if !until(run, has_symbols(&shell), 3000) {
                return;
            }
            eprintln!("symbols: #norm answered: {:?}", rows(&shell));
            for _ in 0.."norm".len() {
                key(run, Key::Backspace.into());
            }
        }
        7224 => {
            eprintln!("symbols: cleared to {:?}", shell.get_picker_text());
            app.hold_next_symbol_answer(HOLD);
            typed(run, "cro");
        }
        // The pause runs out and `cro` is asked — its answer held back —
        // and then the name is typed on, before that answer lands.
        7225 => {
            let (asked, shown, dropped) = app.symbol_search();
            if !until(run, asked.as_deref() == Some("cro"), 100) {
                return;
            }
            DROPPED.set(dropped);
            eprintln!(
                "symbols: #cro asked, its answer held {HOLD:?}: rows {:?}, showing the answer for {shown:?}",
                rows(&shell)
            );
            snapshot(run, "workspace-asking.png");
            typed(run, "ss");
        }
        // Until the held answer has landed and a little more: which answer
        // is on screen, at every tick, under which name.
        7226 => {
            let (asked, shown, dropped) = app.symbol_search();
            let late = dropped - DROPPED.get();
            let text = shell.get_picker_text().to_string();
            if shown.as_deref() == Some("cro") {
                eprintln!("symbols: WRONG — the answer for cro shown under {text:?}");
            }
            let waited = run.waited.get();
            if waited.is_multiple_of(10) {
                eprintln!(
                    "symbols: tick {waited}: {text:?} asked {asked:?}, shows the answer for {shown:?}, {late} dropped since: {:?}",
                    rows(&shell)
                );
            }
            if shown.as_deref() == Some("cross") && !SNAPPED.replace(true) {
                snapshot(run, "workspace-cross.png");
            }
            if !until(run, late > 0 && shown.as_deref() == Some("cross"), 250) {
                return;
            }
            eprintln!(
                "symbols: after {waited} ticks the held answer for cro landed and was dropped ({late}); {text:?} shows the answer for {shown:?}: {:?}",
                rows(&shell)
            );
            snapshot(run, "workspace-after.png");
            key(run, Key::Return.into());
        }
        7236 => {
            eprintln!("symbols: Enter on #cross went to {}", view(&code));
            snapshot(run, "workspace-jumped.png");
            app.open_file("Cargo.toml".into());
        }
        7237 => {
            if !until(
                run,
                app.editor().current_path().as_deref() == Some("Cargo.toml"),
                250,
            ) {
                return;
            }
            chord(run, &[Key::Control, Key::Shift], "o".into());
        }
        7239 => {
            eprintln!("symbols: @ over Cargo.toml: {:?}", rows(&shell));
            snapshot(run, "symbols-no-server.png");
            key(run, Key::Escape.into());
        }
        7240 => {
            chord(run, &[Key::Control], "p".into());
        }
        7241 => {
            typed(run, "#");
        }
        7243 => {
            eprintln!("symbols: Ctrl+P then #: {:?}", rows(&shell));
            key(run, Key::Escape.into());
        }
        7245 => app.act("app.exit"),
        _ => {}
    }
}
