//! `RUSTY_PROBE_TABDRAG`, kept apart from `probe.rs`'s long list of steps:
//! a tab dragged the ways VS Code drags one, by the events a pointer makes,
//! on a copy of a project with a `Cargo.toml` (it writes files into it).
//!
//! It opens a dozen files, so the strip overflows; a click picks a tab and
//! a middle click closes one; a press carried two pixels is no drag and one
//! carried further is; a tab is moved along its strip — photographed with
//! the marker in its gap — and a drag is given up with Escape; held near
//! the lane's end the strip scrolls; a tab let go of over the right half of
//! the code opens the split with it there; the other group's front file,
//! scrolled to its end, is dragged onto the keys group's strip and arrives
//! where its view was with its caret; a tab goes onto the other group's
//! code, and the last tab of a group out of it ends the split; a tab is
//! dragged out beyond the window's edge into a window of its own, and
//! dragged from there back into a gap of the main window's strip and then,
//! out again, onto the right half of its code; last, with a page in one
//! group and code in the other, the divider is dragged to each end. What
//! the strips hold is printed at every stop, and the strip `workbench.toml`
//! kept at the end.

use std::cell::{Cell, RefCell};

use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};

use crate::{Shell, TornWindow, probe_surfaces::Run, work};

thread_local! {
    /// How many tabs the strip is to hold once every file has opened.
    static GOAL: Cell<usize> = const { Cell::new(0) };
    /// The strip as it was before a drag, to hold the drag to.
    static BEFORE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Where `long.txt`'s view was and its caret, before it moved groups.
    static VIEW: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
    static CARET: RefCell<String> = const { RefCell::new(String::new()) };
    /// Where the strip was scrolled before a tab was held near its end.
    static SCROLLED: Cell<f32> = const { Cell::new(0.0) };
    /// The tab a step pressed, and where the pointer is now.
    static HELD: RefCell<String> = const { RefCell::new(String::new()) };
    static AT: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
}

/// The files the run writes into the project and opens.
const NOTES: usize = 9;

fn send(run: &Run, event: WindowEvent) {
    run.window.window().dispatch_event(event);
}

fn left() -> PointerEventButton {
    PointerEventButton::Left
}

/// The left button pressed at `at`, the pointer there first.
fn press(run: &Run, at: (f32, f32)) {
    let position = LogicalPosition::new(at.0, at.1);
    send(run, WindowEvent::PointerMoved { position });
    send(
        run,
        WindowEvent::PointerPressed {
            position,
            button: left(),
        },
    );
    AT.set(at);
}

/// The pointer carried to `to` in steps, as a hand moves it.
fn carry(run: &Run, to: (f32, f32)) {
    let from = AT.get();
    for step in 1..=6 {
        let t = step as f32 / 6.0;
        let position =
            LogicalPosition::new(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        send(run, WindowEvent::PointerMoved { position });
    }
    AT.set(to);
}

/// The pointer put back where the probe carried it. The system moves it
/// too: a window shown, raised or moved under the real cursor is sent a
/// move to wherever that cursor is, and a held tab follows it — so before
/// every look and every release the probe's own place is said again.
fn hold(run: &Run) {
    let at = AT.get();
    send(
        run,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(at.0, at.1),
        },
    );
}

/// The same for a tab held from the first window of its own: the pointer
/// put back over the main window's `AT`, in that window's coordinates.
fn torn_hold() {
    let app = work::app();
    let torn = app.torn_windows();
    let to = AT.get();
    if let (Some((torn, _)), Some(at)) = (torn.first(), app.main_point_in_torn(0, to.0, to.1)) {
        torn.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(at.0, at.1),
        });
    }
}

fn release(run: &Run, button: PointerEventButton) {
    hold(run);
    let at = AT.get();
    send(
        run,
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(at.0, at.1),
            button,
        },
    );
}

/// A click of `button` at `at`.
fn click(run: &Run, at: (f32, f32), button: PointerEventButton) {
    let position = LogicalPosition::new(at.0, at.1);
    send(run, WindowEvent::PointerMoved { position });
    send(run, WindowEvent::PointerPressed { position, button });
    send(run, WindowEvent::PointerReleased { position, button });
    AT.set(at);
}

