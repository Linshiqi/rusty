//! The probe's run for a failed test's message (`RUSTY_PROBE_TESTFAIL`):
//! on a file with a test that fails an `assert_eq!` — by default
//! `the_dot_of_two_axes`, whose `assert_eq!(…, 1.0, "two axes …")` is wrong
//! by one; `RUSTY_PROBE_TESTFAIL=<name>` names another — it runs the test
//! from its lens and photographs the squiggle under the place it panicked
//! and the message after the line; opens the hover card from the keyboard
//! and then with the pointer over the message, photographing the whole
//! message each time and saying whether a fix was asked for; moves the file
//! into a window of its own and photographs the marks there; types on the
//! line and photographs the marks gone; mends the test, runs it again and
//! photographs the tick with no mark. Every line it prints says what the
//! rows, the card and the Problems panel held — on a copy, since the run
//! writes the mended test to disk.

use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowEvent},
};

use crate::{Code, Shell, probe_surfaces::Run, work};

/// The wrong expectation, and the mended one.
const WRONG: &str = "1.0, \"two axes";
const RIGHT: &str = "0.0, \"two axes";

fn test_name() -> String {
    std::env::var("RUSTY_PROBE_TESTFAIL")
        .ok()
        .filter(|name| !name.is_empty() && name != "1")
        .unwrap_or_else(|| "the_dot_of_two_axes".to_string())
}

/// Every row with a message: its number, the message and where it starts,
/// and the x of every error squiggle on it.
fn marked(code: &Code) -> Vec<(i32, String, f32, Vec<f32>)> {
    code.get_rows()
        .iter()
        .filter(|row| !row.message.is_empty() || row.marks.iter().any(|mark| mark.kind == 5))
        .map(|row| {
            (
                row.number,
                row.message.to_string(),
                row.message_x,
                row.marks
                    .iter()
                    .filter(|mark| mark.kind == 5)
                    .map(|mark| mark.x)
                    .collect(),
            )
        })
        .collect()
}

/// The lens of the test: its row in the model, and its verdict.
fn lens_of(code: &Code, text: &str, name: &str) -> Option<(usize, i32)> {
    let line = text
        .lines()
        .position(|l| l.trim_start().starts_with(&format!("fn {name}(")))?;
    // The lens sits on the attribute line above the item.
    let rows = code.get_rows();
    (0..=line).rev().take(4).find_map(|at| {
        let row = rows.row_data(at)?;
        (row.lens == 1).then_some((at, row.verdict))
    })
}

fn card(code: &Code) -> Vec<(String, String, bool)> {
    code.get_hover_problems()
        .iter()
        .map(|p| (p.text.to_string(), p.detail.to_string(), p.mono))
        .collect()
}

/// What the Problems panel counts and lists.
fn problems(shell: &Shell) -> (i32, Vec<String>) {
    (
        shell.get_errors(),
        shell
            .get_problems()
            .iter()
            .map(|row| format!("{}:{} {}", row.path, row.line, row.text))
            .collect(),
    )
}

