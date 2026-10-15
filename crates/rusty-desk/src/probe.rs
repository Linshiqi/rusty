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
//! deletes the LED — and prints the sheet file after each save. With
//! `RUSTY_PROBE_STRIP` and no project named it waits for the last project
//! to come back, prints its tabs as the strips show them, and opens its
//! README before leaving. With `RUSTY_PROBE_NAV`, on a file with `let c =`,
//! a `println!`, a `Vector::new` and a `c.norm()` in it (`draw-vectors`'
//! `examples/cross.rs`), it waits for the server, goes to the type of `c`
//! and back, expands the `println!` beside the code (twice: one tab), opens
//! the call hierarchy of `new` with `cross` opened under it, turns it to
//! calls out, and opens the calls out of `norm` with `dot` under it —
//! photographing each, and printing what the strip keeps.
//! With `RUSTY_PROBE_MARKS` it puts the caret beside
//! a brace and photographs the pair outlined, waits for the language server
//! and photographs a name's occurrences washed, then writes a struct
//! literal missing two fields, opens the hover card on it, photographs the
//! fixes the card lists, applies one by its click and prints the lines it
//! wrote — on a copy, since it ends by keeping the draft.
//! With `RUSTY_PROBE_TORN` it moves the file into a window of its own
//! (Move into New Window), types there, opens the same file in the main
//! window and photographs both with the edit in each, types in the main
//! window and undoes it from the other (one history), saves from the
//! window of its own, closes it and types in the main window again; then
//! drags `Cargo.toml`'s tab out beyond the main window's edge and moves it
//! back; then moves a new file there from its tab's menu, renames it in the
//! tree and deletes it there — on a copy, since it saves and deletes.
//! With `RUSTY_PROBE_TREE`, on a copy of a project with a `src/`, it works
//! the file tree by its pointer and its keys: a file made in `src` from the
//! folder's menu (a taken name refused under the box first), a folder made
//! from the root's menu below the rows, a rename, Ctrl+C and Ctrl+V beside
//! the original, an edited file dragged into the new folder — its tab and
//! draft going with it — a cut pasted, a delete to the recycle bin, the
//! arrows and F2 — photographing each and printing the tree as drawn.
//! With `RUSTY_PROBE_TABS`, on a project with a `book/` of files, it opens
//! every one until the strip overflows, wheels the strip back to its start
//! and brings the tab in front into view again, splits, drags the divider
//! and double-clicks it, right-clicks a tab on each strip and uses a few of
//! its menu's rows — Split right, Copy path, the closes — and clicks the
//! logo, photographing each; `RUSTY_PROBE_REVEAL` with it also reveals a
//! file in the file manager, which opens a window of the system's.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use slint::{
    ComponentHandle, LogicalPosition, Model, RenderingState, Timer, TimerMode,
    platform::{Key, PointerEventButton, WindowEvent},
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
    let debugging = std::env::var_os("RUSTY_PROBE_DEBUG").is_some();
    let terminal = std::env::var_os("RUSTY_PROBE_TERM").is_some();
    let repository = std::env::var_os("RUSTY_PROBE_GIT").is_some();
    let measuring = std::env::var_os("RUSTY_PROBE_MEMORY").is_some();
    let dependencies = std::env::var_os("RUSTY_PROBE_CRATES").is_some();
    let asking = std::env::var_os("RUSTY_PROBE_AI").is_some();
    let reading = std::env::var_os("RUSTY_PROBE_MD").is_some();
    let sums = std::env::var_os("RUSTY_PROBE_MATH").is_some();
    let cloning = std::env::var_os("RUSTY_PROBE_CLONE").map(PathBuf::from);
    let following = std::env::var_os("RUSTY_PROBE_WATCH").is_some();
    let lenses = std::env::var_os("RUSTY_PROBE_LENS").is_some();
    let folding = std::env::var_os("RUSTY_PROBE_FOLD").is_some();
    let hinting = std::env::var_os("RUSTY_PROBE_HINTS").is_some();
    let modal = std::env::var_os("RUSTY_PROBE_VIM").is_some();
    let splitting = std::env::var_os("RUSTY_PROBE_SPLIT").is_some();
    let navigating = std::env::var_os("RUSTY_PROBE_NAV").is_some();
    let updating = std::env::var_os("RUSTY_PROBE_UPDATE").is_some();
    let marking = std::env::var_os("RUSTY_PROBE_MARKS").is_some();
    let tearing = std::env::var_os("RUSTY_PROBE_TORN").is_some();
    let exploring = std::env::var_os("RUSTY_PROBE_TREE").is_some();
    // The line the marks probe is looking at.
    let spot = Rc::new(Cell::new(0usize));
    // Started with no project: the last one comes back with its strip.
    let strip = std::env::var_os("RUSTY_PROBE_STRIP").is_some();
    let tabbing = std::env::var_os("RUSTY_PROBE_TABS").is_some();
    let revealing = std::env::var_os("RUSTY_PROBE_REVEAL").is_some();
    // How many tabs the strip is to hold once every file has opened.
    let tab_goal = Rc::new(Cell::new(0usize));
    let waited = Rc::new(Cell::new(0u32));
    let recents_before = Rc::new(Cell::new(0usize));
    // The text before find replaced everything, to hold the undo to.
    let before: Rc<RefCell<Option<String>>> = Rc::default();
    // The file a jump left, and then the expansion opened beside it.
    let start_path: Rc<RefCell<String>> = Rc::default();
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
        // Where `needle` first is in the file on screen, `offset` characters
        // into it: a line and a column from one.
        let find = |needle: &str, offset: usize| -> Option<(usize, usize)> {
            let text = draft()?;
            text.lines().enumerate().find_map(|(index, line)| {
                let at = line.find(needle)?;
                Some((index + 1, line[..at].chars().count() + offset + 1))
            })
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
                if debugging {
                    step.set(900);
                }
                if terminal {
                    step.set(1000);
                }
                if repository {
                    step.set(1100);
                }
                if measuring {
                    step.set(1200);
                }
                if dependencies {
                    step.set(1300);
                }
                if asking {
                    step.set(1400);
                }
                if reading {
                    step.set(1500);
                }
                if sums {
                    step.set(1600);
                }
                if cloning.is_some() {
                    step.set(1800);
                }
                if following {
                    step.set(1900);
                }
                if lenses {
                    step.set(2300);
                }
                if folding {
                    step.set(2400);
                }
                if hinting {
                    step.set(2500);
                }
                if modal {
                    step.set(2600);
                }
                if splitting {
                    step.set(2700);
                }
                if navigating {
                    step.set(3000);
                }
                if updating {
                    step.set(2800);
                }
                if strip {
                    step.set(2900);
                }
                if tabbing {
                    step.set(3700);
                }
                if marking {
                    step.set(3000);
                }
                if tearing {
                    step.set(3600);
                }
                if exploring {
                    step.set(4100);
                }
            }
            500 if sim => app.act("run.run"),
            // A breakpoint beside the line that toggles the LED, then F5: the
            // firmware boots frozen, gdb attaches, and it stops there.
            900 if debugging => {
                let text = draft().unwrap_or_default();
                let line = text
                    .lines()
                    .position(|l| l.contains("toggle()") || l.contains("set_high()") || l.contains("set_level("))
                    .unwrap_or(0);
                eprintln!("debug: a breakpoint on line {}: {:?}", line + 1, text.lines().nth(line));
                code.invoke_toggle_breakpoint(line as i32);
                app.act("run.debug");
            }
            901 if debugging => {
                // Stopped in the firmware's own source — not at the reset the
                // target is frozen at when gdb attaches — and a moment later,
                // when the frame's variables have come in.
                let at_source = shell
                    .get_debug_frames()
                    .iter()
                    .next()
                    .is_some_and(|f| f.place.contains("main.rs"));
                if !(shell.get_debug_stopped() && at_source) {
                    waited.set(waited.get() + 1);
                    if waited.get() < 4000 {
                        step.set(901);
                    }
                }
            }
            902 if debugging => {}
            903 if debugging => {
                shell.invoke_show_panel("bottom".into(), "registers".into());
            }
            908 if debugging => {
                eprintln!(
                    "debug: registers loaded {} svd {} rows {}",
                    shell.get_registers_loaded(),
                    shell.get_registers_svd(),
                    shell.get_registers_rows().row_count()
                );
                snapshot(&window, &dir.join("debug-registers.png"));
                shell.invoke_show_panel("bottom".into(), "debug".into());
                eprintln!(
                    "debug: after {} ticks {:?}; frames {:?}; variables {:?}; error {:?}",
                    waited.get(),
                    shell.get_debug_status(),
                    shell.get_debug_frames().iter().map(|f| format!("{} {}", f.function, f.place)).collect::<Vec<_>>(),
                    shell.get_debug_variables().iter().map(|v| format!("{}={}", v.name, v.value)).collect::<Vec<_>>(),
                    shell.get_debug_error()
                );
                snapshot(&window, &dir.join("debug-stopped.png"));
                app.act("debug.over");
            }
            909 if debugging => {
                if !shell.get_debug_stopped() {
                    step.set(909);
                    return;
                }
                eprintln!(
                    "debug: stepped {:?}; frames {:?}",
                    shell.get_debug_status(),
                    shell.get_debug_frames().iter().take(2).map(|f| format!("{} {}", f.function, f.place)).collect::<Vec<_>>()
                );
                snapshot(&window, &dir.join("debug-stepped.png"));
                app.act("run.stop");
            }
            915 if debugging => {
                eprintln!("debug: after stop live={} status {:?}", shell.get_debug_live(), shell.get_debug_status());
                app.act("app.exit");
            }
            // The Terminal: a shell comes up, a command typed into it runs,
            // and what it printed is on the screen.
            1000 if terminal => {
                shell.invoke_show_panel("bottom".into(), "terminal".into());
            }
            1001 if terminal => {
                // The prompt is up once the screen has any text on it.
                if screen_text(&shell).trim().is_empty() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 500 {
                        step.set(1001);
                    }
                    return;
                }
                eprintln!("terminal: up after {} ticks", waited.get());
                for c in "echo rusty-term-$((6*7))".chars() {
                    key(c.to_string().into());
                }
                key(Key::Return.into());
                waited.set(0);
            }
            1002 if terminal => {
                if !screen_text(&shell).contains("rusty-term-42") {
                    waited.set(waited.get() + 1);
                    if waited.get() < 500 {
                        step.set(1002);
                    }
                    return;
                }
                eprintln!("terminal: answered after {} ticks", waited.get());
            }
            1010 if terminal => {
                let text = screen_text(&shell);
                eprintln!(
                    "terminal: {} rows, cursor {}:{}, exited {:?}\n{}",
                    shell.get_term_rows().row_count(),
                    shell.get_term_cursor_row(),
                    shell.get_term_cursor_col(),
                    shell.get_term_exited(),
                    text.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n")
                );
                snapshot(&window, &dir.join("terminal.png"));
                // The shell asked to leave: the panel says so and offers another.
                for c in "exit".chars() {
                    key(c.to_string().into());
                }
                key(Key::Return.into());
            }
            1040 if terminal => {
                eprintln!("terminal: after exit {:?}", shell.get_term_exited());
                snapshot(&window, &dir.join("terminal-exited.png"));
                app.act("app.exit");
            }
            // The Git panel on a repository with a merge, a tag, a change and
            // a new file: stage, commit, the history, a commit opened, its
            // menu, a new branch through the name field, and a file's diff.
            1100 if repository => {
                shell.invoke_show_panel("left".into(), "git".into());
            }
            1101 if repository => {
                if shell.get_git_state() != 1 || shell.get_git_unstaged().row_count() == 0 {
                    waited.set(waited.get() + 1);
                    if waited.get() < 500 {
                        step.set(1101);
                    }
                    return;
                }
                let names = |model: slint::ModelRc<crate::GitChange>| {
                    model
                        .iter()
                        .map(|c| format!("{} {}", c.letter, c.path))
                        .collect::<Vec<_>>()
                };
                eprintln!(
                    "git: head {:?} ({}); staged {:?}; unstaged {:?}",
                    shell.get_git_head(),
                    shell.get_git_upstream(),
                    names(shell.get_git_staged()),
                    names(shell.get_git_unstaged())
                );
                snapshot(&window, &dir.join("git-panel.png"));
                shell.invoke_git_file("stage".into(), "src/lib.rs".into(), false);
                waited.set(0);
            }
            1102 if repository => {
                if shell.get_git_staged().row_count() == 0 {
                    waited.set(waited.get() + 1);
                    if waited.get() < 200 {
                        step.set(1102);
                    }
                    return;
                }
                eprintln!("git: staged {}", shell.get_git_staged().row_count());
                shell.invoke_git_file("diff".into(), "src/lib.rs".into(), true);
            }
            1110 if repository => {
                eprintln!(
                    "git: diff {:?}, {} rows, note {:?}",
                    shell.get_git_diff_title(),
                    shell.get_git_diff_rows().row_count(),
                    shell.get_git_diff_note()
                );
                snapshot(&window, &dir.join("git-diff.png"));
                shell.set_git_message("Probe: a line\n\nand a body".into());
                app.act("git.commit");
                waited.set(0);
            }
            1111 if repository => {
                if shell.get_git_staged().row_count() != 0 {
                    waited.set(waited.get() + 1);
                    if waited.get() < 500 {
                        step.set(1111);
                    }
                    return;
                }
                eprintln!("git: committed; message box {:?}", shell.get_git_message());
                app.act("git.history");
                waited.set(0);
            }
            1112 if repository => {
                if shell.get_git_log().row_count() < 5 {
                    waited.set(waited.get() + 1);
                    if waited.get() < 500 {
                        step.set(1112);
                    }
                    return;
                }
                let log = shell.get_git_log();
                for row in log.iter() {
                    eprintln!(
                        "git: lane {} {} {:?} labels {:?} edges {} above {}",
                        row.lane,
                        row.short,
                        row.summary,
                        row.labels.iter().map(|l| l.text.to_string()).collect::<Vec<_>>(),
                        row.edges.row_count(),
                        row.above.row_count()
                    );
                }
                eprintln!(
                    "git: refs {:?}",
                    shell
                        .get_git_refs()
                        .iter()
                        .map(|r| format!("{}:{}", r.kind, r.label))
                        .collect::<Vec<_>>()
                );
                shell.invoke_git_pick(1);
            }
            1120 if repository => {
                eprintln!(
                    "git: opened {:?} / {:?}; {} files, {} rows",
                    shell.get_git_detail_summary(),
                    shell.get_git_detail_meta(),
                    shell.get_git_detail_files().row_count(),
                    shell.get_git_detail_rows().row_count()
                );
                snapshot(&window, &dir.join("git-history.png"));
                shell.invoke_git_commit_menu(2, 600.0, 300.0);
            }
            1124 if repository => {
                eprintln!(
                    "git: menu {:?}",
                    shell
                        .get_menu_items()
                        .iter()
                        .map(|m| m.label.to_string())
                        .collect::<Vec<_>>()
                );
                snapshot(&window, &dir.join("git-menu.png"));
                shell.invoke_menu_picked("branch-here".into());
            }
            1128 if repository => {
                shell.set_git_prompt_value("bad name".into());
                shell.invoke_git_prompt_edited();
            }
            1130 if repository => {
                eprintln!(
                    "git: prompt {:?} problem {:?} ok {}",
                    shell.get_git_prompt_title(),
                    shell.get_git_prompt_problem(),
                    shell.get_git_prompt_ok()
                );
                snapshot(&window, &dir.join("git-prompt.png"));
                shell.set_git_prompt_value("probe/branch".into());
                shell.invoke_git_prompt_edited();
                shell.invoke_git_prompt_done(true);
                waited.set(0);
            }
            1131 if repository => {
                let made = shell.get_git_refs().iter().any(|r| r.label == "probe/branch");
                if !made {
                    waited.set(waited.get() + 1);
                    if waited.get() < 500 {
                        step.set(1131);
                    }
                    return;
                }
                eprintln!("git: branch made; head {:?}", shell.get_git_head());
                shell.invoke_git_search("change".into());
                shell.invoke_git_search_step(true);
            }
            1136 if repository => {
                eprintln!("git: search {:?}, selected {}", shell.get_git_search_count(), shell.get_git_selected());
                snapshot(&window, &dir.join("git-branch.png"));
                app.act("app.exit");
            }
            // The Memory page on a project that has been built: the builds,
            // the two readouts, the crates and the sections.
            1200 if measuring => app.act("memory.open"),
            1201 if measuring => {
                if !shell.get_mem_ready() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 500 {
                        step.set(1201);
                    }
                    return;
                }
                eprintln!(
                    "memory: builds {:?}",
                    shell
                        .get_mem_builds()
                        .iter()
                        .map(|b| format!("{} {} {}{}", b.name, b.profile, b.size, if b.chosen { " *" } else { "" }))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "memory: flash {} {} ({}), ram {} {} ({}), unattributed {}",
                    shell.get_mem_flash_value(),
                    shell.get_mem_flash_unit(),
                    shell.get_mem_flash_hint(),
                    shell.get_mem_ram_value(),
                    shell.get_mem_ram_unit(),
                    shell.get_mem_ram_hint(),
                    shell.get_mem_unattributed()
                );
                for c in shell.get_mem_crates().iter().take(5) {
                    eprintln!("memory: crate {} {} ({:.2})", c.name, c.total, c.bar);
                }
                eprintln!("memory: {} sections, more {:?}", shell.get_mem_sections().row_count(), shell.get_mem_more());
                snapshot(&window, &dir.join("memory.png"));
                app.act("app.exit");
            }
            // The Crates page: the graph resolved, a feature flipped against
            // it, and the build directory measured.
            1300 if dependencies => app.act("crates.open"),
            1301 if dependencies => {
                if shell.get_crates_state() != 2 || !shell.get_disk_ready() || !shell.get_feat_ready() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 3000 {
                        step.set(1301);
                    }
                    return;
                }
                eprintln!(
                    "crates: {:?}; vitals {:?}",
                    shell.get_crates_title(),
                    shell
                        .get_crates_vitals()
                        .iter()
                        .map(|v| format!("{} {}", v.label, v.value))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "crates: packages {:?}; {} features; resolved {} ({})",
                    shell.get_feat_packages().iter().map(|p| p.name.to_string()).collect::<Vec<_>>(),
                    shell.get_feat_rows().row_count(),
                    shell.get_feat_resolved(),
                    shell.get_feat_baseline()
                );
                eprintln!(
                    "crates: disk {:?} {:?}; trees {:?}",
                    shell.get_disk_total(),
                    shell.get_disk_volume(),
                    shell
                        .get_disk_trees()
                        .iter()
                        .map(|t| format!("{} {} stale {}", t.label, t.size, t.stale))
                        .collect::<Vec<_>>()
                );
                snapshot(&window, &dir.join("crates.png"));
                let flip = shell.get_feat_rows().iter().find(|f| !f.enabled).map(|f| f.name.to_string());
                if let Some(flip) = flip {
                    eprintln!("crates: flipping {flip}");
                    shell.invoke_feat_toggle(flip.into());
                }
                waited.set(0);
            }
            1340 if dependencies => {
                eprintln!(
                    "crates: after the flip resolved {}, against defaults {}, pulled in {:?}",
                    shell.get_feat_resolved(),
                    shell.get_feat_delta(),
                    shell.get_feat_added()
                );
                snapshot(&window, &dir.join("crates-flipped.png"));
                app.act("app.exit");
            }
            // The assistant against a loopback model: a question, a tool
            // call, a Markdown answer, the meter; then Settings.
            1400 if asking => shell.invoke_show_panel("right".into(), "assistant".into()),
            1402 if asking => {
                eprintln!("ai: ready {} title {:?} attach {:?}", shell.get_ai_ready(), shell.get_ai_title(), shell.get_ai_attach());
                shell.set_ai_draft("What files are in this project?".into());
                app.act("ai.ask");
                waited.set(0);
            }
            1403 if asking => {
                if shell.get_ai_running() || shell.get_ai_entries().row_count() < 2 {
                    waited.set(waited.get() + 1);
                    if waited.get() < 1000 {
                        step.set(1403);
                    }
                    return;
                }
                for entry in shell.get_ai_entries().iter() {
                    eprintln!(
                        "ai: entry {} {:?} {:?} state {} blocks {}",
                        entry.kind,
                        entry.text.chars().take(60).collect::<String>(),
                        entry.attached.chars().take(60).collect::<String>(),
                        entry.state,
                        entry.blocks.row_count()
                    );
                    for block in entry.blocks.iter() {
                        eprintln!("ai:   block {} level {} depth {} marker {:?} lines {} rows {}", block.kind, block.level, block.depth, block.marker, block.lines.row_count(), block.rows.row_count());
                    }
                }
                eprintln!("ai: spend {:?} ({:?})", shell.get_ai_spend(), shell.get_ai_spend_tip());
                snapshot(&window, &dir.join("assistant.png"));
                app.act("settings.open");
            }
            1410 if asking => {
                eprintln!(
                    "ai: settings {:?}; presets {}; model {:?} at {:?}",
                    shell.get_ai_current(),
                    shell.get_ai_presets().row_count(),
                    shell.get_ai_model(),
                    shell.get_ai_base_url()
                );
                app.act("ai.check");
            }
            1440 if asking => {
                eprintln!("ai: check {:?} ok {}", shell.get_ai_check(), shell.get_ai_check_ok());
                snapshot(&window, &dir.join("assistant-settings.png"));
                app.act("app.exit");
            }
            // A Markdown file opened on the command line: the page, then its
            // source and back.
            1500 if reading => {
                eprintln!(
                    "md: file {} page {}; {} blocks {:?}",
                    shell.get_md_file(),
                    shell.get_md_page(),
                    shell.get_md_blocks().row_count(),
                    shell
                        .get_md_blocks()
                        .iter()
                        .map(|b| format!("{}{}", b.kind, if b.kind == 5 { format!(" shown {} {:?}", b.shown, b.marker) } else { String::new() }))
                        .collect::<Vec<_>>()
                );
                snapshot(&window, &dir.join("markdown.png"));
                app.act("markdown.toggle");
            }
            1503 if reading => {
                eprintln!("md: after the toggle page {}", shell.get_md_page());
                snapshot(&window, &dir.join("markdown-source.png"));
                app.act("app.exit");
            }
            // The math toolbox: the Euler example's rows, the attitude row's
            // working played, a row retyped, and the reference.
            1600 if sums => app.act("math.open"),
            1603 if sums => {
                for row in shell.get_math_rows().iter() {
                    eprintln!("math: {:?} -> {:?} {:?} slider {}", row.text, row.value, row.problem, row.slider);
                }
                shell.invoke_math_select(3);
            }
            1605 if sums => {
                eprintln!(
                    "math: steps {:?}; turns {}; value kind {} lines {:?}; nose {:.1} bank {:.1}; {} pieces",
                    shell.get_math_steps().iter().map(|s| s.title.to_string()).collect::<Vec<_>>(),
                    shell.get_math_turns(),
                    shell.get_math_value_kind(),
                    shell.get_math_value_lines().iter().map(|l| format!("{} {}", l.label, l.value)).collect::<Vec<_>>(),
                    shell.get_math_nose(),
                    shell.get_math_bank(),
                    shell.get_math_pieces().row_count()
                );
                app.act("math.play");
            }
            1630 if sums => {
                eprintln!("math: playing {} at {:.2}", shell.get_math_playing(), shell.get_math_progress());
                snapshot(&window, &dir.join("math-playing.png"));
            }
            1700 if sums => {
                eprintln!("math: played {} at {:.2}", shell.get_math_playing(), shell.get_math_progress());
                shell.invoke_math_row(0, "roll = 45°".into());
                shell.invoke_math_row(1, "pitch = 1.2.3".into());
            }
            1702 if sums => {
                for row in shell.get_math_rows().iter().take(4) {
                    eprintln!("math: after {:?} -> {:?} {:?}", row.text, row.value, row.problem);
                }
                snapshot(&window, &dir.join("math.png"));
                app.act("math.help");
            }
            1704 if sums => {
                eprintln!("math: reference {} lines", shell.get_math_help_rows().row_count());
                snapshot(&window, &dir.join("math-help.png"));
                app.act("app.exit");
            }
            // Clone a repository named by RUSTY_PROBE_CLONE into the probe's
            // directory, and the checkout opened.
            1800 if cloning.is_some() => {
                app.act("project.clone");
                let source = cloning.clone().unwrap_or_default();
                shell.set_clone_url(source.to_string_lossy().replace('\\', "/").as_str().into());
                shell.set_clone_into(dir.join("clones").to_string_lossy().as_ref().into());
                let _ = std::fs::create_dir_all(dir.join("clones"));
                shell.invoke_clone_edited();
            }
            1802 if cloning.is_some() => {
                eprintln!("clone: {:?} ready {}", shell.get_clone_creates(), shell.get_clone_ready());
                snapshot(&window, &dir.join("clone.png"));
                app.act("clone.run");
                waited.set(0);
            }
            1803 if cloning.is_some() => {
                let opened = app.root_path().is_some_and(|r| r.starts_with(dir.join("clones")));
                if !opened {
                    waited.set(waited.get() + 1);
                    if waited.get() < 1000 {
                        step.set(1803);
                    }
                    return;
                }
                eprintln!("clone: opened {:?}", app.root_path());
                app.act("app.exit");
            }
            // The disk moved under the window: the open file rewritten and a
            // new one made by somebody else, both seen.
            1900 if following => {
                if let Some(root) = app.root_path() {
                    let _ = std::fs::write(root.join("src/main.rs"), "fn main() {
    // changed outside
}
");
                    let _ = std::fs::write(root.join("src/extra.rs"), "pub fn extra() {}
");
                }
                waited.set(0);
            }
            1901 if following => {
                let reloaded = draft().is_some_and(|t| t.contains("changed outside"));
                let listed = shell.get_tree().iter().any(|r| r.name.as_str() == "extra.rs")
                    || shell.get_tree().iter().any(|r| r.name.as_str() == "src");
                if !reloaded || !listed {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(1901);
                    }
                    return;
                }
                eprintln!("watch: reloaded {reloaded} after {} ticks; draft {:?}", waited.get(), draft());
                app.act("app.exit");
            }
            // The lenses of a file with tests: listed, one run, its verdict.
            2300 if lenses => {
                let found: Vec<(usize, i32)> = code
                    .get_rows()
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.lens > 0)
                    .map(|(i, r)| (i, r.lens))
                    .collect();
                eprintln!("lens: rows {found:?}");
                if let Some((row, _)) = found.iter().find(|(_, kind)| *kind == 1) {
                    code.invoke_lens(*row as i32, "run".into());
                    waited.set(*row as u32);
                }
            }
            2301 if lenses => {
                if app.runner().is_running() {
                    step.set(2301);
                }
            }
            2310 if lenses => {
                let row = waited.get() as usize;
                let verdicts: Vec<(usize, i32)> = code
                    .get_rows()
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.lens > 0)
                    .map(|(i, r)| (i, r.verdict))
                    .collect();
                eprintln!("lens: after the run of row {row}, verdicts {verdicts:?}");
                snapshot(&window, &dir.join("lens.png"));
                app.act("app.exit");
            }
            // Every block folded, Down over one, a jump into one, and every
            // block opened again: the rows on screen counted at each step.
            2400 if folding => {
                let rows = code.get_rows();
                let headers = rows.iter().filter(|r| r.fold == 1).count();
                eprintln!("fold: {} rows, {headers} blocks", rows.row_count());
                app.act("editor.fold-all");
            }
            2401 if folding => {
                let rows = code.get_rows();
                let folded: Vec<i32> = rows.iter().filter(|r| r.fold == 2).map(|r| r.number).collect();
                eprintln!("fold: folded all, {} rows, collapsed at {folded:?}", rows.row_count());
                snapshot(&window, &dir.join("folded.png"));
                if let Some(first) = folded.first() {
                    waited.set(*first as u32);
                    app.editor().go_to(*first as usize, None);
                }
            }
            2402 if folding => {
                key(Key::DownArrow.into());
            }
            2403 if folding => {
                let at = app.editor().status().map(|s| s.0);
                eprintln!("fold: Down from line {} lands on line {at:?}", waited.get());
                // Into the first block's body: it opens.
                app.editor().go_to(waited.get() as usize + 1, None);
            }
            2404 if folding => {
                let rows = code.get_rows();
                let still = rows.iter().filter(|r| r.fold == 2).count();
                eprintln!(
                    "fold: a jump into line {} leaves {} rows, {still} collapsed, caret on {:?}",
                    waited.get() + 1,
                    rows.row_count(),
                    app.editor().status().map(|s| s.0)
                );
                app.act("editor.unfold-all");
            }
            2405 if folding => {
                let rows = code.get_rows();
                eprintln!(
                    "fold: unfolded all, {} rows, {} collapsed",
                    rows.row_count(),
                    rows.iter().filter(|r| r.fold == 2).count()
                );
                // Into the middle of a block: its header sticks.
                code.set_scroll_y(-30.0 * code.get_row_height());
            }
            2407 if folding => {
                let stuck: Vec<i32> = code.get_sticky_rows().iter().map(|r| r.number).collect();
                eprintln!(
                    "sticky: scrolled to row 30, stuck {stuck:?}; minimap {} with {} rows, slider at {} tall {}",
                    code.get_minimap_on(),
                    code.get_minimap_rows(),
                    code.get_minimap_slider_top(),
                    code.get_minimap_slider_height()
                );
                snapshot(&window, &dir.join("sticky.png"));
                app.act("app.exit");
            }
            // The server's colours and hints arrive; a click just past a
            // hint lands on its column, and typing there carries it along.
            2500 if hinting => {
                let told = app.editor().told();
                let arrived = told
                    .as_ref()
                    .is_some_and(|(colours, _, first)| *colours > 0 && first.is_some());
                if !arrived {
                    waited.set(waited.get() + 1);
                    if waited.get() < 2400 {
                        step.set(2500);
                    }
                    return;
                }
                let (colours, hints, first) = told.unwrap_or_default();
                eprintln!("hints: {colours} colours, {hints} hints after {} ticks; first {first:?}", waited.get());
                let rows = code.get_rows();
                let drawn: Vec<(i32, String)> = rows
                    .iter()
                    .filter(|r| r.hints.row_count() > 0)
                    .take(3)
                    .map(|r| {
                        let labels: Vec<String> = r.hints.iter().map(|h| h.text.to_string()).collect();
                        (r.number, labels.join(" "))
                    })
                    .collect();
                eprintln!("hints: drawn on {drawn:?}");
                snapshot(&window, &dir.join("hints.png"));
                if let Some((line, col, _, start)) = first {
                    waited.set(col as u32);
                    code.invoke_pointer(0, line as i32, start + 2.0, 5.0, Default::default());
                    code.invoke_pointer(2, line as i32, start + 2.0, 5.0, Default::default());
                }
            }
            2501 if hinting => {
                eprintln!(
                    "hints: a click just past the hint at column {} lands at {:?}",
                    waited.get() + 1,
                    app.editor().status().map(|s| (s.0, s.1))
                );
                key("x".into());
            }
            2502 if hinting => {
                let first = app.editor().told().and_then(|t| t.2);
                eprintln!("hints: typed at it, the first hint is now {first:?}");
                snapshot(&window, &dir.join("hints-typed.png"));
                chord(&[Key::Control], "z".into());
            }
            2503 if hinting => app.act("app.exit"),
            // Vim's keys: a word changed and undone in one press, two lines
            // deleted by a visual selection and put back.
            2600 if modal => {
                app.act("editor.vim");
                app.editor().go_to(52, Some(9));
            }
            2601..=2620 if modal => {
                let keys: &[&str] = &[
                    "w", "c", "w", "f", "i", "r", "s", "t", "\u{1b}", "u", "V", "j", "d", "u",
                ];
                let at = (n - 2601) as usize;
                let said = |what: &str| {
                    let line = draft()
                        .and_then(|text| text.lines().nth(51).map(str::to_string))
                        .unwrap_or_default();
                    eprintln!(
                        "vim: {what:>4} -> {:?} at {:?}; {} lines; line 52 {line:?}",
                        app.editor().vim_status(),
                        app.editor().status().map(|s| (s.0, s.1)),
                        app.editor().line_count().unwrap_or(0),
                    );
                };
                if at > 0 {
                    said(&keys[at - 1].escape_default().to_string());
                } else {
                    said("on");
                }
                match keys.get(at) {
                    Some(&"\u{1b}") => key(Key::Escape.into()),
                    Some(k) => key((*k).into()),
                    None => {
                        snapshot(&window, &dir.join("vim.png"));
                        app.act("app.exit");
                    }
                }
                if at == 11 {
                    snapshot(&window, &dir.join("vim-visual.png"));
                }
            }
            // Two groups: the file beside itself, another file on the right,
            // a press on the left moving the keys there, the right's last
            // tab closed ending the split.
            2700 if splitting => app.act("editor.split"),
            2701 if splitting => {
                let groups = |what: &str| {
                    let shade = window.global::<crate::Shade>();
                    eprintln!(
                        "split: {what}: split {} keys on {} | {:?} in front {:?} ({} rows) | other {:?} in front {:?} ({} rows) | caret {:?}",
                        shell.get_split(),
                        shell.get_focus_group(),
                        shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                        shell.get_active_tab(),
                        code.get_rows().row_count(),
                        shell.get_shade_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                        shell.get_shade_active(),
                        shade.get_rows().row_count(),
                        app.editor().status().map(|s| (s.0, s.1)),
                    );
                };
                groups("split");
                app.open_file("Cargo.toml".into());
            }
            2702 if splitting => {
                if shell.get_active_tab() != "file:Cargo.toml" {
                    step.set(2702);
                    return;
                }
                key("x".into());
            }
            2703 if splitting => {
                let line = draft().and_then(|t| t.lines().next().map(str::to_string));
                eprintln!("split: typed on the right: {line:?}");
                snapshot(&window, &dir.join("split.png"));
                chord(&[Key::Control], "z".into());
            }
            2704 if splitting => {
                window.global::<crate::Shade>().invoke_pressed(5, 40.0, 5.0);
            }
            2705 if splitting => {
                eprintln!(
                    "split: a press on the left: keys on {}, front {:?}, caret {:?}",
                    shell.get_focus_group(),
                    shell.get_active_tab(),
                    app.editor().status().map(|s| (s.0, s.1))
                );
                key("y".into());
            }
            2706 if splitting => {
                let line = draft().and_then(|t| t.lines().nth(5).map(str::to_string));
                eprintln!("split: typed on the left, line 6 {line:?}");
                snapshot(&window, &dir.join("split-left.png"));
                chord(&[Key::Control], "z".into());
            }
            2707 if splitting => {
                shell.invoke_shade_close("file:Cargo.toml".into());
            }
            2708 if splitting => {
                eprintln!(
                    "split: the right's last tab closed: split {}, tabs {:?}",
                    shell.get_split(),
                    shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>()
                );
                app.act("app.exit");
            }
            // The strip, the split and a tab's menu: every file under `book/`
            // opened until the strip overflows, the lane wheeled back to its
            // start and the front tab brought into view again; the editor
            // split, its divider dragged and double-clicked; each strip's tab
            // menu opened by a right-click and a few of its rows used; and
            // the logo clicked.
            3700 if tabbing => {
                // The layout told the window's size as the window has it,
                // so what is measured here is where things are drawn: a
                // scale changed after the window was laid out can leave the
                // two apart.
                let size = window
                    .window()
                    .size()
                    .to_logical(window.window().scale_factor());
                pointer(&window, WindowEvent::Resized { size });
                let root = app.root_path().unwrap_or_default();
                let mut names: Vec<String> = std::fs::read_dir(root.join("book"))
                    .map(|entries| {
                        entries
                            .filter_map(Result::ok)
                            .map(|e| format!("book/{}", e.file_name().to_string_lossy()))
                            .collect()
                    })
                    .unwrap_or_default();
                names.sort();
                eprintln!("tabs: opening {} files", names.len());
                tab_goal.set(names.len() + shell.get_tabs().row_count());
                for name in names {
                    app.open_file(name);
                }
            }
            3701 if tabbing => {
                if shell.get_tabs().row_count() < tab_goal.get() {
                    step.set(3701);
                }
            }
            3706 if tabbing => {
                eprintln!(
                    "tabs: {} open, {:?} in front",
                    shell.get_tabs().row_count(),
                    shell.get_active_tab()
                );
                snapshot(&window, &dir.join("tabs-overflow.png"));
                // The wheel over the strip, turned towards its start.
                let (left, _) = editor_span(&window, &shell);
                let at = LogicalPosition::new(left + 200.0, STRIP_Y);
                pointer(&window, WindowEvent::PointerMoved { position: at });
                pointer(
                    &window,
                    WindowEvent::PointerScrolled {
                        position: at,
                        delta_x: 0.0,
                        delta_y: 4000.0,
                    },
                );
            }
            3709 if tabbing => {
                snapshot(&window, &dir.join("tabs-wheel.png"));
                let first = shell.get_tabs().row_data(0).map(|t| t.id).unwrap_or_default();
                shell.invoke_pick_tab(first);
            }
            3711 if tabbing => {
                // The last tab picked from out of view: brought back into it.
                let count = shell.get_tabs().row_count();
                let last = shell
                    .get_tabs()
                    .row_data(count.saturating_sub(1))
                    .map(|t| t.id)
                    .unwrap_or_default();
                shell.invoke_pick_tab(last);
            }
            3715 if tabbing => {
                snapshot(&window, &dir.join("tabs-reveal.png"));
                app.act("editor.split");
            }
            3719 if tabbing => {
                let (left, width) = editor_span(&window, &shell);
                let share = shell.get_split_share();
                eprintln!(
                    "split: share {share}, the divider at {}",
                    left + divider_at(width, share)
                );
                snapshot(&window, &dir.join("split-half.png"));
                // The divider taken hold of a pixel beside its line and
                // dragged 250 px to the left.
                let x = left + divider_at(width, share) + 2.0;
                let from = LogicalPosition::new(x, 300.0);
                let to = LogicalPosition::new(x - 250.0, 300.0);
                pointer(&window, WindowEvent::PointerMoved { position: from });
                pointer(
                    &window,
                    WindowEvent::PointerPressed {
                        position: from,
                        button: PointerEventButton::Left,
                    },
                );
                pointer(&window, WindowEvent::PointerMoved { position: to });
                pointer(
                    &window,
                    WindowEvent::PointerReleased {
                        position: to,
                        button: PointerEventButton::Left,
                    },
                );
            }
            3722 if tabbing => {
                eprintln!(
                    "split: dragged: share {:.3}, kept as {:.3}",
                    shell.get_split_share(),
                    rusty_embed::config::workbench().layout.split
                );
                snapshot(&window, &dir.join("split-dragged.png"));
                // A double-click on the divider where it now is.
                let (left, width) = editor_span(&window, &shell);
                let at = LogicalPosition::new(
                    left + divider_at(width, shell.get_split_share()) - 2.0,
                    300.0,
                );
                pointer(&window, WindowEvent::PointerMoved { position: at });
                for _ in 0..2 {
                    pointer(
                        &window,
                        WindowEvent::PointerPressed {
                            position: at,
                            button: PointerEventButton::Left,
                        },
                    );
                    pointer(
                        &window,
                        WindowEvent::PointerReleased {
                            position: at,
                            button: PointerEventButton::Left,
                        },
                    );
                }
            }
            3725 if tabbing => {
                eprintln!(
                    "split: double-clicked: share {:.3}, kept as {:.3}",
                    shell.get_split_share(),
                    rusty_embed::config::workbench().layout.split
                );
                snapshot(&window, &dir.join("split-reset.png"));
                // A right-click on a tab of the left strip, the group
                // without the keys.
                let (left, _) = editor_span(&window, &shell);
                right_click(&window, LogicalPosition::new(left + 40.0, STRIP_Y));
            }
            3728 if tabbing => {
                eprintln!(
                    "menu: open {} for {:?}: {:?}",
                    shell.get_menu_open(),
                    app.tab_menu_for(),
                    menu_rows(&shell)
                );
                snapshot(&window, &dir.join("tab-menu-left.png"));
                shell.invoke_menu_picked("tab.split-right".into());
            }
            3731 if tabbing => {
                eprintln!("split right: {}", groups(&shell));
                snapshot(&window, &dir.join("tab-split-right.png"));
                // And on the right strip, the group with the keys.
                let (left, width) = editor_span(&window, &shell);
                let x = left + divider_at(width, shell.get_split_share()) + 1.0 + 40.0;
                right_click(&window, LogicalPosition::new(x, STRIP_Y));
            }
            3734 if tabbing => {
                eprintln!(
                    "menu: open {} for {:?}: {:?}",
                    shell.get_menu_open(),
                    app.tab_menu_for(),
                    menu_rows(&shell)
                );
                snapshot(&window, &dir.join("tab-menu-right.png"));
                shell.invoke_menu_picked("tab.copy-path".into());
            }
            3735 if tabbing => {
                eprintln!("copy path: {:?}", clipboard());
                let id = shell.get_active_tab();
                shell.invoke_tab_menu(id, false, 400.0, 80.0);
                shell.invoke_menu_picked("tab.copy-relative-path".into());
            }
            3736 if tabbing => {
                eprintln!("copy relative path: {:?}", clipboard());
                // Close to the right of the left group's fourth tab.
                let id = shell.get_shade_tabs().row_data(3).map(|t| t.id).unwrap_or_default();
                shell.invoke_tab_menu(id, true, 100.0, 80.0);
                shell.invoke_menu_picked("tab.close-right".into());
            }
            3738 if tabbing => {
                eprintln!("close to the right: {}", groups(&shell));
                // Something typed in the group's Rust file, then Close saved
                // on the group: every tab but that one goes, and nothing
                // asks.
                let rust = shell
                    .get_tabs()
                    .iter()
                    .find(|t| t.title == "lib.rs")
                    .map(|t| t.id)
                    .unwrap_or_default();
                shell.invoke_pick_tab(rust);
            }
            3739 if tabbing => key("x".into()),
            3740 if tabbing => {
                let id = shell.get_tabs().row_data(0).map(|t| t.id).unwrap_or_default();
                shell.invoke_tab_menu(id, false, 100.0, 80.0);
                shell.invoke_menu_picked("tab.close-saved".into());
            }
            3741 if tabbing => {
                eprintln!("close saved: {}", groups(&shell));
                // Close all on the other group: the split ends.
                let id = shell.get_shade_tabs().row_data(0).map(|t| t.id).unwrap_or_default();
                shell.invoke_tab_menu(id, true, 600.0, 80.0);
                shell.invoke_menu_picked("tab.close-all".into());
            }
            3743 if tabbing => {
                eprintln!("close all on the other group: {}", groups(&shell));
                snapshot(&window, &dir.join("tabs-closed.png"));
                chord(&[Key::Control], "z".into());
                if revealing {
                    let id = shell.get_active_tab();
                    shell.invoke_tab_menu(id, false, 100.0, 80.0);
                    shell.invoke_menu_picked("tab.reveal".into());
                    eprintln!("reveal: asked for {:?}", shell.get_active_tab());
                }
            }
            3745 if tabbing => {
                // The logo at the title bar's far left: the window's menu.
                let at = LogicalPosition::new(21.0, 20.0);
                pointer(&window, WindowEvent::PointerMoved { position: at });
                pointer(
                    &window,
                    WindowEvent::PointerPressed {
                        position: at,
                        button: PointerEventButton::Left,
                    },
                );
                pointer(
                    &window,
                    WindowEvent::PointerReleased {
                        position: at,
                        button: PointerEventButton::Left,
                    },
                );
            }
            3748 if tabbing => {
                eprintln!(
                    "logo: the picker open {}, saying {:?}, {} rows",
                    shell.get_picker_open(),
                    shell.get_picker_hint(),
                    shell.get_picker_rows().row_count()
                );
                snapshot(&window, &dir.join("logo-menu.png"));
                shell.set_picker_open(false);
                shell.invoke_picker_pick(-1);
            }
            3750 if tabbing => app.act("app.exit"),
            // Check for updates by hand, download what the feed offers, and
            // stop at the restart: what Settings says at each step.
            2800 if updating => app.act("update.check"),
            2801 if updating => {
                if shell.get_update_state() == 1 {
                    step.set(2801);
                    return;
                }
                eprintln!(
                    "update: checked: state {} {:?} — {:?}",
                    shell.get_update_state(),
                    shell.get_update_title(),
                    shell.get_update_detail()
                );
                if shell.get_update_state() == 3 {
                    app.act("update.download");
                }
            }
            2802 if updating => {
                let state = shell.get_update_state();
                if state == 4 {
                    step.set(2802);
                    return;
                }
                eprintln!(
                    "update: downloaded: state {state} {:?} — {:?}; {} blocks of notes",
                    shell.get_update_title(),
                    shell.get_update_detail(),
                    shell.get_update_notes().row_count()
                );
                snapshot(&window, &dir.join("update.png"));
                app.act("app.exit");
            }
            // What came back: each strip's tabs with what they say beside
            // their names, then one more file opened, so the strip the
            // exit leaves in workbench.toml can be read against it.
            2900 if strip => {
                if shell.get_active_kind() != "file" && waited.get() < 500 {
                    waited.set(waited.get() + 1);
                    step.set(2900);
                    return;
                }
                let said = |tabs: slint::ModelRc<crate::DocTab>| -> Vec<String> {
                    tabs.iter()
                        .map(|tab| {
                            if tab.hint.is_empty() {
                                tab.title.to_string()
                            } else {
                                format!("{} [{}]", tab.title, tab.hint)
                            }
                        })
                        .collect()
                };
                eprintln!(
                    "strip: {:?} in front {:?}; split {} beside {:?} in front {:?}",
                    said(shell.get_tabs()),
                    shell.get_active_title(),
                    shell.get_split(),
                    said(shell.get_shade_tabs()),
                    shell.get_shade_title(),
                );
                snapshot(&window, &dir.join("strip.png"));
                app.open_file("README.md".into());
            }
            2910 if strip => app.act("app.exit"),
            // Navigation by what the code means, once the server has the
            // file: the type of a binding, a macro expanded beside the
            // code, and the call hierarchy both ways.
            3000 if navigating => {
                let colours = app.editor().told().map_or(0, |told| told.0);
                if shell.get_active_kind() != "file" || colours == 0 {
                    waited.set(waited.get() + 1);
                    if waited.get() < 3000 {
                        step.set(3000);
                        return;
                    }
                }
                eprintln!("nav: the server painted {colours} spans after {} ticks", waited.get());
                waited.set(0);
                let Some((line, col)) = find("let c =", 4) else {
                    eprintln!("nav: no `let c =` in the file");
                    app.act("app.exit");
                    return;
                };
                eprintln!("nav: type definition asked at {line}:{col}");
                *start_path.borrow_mut() = app.editor().current_path().unwrap_or_default();
                app.editor().go_to(line, Some(col));
                app.act("editor.type-definition");
            }
            3001 if navigating => {
                let path = app.editor().current_path().unwrap_or_default();
                if path == start_path.borrow().as_str() && waited.get() < 1500 {
                    // A server still loading the workspace answers nothing:
                    // asked again every four seconds, as a person would.
                    waited.set(waited.get() + 1);
                    if waited.get().is_multiple_of(100) {
                        app.act("editor.type-definition");
                    }
                    step.set(3001);
                    return;
                }
                eprintln!(
                    "nav: type definition landed after {} ticks in {path} at {:?}; the line reads {:?}",
                    waited.get(),
                    app.editor().status().map(|s| (s.0, s.1)),
                    app.editor()
                        .status()
                        .and_then(|s| draft().and_then(|t| t.lines().nth(s.0 - 1).map(str::to_string)))
                );
                snapshot(&window, &dir.join("type-definition.png"));
                app.act("nav.back");
            }
            3002 if navigating => {
                eprintln!(
                    "nav: back in {:?} at {:?}",
                    app.editor().current_path(),
                    app.editor().status().map(|s| (s.0, s.1))
                );
                waited.set(0);
                let Some((line, col)) = find("println!", 1) else {
                    eprintln!("nav: no println! in the file");
                    return;
                };
                eprintln!("nav: expand the macro at {line}:{col}");
                app.editor().go_to(line, Some(col));
                app.act("editor.expand-macro");
            }
            3003 if navigating => {
                let beside = shell
                    .get_shade_tabs()
                    .iter()
                    .any(|t| t.id.contains("expansion:/"));
                if !beside && waited.get() < 500 {
                    waited.set(waited.get() + 1);
                    step.set(3003);
                    return;
                }
                let expansion = shell
                    .get_shade_tabs()
                    .iter()
                    .find(|t| t.id.contains("expansion:/"))
                    .map(|t| t.id.trim_start_matches("file:").to_string())
                    .unwrap_or_default();
                let text = app
                    .editor()
                    .draft_of(&expansion)
                    .map(|(text, ..)| text)
                    .unwrap_or_default();
                eprintln!(
                    "nav: expanded after {} ticks: split {} keys on {} | left {:?} in front {:?} | right {:?} in front {:?} | {} lines, starting {:?}",
                    waited.get(),
                    shell.get_split(),
                    shell.get_focus_group(),
                    shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                    shell.get_active_tab(),
                    shell
                        .get_shade_tabs()
                        .iter()
                        .map(|t| format!("{} [{}]", t.title, t.hint))
                        .collect::<Vec<_>>(),
                    shell.get_shade_active(),
                    text.lines().count(),
                    text.lines().take(4).collect::<Vec<_>>()
                );
                *start_path.borrow_mut() = expansion;
                *before.borrow_mut() = Some(text);
                snapshot(&window, &dir.join("expansion.png"));
                // The same call again: the same tab, not a second.
                app.act("editor.expand-macro");
            }
            3040 if navigating => {
                let tabs = shell
                    .get_shade_tabs()
                    .iter()
                    .filter(|t| t.id.contains("expansion:/"))
                    .count();
                eprintln!("nav: expanded again: {tabs} expansion tab(s)");
                // A press in the expansion moves the keys there: typed into
                // and saved, it is to stay what it was and write nothing.
                window.global::<crate::Shade>().invoke_pressed(3, 40.0, 5.0);
            }
            3041 if navigating => {
                key("x".into());
                chord(&[Key::Control], "s".into());
            }
            3050 if navigating => {
                let expansion = start_path.borrow().clone();
                let held = app.editor().draft_of(&expansion).map(|(text, ..)| text);
                let written: Vec<String> = app
                    .root_path()
                    .and_then(|root| std::fs::read_dir(root).ok())
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with("expansion"))
                    .collect();
                eprintln!(
                    "nav: typed into and saved: keys on {} in front {:?}; text unchanged {}; dirty {}; kept as a draft {}; written to the project {written:?}; the status bar says {:?}",
                    shell.get_focus_group(),
                    shell.get_active_tab(),
                    held.is_some() && held == *before.borrow(),
                    app.editor().is_dirty(&expansion),
                    app.editor().backup_of(&expansion).is_some(),
                    shell.get_language(),
                );
                // And a press on the left gives the code its keys back.
                window.global::<crate::Shade>().invoke_pressed(12, 40.0, 5.0);
            }
            3051 if navigating => {
                eprintln!(
                    "nav: back in the code: keys on {} in front {:?}",
                    shell.get_focus_group(),
                    shell.get_active_tab()
                );
                waited.set(0);
                let Some((line, col)) = find("Vector::new", 9) else {
                    eprintln!("nav: no Vector::new in the file");
                    return;
                };
                eprintln!("nav: the call hierarchy of `new`, asked at {line}:{col} from the right-click menu");
                app.editor().go_to(line, Some(col));
                // A right-click on the caret: the menu, in the window.
                code.invoke_context_menu(
                    code.get_caret_line(),
                    code.get_caret_x() + 1.0,
                    5.0,
                    560.0,
                    300.0,
                );
            }
            3052 if navigating => {
                let items: Vec<String> = shell
                    .get_menu_items()
                    .iter()
                    .map(|item| {
                        if item.id.is_empty() {
                            "—".to_string()
                        } else if item.chord.is_empty() {
                            item.label.to_string()
                        } else {
                            format!("{} ({})", item.label, item.chord)
                        }
                    })
                    .collect();
                eprintln!(
                    "nav: the editor's menu (open {}): {items:?}; caret at {:?}",
                    shell.get_menu_open(),
                    app.editor().status().map(|s| (s.0, s.1))
                );
                snapshot(&window, &dir.join("menu.png"));
                shell.invoke_menu_picked("editor.call-hierarchy".into());
            }
            3053 if navigating => {
                let rows = shell.get_calls_rows();
                if rows.row_count() < 2 && shell.get_calls_empty().is_empty() && waited.get() < 750 {
                    waited.set(waited.get() + 1);
                    step.set(3053);
                    return;
                }
                eprintln!("nav: calls in, after {} ticks: {}", waited.get(), calls_said(&shell));
                waited.set(0);
                let cross = rows.iter().find(|r| r.depth == 1 && r.name == "cross");
                match cross {
                    Some(row) => shell.invoke_calls_toggle(row.id),
                    None => eprintln!("nav: no `cross` row to open"),
                }
            }
            3054 if navigating => {
                let rows = shell.get_calls_rows();
                if !rows.iter().any(|r| r.depth == 2) && waited.get() < 500 {
                    waited.set(waited.get() + 1);
                    step.set(3054);
                    return;
                }
                eprintln!("nav: `cross` opened: {}", calls_said(&shell));
                snapshot(&window, &dir.join("calls-in.png"));
                // A click on the row goes where its call is made.
                if let Some(row) = rows.iter().find(|r| r.depth == 1 && r.name == "cross") {
                    shell.invoke_calls_go(row.id, false);
                }
            }
            3055 if navigating => {
                eprintln!(
                    "nav: a click on the `cross` row went to {:?} at {:?}",
                    app.editor().current_path(),
                    app.editor().status().map(|s| (s.0, s.1))
                );
                app.act("nav.back");
                shell.invoke_calls_direction(false);
                waited.set(0);
            }
            3056 if navigating => {
                let rows = shell.get_calls_rows();
                if rows.row_count() < 2 && shell.get_calls_empty().is_empty() && waited.get() < 500 {
                    waited.set(waited.get() + 1);
                    step.set(3056);
                    return;
                }
                eprintln!(
                    "nav: turned to calls out (incoming {}): {}",
                    shell.get_calls_incoming(),
                    calls_said(&shell)
                );
                snapshot(&window, &dir.join("calls-out-new.png"));
                waited.set(0);
                // The next hierarchy reads the way the last was turned.
                let Some((line, col)) = find("c.norm()", 3) else {
                    eprintln!("nav: no c.norm() in the file");
                    return;
                };
                eprintln!("nav: the call hierarchy of `norm`, asked at {line}:{col}");
                app.editor().go_to(line, Some(col));
                app.act("editor.call-hierarchy");
            }
            3057 if navigating => {
                let rows = shell.get_calls_rows();
                if rows.row_count() < 2 && shell.get_calls_empty().is_empty() && waited.get() < 750 {
                    waited.set(waited.get() + 1);
                    step.set(3057);
                    return;
                }
                eprintln!("nav: calls out of `norm`: {}", calls_said(&shell));
                waited.set(0);
                if let Some(row) = rows.iter().find(|r| r.depth == 1 && r.name == "dot") {
                    shell.invoke_calls_toggle(row.id);
                }
            }
            3058 if navigating => {
                let rows = shell.get_calls_rows();
                let asking = rows.iter().any(|r| r.open == 1);
                if asking && waited.get() < 500 {
                    waited.set(waited.get() + 1);
                    step.set(3058);
                    return;
                }
                eprintln!("nav: `dot` opened: {}", calls_said(&shell));
                snapshot(&window, &dir.join("calls-out.png"));
            }
            // What the strip keeps for next time: files, never the expansion.
            3060 if navigating => {
                let strip = app
                    .root_path()
                    .and_then(|root| rusty_embed::config::tabs_for(&root.to_string_lossy()));
                eprintln!(
                    "nav: the strip kept for next time: {:?}",
                    strip.map(|s| (s.tabs, s.active, s.second, s.second_active))
                );
            }
            3070 if navigating => app.act("app.exit"),
            // A tab moved into a window of its own, and the document the two
            // windows share.
            3600 if tearing => {
                if shell.get_active_kind() != "file" {
                    step.set(3600);
                    return;
                }
                *start_path.borrow_mut() = app.editor().current_path().unwrap_or_default();
                app.act("editor.detach");
            }
            3601 if tearing => {
                let torn = app.torn_windows();
                eprintln!(
                    "torn: moved {:?}: {} window(s) of its own | main strip {:?} | its caption {:?} tab {:?} | its rows {}",
                    start_path.borrow(),
                    torn.len(),
                    shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                    torn.first().map(|(w, _)| w.get_caption().to_string()),
                    torn.first().map(|(w, _)| w.get_name().to_string()),
                    torn.first().map_or(0, |(w, _)| w.global::<Code>().get_rows().row_count()),
                );
                let size = |w: &slint::Window| (w.scale_factor(), w.size().width, w.size().height);
                eprintln!(
                    "torn: scale and size: main {:?}, its {:?}",
                    size(window.window()),
                    torn.first().map(|(w, _)| size(w.window()))
                );
            }
            3605 if tearing => {
                if let Some((torn, _)) = app.torn_windows().first() {
                    for ch in ["/", "*", "t", "o", "r", "n", "*", "/"] {
                        torn.window().dispatch_event(WindowEvent::KeyPressed { text: ch.into() });
                        torn.window().dispatch_event(WindowEvent::KeyReleased { text: ch.into() });
                    }
                }
            }
            3606 if tearing => {
                let path = start_path.borrow().clone();
                let first = app
                    .editor()
                    .draft_of(&path)
                    .and_then(|(text, ..)| text.lines().next().map(str::to_string));
                eprintln!(
                    "torn: typed in the window of its own: line 1 {first:?}, dirty {}",
                    app.editor().is_dirty(&path)
                );
                // Ctrl+F there: its own find bar.
                if let Some((torn, _)) = app.torn_windows().first() {
                    torn_chord(torn, &[Key::Control], "f");
                }
            }
            3607 if tearing => {
                if let Some((torn, _)) = app.torn_windows().first() {
                    eprintln!(
                        "torn: Ctrl+F in the window of its own: its find bar open {}, query {:?}; the main window's open {}",
                        torn.global::<Find>().get_open(),
                        torn.global::<Find>().get_query(),
                        window.global::<Find>().get_open()
                    );
                    snapshot_of(torn.window(), &dir.join("torn-find.png"));
                    torn_chord(torn, &[], &slint::SharedString::from(Key::Escape));
                }
            }
            3608 if tearing => {
                // A function at the end of the file, typed there, asking for
                // `v.no` to be completed.
                if let Some((torn, _)) = app.torn_windows().first() {
                    torn_chord(torn, &[Key::Control], &slint::SharedString::from(Key::End));
                    torn_chord(torn, &[], &slint::SharedString::from(Key::Return));
                    for ch in "fn torn_probe(v: Vector) -> f32 { v.no".chars() {
                        torn_chord(torn, &[], &ch.to_string());
                    }
                }
                waited.set(0);
            }
            3609 if tearing => {
                let Some((torn, _)) = app.torn_windows().into_iter().next() else {
                    return;
                };
                let popup = torn.global::<Code>();
                let labels: Vec<String> = popup
                    .get_completion_rows()
                    .iter()
                    .map(|row| row.label.to_string())
                    .collect();
                if !(popup.get_completion_open() && labels.iter().any(|l| l.starts_with("norm"))) {
                    waited.set(waited.get() + 1);
                    if waited.get() < 1500 {
                        step.set(3609);
                        return;
                    }
                }
                eprintln!(
                    "torn: completion in the window of its own after {} ms: open {}, rows {:?}",
                    waited.get() * 40,
                    popup.get_completion_open(),
                    labels.iter().take(6).collect::<Vec<_>>()
                );
                snapshot_of(torn.window(), &dir.join("torn-completion.png"));
                torn_chord(&torn, &[], &slint::SharedString::from(Key::Escape));
                // A line that does not parse, for a squiggle.
                for ch in "; let = ;".chars() {
                    torn_chord(&torn, &[], &ch.to_string());
                }
                waited.set(0);
                step.set(3680);
            }
            3680 if tearing => {
                let Some((torn, surface)) = app.torn_windows().into_iter().next() else {
                    return;
                };
                let squiggled: Vec<i32> = torn
                    .global::<Code>()
                    .get_rows()
                    .iter()
                    .filter(|row| row.marks.iter().any(|m| m.kind == 5))
                    .map(|row| row.number)
                    .collect();
                if squiggled.is_empty() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 1500 {
                        step.set(3680);
                        return;
                    }
                }
                eprintln!(
                    "torn: an error in the window of its own after {} ms, squiggled on rows {squiggled:?}",
                    waited.get() * 40
                );
                // The card for the name `Vector` in the line typed there.
                let text = surface
                    .current_path()
                    .and_then(|path| app.editor().draft_of(&path))
                    .map(|(text, ..)| text)
                    .unwrap_or_default();
                if let Some((line, col)) = text.lines().enumerate().find_map(|(index, line)| {
                    line.contains("torn_probe")
                        .then(|| line.find("Vector").map(|at| (index + 1, at + 3)))
                        .flatten()
                }) {
                    surface.go_to(line, Some(col));
                }
                surface.hover_caret();
                waited.set(0);
            }
            3681 if tearing => {
                let Some((torn, _)) = app.torn_windows().into_iter().next() else {
                    return;
                };
                let card = torn.global::<Code>();
                let answered = card.get_hover_open()
                    && (card.get_hover_code().row_count() > 0 || card.get_hover_has_docs());
                if !answered {
                    waited.set(waited.get() + 1);
                    if waited.get() < 750 {
                        step.set(3681);
                        return;
                    }
                }
                let code: Vec<String> = card
                    .get_hover_code()
                    .iter()
                    .map(|row| row.pieces.iter().map(|p| p.text.to_string()).collect())
                    .collect();
                eprintln!(
                    "torn: the hover card in the window of its own after {} ms: open {}, its code {code:?}",
                    waited.get() * 40,
                    card.get_hover_open()
                );
                snapshot_of(torn.window(), &dir.join("torn-hover.png"));
                torn_chord(&torn, &[], &slint::SharedString::from(Key::Escape));
                app.open_file(start_path.borrow().clone());
                step.set(3610);
            }
            3610 if tearing => {
                let path = start_path.borrow().clone();
                if shell.get_active_tab() != format!("file:{path}").as_str() {
                    step.set(3610);
                    return;
                }
                let row = code.get_rows().row_data(0).map(|row| {
                    row.pieces.iter().map(|p| p.text.to_string()).collect::<String>()
                });
                eprintln!("torn: the same file in the main window, its row 1 {row:?}");
                let size = |w: &slint::Window| (w.scale_factor(), w.size().width, w.size().height);
                eprintln!(
                    "torn: scale and size: main {:?}, its {:?}",
                    size(window.window()),
                    app.torn_windows().first().map(|(w, _)| size(w.window()))
                );
                snapshot(&window, &dir.join("torn-main.png"));
                if let Some((torn, _)) = app.torn_windows().first() {
                    snapshot_of(torn.window(), &dir.join("torn-window.png"));
                }
                // The caret to the line's end in the main window: a new undo
                // step, apart from the typing in the other.
                key(Key::End.into());
            }
            3611 if tearing => key("M".into()),
            3612 if tearing => {
                if let Some((torn, surface)) = app.torn_windows().first() {
                    let line = surface.status().map_or(1, |s| s.0);
                    let row = torn
                        .global::<Code>()
                        .get_rows()
                        .iter()
                        .find(|row| row.number == line as i32)
                        .map(|row| row.pieces.iter().map(|p| p.text.to_string()).collect::<String>());
                    eprintln!(
                        "torn: typed in the main window, the other's row {line} {row:?}, its caret {:?}",
                        surface.status().map(|s| (s.0, s.1))
                    );
                    snapshot_of(torn.window(), &dir.join("torn-window-follows.png"));
                    // Undone from the window of its own: one history.
                    let send = |event| torn.window().dispatch_event(event);
                    send(WindowEvent::KeyPressed { text: Key::Control.into() });
                    send(WindowEvent::KeyPressed { text: "z".into() });
                    send(WindowEvent::KeyReleased { text: "z".into() });
                    send(WindowEvent::KeyReleased { text: Key::Control.into() });
                }
            }
            3613 if tearing => {
                let path = start_path.borrow().clone();
                let typed = app.editor().draft_of(&path).and_then(|(text, ..)| {
                    text.lines()
                        .find(|line| line.contains("torn_probe"))
                        .map(str::to_string)
                });
                eprintln!("torn: Ctrl+Z in the window of its own: the line typed there {typed:?}");
                // Saved from there.
                if let Some((torn, _)) = app.torn_windows().first() {
                    let send = |event| torn.window().dispatch_event(event);
                    send(WindowEvent::KeyPressed { text: Key::Control.into() });
                    send(WindowEvent::KeyPressed { text: "s".into() });
                    send(WindowEvent::KeyReleased { text: "s".into() });
                    send(WindowEvent::KeyReleased { text: Key::Control.into() });
                }
            }
            3640 if tearing => {
                let path = start_path.borrow().clone();
                let disk = app
                    .root_path()
                    .and_then(|root| std::fs::read_to_string(root.join(&path)).ok())
                    .and_then(|text| text.lines().next().map(str::to_string));
                let torn = app.torn_windows();
                eprintln!(
                    "torn: Ctrl+S in the window of its own: on disk line 1 {disk:?}, dirty {}, its tab dirty {:?}",
                    app.editor().is_dirty(&path),
                    torn.first().map(|(w, _)| w.get_dirty())
                );
                snapshot_of(torn[0].0.window(), &dir.join("torn-saved.png"));
                // Closed with Ctrl+W there: the main window still shows it.
                let send = |event| torn[0].0.window().dispatch_event(event);
                send(WindowEvent::KeyPressed { text: Key::Control.into() });
                send(WindowEvent::KeyPressed { text: "w".into() });
                send(WindowEvent::KeyReleased { text: "w".into() });
                send(WindowEvent::KeyReleased { text: Key::Control.into() });
            }
            3642 if tearing => {
                let path = start_path.borrow().clone();
                eprintln!(
                    "torn: closed: {} window(s) of its own, main holds it {}, main strip {:?}",
                    app.torn_windows().len(),
                    app.editor().holds(&path),
                    shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                );
                key("Q".into());
            }
            3643 if tearing => {
                let first = draft().and_then(|t| t.lines().next().map(str::to_string));
                eprintln!("torn: typed in the main window after: line 1 {first:?}");
                snapshot(&window, &dir.join("torn-closed.png"));
                chord(&[Key::Control], "z".into());
                app.open_file("Cargo.toml".into());
            }
            3646 if tearing => {
                if shell.get_active_tab() != "file:Cargo.toml" {
                    step.set(3646);
                    return;
                }
                // The tab pressed, dragged over the window's left edge and
                // let go beyond it, through the window's own pointer events:
                // the strip's TouchArea keeps the press while it is outside.
                let tab = slint::LogicalPosition::new(450.0, 58.0);
                let send = |event| window.window().dispatch_event(event);
                let left = slint::platform::PointerEventButton::Left;
                send(WindowEvent::PointerPressed { position: tab, button: left });
                for x in [400.0, 300.0, 150.0, 20.0, -60.0] {
                    send(WindowEvent::PointerMoved {
                        position: slint::LogicalPosition::new(x, 90.0),
                    });
                }
                let out = slint::LogicalPosition::new(-60.0, 90.0);
                send(WindowEvent::PointerReleased { position: out, button: left });
            }
            3650 if tearing => {
                let torn = app.torn_windows();
                eprintln!(
                    "torn: Cargo.toml dragged out: {} window(s) of its own, its tab {:?}, main strip {:?}",
                    torn.len(),
                    torn.first().map(|(w, _)| w.get_name().to_string()),
                    shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                );
                if let Some((torn, _)) = torn.first() {
                    snapshot_of(torn.window(), &dir.join("torn-dragged.png"));
                    torn.invoke_move_back();
                }
            }
            3652 if tearing => {
                eprintln!(
                    "torn: moved back: {} window(s) of its own, main strip {:?}, in front {:?}",
                    app.torn_windows().len(),
                    shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                    shell.get_active_tab(),
                );
                snapshot(&window, &dir.join("torn-back.png"));
            }
            // A file in a window of its own follows the tree: renamed there,
            // it is the renamed file; deleted there, its window closes.
            3653 if tearing => {
                if let Some(root) = app.root_path() {
                    let _ = std::fs::write(root.join("notes.txt"), "a note\n");
                }
                app.open_file("notes.txt".into());
                waited.set(0);
            }
            3654 if tearing => {
                if shell.get_active_tab() != "file:notes.txt" || app.tree_row_of("notes.txt").is_none() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(3654);
                    }
                    return;
                }
                // From the tab's own menu, as a right-click there offers it.
                shell.invoke_tab_menu("file:notes.txt".into(), false, 450.0, 58.0);
            }
            3655 if tearing => {
                eprintln!(
                    "torn: the tab's menu offers {:?}",
                    shell.get_menu_items().iter().map(|i| i.id.to_string()).collect::<Vec<_>>()
                );
                shell.invoke_menu_picked("editor.detach".into());
            }
            3656 if tearing => {
                let torn = app.torn_windows();
                eprintln!(
                    "torn: notes.txt from its tab's menu: {} window(s) of its own, its tab {:?}, main strip {:?}",
                    torn.len(),
                    torn.first().map(|(w, _)| w.get_name().to_string()),
                    shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                );
                if let Some(row) = app.tree_row_of("notes.txt") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            3657 if tearing => shell.invoke_menu_picked("rename".into()),
            3658 if tearing => {
                if shell.get_tree_name_row() < 0 {
                    step.set(3658);
                    return;
                }
                for c in "moved".chars() {
                    key(c.to_string().into());
                }
                key(Key::Return.into());
                waited.set(0);
            }
            3659 if tearing => {
                let torn = app.torn_windows();
                let named = torn.first().map(|(w, _)| w.get_name().to_string());
                if named.as_deref() != Some("moved.txt") {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(3659);
                    }
                    return;
                }
                if let Some((torn, surface)) = torn.first() {
                    torn_chord(torn, &[], "x");
                    eprintln!(
                        "torn: renamed in the tree: its tab {named:?}, its title {:?}, its file {:?}, the draft {:?}, notes.txt open {}",
                        torn.get_caption().to_string(),
                        surface.current_path(),
                        app.editor().draft_of("moved.txt").map(|(text, ..)| text),
                        app.editor().holds("notes.txt"),
                    );
                    snapshot_of(torn.window(), &dir.join("torn-renamed.png"));
                }
                if let Some(row) = app.tree_row_of("moved.txt") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            3660 if tearing => shell.invoke_menu_picked("delete".into()),
            3661 if tearing => {
                if !app.torn_windows().is_empty() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(3661);
                    }
                    return;
                }
                eprintln!(
                    "torn: deleted in the tree: {} window(s) of its own, moved.txt open {}, main strip {:?}",
                    app.torn_windows().len(),
                    app.editor().holds("moved.txt"),
                    shell.get_tabs().iter().map(|t| t.title.to_string()).collect::<Vec<_>>(),
                );
                snapshot(&window, &dir.join("torn-deleted.png"));
            }
            3665 if tearing => app.act("app.exit"),
            // Where the caret rests: the brace beside it and its partner
            // outlined; the name under it washed where else it occurs, and
            // the washes gone at the next key; a problem's fixes listed on
            // the hover card, and one applied by its click.
            3000 if marking => {
                let text = draft().unwrap_or_default();
                let brace = text.lines().enumerate().find_map(|(i, l)| {
                    let at = l.find("impl ")?;
                    let open = l[at..].find('{')? + at;
                    Some((i, l[..open].chars().count()))
                });
                if let Some((line, col)) = brace {
                    app.editor().go_to(line + 1, Some(col + 1));
                }
            }
            3002 if marking => {
                eprintln!(
                    "marks: the caret before a brace at {:?}: outlined {:?}",
                    app.editor().status().map(|s| (s.0, s.1)),
                    drawn(&code, 9)
                );
                snapshot(&window, &dir.join("marks-brackets.png"));
                key(Key::End.into());
            }
            3003 if marking => {
                eprintln!(
                    "marks: the caret after it, at {:?}: outlined {:?}",
                    app.editor().status().map(|s| (s.0, s.1)),
                    drawn(&code, 9)
                );
                waited.set(0);
            }
            3004 if marking => {
                // The server, once its colours say it is answering.
                let ready = app.editor().told().is_some_and(|(colours, ..)| colours > 0);
                if !ready && waited.get() < 2400 {
                    waited.set(waited.get() + 1);
                    step.set(3004);
                    return;
                }
                eprintln!("marks: the server answered after {} ticks", waited.get());
                waited.set(0);
                let text = draft().unwrap_or_default();
                let name = text
                    .lines()
                    .enumerate()
                    .find_map(|(i, l)| l.find("other: ").map(|at| (i, l[..at].chars().count())));
                if let Some((line, col)) = name {
                    app.editor().go_to(line + 1, Some(col + 3));
                }
            }
            3005 if marking => {
                let (_, washed) = app.editor().resting_marks();
                if washed == 0 && waited.get() < 750 {
                    waited.set(waited.get() + 1);
                    // A server still loading answers with nothing: a step
                    // off and back asks again.
                    if waited.get().is_multiple_of(50) {
                        key(Key::RightArrow.into());
                        key(Key::LeftArrow.into());
                    }
                    step.set(3005);
                    return;
                }
                eprintln!(
                    "marks: `other` washed in {washed} places after {} ticks: {:?}",
                    waited.get(),
                    drawn(&code, 10)
                );
                snapshot(&window, &dir.join("marks-occurrences.png"));
                key(" ".into());
            }
            3006 if marking => {
                eprintln!(
                    "marks: after a key, {:?} washes drawn",
                    drawn(&code, 10).len()
                );
                chord(&[Key::Control], "z".into());
            }
            3007 if marking => {
                // A struct built without two of its fields, which the server
                // offers to fill.
                let Some(path) = app.editor().current_path() else {
                    step.set(3030);
                    return;
                };
                let text = draft().unwrap_or_default();
                let version = app.editor().version_of(&path).unwrap_or(0);
                let with = format!(
                    "{}\n\npub fn probe_point() -> Vector {{\n    Vector {{ x: 1.0 }}\n}}\n",
                    text.trim_end()
                );
                // The errors before it, to wait for one more.
                spot.set(shell.get_errors().max(0) as usize);
                app.editor().replace(&path, version, with);
                waited.set(0);
            }
            3008 if marking => {
                let text = draft().unwrap_or_default();
                let line = text.lines().position(|l| l.trim() == "Vector { x: 1.0 }");
                if shell.get_errors().max(0) as usize <= spot.get() && waited.get() < 1500 {
                    waited.set(waited.get() + 1);
                    step.set(3008);
                    return;
                }
                eprintln!(
                    "marks: {} errors after {} ticks, the literal on line {:?}",
                    shell.get_errors(),
                    waited.get(),
                    line.map(|l| l + 1)
                );
                if let Some(line) = line {
                    spot.set(line);
                    app.editor().go_to(line + 1, Some(7));
                }
            }
            3009 if marking => {
                app.act("editor.show-hover");
                waited.set(0);
            }
            3010 if marking => {
                let fixes = app.editor().card_fixes();
                if fixes.is_empty() && waited.get() < 750 {
                    waited.set(waited.get() + 1);
                    // A server that had not settled offers nothing yet:
                    // the card is asked for again.
                    if waited.get().is_multiple_of(100) {
                        app.act("editor.show-hover");
                    }
                    step.set(3010);
                    return;
                }
                let problems: Vec<String> = code
                    .get_hover_problems()
                    .iter()
                    .map(|p| p.text.to_string())
                    .collect();
                eprintln!(
                    "marks: the card on line {} says {problems:?} and offers {fixes:?} after {} ticks",
                    spot.get() + 1,
                    waited.get()
                );
                snapshot(&window, &dir.join("marks-fixes.png"));
                let pick = fixes.iter().position(|f| f.contains("Fill")).unwrap_or(0);
                if !fixes.is_empty() {
                    code.invoke_hover_fix(pick as i32);
                }
            }
            3012 if marking => {
                let text = draft().unwrap_or_default();
                let wrote: Vec<&str> = text.lines().skip(spot.get()).take(3).collect();
                eprintln!(
                    "marks: the fix wrote {wrote:?}; the card {}",
                    if code.get_hover_open() { "is still up" } else { "went" }
                );
                snapshot(&window, &dir.join("marks-fixed.png"));
                // Both at once: the caret between a call's name and its
                // parenthesis, in each theme.
                let call = text
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| l.trim_start().starts_with("Vector::new("))
                    .find_map(|(i, l)| l.find("::new(").map(|at| (i, l[..at].chars().count() + 5)));
                if let Some((line, col)) = call {
                    app.editor().go_to(line + 1, Some(col + 1));
                }
                waited.set(0);
            }
            3013 if marking => {
                let (_, washed) = app.editor().resting_marks();
                if washed == 0 && waited.get() < 250 {
                    waited.set(waited.get() + 1);
                    step.set(3013);
                    return;
                }
                eprintln!(
                    "marks: beside `new(`, outlined {:?}, washed {:?}",
                    drawn(&code, 9),
                    drawn(&code, 10)
                );
                shell.set_theme_choice("light".into());
            }
            3015 if marking => {
                snapshot(&window, &dir.join("marks-light.png"));
                shell.set_theme_choice("dark".into());
            }
            3017 if marking => snapshot(&window, &dir.join("marks-dark.png")),
            3018 if marking => app.act("app.exit"),
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
            // With a sensor declared, the Flight tab and the loop closed
            // through the plant.
            576 if sim => {
                if shell.get_flight_ready() {
                    shell.invoke_show_panel("bottom".into(), "flight".into());
                    app.act("flight.close");
                }
            }
            // A signal on the sheet: the Signals tab, each instrument in turn.
            579 if sim => {
                if shell.get_lab_sources().row_count() > 0 {
                    shell.invoke_show_panel("bottom".into(), "signals".into());
                }
            }
            581 if sim => {
                if shell.get_bottom_front() == "signals" {
                    eprintln!(
                        "sim: signals sources {:?}; lanes {:?}",
                        shell.get_lab_sources().iter().map(|s| format!("{} {}", s.label, s.text)).collect::<Vec<_>>(),
                        shell.get_lab_lanes().iter().map(|l| format!("{} {}", l.label, l.scale)).collect::<Vec<_>>()
                    );
                    snapshot(&window, &dir.join("signals-time.png"));
                    shell.invoke_lab_set("view".into(), 1);
                }
            }
            583 if sim => {
                if shell.get_bottom_front() == "signals" {
                    eprintln!("sim: spectrum {:?}", shell.get_lab_spectrum_caption());
                    snapshot(&window, &dir.join("signals-spectrum.png"));
                    shell.invoke_lab_set("view".into(), 3);
                }
            }
            585 if sim => {
                if shell.get_bottom_front() == "signals" {
                    eprintln!("sim: design ok {} note {:?}", shell.get_lab_design_ok(), shell.get_lab_design_note());
                    snapshot(&window, &dir.join("signals-design.png"));
                    shell.invoke_lab_set("view".into(), 2);
                    app.act("lab.measure-recorded");
                    eprintln!("sim: response {:?}", shell.get_lab_status());
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
            586 if sim => {
                if shell.get_flight_closed() {
                    eprintln!(
                        "sim: flight closed, feeding {:?}; craft {:?} {:?}; motors {:?}; axes {:?}",
                        shell.get_flight_feeds(),
                        shell.get_flight_craft_text(),
                        shell.get_flight_craft_source(),
                        shell.get_flight_motors().iter().map(|m| m.text.to_string()).collect::<Vec<_>>(),
                        shell.get_flight_axes().iter().map(|a| format!("{} {} {}", a.label, a.channel, a.value)).collect::<Vec<_>>()
                    );
                    snapshot(&window, &dir.join("flight.png"));
                }
            }
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
            252..=1750 if !(build || targets || sim || edits || drawing || debugging || terminal || repository || measuring || dependencies || asking || reading || sums || cloning.is_some() || following || lenses || folding || hinting || modal || splitting || navigating || updating || marking) => {
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
            // The tree's own verbs, on a copy of a project with `src/`: a
            // file made in `src` from its menu (a taken name refused first),
            // a folder at the root from the space below the rows, a rename,
            // a copy pasted beside its original, a file with an open,
            // edited tab dragged into the folder — the tab and its draft
            // go with it — a cut pasted, a delete, and the arrows and F2.
            // Clicks, right-clicks and the drag are pointer events at the
            // rows, so the view's own hit-testing is what is driven.
            4100 if exploring => {
                shell.invoke_show_panel("left".into(), "project".into());
            }
            4101 if exploring => {
                eprintln!("tree: opened: {}", tree_said(&shell));
                match app.tree_row_of("src") {
                    Some(row) => tree_click(&window, row, PointerEventButton::Right),
                    None => {
                        eprintln!("tree: no src/ to work in");
                        app.act("app.exit");
                    }
                }
            }
            4102 if exploring => {
                eprintln!("tree: src's menu: {}", menu_said(&shell));
                snapshot(&window, &dir.join("tree-menu-folder.png"));
                shell.invoke_menu_picked("new-file".into());
            }
            4103 if exploring => {
                if shell.get_tree_name_row() < 0 {
                    step.set(4103);
                    return;
                }
                for c in "lib.rs".chars() {
                    key(c.to_string().into());
                }
            }
            4104 if exploring => {
                eprintln!(
                    "tree: typed lib.rs into the box on row {}: {:?}",
                    shell.get_tree_name_row(),
                    shell.get_tree_name_problem()
                );
                snapshot(&window, &dir.join("tree-name-taken.png"));
                chord(&[Key::Control], "a".into());
                for c in "probe.rs".chars() {
                    key(c.to_string().into());
                }
            }
            4105 if exploring => {
                eprintln!("tree: then probe.rs: {:?}", shell.get_tree_name_problem());
                snapshot(&window, &dir.join("tree-name-new.png"));
                key(Key::Return.into());
                waited.set(0);
            }
            4106 if exploring => {
                if shell.get_active_tab() != "file:src/probe.rs" {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(4106);
                    }
                    return;
                }
                eprintln!(
                    "tree: made src/probe.rs, on disk {}, open {:?}: {}",
                    app.root_path().is_some_and(|r| r.join("src/probe.rs").is_file()),
                    shell.get_active_tab(),
                    tree_said(&shell)
                );
                snapshot(&window, &dir.join("tree-new-file.png"));
                // Below the last row: the root's menu.
                let below = shell.get_tree().row_count() as i32 + 2;
                tree_click(&window, below, PointerEventButton::Right);
            }
            4107 if exploring => {
                eprintln!("tree: the root's menu: {}", menu_said(&shell));
                snapshot(&window, &dir.join("tree-menu-root.png"));
                shell.invoke_menu_picked("new-folder".into());
            }
            4108 if exploring => {
                if shell.get_tree_name_row() < 0 {
                    step.set(4108);
                    return;
                }
                for c in "docs".chars() {
                    key(c.to_string().into());
                }
                key(Key::Return.into());
                waited.set(0);
            }
            4109 if exploring => {
                if app.tree_row_of("docs").is_none() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(4109);
                    }
                    return;
                }
                eprintln!("tree: made docs/: {}", tree_said(&shell));
                if let Some(row) = app.tree_row_of("src/probe.rs") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            4110 if exploring => shell.invoke_menu_picked("rename".into()),
            4111 if exploring => {
                if shell.get_tree_name_row() < 0 {
                    step.set(4111);
                    return;
                }
                // The name before `.rs` is selected: typing replaces it.
                for c in "renamed".chars() {
                    key(c.to_string().into());
                }
                snapshot(&window, &dir.join("tree-rename.png"));
                key(Key::Return.into());
                waited.set(0);
            }
            4112 if exploring => {
                if shell.get_active_tab() != "file:src/renamed.rs" {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(4112);
                    }
                    return;
                }
                eprintln!(
                    "tree: renamed — the tab followed: {:?}; {}",
                    shell.get_tabs().iter().map(|t| t.id.to_string()).collect::<Vec<_>>(),
                    tree_said(&shell)
                );
                snapshot(&window, &dir.join("tree-renamed.png"));
                // A left click selects it and keeps the keys in the tree;
                // then the clipboard's keys.
                if let Some(row) = app.tree_row_of("src/renamed.rs") {
                    tree_click(&window, row, PointerEventButton::Left);
                }
            }
            4114 if exploring => {
                chord(&[Key::Control], "c".into());
                chord(&[Key::Control], "v".into());
                waited.set(0);
            }
            4115 if exploring => {
                if app.tree_row_of("src/renamed copy.rs").is_none() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(4115);
                    }
                    return;
                }
                eprintln!("tree: Ctrl+C, Ctrl+V beside it: {}", tree_said(&shell));
                snapshot(&window, &dir.join("tree-pasted.png"));
                // Edited, and not saved: the draft is to go with the move.
                app.open_file("src/renamed.rs".into());
            }
            4116 if exploring => {
                for c in "// moved with me".chars() {
                    key(c.to_string().into());
                }
                key(Key::Return.into());
            }
            4117 if exploring => {
                eprintln!(
                    "tree: typed into renamed.rs, dirty {}",
                    app.editor().is_dirty("src/renamed.rs")
                );
                // Carried by the pointer onto docs/.
                let (Some(from), Some(to)) = (app.tree_row_of("src/renamed.rs"), app.tree_row_of("docs")) else {
                    eprintln!("tree: rows to drag between are missing");
                    return;
                };
                let send = |event| window.window().dispatch_event(event);
                let (x, y0, y1) = (90.0, tree_row_y(from), tree_row_y(to));
                send(WindowEvent::PointerMoved { position: LogicalPosition::new(x, y0) });
                send(WindowEvent::PointerPressed {
                    position: LogicalPosition::new(x, y0),
                    button: PointerEventButton::Left,
                });
                for i in 1..=8 {
                    let y = y0 + (y1 - y0) * i as f32 / 8.0;
                    send(WindowEvent::PointerMoved { position: LogicalPosition::new(x + 2.0, y) });
                }
            }
            4118 if exploring => {
                eprintln!("tree: carried over docs/: {}", tree_said(&shell));
                snapshot(&window, &dir.join("tree-drag.png"));
                if let Some(to) = app.tree_row_of("docs") {
                    window.window().dispatch_event(WindowEvent::PointerReleased {
                        position: LogicalPosition::new(92.0, tree_row_y(to)),
                        button: PointerEventButton::Left,
                    });
                }
                waited.set(0);
            }
            4119 if exploring => {
                if !shell.get_tabs().iter().any(|t| t.id == "file:docs/renamed.rs") {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(4119);
                    }
                    return;
                }
                eprintln!(
                    "tree: dropped — tabs {:?}, in front {:?}, its first line {:?}, dirty {}, on disk {}: {}",
                    shell.get_tabs().iter().map(|t| t.id.to_string()).collect::<Vec<_>>(),
                    shell.get_active_tab(),
                    draft().and_then(|t| t.lines().next().map(str::to_string)),
                    app.editor().is_dirty("docs/renamed.rs"),
                    app.root_path().is_some_and(|r| r.join("docs/renamed.rs").is_file()),
                    tree_said(&shell)
                );
                snapshot(&window, &dir.join("tree-moved.png"));
                if let Some(row) = app.tree_row_of("src/renamed copy.rs") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            4120 if exploring => shell.invoke_menu_picked("cut".into()),
            4121 if exploring => {
                eprintln!("tree: cut, drawn faint: {}", tree_said(&shell));
                snapshot(&window, &dir.join("tree-cut.png"));
                if let Some(row) = app.tree_row_of("docs") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            4122 if exploring => {
                eprintln!("tree: docs' menu with something cut: {}", menu_said(&shell));
                shell.invoke_menu_picked("paste".into());
                waited.set(0);
            }
            4123 if exploring => {
                if app.tree_row_of("docs/renamed copy.rs").is_none() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(4123);
                    }
                    return;
                }
                eprintln!("tree: pasted into docs/: {}", tree_said(&shell));
                if let Some(row) = app.tree_row_of("docs/renamed.rs") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            4124 if exploring => {
                eprintln!("tree: a file's menu: {}", menu_said(&shell));
                snapshot(&window, &dir.join("tree-menu-file.png"));
                shell.invoke_menu_picked("delete".into());
                waited.set(0);
            }
            4125 if exploring => {
                if app.tree_row_of("docs/renamed.rs").is_some() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(4125);
                    }
                    return;
                }
                eprintln!(
                    "tree: deleted — tabs {:?}, selected {:?}: {}",
                    shell.get_tabs().iter().map(|t| t.id.to_string()).collect::<Vec<_>>(),
                    app.tree_selected(),
                    tree_said(&shell)
                );
                snapshot(&window, &dir.join("tree-deleted.png"));
                key(Key::UpArrow.into());
            }
            4126 if exploring => {
                eprintln!("tree: Up: {:?}", app.tree_selected());
                key(Key::LeftArrow.into());
            }
            4127 if exploring => {
                eprintln!("tree: Left: {:?} {}", app.tree_selected(), tree_said(&shell));
                key(Key::LeftArrow.into());
            }
            4128 if exploring => {
                eprintln!("tree: Left again: {:?} {}", app.tree_selected(), tree_said(&shell));
                key(Key::RightArrow.into());
            }
            4129 if exploring => {
                eprintln!("tree: Right: {}", tree_said(&shell));
                key(Key::DownArrow.into());
            }
            4130 if exploring => {
                eprintln!("tree: Down: {:?}", app.tree_selected());
                key(Key::F2.into());
            }
            4131 if exploring => {
                eprintln!(
                    "tree: F2: box on row {}, {:?}",
                    shell.get_tree_name_row(),
                    shell.get_tree_name()
                );
                snapshot(&window, &dir.join("tree-f2.png"));
                key(Key::Escape.into());
            }
            4132 if exploring => {
                eprintln!("tree: Escape: box on row {}: {}", shell.get_tree_name_row(), tree_said(&shell));
                if let Some(row) = app.tree_row_of("docs/renamed copy.rs") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            4133 if exploring => shell.invoke_menu_picked("copy-relative-path".into()),
            4134 if exploring => {
                let copied = arboard::Clipboard::new().and_then(|mut c| c.get_text());
                eprintln!("tree: Copy relative path: {copied:?}");
                if let Some(row) = app.tree_row_of("docs/renamed copy.rs") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            4135 if exploring => shell.invoke_menu_picked("copy-path".into()),
            4136 if exploring => {
                let copied = arboard::Clipboard::new().and_then(|mut c| c.get_text());
                eprintln!("tree: Copy path: {copied:?}");
                if let Some(row) = app.tree_row_of("docs/renamed copy.rs") {
                    tree_click(&window, row, PointerEventButton::Right);
                }
            }
            4137 if exploring => {
                shell.invoke_menu_picked("open-side".into());
                waited.set(0);
            }
            4138 if exploring => {
                if !shell.get_split() {
                    waited.set(waited.get() + 1);
                    if waited.get() < 250 {
                        step.set(4138);
                    }
                    return;
                }
                eprintln!(
                    "tree: Open to the side: split {}, keys on group {}, front {:?} of {:?}, beside {:?}",
                    shell.get_split(),
                    shell.get_focus_group(),
                    shell.get_active_tab(),
                    shell.get_tabs().iter().map(|t| t.id.to_string()).collect::<Vec<_>>(),
                    shell.get_shade_tabs().iter().map(|t| t.id.to_string()).collect::<Vec<_>>()
                );
                snapshot(&window, &dir.join("tree-open-side.png"));
                app.act("app.exit");
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

/// The marks of `kind` on the code view's rows: each row's line number and
/// where each of its marks starts.
fn drawn(code: &Code, kind: i32) -> Vec<(i32, Vec<f32>)> {
    code.get_rows()
        .iter()
        .filter_map(|row| {
            let xs: Vec<f32> = row
                .marks
                .iter()
                .filter(|m| m.kind == kind)
                .map(|m| m.x)
                .collect();
            (!xs.is_empty()).then_some((row.number, xs))
        })
        .collect()
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

/// The Calls panel as it reads: each row indented by its depth, with its
/// place, its count and whether it is open; or what it says instead.
fn calls_said(shell: &Shell) -> String {
    let rows: Vec<String> = shell
        .get_calls_rows()
        .iter()
        .map(|r| {
            format!(
                "{}{}{} {}{}{}",
                "  ".repeat(r.depth as usize),
                ["▸ ", "… ", "▾ ", "· "][r.open.clamp(0, 3) as usize],
                r.name,
                r.place,
                if r.count > 1 {
                    format!(" ×{}", r.count)
                } else {
                    String::new()
                },
                if r.recursive { " (recursive)" } else { "" }
            )
        })
        .collect();
    if rows.is_empty() {
        format!("{:?}", shell.get_calls_said().as_str())
    } else {
        format!(
            "{} {:?}",
            rows.join(" | "),
            shell.get_calls_empty().as_str()
        )
    }
}

/// The project tree as drawn: each row indented by its depth, a folder
/// with its chevron, and what marks it — selected, cut, lit for a drop, or
/// the box a name is typed in.
fn tree_said(shell: &Shell) -> String {
    shell
        .get_tree()
        .iter()
        .map(|r| {
            let mut marks = String::new();
            if r.selected {
                marks.push_str(" [selected]");
            }
            if r.cut {
                marks.push_str(" [cut]");
            }
            if r.drop {
                marks.push_str(" [drop]");
            }
            let name = if r.naming {
                format!("[box {:?}]", shell.get_tree_name().as_str())
            } else {
                r.name.to_string()
            };
            let chevron = match (r.dir, r.open) {
                (true, true) => "▾ ",
                (true, false) => "▸ ",
                _ => "",
            };
            format!("{}{chevron}{name}{marks}", "  ".repeat(r.depth as usize))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The right-click menu's rows, with their keys.
fn menu_said(shell: &Shell) -> String {
    shell
        .get_menu_items()
        .iter()
        .map(|item| {
            if item.id.is_empty() {
                "—".to_string()
            } else if item.chord.is_empty() {
                item.label.to_string()
            } else {
                format!("{} ({})", item.label, item.chord)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The middle of the tree's row `row`, in the window: under the title bar,
/// the left dock's tabs and the hairlines between them, 22 pixels a row.
fn tree_row_y(row: i32) -> f32 {
    40.0 + 1.0 + 34.0 + 1.0 + row as f32 * 22.0 + 11.0
}

/// A press and a release on the tree's row `row` (past the last: the
/// space below the rows).
fn tree_click(window: &AppWindow, row: i32, button: PointerEventButton) {
    let position = LogicalPosition::new(90.0, tree_row_y(row));
    let send = |event| window.window().dispatch_event(event);
    send(WindowEvent::PointerMoved { position });
    send(WindowEvent::PointerPressed { position, button });
    send(WindowEvent::PointerReleased { position, button });
}

/// What the terminal shows, a line a row.
fn screen_text(shell: &Shell) -> String {
    shell
        .get_term_rows()
        .iter()
        .map(|row| {
            row.spans
                .iter()
                .map(|s| s.text.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A key with modifiers held, sent to a window of its own as a hand presses
/// it: each modifier first and let go after.
fn torn_chord(torn: &crate::TornWindow, held: &[Key], text: &str) {
    let send = |event| torn.window().dispatch_event(event);
    for key in held {
        send(WindowEvent::KeyPressed {
            text: (*key).into(),
        });
    }
    send(WindowEvent::KeyPressed { text: text.into() });
    send(WindowEvent::KeyReleased { text: text.into() });
    for key in held.iter().rev() {
        send(WindowEvent::KeyReleased {
            text: (*key).into(),
        });
    }
}

/// The middle of the tab strips across the window: under the title bar and
/// its line, half a strip down.
const STRIP_Y: f32 = 41.0 + 18.0;

/// Where the editor area is across the window, in logical pixels: its left
/// edge and its width, from the docks either side.
fn editor_span(window: &AppWindow, shell: &Shell) -> (f32, f32) {
    let size = window
        .window()
        .size()
        .to_logical(window.window().scale_factor());
    let left = if shell.get_left_open() {
        shell.get_left_size() + 1.0
    } else {
        0.0
    };
    let right = if shell.get_right_open() {
        shell.get_right_size() + 1.0
    } else {
        0.0
    };
    (left, size.width - left - right)
}

/// Where the editor groups part, from the editor area's left edge: the
/// arithmetic `ui/pages.slint` places the divider by.
fn divider_at(width: f32, share: f32) -> f32 {
    let least = 220f32.min((width - 1.0) / 2.0);
    (width * share).round().clamp(least, width - 1.0 - least)
}

/// A pointer event, as the platform would deliver it.
fn pointer(window: &AppWindow, event: WindowEvent) {
    window.window().dispatch_event(event);
}

fn right_click(window: &AppWindow, at: LogicalPosition) {
    pointer(window, WindowEvent::PointerMoved { position: at });
    pointer(
        window,
        WindowEvent::PointerPressed {
            position: at,
            button: PointerEventButton::Right,
        },
    );
    pointer(
        window,
        WindowEvent::PointerReleased {
            position: at,
            button: PointerEventButton::Right,
        },
    );
}

/// The open menu's rows as they read, a divider as `—`.
fn menu_rows(shell: &Shell) -> Vec<String> {
    shell
        .get_menu_items()
        .iter()
        .map(|item| {
            if item.id.is_empty() {
                "—".to_string()
            } else if item.chord.is_empty() {
                item.label.to_string()
            } else {
                format!("{} ({})", item.label, item.chord)
            }
        })
        .collect()
}

/// Both groups' strips and what each has in front.
fn groups(shell: &Shell) -> String {
    let titles = |tabs: slint::ModelRc<crate::DocTab>| {
        tabs.iter().map(|t| t.title.to_string()).collect::<Vec<_>>()
    };
    format!(
        "split {} | keys {:?} front {:?} | other {:?} front {:?}",
        shell.get_split(),
        titles(shell.get_tabs()),
        shell.get_active_tab(),
        titles(shell.get_shade_tabs()),
        shell.get_shade_active()
    )
}

fn clipboard() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

fn snapshot(window: &AppWindow, path: &Path) {
    snapshot_of(window.window(), path);
}

/// [`snapshot`] of any of the process's windows.
fn snapshot_of(window: &slint::Window, path: &Path) {
    match window.take_snapshot() {
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