fn key(run: &Run, held: &[Key], text: slint::SharedString) {
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

/// A strip's tabs by their titles: the group with the keys', or the other's.
fn strip(shell: &Shell, other: bool) -> Vec<String> {
    let tabs = if other {
        shell.get_shade_tabs()
    } else {
        shell.get_tabs()
    };
    tabs.iter().map(|tab| tab.title.to_string()).collect()
}

/// The id of tab `index` of a strip.
fn id_of(shell: &Shell, other: bool, index: usize) -> String {
    let tabs = if other {
        shell.get_shade_tabs()
    } else {
        shell.get_tabs()
    };
    tabs.row_data(index)
        .map(|tab| tab.id.to_string())
        .unwrap_or_default()
}

/// A strip turned back to its start by the wheel over it, so the tabs a
/// step aims at are on screen.
fn to_start(run: &Run, other: bool) {
    let Some((x, y, _, _)) = work::app().group_pane(other) else {
        return;
    };
    let position = LogicalPosition::new(x + 60.0, y + 18.0);
    send(run, WindowEvent::PointerMoved { position });
    send(
        run,
        WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: 4000.0,
        },
    );
}

fn index_of(shell: &Shell, other: bool, title: &str) -> Option<usize> {
    strip(shell, other).iter().position(|t| t == title)
}

/// A point `share` of the way across tab `index` of a strip, on its middle
/// line — where it is drawn, the scroll counted.
fn into_tab(other: bool, index: usize, share: f32) -> Option<(f32, f32)> {
    let app = work::app();
    let (x, y) = app.tab_point(other, index)?;
    let width = app.doc_tab_width(other, index)?;
    Some((x - width / 2.0 + width * share, y))
}

/// A point of a group's code, `across` and `down` of the way into it.
fn into_code(other: bool, across: f32, down: f32) -> Option<(f32, f32)> {
    let (x, y, w, h) = work::app().group_pane(other)?;
    let strip = 36.0;
    Some((x + w * across, y + strip + (h - strip) * down))
}

/// A group's pane in the window: across, down, wide, tall.
type Pane = (f32, f32, f32, f32);

/// The groups' panes as they are on screen, the left one first.
fn panes() -> Option<(Pane, Pane)> {
    let app = work::app();
    let (a, b) = (app.group_pane(false)?, app.group_pane(true)?);
    Some(if a.0 < b.0 { (a, b) } else { (b, a) })
}

/// Where the divider between the groups is drawn.
fn divider() -> Option<f32> {
    let (left, _) = panes()?;
    Some(left.0 + left.2)
}

fn said(shell: &Shell) -> String {
    let drag = shell.get_tab_drag();
    format!(
        "on {} inside {} | marker {} at {:.0},{:.0} | overlay {} {:.0},{:.0} {:.0}x{:.0} | scroll strip {} dir {}",
        drag.on,
        drag.inside,
        drag.marker,
        drag.marker_x,
        drag.marker_y,
        drag.overlay,
        drag.overlay_x,
        drag.overlay_y,
        drag.overlay_w,
        drag.overlay_h,
        drag.scroll_strip,
        drag.scroll_dir
    )
}

fn groups(shell: &Shell) -> String {
    format!(
        "split {} keys on {} | keys {:?} front {:?} | other {:?} front {:?}",
        shell.get_split(),
        if shell.get_focus_group() == 1 {
            "right"
        } else {
            "left"
        },
        strip(shell, false),
        shell.get_active_tab(),
        strip(shell, true),
        shell.get_shade_active()
    )
}

fn verdict(ok: bool) -> &'static str {
    if ok { "ok" } else { "WRONG" }
}

/// Wait at this step until `ready`, for at most ten seconds.
fn wait_for(run: &Run, n: u32, ready: bool) -> bool {
    if ready {
        run.waited.set(0);
        return true;
    }
    run.waited.set(run.waited.get() + 1);
    if run.waited.get() < 250 {
        run.step.set(n);
        return false;
    }
    eprintln!("tabdrag: gave up waiting at step {n}");
    run.waited.set(0);
    true
}

