//! A run that drives itself, for measuring and for looking: with
//! `RUSTY_PROBE=<dir>` the window photographs itself in each of its states,
//! types and scrolls in the open file, prints how long each frame took, and
//! quits.
//!
//! It is the check a person at the keyboard cannot make: a key's latency is
//! the time from the event to the frame that shows it, read off the
//! renderer's own notifier, and a picture of the window is the window's own
//! snapshot, not a screen grab of whatever is in front of it.
//!
//! It ends the way a person does, through File > Exit, so the draft it typed
//! is kept as a draft (hot exit) — which is why it needs a data directory of
//! its own: run in the real one, its typing would come back the next time
//! somebody opened that project. With `RUSTY_PROBE_SAVES` it also replaces
//! the file's text with badly formatted Rust and saves it, and says whether
//! rustfmt formatted what reached the disk — point it at a copy. With
//! `RUSTY_PROBE_NEW=<folder>` it ends by creating a project there through
//! the wizard — the first part rusty writes from its own template, so
//! nothing is generated or downloaded — and says what opened. With
//! `RUSTY_PROBE_PLAYGROUND` it ends by opening the first playground the
//! welcome page offers, and says whether it stayed out of the recents.
//! With `RUSTY_PROBE_BUILD` it types nothing: it builds the project as
//! Ctrl+Shift+B does, waits for the verdict, and says what the title bar,
//! the dock and Problems made of it. With `RUSTY_PROBE_TARGETS` it asks to
//! flash with no board chosen, photographs the list that opens, and picks
//! the simulator from it — opening no port and writing nothing. With
//! `RUSTY_PROBE_SIM` it runs the project in the simulator, lets it run for
//! three seconds once the firmware has spoken, and says what the pin
//! channel reported before stopping it. With `RUSTY_PROBE_EDIT` it edits
//! the board as a hand would — drags the first part, turns it, adds an LED
//! from the library, wires it to the devkit, undoes and redoes the wire,
//! deletes the LED — and prints the sheet file after each save.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use slint::{
    ComponentHandle, Model, RenderingState, Timer, TimerMode,
    platform::{Key, WindowEvent},
};

use crate::{AppWindow, Code, Find, Shell, work};

