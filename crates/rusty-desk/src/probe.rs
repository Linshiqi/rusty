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
//! rustfmt formatted what reached the disk — point it at a copy.

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
            25 => snapshot(&window, &dir.join("open.png")),
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
            // Back to the file, and with RUSTY_PROBE_SAVES a save through
            // rustfmt.
            190 => {
                let file = shell.get_tabs().iter().find(|tab| tab.kind == "file");
                if let Some(tab) = file {
                    shell.invoke_pick_tab(tab.id);
                }
            }
            191 if saves => {
                chord(&[Key::Control], "a".into());
                code.invoke_typed("fn  main( ){let x=1;}\n".into());
                chord(&[Key::Control], "s".into());
            }
            // rustfmt has a second; the write follows it.
            235 if saves => {
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
            260 => {
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