fn photo(run: &Run, name: &str) {
    crate::probe::snapshot(run.window, &run.dir.join(name));
}

fn photo_torn(run: &Run, torn: &TornWindow, name: &str) {
    crate::probe::snapshot_of(torn.window(), &run.dir.join(name));
}

/// The first window of its own, pressed on its tab and carried to the
/// main window's `to` — through the screen, as a hand carries it.
fn torn_carry(torn: &TornWindow, to: (f32, f32)) -> Option<(f32, f32)> {
    let send = |event| torn.window().dispatch_event(event);
    let tab = LogicalPosition::new(40.0, 18.0);
    send(WindowEvent::PointerMoved { position: tab });
    send(WindowEvent::PointerPressed {
        position: tab,
        button: left(),
    });
    send(WindowEvent::PointerMoved {
        position: LogicalPosition::new(60.0, 24.0),
    });
    let at = work::app().main_point_in_torn(0, to.0, to.1)?;
    for step in 1..=6 {
        let t = step as f32 / 6.0;
        send(WindowEvent::PointerMoved {
            position: LogicalPosition::new(60.0 + (at.0 - 60.0) * t, 24.0 + (at.1 - 24.0) * t),
        });
    }
    Some(at)
}

/// The first window of its own put beside the main window rather than over
/// it, as a hand moves it aside: let go over itself, its tab stays.
fn aside(run: &Run) {
    let app = work::app();
    let torn = app.torn_windows();
    let Some((torn, _)) = torn.first() else {
        return;
    };
    let main = run.window.window();
    let corner = main.position();
    let width = main.size().width as i32;
    torn.window().set_position(slint::PhysicalPosition::new(
        corner.x + width + 40,
        corner.y,
    ));
}

/// The first window of its own, photographed and its tab let go of where
/// the pointer was carried to (`AT`, in the main window).
fn torn_release(run: &Run, name: &str) {
    let app = work::app();
    let torn = app.torn_windows();
    let Some((torn, _)) = torn.first() else {
        return;
    };
    photo_torn(run, torn, name);
    torn_hold();
    let to = AT.get();
    if let Some(at) = app.main_point_in_torn(0, to.0, to.1) {
        torn.window().dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(at.0, at.1),
            button: left(),
        });
    }
}

