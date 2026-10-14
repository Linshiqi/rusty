//! A run that drives itself, for measuring and for looking: with
//! `RUSTY_PROBE=<dir>` the window types, scrolls and takes pictures of
//! itself, prints how long each frame took, and quits.
//!
//! It is the check a person at the keyboard cannot make: a key's latency is
//! the time from the event to the frame that shows it, read off the
//! renderer's own notifier, and a picture of the window is the window's own
//! snapshot, not a screen grab of whatever is in front of it.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use slint::{ComponentHandle, RenderingState, Timer, TimerMode, platform::WindowEvent};

use crate::AppWindow;

pub fn start(window: &AppWindow) {
    let Some(dir) = std::env::var_os("RUSTY_PROBE").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&dir).expect("the probe's directory");
    // When each frame finished, and when the event it is waiting on went in.
    let frames: Rc<RefCell<Vec<Instant>>> = Rc::default();
    let waiting: Rc<Cell<Option<Instant>>> = Rc::default();
    let latencies: Rc<RefCell<Vec<Duration>>> = Rc::default();
    {
        let (frames, waiting, latencies) = (frames.clone(), waiting.clone(), latencies.clone());
        window
            .window()
            .set_rendering_notifier(move |state, _| {
                if let RenderingState::AfterRendering = state {
                    let now = Instant::now();
                    frames.borrow_mut().push(now);
                    if let Some(sent) = waiting.take() {
                        latencies.borrow_mut().push(now - sent);
                    }
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
        let n = step.get();
        step.set(n + 1);
        match n {
            // Let the first frames settle.
            0..=19 => {}
            20 => snapshot(&window, &dir.join("open.png")),
            // The caret after the last character of a line of mixed scripts.
            21 => {
                let end: slint::SharedString = slint::platform::Key::End.into();
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyPressed { text: end.clone() });
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyReleased { text: end });
            }
            22 => snapshot(&window, &dir.join("end.png")),
            // Type a word a key at a time.
            23..=80 => {
                let text = ["f", "o", "o", "_", "b", "a", "r", " "][(n as usize) % 8];
                waiting.set(Some(Instant::now()));
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
            }
            81 => {
                report("typing: key to frame", &latencies.borrow());
                latencies.borrow_mut().clear();
                snapshot(&window, &dir.join("typed.png"));
            }
            // Scroll a screen at a time.
            82..=181 => {
                waiting.set(Some(Instant::now()));
                let row = window.get_row_height();
                window.set_scroll_y(-(row * 3.0 * (n - 81) as f32));
            }
            182 => {
                report("scrolling: step to frame", &latencies.borrow());
                snapshot(&window, &dir.join("scrolled.png"));
                let frames = frames.borrow();
                eprintln!("{} frames in all", frames.len());
                slint::quit_event_loop().ok();
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

fn snapshot(window: &AppWindow, path: &std::path::Path) {
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