pub fn run(n: u32, run: &Run) {
    let window = run.window;
    let (step, waited) = (run.step, run.waited);
    let shell = window.global::<Shell>();
    let code = window.global::<Code>();
    let app = work::app();
    let editor = app.editor();
    let name = test_name();
    let draft = || {
        editor
            .current_path()
            .and_then(|path| editor.draft_of(&path))
            .map(|(text, ..)| text)
            .unwrap_or_default()
    };
    // The line the failing assertion is on, from one, and the column of
    // its macro, from one.
    let assertion = || {
        draft().lines().enumerate().find_map(|(index, line)| {
            let at = line.find("assert_eq!(x.dot(y)")?;
            Some((index + 1, line[..at].chars().count() + 1))
        })
    };
    let wait = |ticks: u32| -> bool {
        if waited.get() < ticks {
            waited.set(waited.get() + 1);
            step.set(n);
            true
        } else {
            false
        }
    };
    let photo = |file: &str| crate::probe::snapshot(window, &run.dir.join(file));
    let send = |event| window.window().dispatch_event(event);
    let key = |text: &str| {
        send(WindowEvent::KeyPressed { text: text.into() });
        send(WindowEvent::KeyReleased { text: text.into() });
    };
    match n {
        7300 => {
            if shell.get_active_kind() != "file" || lens_of(&code, &draft(), &name).is_none() {
                if wait(1500) {
                    return;
                }
                eprintln!(
                    "testfail: no lens for `{name}` in {:?}",
                    editor.current_path()
                );
                app.act("app.exit");
                return;
            }
            waited.set(0);
            if let Some((line, col)) = assertion() {
                editor.go_to(line, Some(col));
            }
            eprintln!(
                "testfail: before the run, Problems {:?}, marks {:?}",
                problems(&shell),
                marked(&code)
            );
            let (row, _) = lens_of(&code, &draft(), &name).expect("found above");
            code.invoke_lens(row as i32, "run".into());
        }
        7301 => {
            // The run, then its marks: cargo is asked for the workspace's
            // root beside it.
            let ran = !app.runner().is_running();
            let shown = marked(&code)
                .iter()
                .any(|(_, message, ..)| !message.is_empty());
            if (!ran || !shown) && wait(3000) {
                return;
            }
            eprintln!(
                "testfail: after the run ({} ticks): marks {:?}; lens {:?}; Problems {:?}",
                waited.get(),
                marked(&code),
                lens_of(&code, &draft(), &name),
                problems(&shell)
            );
            waited.set(0);
            // The line on screen, wherever the view was when the file came.
            if let Some((line, col)) = assertion() {
                editor.go_to(line, Some(col));
            }
        }
        7302 => {
            photo("testfail-marked.png");
            app.act("editor.show-hover");
        }
        7303 => {
            if (!code.get_hover_open() || card(&code).is_empty()) && wait(250) {
                return;
            }
            // A moment for a fix that would have been asked for to land.
            if waited.get() < 60 && wait(60) {
                return;
            }
            eprintln!(
                "testfail: the card from the keys, caret at {:?}: {:?}; fixes {:?}",
                editor.status().map(|s| (s.0, s.1)),
                card(&code),
                editor.card_fixes()
            );
            waited.set(0);
            photo("testfail-card.png");
            key(&slint::SharedString::from(Key::Escape));
        }
        7304 => {
            // The server settled first — its hints arriving draw every row
            // again, and a card a pointer rests on goes with the rows it
            // was over — then where the code's rows are in the window: a
            // move over the middle of the window, and what the row it
            // landed on says.
            let hinted = editor.told().is_some_and(|(_, hints, _)| hints > 0);
            if !hinted && wait(1500) {
                return;
            }
            waited.set(0);
            let size = window
                .window()
                .size()
                .to_logical(window.window().scale_factor());
            send(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(size.width / 2.0, size.height / 4.0),
            });
        }
        7305 => {
            let Some((row, x, y, (wx, wy))) = editor.link_pointer() else {
                eprintln!("testfail: the middle of the window is not over the code");
                step.set(7310);
                return;
            };
            let rows = code.get_row_height();
            let (left, top) = (wx - x, wy - y - row as f32 * rows);
            let Some((at, message_x)) = code
                .get_rows()
                .iter()
                .enumerate()
                .find(|(_, r)| !r.message.is_empty())
                .map(|(at, r)| (at, r.message_x))
            else {
                eprintln!("testfail: no message to point at");
                step.set(7310);
                return;
            };
            let target = (left + message_x + 20.0, top + at as f32 * rows + rows / 2.0);
            eprintln!(
                "testfail: pointing at the message of row {} at ({:.0}, {:.0})",
                at + 1,
                target.0,
                target.1
            );
            send(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(target.0, target.1),
            });
            // And on along the message, as a hand's pointer goes on moving:
            // after one jump, the row it left says so after the row it
            // reached has, and that "left" stops the card's timer.
            send(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(target.0 + 2.0, target.1),
            });
            waited.set(0);
        }
        7306 => {
            if (!code.get_hover_open() || card(&code).is_empty()) && wait(250) {
                return;
            }
            eprintln!(
                "testfail: the card under the pointer after {} ticks, open {}: {:?}",
                waited.get(),
                code.get_hover_open(),
                card(&code)
            );
            waited.set(0);
            photo("testfail-card-pointer.png");
        }
        7307 => {
            // A moment for a fix that would have been asked for to land.
            if wait(40) {
                return;
            }
            eprintln!(
                "testfail: and a moment later: open {}, fixes {:?}",
                code.get_hover_open(),
                editor.card_fixes()
            );
            // Off the code, so the card goes.
            send(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(4.0, 4.0),
            });
        }
        // The file in a window of its own: the same marks there.
        7310 => {
            app.act("editor.detach");
        }
        7312 => {
            let Some((torn, _)) = app.torn_windows().into_iter().next() else {
                if wait(250) {
                    return;
                }
                eprintln!("testfail: no window of its own");
                step.set(7320);
                return;
            };
            let torn_code = torn.global::<Code>();
            if marked(&torn_code).is_empty() && wait(100) {
                return;
            }
            waited.set(0);
            eprintln!(
                "testfail: in a window of its own, marks {:?}",
                marked(&torn_code)
            );
            crate::probe::snapshot_of(torn.window(), &run.dir.join("testfail-torn.png"));
        }
        7314 => {
            if let Some((torn, surface)) = app.torn_windows().into_iter().next() {
                let path = surface.current_path();
                let send = |event| torn.window().dispatch_event(event);
                send(WindowEvent::KeyPressed {
                    text: Key::Control.into(),
                });
                send(WindowEvent::KeyPressed { text: "w".into() });
                send(WindowEvent::KeyReleased { text: "w".into() });
                send(WindowEvent::KeyReleased {
                    text: Key::Control.into(),
                });
                if let Some(path) = path {
                    app.open_file(path);
                }
            }
        }
        // Typed on: the marks go, and an undo does not bring them back.
        7320 => {
            if shell.get_active_kind() != "file" && wait(500) {
                return;
            }
            waited.set(0);
            window.invoke_focus_code();
            if let Some((line, _)) = assertion() {
                editor.go_to(line, Some(usize::MAX));
            }
        }
        7321 => {
            key(" ");
        }
        7323 => {
            eprintln!(
                "testfail: typed on line {:?}: marks {:?}; lens {:?}",
                assertion().map(|a| a.0),
                marked(&code),
                lens_of(&code, &draft(), &name)
            );
            photo("testfail-typed.png");
            send(WindowEvent::KeyPressed {
                text: Key::Control.into(),
            });
            key("z");
            send(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
        }
        7325 => {
            eprintln!("testfail: undone: marks {:?}", marked(&code));
            // The test mended and run again.
            let Some(path) = editor.current_path() else {
                return;
            };
            let text = draft();
            let version = editor.version_of(&path).unwrap_or(0);
            if !text.contains(WRONG) {
                eprintln!("testfail: no `{WRONG}` to mend");
                app.act("app.exit");
                return;
            }
            editor.replace(&path, version, text.replacen(WRONG, RIGHT, 1));
        }
        7327 => {
            let Some((row, _)) = lens_of(&code, &draft(), &name) else {
                return;
            };
            code.invoke_lens(row as i32, "run".into());
            waited.set(0);
        }
        7328 => {
            let passed = lens_of(&code, &draft(), &name).is_some_and(|(_, verdict)| verdict == 1);
            if (app.runner().is_running() || !passed) && wait(3000) {
                return;
            }
            eprintln!(
                "testfail: mended and run again ({} ticks): lens {:?}; marks {:?}; Problems {:?}",
                waited.get(),
                lens_of(&code, &draft(), &name),
                marked(&code),
                problems(&shell)
            );
            photo("testfail-mended.png");
        }
        7330 => app.act("app.exit"),
        _ => {}
    }
}