pub fn tab_drag(n: u32, run: &Run) {
    let window = run.window;
    let shell = window.global::<Shell>();
    let app = work::app();
    match n {
        7400 => {
            if !wait_for(run, n, shell.get_active_kind() == "file") {
                return;
            }
            let Some(root) = app.root_path() else { return };
            let mut names: Vec<String> = (1..=NOTES).map(|i| format!("t{i:02}.txt")).collect();
            for name in &names {
                let _ = std::fs::write(root.join(name), format!("{name}\n"));
            }
            let long: String = (1..=300)
                .map(|i| format!("line {i} of a long file\n"))
                .collect();
            let _ = std::fs::write(root.join("long.txt"), long);
            names.push("long.txt".into());
            names.push("Cargo.toml".into());
            names.push("examples/cross.rs".into());
            GOAL.set(shell.get_tabs().row_count() + names.len());
            for name in names {
                app.open_file(name);
            }
        }
        7401 => {
            if !wait_for(run, n, shell.get_tabs().row_count() >= GOAL.get()) {
                return;
            }
            eprintln!("tabdrag: opened: {}", groups(&shell));
            if let Some([(shown, scroll), _]) = app.strip_lanes() {
                let widths: f32 = (0..shell.get_tabs().row_count())
                    .filter_map(|i| app.doc_tab_width(false, i))
                    .sum();
                eprintln!(
                    "tabdrag: the strip's lane shows {shown:.0} of {widths:.0}, scrolled {scroll:.0}"
                );
            }
            photo(run, "tabdrag-open.png");
            shell.invoke_pick_tab("file:long.txt".into());
        }
        // long.txt at its end, so its view and caret have somewhere to be
        // carried from.
        7403 => {
            window.invoke_focus_code();
            key(run, &[Key::Control], Key::End.into());
        }
        7406 => {
            VIEW.set(app.editor().scroll());
            CARET.replace(shell.get_caret().to_string());
            eprintln!(
                "tabdrag: long.txt at its end: view {:?}, caret {:?}",
                VIEW.get(),
                CARET.with_borrow(|c| c.clone())
            );
            // A click on the third tab picks it.
            if let Some(at) = into_tab(false, 2, 0.5) {
                click(run, at, left());
            }
        }
        7408 => {
            let third = strip(&shell, false).get(2).cloned().unwrap_or_default();
            eprintln!(
                "tabdrag: a click on {third:?}: in front {:?} — {}",
                shell.get_active_tab(),
                verdict(shell.get_active_tab().ends_with(&*third))
            );
            // A middle click on the fourth closes it.
            BEFORE.replace(strip(&shell, false));
            if let Some(at) = into_tab(false, 3, 0.5) {
                click(run, at, PointerEventButton::Middle);
            }
        }
        7410 => {
            let before = BEFORE.with_borrow(|b| b.clone());
            let after = strip(&shell, false);
            let mut expected = before.clone();
            expected.remove(3);
            eprintln!(
                "tabdrag: a middle click on {:?}: {:?} — {}",
                before.get(3),
                after,
                verdict(after == expected)
            );
            // Along the strip: the first tab pressed, carried two pixels —
            // no drag yet — then on to the fourth tab's right half.
            BEFORE.replace(after);
            to_start(run, false);
            HELD.replace(id_of(&shell, false, 0));
            if let Some(at) = into_tab(false, 0, 0.5) {
                press(run, at);
                carry(run, (at.0 + 2.0, at.1));
            }
            eprintln!(
                "tabdrag: carried two pixels: dragging {} — {}",
                shell.get_tab_drag().on,
                verdict(!shell.get_tab_drag().on)
            );
            if let Some(at) = into_tab(false, 3, 0.7) {
                carry(run, at);
            }
        }
        7413 => {
            hold(run);
            eprintln!(
                "tabdrag: along the strip, held over the fourth tab's right half: {}",
                said(&shell)
            );
            photo(run, "tabdrag-reorder-marker.png");
            release(run, left());
        }
        7415 => {
            let before = BEFORE.with_borrow(|b| b.clone());
            let after = strip(&shell, false);
            let mut expected = before.clone();
            let first = expected.remove(0);
            expected.insert(3, first.clone());
            let front = shell.get_active_tab() == HELD.with_borrow(|h| h.clone()).as_str();
            eprintln!(
                "tabdrag: let go: {after:?}, {first:?} in front {front} — {}",
                verdict(after == expected && front)
            );
            photo(run, "tabdrag-reordered.png");
            // Given up with Escape.
            BEFORE.replace(after);
            to_start(run, false);
            if let (Some(from), Some(to)) = (into_tab(false, 1, 0.5), into_tab(false, 4, 0.7)) {
                press(run, from);
                carry(run, to);
            }
        }
        7417 => {
            hold(run);
            eprintln!("tabdrag: held for Escape: {}", said(&shell));
            key(run, &[], Key::Escape.into());
        }
        7418 => {
            eprintln!("tabdrag: Escape pressed: {}", said(&shell));
            photo(run, "tabdrag-escape.png");
            release(run, left());
        }
        7420 => {
            let before = BEFORE.with_borrow(|b| b.clone());
            let after = strip(&shell, false);
            eprintln!(
                "tabdrag: let go after Escape: {after:?} — {}",
                verdict(after == before)
            );
            // The strip back at its start, then a tab held near its end.
            let (x, y, _, _) = app.group_pane(false).unwrap_or((300.0, 41.0, 0.0, 0.0));
            let position = LogicalPosition::new(x + 100.0, y + 18.0);
            send(run, WindowEvent::PointerMoved { position });
            send(
                run,
                WindowEvent::PointerScrolled {
                    position,
                    delta_x: 0.0,
                    delta_y: 4000.0,
                },
            );
        }
        7422 => {
            let Some([(shown, scroll), _]) = app.strip_lanes() else {
                return;
            };
            SCROLLED.set(scroll);
            let Some((x, y, _, _)) = app.group_pane(false) else {
                return;
            };
            if let Some(from) = into_tab(false, 0, 0.5) {
                press(run, from);
                carry(run, (x + shown - 10.0, y + 18.0));
            }
        }
        // Held there for a second: the lane scrolls under it.
        7423..=7447 => hold(run),
        7448 => {
            hold(run);
            let scroll = app.strip_lanes().map_or(0.0, |[(_, scroll), _]| scroll);
            eprintln!(
                "tabdrag: held near the lane's end for a second: scrolled {:.0} -> {scroll:.0} — {}; {}",
                SCROLLED.get(),
                verdict(scroll > SCROLLED.get()),
                said(&shell)
            );
            photo(run, "tabdrag-autoscroll.png");
            key(run, &[], Key::Escape.into());
            release(run, left());
        }
        7450 => {
            // Back to the start, and long.txt in front again.
            let (x, y, _, _) = app.group_pane(false).unwrap_or((300.0, 41.0, 0.0, 0.0));
            let position = LogicalPosition::new(x + 100.0, y + 18.0);
            send(
                run,
                WindowEvent::PointerScrolled {
                    position,
                    delta_x: 0.0,
                    delta_y: 4000.0,
                },
            );
            shell.invoke_pick_tab("file:long.txt".into());
        }
        // The split: a tab other than long.txt, let go of over the right
        // half of the code.
        7453 => {
            let Some((x, _, w, _)) = app.group_pane(false) else {
                return;
            };
            let tabs = strip(&shell, false);
            let chosen = (0..tabs.len()).find(|&i| {
                tabs[i] != "long.txt"
                    && app
                        .tab_point(false, i)
                        .is_some_and(|(at, _)| at > x + 20.0 && at < x + w - 80.0)
            });
            let (Some(i), Some(to)) = (chosen, into_code(false, 0.78, 0.5)) else {
                return;
            };
            HELD.replace(tabs[i].clone());
            if let Some(from) = into_tab(false, i, 0.5) {
                press(run, from);
                carry(run, to);
            }
        }
        7455 => {
            hold(run);
            eprintln!(
                "tabdrag: {:?} over the code's right half: {}",
                HELD.with_borrow(|h| h.clone()),
                said(&shell)
            );
            photo(run, "tabdrag-split-overlay.png");
            release(run, left());
        }
        7458 => {
            let held = HELD.with_borrow(|h| h.clone());
            let ok = shell.get_split()
                && strip(&shell, false) == [held.clone()]
                && shell.get_shade_active() == "file:long.txt";
            let shade = app.editor().shade_scroll();
            eprintln!(
                "tabdrag: let go over the right half: {} — {}; long.txt's view beside it {shade:?} (was {:?}) — {}",
                groups(&shell),
                verdict(ok),
                VIEW.get(),
                verdict(shade == VIEW.get())
            );
            photo(run, "tabdrag-split.png");
            // long.txt from the other group onto the keys group's strip,
            // before its one tab.
            let (Some(at), Some(to)) = (
                index_of(&shell, true, "long.txt").and_then(|i| into_tab(true, i, 0.5)),
                into_tab(false, 0, 0.2),
            ) else {
                return;
            };
            press(run, at);
            carry(run, to);
        }
        7460 => {
            hold(run);
            eprintln!(
                "tabdrag: long.txt over the other strip's first gap: {}",
                said(&shell)
            );
            photo(run, "tabdrag-to-strip.png");
            release(run, left());
        }
        7463 => {
            let held = HELD.with_borrow(|h| h.clone());
            let view = app.editor().scroll();
            let caret = shell.get_caret().to_string();
            let ok = strip(&shell, false) == ["long.txt".to_string(), held]
                && shell.get_active_tab() == "file:long.txt";
            eprintln!(
                "tabdrag: long.txt moved groups: {} — {}; its view {view:?} (was {:?}) — {}, caret {caret:?} (was {:?}) — {}",
                groups(&shell),
                verdict(ok),
                VIEW.get(),
                verdict(view == VIEW.get()),
                CARET.with_borrow(|c| c.clone()),
                verdict(CARET.with_borrow(|c| *c == caret))
            );
            photo(run, "tabdrag-moved.png");
            // The other tab of the keys group onto the other group's code.
            let held = HELD.with_borrow(|h| h.clone());
            let (Some(at), Some(to)) = (
                index_of(&shell, false, &held).and_then(|i| into_tab(false, i, 0.5)),
                into_code(true, 0.5, 0.5),
            ) else {
                return;
            };
            BEFORE.replace(strip(&shell, true));
            press(run, at);
            carry(run, to);
        }
        7465 => {
            hold(run);
            eprintln!("tabdrag: over the other group's code: {}", said(&shell));
            photo(run, "tabdrag-to-code.png");
            release(run, left());
        }
        7468 => {
            let held = HELD.with_borrow(|h| h.clone());
            let mut expected = BEFORE.with_borrow(|b| b.clone());
            expected.push(held.clone());
            let ok = strip(&shell, false) == expected && shell.get_active_tab().ends_with(&*held);
            eprintln!(
                "tabdrag: let go on its code: {} — {}",
                groups(&shell),
                verdict(ok)
            );
            // long.txt, the other group's one tab, onto this group's strip:
            // the group it leaves is empty, and the split ends.
            to_start(run, false);
            let (Some(at), Some(to)) = (
                index_of(&shell, true, "long.txt").and_then(|i| into_tab(true, i, 0.5)),
                into_tab(false, 0, 0.2),
            ) else {
                return;
            };
            press(run, at);
            carry(run, to);
        }
        7470 => release(run, left()),
        7472 => {
            eprintln!(
                "tabdrag: a group's last tab moved out of it: {} — {}",
                groups(&shell),
                verdict(
                    !shell.get_split()
                        && strip(&shell, false)
                            .first()
                            .is_some_and(|t| t == "long.txt")
                )
            );
            photo(run, "tabdrag-unsplit.png");
            // long.txt beyond the window's left edge.
            to_start(run, false);
            if let Some(at) = into_tab(false, 0, 0.5) {
                press(run, at);
                carry(run, (-60.0, at.1 + 30.0));
            }
        }
        7474 => {
            hold(run);
            eprintln!("tabdrag: beyond the window's edge: {}", said(&shell));
            release(run, left());
        }
        7476 => {
            let torn = app.torn_windows();
            if !wait_for(run, n, !torn.is_empty()) {
                return;
            }
            eprintln!(
                "tabdrag: let go beyond the edge: {} window(s) of its own, its tab {:?}; {}",
                torn.len(),
                torn.first().map(|(w, _)| w.get_name().to_string()),
                groups(&shell)
            );
            photo(run, "tabdrag-out.png");
            if let Some((torn, _)) = torn.first() {
                photo_torn(run, torn, "tabdrag-out-torn.png");
            }
            aside(run);
        }
        // From the window of its own back into a gap of the main window's
        // strip: before the second tab.
        7480 => {
            to_start(run, false);
            let torn = app.torn_windows();
            let (Some((torn, _)), Some(to)) = (torn.first(), into_tab(false, 1, 0.2)) else {
                return;
            };
            BEFORE.replace(strip(&shell, false));
            let at = torn_carry(torn, to);
            AT.set(to);
            eprintln!(
                "tabdrag: the window of its own's tab carried to {at:?} of it, over the main window's {to:?}"
            );
        }
        7482 => {
            torn_hold();
            eprintln!(
                "tabdrag: from a window of its own, over the strip's second gap: {}",
                said(&shell)
            );
            photo(run, "tabdrag-back-marker.png");
            torn_release(run, "tabdrag-back-torn.png");
        }
        7485 => {
            let mut expected = BEFORE.with_borrow(|b| b.clone());
            expected.insert(1, "long.txt".into());
            let ok = app.torn_windows().is_empty()
                && strip(&shell, false) == expected
                && shell.get_active_tab() == "file:long.txt";
            eprintln!(
                "tabdrag: let go there: {} window(s) of its own; {} — {}; its view {:?} — {}",
                app.torn_windows().len(),
                groups(&shell),
                verdict(ok),
                app.editor().scroll(),
                verdict(app.editor().scroll() == VIEW.get())
            );
            photo(run, "tabdrag-back.png");
            // Out again from its tab's menu, to come back as the split.
            shell.invoke_tab_menu("file:long.txt".into(), false, 400.0, 58.0);
            shell.invoke_menu_picked("editor.detach".into());
        }
        7487 => {
            if !wait_for(run, n, !app.torn_windows().is_empty()) {
                return;
            }
            aside(run);
        }
        7488 => {
            let torn = app.torn_windows();
            let (Some((torn, _)), Some(to)) = (torn.first(), into_code(false, 0.8, 0.5)) else {
                return;
            };
            torn_carry(torn, to);
            AT.set(to);
        }
        7490 => {
            torn_hold();
            eprintln!(
                "tabdrag: from a window of its own, over the code's right half: {}",
                said(&shell)
            );
            photo(run, "tabdrag-back-split-overlay.png");
            torn_release(run, "tabdrag-back-split-torn.png");
        }
        7492 => {
            let ok = app.torn_windows().is_empty()
                && shell.get_split()
                && strip(&shell, false) == ["long.txt"]
                && shell.get_focus_group() == 1;
            eprintln!(
                "tabdrag: let go there: {} window(s) of its own; {} — {}",
                app.torn_windows().len(),
                groups(&shell),
                verdict(ok)
            );
            photo(run, "tabdrag-back-split.png");
            // A page in one group and code in the other: the History page,
            // opened in the right group, dragged onto the left one's strip.
            app.act("git.history");
        }
        7494 => {
            if !wait_for(run, n, shell.get_active_tab() == "page:git") {
                return;
            }
            let history = shell.get_tabs().iter().position(|tab| tab.id == "page:git");
            let (Some(at), Some(to)) = (
                history.and_then(|i| into_tab(false, i, 0.5)),
                into_code(true, 0.5, 0.5),
            ) else {
                return;
            };
            press(run, at);
            carry(run, to);
        }
        7496 => {
            hold(run);
            eprintln!(
                "tabdrag: the History page over the other group's code: {}",
                said(&shell)
            );
            release(run, left());
        }
        7499 => {
            eprintln!(
                "tabdrag: a page moved groups: {} — {}",
                groups(&shell),
                verdict(shell.get_active_tab() == "page:git" && shell.get_focus_group() == 0)
            );
        }
        // The divider dragged to each end, once the History page has read
        // the log, and what each group draws.
        7520 => {
            photo(run, "split-history-code.png");
            let (Some(x), Some(((px, py, _, h), _))) = (divider(), panes()) else {
                return;
            };
            let y = py + h / 2.0;
            press(run, (x + 0.5, y));
            carry(run, (px + 2.0, y));
            release(run, left());
        }
        7523 => {
            eprintln!(
                "tabdrag: the divider at the left end: share {:.3} | the panes, left and right: {:?}",
                shell.get_split_share(),
                panes()
            );
            photo(run, "split-tight-left.png");
            let (Some(x), Some((_, (px, py, pw, ph)))) = (divider(), panes()) else {
                return;
            };
            let y = py + ph / 2.0;
            press(run, (x + 0.5, y));
            carry(run, (px + pw - 2.0, y));
            release(run, left());
        }
        7526 => {
            eprintln!(
                "tabdrag: the divider at the right end: share {:.3} | the panes, left and right: {:?}",
                shell.get_split_share(),
                panes()
            );
            photo(run, "split-tight-right.png");
        }
        7528 => {
            let strip = app
                .root_path()
                .and_then(|root| rusty_embed::config::tabs_for(&root.to_string_lossy()));
            eprintln!("tabdrag: the strip workbench.toml kept: {strip:?}");
            app.act("app.exit");
        }
        _ => {}
    }
}