/// The sheet file as the editor last wrote it.
fn sheet_file(app: &crate::app::App) -> String {
    let Some(root) = app.root_path() else {
        return "no project".to_string();
    };
    let firmware = rusty_embed::project::firmware_root(&root);
    std::fs::read_to_string(firmware.join(".rusty").join("sim.toml"))
        .map(|text| {
            text.lines()
                .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .unwrap_or_else(|e| e.to_string())
}

/// Refuse to probe in the data directory somebody works in.
pub fn ready() -> Result<(), &'static str> {
    if std::env::var_os("RUSTY_PROBE").is_some() && std::env::var_os("RUSTY_CONFIG_DIR").is_none() {
        return Err("the probe keeps what it types as a draft: \
             give it a data directory of its own with RUSTY_CONFIG_DIR");
    }
    Ok(())
}

pub fn start(window: &AppWindow) {
    let Some(dir) = std::env::var_os("RUSTY_PROBE").map(PathBuf::from) else {
        return;
    };
    let saves = std::env::var_os("RUSTY_PROBE_SAVES").is_some();
    let made = std::env::var_os("RUSTY_PROBE_NEW").map(PathBuf::from);
    let playground = std::env::var_os("RUSTY_PROBE_PLAYGROUND").is_some();
    let build = std::env::var_os("RUSTY_PROBE_BUILD").is_some();
    let targets = std::env::var_os("RUSTY_PROBE_TARGETS").is_some();
    let sim = std::env::var_os("RUSTY_PROBE_SIM").is_some();
    let edits = std::env::var_os("RUSTY_PROBE_EDIT").is_some();
    let drawing = std::env::var_os("RUSTY_PROBE_DRAW").is_some();
    let waited = Rc::new(Cell::new(0u32));
    let recents_before = Rc::new(Cell::new(0usize));
    // The text before find replaced everything, to hold the undo to.
    let before: Rc<RefCell<Option<String>>> = Rc::default();
    std::fs::create_dir_all(&dir).expect("the probe's directory");
    let waiting: Rc<Cell<Option<Instant>>> = Rc::default();
    let latencies: Rc<RefCell<Vec<Duration>>> = Rc::default();
    {
        let (waiting, latencies) = (waiting.clone(), latencies.clone());
        window
            .window()
            .set_rendering_notifier(move |state, _| {
                if let RenderingState::AfterRendering = state
                    && let Some(sent) = waiting.take()
                {
                    latencies.borrow_mut().push(sent.elapsed());
                }
            })
            .expect("a renderer that reports its frames");
    }

    let weak = window.as_weak();
    let step = Rc::new(Cell::new(0u32));
    let timer = Rc::new(Timer::default());
    let keep = timer.clone();
    timer.start(TimerMode::Repeated, Duration::from_millis(40), move || {
        let _ = &keep;
        let Some(window) = weak.upgrade() else { return };
        let shell = window.global::<Shell>();
        let code = window.global::<Code>();
        let app = work::app();
        let n = step.get();
        // Wait for the file the command line named before typing into it.
        if (30..=40).contains(&n) && shell.get_active_kind() != "file" {
            return;
        }
        // The firmware is running once it or the emulator has reported a
        // pin or a sample — a wire on a rail has a level from the start, so
        // the board's colours prove nothing; a run that failed ends the wait.
        let app_heard_from_firmware = || work::app().sim_heard();
        if sim && n == 502 && shell.get_activity_state() != 3 && !app_heard_from_firmware() {
            // Two minutes at most, then on regardless, to say what is there.
            waited.set(waited.get() + 1);
            if waited.get() < 3000 {
                return;
            }
        }
        // The list is up once the scan has answered.
        if targets && (n == 402 || n == 412) && !shell.get_picker_open() {
            return;
        }
        // A build has an answer when the title bar has a verdict.
        if build && n == 302 && matches!(shell.get_activity_state(), 0 | 1) {
            return;
        }
        // And for the machine's answer before photographing the page.
        if n == 193 && shell.get_env_probing() {
            return;
        }
        step.set(n + 1);
        // A key with modifiers held: each modifier pressed first and let go
        // after, as a hand does, which is how Slint knows they are down.
        let chord = |held: &[Key], text: slint::SharedString| {
            let send = |event| window.window().dispatch_event(event);
            for key in held {
                send(WindowEvent::KeyPressed {
                    text: (*key).into(),
                });
            }
            send(WindowEvent::KeyPressed { text: text.clone() });
            send(WindowEvent::KeyReleased { text });
            for key in held.iter().rev() {
                send(WindowEvent::KeyReleased {
                    text: (*key).into(),
                });
            }
        };
        let key = |text: slint::SharedString| chord(&[], text);
        let draft = || {
            let editor = app.editor();
            editor
                .current_path()
                .and_then(|path| editor.draft_of(&path))
                .map(|(text, ..)| text)
        };
        match n {
            25 => {
                snapshot(&window, &dir.join("open.png"));
                // Nothing typed: the build writes every draft first.
                if build {
                    step.set(300);
                }
                if targets {
                    step.set(400);
                }
                if sim {
                    step.set(500);
                }
                if edits {
                    step.set(700);
                }
                if drawing {
                    step.set(800);
                }
            }
            500 if sim => app.act("run.run"),
            // An example that draws, run as its lens runs it.
            800 if drawing => {
                let plan = rusty_embed::CommandPlan::new(
                    "cargo",
                    vec!["run".into(), "--example".into(), "cross".into()],
                    "the example that draws",
                );
                app.run(&plan, app.root_path(), |_, _| {});
            }
            801 if drawing => {
                if shell.get_running() {
                    step.set(801);
                    return;
                }
                eprintln!(
                    "draw: {:?}: {:?}",
                    shell.get_draw_heading(),
                    shell
                        .get_draw_rows()
                        .iter()
                        .map(|r| format!("{} {} {}", r.kind, r.text, r.numbers))
                        .collect::<Vec<_>>()
                );
                eprintln!("draw: {} pieces", shell.get_draw_pieces().row_count());
                snapshot(&window, &dir.join("drawn.png"));
            }
            804 if drawing => {
                shell.invoke_draw_orbit(60.0, -20.0);
                app.act("draw.top");
                snapshot(&window, &dir.join("drawn-top.png"));
                app.act("app.exit");
            }
            // The board drawn, then edited as a hand would.
            700 if edits => {
                if !shell.get_right_open() || shell.get_right_front() != "board" {
                    shell.invoke_show_panel("right".into(), "board".into());
                }
                if !shell.get_board_shown() {
                    step.set(700);
                    return;
                }
                snapshot(&window, &dir.join("board.png"));
                eprintln!("edit: {}", app.board_report());
            }
            702 if edits => {
                let (x, y) = app.board_centre(1).expect("a part beside the devkit");
                let took = shell.invoke_board_down(x, y, false);
                shell.invoke_board_drag(x + 20.0, y + 12.0);
                shell.invoke_board_drag(x + 40.0, y + 24.0);
                shell.invoke_board_up(x + 40.0, y + 24.0);
                eprintln!("edit: dragged part 1 (taken {took}): {}", app.board_report());
            }
            716 if edits => {
                eprintln!("edit: saved {}", sheet_file(&work::app()));
                app.act("board.rotate");
                eprintln!("edit: turned: {}", app.board_report());
            }
            720 if edits => {
                shell.invoke_board_library_query("LED".into());
                let found: Vec<String> = shell.get_board_library().iter().map(|r| r.id.to_string()).collect();
                eprintln!("edit: the library has {} for LED: {:?}", found.len(), found.iter().take(6).collect::<Vec<_>>());
                shell.invoke_board_add("Device:LED".into());
                eprintln!("edit: added: {}", app.board_report());
            }
            724 if edits => {
                let added = app.board_last_reference();
                let from = app.board_pin_point(&added, "A");
                let to = app.board_pin_point("U1", "GPIO2");
                eprintln!("edit: wiring {added}.A {from:?} to U1.GPIO2 {to:?}");
                if let (Some(from), Some(to)) = (from, to) {
                    shell.invoke_board_down(from.0, from.1, false);
                    shell.invoke_board_drag((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
                    shell.invoke_board_drag(to.0, to.1);
                    shell.invoke_board_up(to.0, to.1);
                }
                eprintln!("edit: wired: {}", app.board_report());
                snapshot(&window, &dir.join("wired.png"));
            }
            728 if edits => {
                app.act("board.undo");
                eprintln!("edit: undone: {}", app.board_report());
                app.act("board.redo");
                eprintln!("edit: redone: {}", app.board_report());
            }
            732 if edits => {
                let last = app.board_parts() - 1;
                let (x, y) = app.board_centre(last).expect("the LED");
                shell.invoke_board_down(x, y, false);
                shell.invoke_board_up(x, y);
            }
            734 if edits => {
                eprintln!("edit: inspecting {:?} {:?}", shell.get_board_inspect(), shell.get_board_ref());
                snapshot(&window, &dir.join("inspecting.png"));
                app.act("board.delete");
                eprintln!("edit: deleted: {}", app.board_report());
            }
            750 if edits => {
                eprintln!("edit: saved {}", sheet_file(&work::app()));
                snapshot(&window, &dir.join("edited.png"));
                app.act("app.exit");
            }
            // The first switch on the board held down, as a hand would.
            560 if sim => {
                if let Some(cap) = shell.get_board_switches().iter().next() {
                    shell.invoke_board_press(cap.index, cap.key, true);
                }
            }
            570 if sim => {
                if let Some(cap) = shell.get_board_switches().iter().next() {
                    snapshot(&window, &dir.join("pressed.png"));
                    eprintln!("sim: held {}: {}", cap.index, app.sim_report());
                    shell.invoke_board_press(cap.index, cap.key, false);
                }
            }
            // What the firmware plots and lets be tuned, and one tunable set.
            574 if sim => {
                shell.invoke_show_panel("bottom".into(), "plot".into());
                if let Some(tunable) = shell.get_plot_tunables().iter().next() {
                    let asked = if tunable.ranged { tunable.max * 2.0 } else { tunable.value + 1.0 };
                    eprintln!("sim: setting {} to {asked}", tunable.name);
                    shell.invoke_plot_set(tunable.name, asked);
                }
            }
            // The first slider under a part moved a quarter of its way, as a
            // hand would.
            575 if sim => {
                let controls: Vec<_> = shell.get_board_controls().iter().collect();
                eprintln!(
                    "sim: controls {:?}",
                    controls
                        .iter()
                        .map(|c| format!("{}:{} {:?} {:?} live={}", c.index, c.key, c.label, c.text, c.live))
                        .collect::<Vec<_>>()
                );
                if let Some(control) = controls.iter().find(|c| c.slider) {
                    let to = control.min + (control.max - control.min) * 0.25;
                    eprintln!("sim: sliding {}:{} to {to}", control.index, control.key);
                    shell.invoke_board_control(control.index, control.key.clone(), to);
                }
            }
            578 if sim => {
                eprintln!(
                    "sim: controls now {:?}; texts {:?}; screens {:?}; shapes {}; lamps {}; rotors {}",
                    shell
                        .get_board_controls()
                        .iter()
                        .map(|c| format!("{}:{} {:?}", c.index, c.key, c.text))
                        .collect::<Vec<_>>(),
                    shell
                        .get_board_texts()
                        .iter()
                        .map(|w| w.text.to_string())
                        .collect::<Vec<_>>(),
                    shell
                        .get_board_screens()
                        .iter()
                        .map(|s| format!("{}x{} on={} runs={}", s.columns, s.rows, s.on, s.commands.matches('M').count()))
                        .collect::<Vec<_>>(),
                    shell.get_board_shapes().row_count(),
                    shell.get_board_lamps().row_count(),
                    shell.get_board_rotors().row_count(),
                );
                let output = shell.get_output();
                eprintln!(
                    "sim: waited {} ticks; the dock ends {:#?}",
                    waited.get(),
                    output
                        .iter()
                        .skip(output.row_count().saturating_sub(6))
                        .map(|line| line.text.to_string())
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "sim: plot {:?}; tunables {:?}",
                    shell
                        .get_plot_curves()
                        .iter()
                        .map(|c| format!("{} {} {}", c.name, c.value, c.swing))
                        .collect::<Vec<_>>(),
                    shell
                        .get_plot_tunables()
                        .iter()
                        .map(|t| format!("{}={} {}..{}", t.name, t.text, t.min, t.max))
                        .collect::<Vec<_>>()
                );
                snapshot(&window, &dir.join("simulating.png"));
                eprintln!("sim: {:?}; {}", shell.get_activity(), app.sim_report());
                eprintln!(
                    "sim: board words {:?}",
                    shell
                        .get_board_words()
                        .iter()
                        .map(|w| format!("{:?}@{:.0},{:.0}", w.text, w.x, w.y))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "sim: notes {:?}",
                    shell.get_notes().iter().map(|n| n.text).collect::<Vec<_>>()
                );
                let output = shell.get_output();
                let tail: Vec<String> = output
                    .iter()
                    .skip(output.row_count().saturating_sub(8))
                    .map(|line| line.text.to_string())
                    .collect();
                eprintln!("sim: the dock ends {tail:#?}");
            }
            // Half a second on: the LED the other way round, and the edges.
            588 if sim => {
                shell.invoke_show_panel("bottom".into(), "waves".into());
                eprintln!(
                    "sim: waves {:?} lanes {:?} ticks {}",
                    shell.get_waves_clock(),
                    shell.get_waves_lanes().iter().map(|l| format!("{} ({} steps)", l.name, l.commands.matches('V').count())).collect::<Vec<_>>(),
                    shell.get_waves_ticks().row_count()
                );
            }
            591 if sim => {
                snapshot(&window, &dir.join("simulating-2.png"));
                eprintln!("sim: then {}", app.sim_report());
                app.act("run.stop");
            }
            600 if sim => {
                eprintln!("sim: after Stop {:?} (state {})", shell.get_activity(), shell.get_activity_state());
                app.act("app.exit");
            }
            400 if targets => app.act("run.flash"),
            404 if targets => {
                snapshot(&window, &dir.join("flash-asks.png"));
                eprintln!(
                    "targets: flash with nothing chosen asks {:?}: {:?}",
                    shell.get_picker_hint(),
                    shell
                        .get_picker_rows()
                        .iter()
                        .map(|row| format!("{} — {}", row.title, row.detail))
                        .collect::<Vec<_>>()
                );
                shell.set_picker_open(false);
                app.act("run.choose-target");
            }
            414 if targets => {
                // The simulator is the first row when the part has one.
                shell.invoke_picker_pick(0);
            }
            416 if targets => {
                snapshot(&window, &dir.join("target-chosen.png"));
                eprintln!(
                    "targets: chosen {:?} ({:?}); notes {:?}",
                    shell.get_target(),
                    shell.get_target_kind(),
                    shell.get_notes().iter().map(|n| n.text).collect::<Vec<_>>()
                );
                app.act("app.exit");
            }
            300 if build => app.act("run.build"),
            303 if build => {
                snapshot(&window, &dir.join("built.png"));
                let output = shell.get_output();
                let links = output.iter().filter(|line| !line.path.is_empty()).count();
                eprintln!(
                    "build: {:?} (state {}); {} lines, {} of them places; problems: {:?}",
                    shell.get_activity(),
                    shell.get_activity_state(),
                    output.row_count(),
                    links,
                    shell
                        .get_problems()
                        .iter()
                        .map(|row| format!("{} {}", row.text, row.detail))
                        .collect::<Vec<_>>()
                );
                // The first place a line names, clicked.
                if let Some(at) = output.iter().position(|line| !line.path.is_empty()) {
                    shell.invoke_output_click(at as i32);
                }
            }
            310 if build => {
                snapshot(&window, &dir.join("place.png"));
                eprintln!(
                    "build: the click opened {:?}",
                    shell.get_active_title()
                );
                app.act("app.exit");
            }
            41 => key(Key::End.into()),
            42..=101 => {
                let text = ["f", "o", "o", "_", "b", "a", "r", " "][(n as usize) % 8];
                waiting.set(Some(Instant::now()));
                key(text.into());
            }
            102 => {
                report("typing: key to frame", &latencies.borrow());
                latencies.borrow_mut().clear();
                snapshot(&window, &dir.join("typed.png"));
            }
            // A selection, three cursors typing at once, an undo of it, a
            // line moved, a comment toggled.
            103 => chord(&[Key::Shift], Key::Home.into()),
            104 => snapshot(&window, &dir.join("selected.png")),
            // Ctrl+D three times: the word under the caret and the next two
            // like it, then typed over at once. (Not Ctrl+Alt+Down: a key
            // dispatched here carries no text-without-modifiers, and Slint
            // then reads Ctrl+Alt on Windows as AltGr.)
            105 => {
                chord(&[Key::Control], Key::Home.into());
                key(Key::DownArrow.into());
                for _ in 0..3 {
                    chord(&[Key::Control], "d".into());
                }
                key("X".into());
            }
            106 => snapshot(&window, &dir.join("cursors.png")),
            107 => {
                key(Key::Escape.into());
                chord(&[Key::Control], "z".into());
            }
            108 => snapshot(&window, &dir.join("undone.png")),
            109 => chord(&[Key::Alt], Key::DownArrow.into()),
            110 => snapshot(&window, &dir.join("moved.png")),
            111 => chord(&[Key::Control], "/".into()),
            112 => snapshot(&window, &dir.join("commented.png")),
            113 => {
                chord(&[Key::Control], "z".into());
                chord(&[Key::Control], "z".into());
            }
            114..=173 => {
                waiting.set(Some(Instant::now()));
                let row = code.get_row_height();
                code.set_scroll_y(-(row * 3.0 * (n - 113) as f32));
            }
            174 => {
                report("scrolling: step to frame", &latencies.borrow());
                app.act("dock.bottom");
                app.act("dock.right");
            }
            177 => snapshot(&window, &dir.join("docks.png")),
            178 => app.act("palette.open"),
            181 => snapshot(&window, &dir.join("palette.png")),
            182 => {
                shell.set_picker_open(false);
                app.act("settings.open");
                app.act("theme.light");
            }
            185 => snapshot(&window, &dir.join("light.png")),
            186 => {
                app.act("theme.system");
                app.act("dock.bottom");
                app.act("dock.right");
                app.act("welcome.open");
            }
            189 => snapshot(&window, &dir.join("welcome.png")),
            // The machine against the project, once it has been asked.
            190 => app.act("environment.open"),
            194 => {
                snapshot(&window, &dir.join("environment.png"));
                eprintln!(
                    "environment: {} ({} rows); target: {}; chip: {}",
                    shell.get_env_verdict(),
                    shell
                        .get_env_rows()
                        .iter()
                        .filter(|row| !row.header)
                        .count(),
                    shell.get_target(),
                    shell.get_chip(),
                );
            }
            // New project, step by step, as far as the button and no
            // further: a name cargo would refuse, then one it takes.
            195 => app.act("project.new"),
            197 => snapshot(&window, &dir.join("wizard-part.png")),
            198 => shell.invoke_wiz_go(1),
            200 => snapshot(&window, &dir.join("wizard-runtime.png")),
            201 => shell.invoke_wiz_go(2),
            203 => {
                if let Some(option) = shell.get_wiz_rows().iter().find(|row| row.check) {
                    shell.invoke_wiz_pick(option.id);
                }
            }
            205 => snapshot(&window, &dir.join("wizard-options.png")),
            206 => {
                shell.invoke_wiz_go(3);
                shell.invoke_wiz_name_edited("2fast".into());
            }
            208 => {
                snapshot(&window, &dir.join("wizard-name.png"));
                eprintln!(
                    "wizard: `2fast` refused: {:?}; create offered: {}",
                    shell.get_wiz_name_problem(),
                    shell.get_wiz_can_create()
                );
                shell.invoke_wiz_name_edited("blinky".into());
            }
            210 => {
                snapshot(&window, &dir.join("wizard-review.png"));
                eprintln!(
                    "wizard: `blinky` → {:?}; {} notes; missing {:?}; refusal {:?}; create offered: {}",
                    shell.get_wiz_command(),
                    shell.get_wiz_notes().row_count(),
                    shell.get_wiz_missing(),
                    shell.get_wiz_refusal(),
                    shell.get_wiz_can_create()
                );
            }
            // A part whose options step says rusty writes the project
            // itself, found by asking each row rather than by name.
            211 if made.is_some() => {
                let ids: Vec<_> = {
                    shell.invoke_wiz_go(0);
                    shell.get_wiz_rows().iter().map(|row| row.id).collect()
                };
                let template = ids.into_iter().find(|id| {
                    shell.invoke_wiz_go(0);
                    shell.invoke_wiz_pick(id.clone());
                    shell.invoke_wiz_go(2);
                    shell.get_wiz_list_note() != ""
                });
                eprintln!("wizard: a template part found: {}", template.is_some());
                shell.invoke_wiz_go(3);
                shell.invoke_wiz_name_edited("probemade".into());
                if let Some(parent) = made.clone() {
                    app.create_project_in(parent);
                }
            }
            211 if playground => {
                recents_before.set(shell.get_recents().row_count());
                if let Some(card) = shell.get_playgrounds().iter().next() {
                    shell.invoke_open_playground(card.id);
                }
            }
            240 if playground => {
                snapshot(&window, &dir.join("playground.png"));
                eprintln!(
                    "playground: {:?} open as {:?}, on {:?}; recents {} before, {} after",
                    shell.get_playground(),
                    shell.get_project(),
                    shell.get_active_title(),
                    recents_before.get(),
                    shell.get_recents().row_count()
                );
                app.act("app.exit");
            }
            240 if made.is_some() => {
                snapshot(&window, &dir.join("created.png"));
                eprintln!(
                    "wizard: opened {:?}, chip {:?}, target {:?}, {} entries in the tree",
                    shell.get_project(),
                    shell.get_chip(),
                    shell.get_target(),
                    shell.get_tree().row_count()
                );
                app.act("app.exit");
            }
            // Back to the file. (With RUSTY_PROBE_SAVES, the save through
            // rustfmt comes last: it replaces the whole text.)
            211 => {
                let file = shell.get_tabs().iter().find(|tab| tab.kind == "file");
                if let Some(tab) = file {
                    shell.invoke_pick_tab(tab.id);
                }
            }
            2161 if saves => {
                chord(&[Key::Control], "a".into());
                code.invoke_typed("fn  main( ){let x=1;}\n".into());
                chord(&[Key::Control], "s".into());
            }
            // rustfmt has a second; the write follows it.
            2205 if saves => {
                snapshot(&window, &dir.join("saved.png"));
                let mut args = std::env::args().skip(1);
                if let (Some(root), Some(file)) = (args.next(), args.next()) {
                    let disk = std::fs::read_to_string(Path::new(&root).join(&file))
                        .unwrap_or_default()
                        .replace("\r\n", "\n");
                    let formatted = disk == "fn main() {\n    let x = 1;\n}\n";
                    eprintln!(
                        "save: {} on disk",
                        if formatted {
                            "formatted"
                        } else {
                            "NOT formatted"
                        }
                    );
                }
                code.invoke_typed("// kept for next time\n".into());
            }
            // Find: opened on the word at the top, a query typed, a step,
            // every `fn` replaced, the replacement undone in one step.
            240 => {
                chord(&[Key::Control], Key::Home.into());
                before.replace(draft());
                chord(&[Key::Control], "f".into());
            }
            241 => {
                for letter in ["f", "n"] {
                    key(letter.into());
                }
            }
            242 => key(Key::Return.into()),
            244 => snapshot(&window, &dir.join("find.png")),
            245 => {
                chord(&[Key::Control], "h".into());
                for letter in ["F", "N"] {
                    key(letter.into());
                }
                window.global::<Find>().invoke_replace(true);
            }
            247 => snapshot(&window, &dir.join("replaced.png")),
            248 => {
                key(Key::Escape.into());
                chord(&[Key::Control], "z".into());
            }
            249 => {
                let undone = before.borrow().as_deref() == draft().as_deref();
                eprintln!(
                    "find: replace all undone in one step: {}",
                    if undone { "yes" } else { "NO" }
                );
                // Go to line 3, from the finder's `:`.
                app.act("editor.go-to-line");
            }
            250 => {
                key("3".into());
                key(Key::Return.into());
            }
            251 => {
                let caret = app.editor().status().map(|(line, ..)| line);
                eprintln!("go to line 3: the caret is on line {caret:?}");
            }
            // What the language server found: waited for, up to a minute,
            // then the Problems panel photographed. A project with nothing
            // wrong in it waits the whole minute and says so.
            // Only in the ordinary run: a probe of its own owns these numbers.
            252..=1750 if !(build || targets || sim || edits || drawing) => {
                if shell.get_errors() + shell.get_warnings() > 0 || n == 1750 {
                    eprintln!(
                        "language server: {:?}, {} errors, {} warnings after {:.1} s",
                        shell.get_server().as_str(),
                        shell.get_errors(),
                        shell.get_warnings(),
                        f64::from(n - 252) * 0.04
                    );
                    app.act("problems.show");
                    step.set(1760);
                }
            }
            1762 => {
                chord(&[Key::Control], Key::Home.into());
                app.act("problem.next");
            }
            1765 => snapshot(&window, &dir.join("problems.png")),
            // The hover card: on the problem F8 went to, and then on the
            // second word of the line below the first, whatever it is.
            1766 => app.act("editor.show-hover"),
            1790 => snapshot(&window, &dir.join("hover-problem.png")),
            1791 => {
                app.editor().go_to(15, Some(21));
                app.act("editor.show-hover");
            }
            1830 => snapshot(&window, &dir.join("hover.png")),
            // And on a type with documentation: the struct named on the
            // twelfth line, its doc comment rendered as Markdown.
            1831 => {
                app.editor().hide_hover();
                app.editor().go_to(12, Some(14));
                app.act("editor.show-hover");
            }
            1870 => snapshot(&window, &dir.join("hover-docs.png")),
            // Completion: a field asked for after `g.` on a line of its
            // own, narrowed by a letter and taken with Enter; then a call
            // opened, and what it takes shown above it.
            1871 => {
                app.editor().hide_hover();
                app.editor().go_to(14, Some(999));
                key(Key::Return.into());
                key("g".into());
                key(".".into());
            }
            1920 => snapshot(&window, &dir.join("completion.png")),
            1921 => key("k".into()),
            1940 => {
                key(Key::Return.into());
                let line = app.editor().status().map(|(line, ..)| line);
                let text = draft().unwrap_or_default();
                let typed = line
                    .and_then(|line| text.lines().nth(line - 1))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                eprintln!("completion: the line reads {typed:?}");
                key(Key::Return.into());
                for letter in ["s", "c", "a", "l", "e", "("] {
                    key(letter.into());
                }
            }
            1990 => snapshot(&window, &dir.join("signature.png")),
            // Navigation: F12 from a use of a name to where it is declared,
            // Alt+Left back; Shift+F12 for the references to a function;
            // Ctrl+. on the problem F8 finds first.
            1991 => {
                key(Key::Escape.into());
                app.editor().go_to(8, Some(12));
                app.act("editor.definition");
            }
            2030 => {
                let at = |app: &crate::app::App| app.editor().status().map(|(l, c, ..)| (l, c));
                eprintln!("definition: from 8:12 to {:?}", at(&app));
                app.act("nav.back");
                eprintln!("back: at {:?}", at(&app));
                app.editor().go_to(7, Some(5));
                app.act("editor.references");
            }
            2070 => snapshot(&window, &dir.join("references.png")),
            2071 => {
                shell.set_picker_open(false);
                shell.invoke_picker_pick(-1);
                chord(&[Key::Control], Key::Home.into());
                app.act("problem.next");
                app.act("editor.quick-fix");
            }
            2110 => snapshot(&window, &dir.join("fixes.png")),
            2111 => {
                shell.set_picker_open(false);
                shell.invoke_picker_pick(-1);
                if saves {
                    app.editor().go_to(2, Some(9));
                    app.act("editor.rename");
                }
            }
            2115 if saves => {
                for letter in ["P", "i", "d"] {
                    key(letter.into());
                }
            }
            2120 if saves => snapshot(&window, &dir.join("rename.png")),
            2121 if saves => key(Key::Return.into()),
            2159 if saves => snapshot(&window, &dir.join("renamed.png")),
            2160 if saves => {
                let text = draft().unwrap_or_default();
                eprintln!(
                    "rename: `struct Pid` {} and `Gain` {}",
                    if text.contains("struct Pid") {
                        "is there"
                    } else {
                        "is NOT there"
                    },
                    if text.contains("Gain") {
                        "is still there"
                    } else {
                        "is gone"
                    }
                );
            }
            2210 => {
                app.act("app.exit");
                let kept = rusty_embed::config::data_dir()
                    .and_then(|data| std::fs::read_dir(data.join("backups")).ok())
                    .map_or(0, |entries| entries.count());
                eprintln!("hot exit: {kept} drafts kept");
            }
            _ => {}
        }
    });
    std::mem::forget(timer);
}

fn report(what: &str, samples: &[Duration]) {
    if samples.is_empty() {
        eprintln!("{what}: no frames");
        return;
    }
    let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    let at = |q: f64| ms[((ms.len() - 1) as f64 * q) as usize];
    eprintln!(
        "{what}: n={} median {:.1} ms, p90 {:.1} ms, max {:.1} ms",
        ms.len(),
        at(0.5),
        at(0.9),
        ms[ms.len() - 1]
    );
}

fn snapshot(window: &AppWindow, path: &Path) {
    match window.window().take_snapshot() {
        Ok(buffer) => {
            let (width, height) = (buffer.width(), buffer.height());
            let bytes = buffer.as_bytes().to_vec();
            match image::RgbaImage::from_raw(width, height, bytes) {
                Some(picture) => {
                    picture.save(path).expect("the snapshot is written");
                    eprintln!("snapshot: {}", path.display());
                }
                None => eprintln!("snapshot: a buffer of the wrong size"),
            }
        }
        Err(error) => eprintln!("snapshot failed: {error}"),
    }
}
