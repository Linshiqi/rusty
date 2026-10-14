//! A run that drives itself, for measuring and for looking: with
//! `RUSTY_PROBE=<dir>` the window photographs itself in each of its states,
//! types and scrolls in the open file, prints how long each frame took, and
//! quits.
//!
//! It is the check a person at the keyboard cannot make: a key's latency is
//! the time from the event to the frame that shows it, read off the
//! renderer's own notifier, and a picture of the window is the window's own
//! snapshot, not a screen grab of whatever is in front of it.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use slint::{ComponentHandle, RenderingState, Timer, TimerMode, platform::WindowEvent};

use crate::{AppWindow, Code, Shell, work};

pub fn start(window: &AppWindow) {
    let Some(dir) = std::env::var_os("RUSTY_PROBE").map(PathBuf::from) else {
        return;
    };
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
        let key = |text: slint::SharedString| {
            window
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            window
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        };
        match n {
            25 => snapshot(&window, &dir.join("open.png")),
            41 => key(slint::platform::Key::End.into()),
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
            103..=162 => {
                waiting.set(Some(Instant::now()));
                let row = code.get_row_height();
                code.set_scroll_y(-(row * 3.0 * (n - 102) as f32));
            }
            163 => {
                report("scrolling: step to frame", &latencies.borrow());
                app.act("dock.bottom");
                app.act("dock.right");
            }
            166 => snapshot(&window, &dir.join("docks.png")),
            167 => app.act("palette.open"),
            170 => snapshot(&window, &dir.join("palette.png")),
            171 => {
                shell.set_picker_open(false);
                app.act("settings.open");
                app.act("theme.light");
            }
            174 => snapshot(&window, &dir.join("light.png")),
            175 => {
                app.act("theme.system");
                app.act("dock.bottom");
                app.act("dock.right");
                app.act("welcome.open");
            }
            178 => snapshot(&window, &dir.join("welcome.png")),
            179 => {
                let _ = slint::quit_event_loop();
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
