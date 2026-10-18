//! `RUSTY_PROBE_HISTORY`, from step 7700: the History page worked by its
//! pointer and its keys. Each of its four dividers dragged — the refs, the
//! log against the commit, the message against the diff, the diff's two
//! columns — and one put back by a double-click, with what each left in
//! `workbench.toml`; a selection dragged in the opened commit's subject and
//! copied with Ctrl+C, and its menu's Copy message; a selection dragged in
//! each column of a side-by-side diff and copied, a word double-clicked,
//! Ctrl+A, the diff's menu, Shift and the wheel sideways, and the one
//! column copied without its markers; then the page in the left of two
//! editor groups, the groups' divider dragged until the page is 900, 550
//! and 300 pixels wide. Every pointer and key is dispatched as the platform
//! dispatches them, at places worked out from the page's own sizes, and
//! what each copy put on the clipboard is printed; the clipboard is put
//! back as it was found.
//!
//! On a repository whose second-newest commit, "Change a long line…",
//! changes a long line of a Markdown file and a tab-indented one, opened
//! with that file — `target/probe-book` is one.

use std::cell::RefCell;

use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};

use crate::{Shell, panes::fit, probe_surfaces::Run, work};

thread_local! {
    /// What the clipboard held before the run took it.
    static CLIPBOARD: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The row of the diff the long line is on.
    static CHANGED: RefCell<usize> = const { RefCell::new(0) };
}

/// The page's place in the window: the group it is in when split.
struct Page {
    x: f32,
    w: f32,
    top: f32,
    h: f32,
}

fn page(run: &Run) -> Page {
    let shell = run.window.global::<Shell>();
    let (_, height) = run.size();
    let left = run.editor_left();
    let area = run.editor_right() - left;
    let (x, w) = if shell.get_split() {
        let at = crate::probe_surfaces::divider_at(area, shell.get_split_share());
        if shell.get_focus_group() == 0 {
            (left, at)
        } else {
            (left + at + 1.0, area - at - 1.0)
        }
    } else {
        (left, area)
    };
    // Under the title bar's forty pixels and its line, the strip's 36.
    let top = 41.0 + 36.0;
    let bottom = if shell.get_bottom_open() {
        shell.get_bottom_size() + 1.0
    } else {
        0.0
    };
    Page {
        x,
        w,
        top,
        h: height - top - 24.0 - bottom,
    }
}

/// Where the page's panes are, by the arithmetic `ui/git.slint` lays them
/// out with (`panes::fit` over `WindowLayout.history`).
struct Panes {
    stacked: bool,
    refs_shown: bool,
    /// The refs divider's line.
    refs_x: f32,
    log_x: f32,
    log_w: f32,
    below_top: f32,
    log_h: f32,
    detail_top: f32,
    details_w: f32,
    diff_x: f32,
    diff_w: f32,
    diff_top: f32,
    left_w: f32,
}

impl Panes {
    fn of(run: &Run) -> Panes {
        let shell = run.window.global::<Shell>();
        let page = page(run);
        let refs_shown = page.w >= 140.0 + 1.0 + 360.0;
        let refs_w = fit(shell.get_history_refs(), page.w - 1.0, 140.0, 360.0);
        let log_x = page.x + if refs_shown { refs_w + 1.0 } else { 0.0 };
        let log_w = page.x + page.w - log_x;
        // The toolbar's forty pixels and its line.
        let below_top = page.top + 41.0;
        let below_h = page.h - 41.0;
        let log_h = fit(
            below_h * shell.get_history_log(),
            below_h - 1.0,
            80.0,
            140.0,
        );
        let detail_top = below_top + log_h + 1.0;
        let detail_h = below_h - log_h - 1.0;
        // Too narrow for both, the message and files sit above the diff.
        let stacked = log_w < 180.0 + 1.0 + 260.0;
        let details_w = if stacked {
            log_w
        } else {
            fit(
                log_w * shell.get_history_details(),
                log_w - 1.0,
                180.0,
                260.0,
            )
        };
        let (diff_x, diff_w, diff_y) = if stacked {
            (log_x, log_w, detail_top + (detail_h * 0.45).round() + 1.0)
        } else {
            (log_x + details_w + 1.0, log_w - details_w - 1.0, detail_top)
        };
        Panes {
            stacked,
            refs_shown,
            refs_x: page.x + refs_w,
            log_x,
            log_w,
            below_top,
            log_h,
            detail_top,
            details_w,
            diff_x,
            diff_w,
            // Under the one-or-two toggle's row.
            diff_top: diff_y + 36.0,
            left_w: fit(
                diff_w * shell.get_history_sides(),
                diff_w - 1.0,
                140.0,
                140.0,
            ),
        }
    }

    /// The middle of a row of the diff.
    fn row_y(&self, row: usize) -> f32 {
        self.diff_top + row as f32 * 18.0 + 9.0
    }

    /// Where the left column's text starts — or, in one column, the text's.
    fn left_text(&self, split: bool) -> f32 {
        if split {
            self.diff_x + 40.0 + 6.0
        } else {
            self.diff_x + 80.0 + 14.0 + 6.0
        }
    }

    fn right_text(&self) -> f32 {
        self.diff_x + self.left_w + 1.0 + 40.0 + 6.0
    }

    fn said(&self) -> String {
        format!(
            "refs {} at {}, log {}..+{} high {}, details {} wide {}, diff at {} {} wide, left column {}",
            if self.refs_shown { "shown" } else { "gone" },
            self.refs_x,
            self.log_x,
            self.log_w,
            self.log_h,
            self.details_w,
            if self.stacked {
                "above the diff"
            } else {
                "beside it"
            },
            self.diff_x,
            self.diff_w,
            self.left_w
        )
    }
}

fn at(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

/// The left button pressed at one place and let go at another, moving
/// through a point between, as a hand drags.
fn drag(run: &Run, from: LogicalPosition, to: LogicalPosition) {
    let button = PointerEventButton::Left;
    run.send(WindowEvent::PointerMoved { position: from });
    run.send(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    let middle = at((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    run.send(WindowEvent::PointerMoved { position: middle });
    run.send(WindowEvent::PointerMoved { position: to });
    run.send(WindowEvent::PointerReleased {
        position: to,
        button,
    });
}

fn double_click(run: &Run, position: LogicalPosition) {
    let button = PointerEventButton::Left;
    run.send(WindowEvent::PointerMoved { position });
    for _ in 0..2 {
        run.send(WindowEvent::PointerPressed { position, button });
        run.send(WindowEvent::PointerReleased { position, button });
    }
}

fn kept() -> String {
    let h = rusty_embed::config::workbench().layout.history;
    format!(
        "refs {} log {:.3} details {:.3} sides {:.3}",
        h.refs, h.log, h.details, h.sides
    )
}

fn shown(shell: &Shell) -> String {
    format!(
        "refs {} log {:.3} details {:.3} sides {:.3}",
        shell.get_history_refs(),
        shell.get_history_log(),
        shell.get_history_details(),
        shell.get_history_sides()
    )
}

/// A copy's text, line by line, short enough to read.
fn copied() -> String {
    let text = crate::probe_surfaces::read_clipboard()
        .unwrap_or_else(|error| format!("(the clipboard could not be read: {error})"));
    format!("{} lines {:?}", text.split('\n').count(), text)
}

/// A copy from the History page's diff: what the clipboard holds, and
/// whether that is what the page has selected.
fn copied_diff() -> String {
    let selected = work::app().diff_selection_text("history");
    let text = crate::probe_surfaces::read_clipboard()
        .unwrap_or_else(|error| format!("(the clipboard could not be read: {error})"));
    let verdict = if text == selected {
        "the selection, exactly".to_string()
    } else {
        format!("but the selection is {selected:?}")
    };
    format!("{} lines {text:?} — {verdict}", text.split('\n').count())
}

/// The left group dragged to `width` pixels, by its divider.
fn group_to(run: &Run, width: f32) {
    let shell = run.window.global::<Shell>();
    let left = run.editor_left();
    let area = run.editor_right() - left;
    let x = left + crate::probe_surfaces::divider_at(area, shell.get_split_share()) + 2.0;
    drag(run, at(x, 300.0), at(left + width + 2.0, 300.0));
}

/// `RUSTY_PROBE_HISTORY`, from step 7700.
pub fn history(n: u32, run: &Run) {
    let shell = run.window.global::<Shell>();
    let app = work::app();
    let panes = || Panes::of(run);
    match n {
        7700 => {
            CLIPBOARD.with(|c| {
                *c.borrow_mut() = Some(
                    crate::probe_surfaces::read_clipboard().unwrap_or_else(|error| {
                        format!("(the clipboard could not be read: {error})")
                    }),
                )
            });
            // The whole width for the page: the left dock closed.
            if shell.get_left_open() {
                app.act("dock.left");
            }
            app.act("git.history");
        }
        7701 => {
            run.until(
                shell.get_git_state() == 1 && shell.get_git_log().row_count() >= 3,
                500,
            );
        }
        7702 => {
            let log = shell.get_git_log();
            let Some(index) = log
                .iter()
                .position(|row| row.summary.starts_with("Change a long line"))
            else {
                eprintln!("history: no commit changing a long line in this repository");
                app.act("app.exit");
                return;
            };
            // Its row clicked, as a hand picks it.
            let p = panes();
            run.click(
                p.log_x + 200.0,
                p.below_top + index as f32 * 26.0 + 13.0,
                PointerEventButton::Left,
            );
        }
        7703 => {
            if run.until(shell.get_git_detail_rows().row_count() > 0, 300) && !shell.get_git_split()
            {
                app.act("git.split");
            }
        }
        7704 => {
            // A branch in the refs right-clicked: its menu, as before —
            // nothing the page gained took its pointer.
            let p = page(run);
            run.click(
                p.x + 60.0,
                p.top + 38.0 + 26.0 + 11.0,
                PointerEventButton::Right,
            );
        }
        7705 => {
            eprintln!("history: a branch's menu: {:?}", run.menu());
            shell.invoke_menu_dismissed();
        }
        7706 => {
            let p = panes();
            eprintln!("history: opened {:?}", shell.get_git_detail_summary());
            eprintln!("history: laid out {}", p.said());
            eprintln!("history: dividers {} — kept {}", shown(&shell), kept());
            let rows = shell.get_git_detail_rows();
            let changed = rows.iter().position(|row| row.kind == 2).unwrap_or(0);
            CHANGED.with(|c| *c.borrow_mut() = changed);
            let header = rows
                .row_data(0)
                .map(|r| r.text.to_string())
                .unwrap_or_default();
            eprintln!(
                "history: {} rows, the header {header:?} once per side, the long line on row {changed}",
                rows.row_count()
            );
            run.snapshot("history.png");
            // The refs divider, taken a pixel beside its line, 120 px right.
            let y = p.below_top + 100.0;
            drag(run, at(p.refs_x + 1.0, y), at(p.refs_x + 121.0, y));
        }
        7708 => {
            eprintln!("history: refs dragged: {} — kept {}", shown(&shell), kept());
            run.snapshot("refs-dragged.png");
            // The log's divider, 90 px up.
            let p = panes();
            let y = p.below_top + p.log_h;
            drag(
                run,
                at(p.log_x + 300.0, y + 1.0),
                at(p.log_x + 300.0, y - 89.0),
            );
        }
        7710 => {
            eprintln!("history: log dragged: {} — kept {}", shown(&shell), kept());
            run.snapshot("log-dragged.png");
            // The message's divider, 90 px right.
            let p = panes();
            let x = p.log_x + p.details_w;
            let y = p.detail_top + 100.0;
            drag(run, at(x + 1.0, y), at(x + 91.0, y));
        }
        7712 => {
            eprintln!(
                "history: details dragged: {} — kept {}",
                shown(&shell),
                kept()
            );
            run.snapshot("details-dragged.png");
            // The diff's centre line, 80 px left.
            let p = panes();
            let x = p.diff_x + p.left_w;
            let y = p.diff_top + 140.0;
            drag(run, at(x + 1.0, y), at(x - 79.0, y));
        }
        7714 => {
            eprintln!(
                "history: sides dragged: {} — kept {}",
                shown(&shell),
                kept()
            );
            eprintln!("history: laid out {}", panes().said());
            run.snapshot("sides-dragged.png");
            // The refs divider double-clicked: back at its default.
            let p = panes();
            double_click(run, at(p.refs_x - 1.0, p.below_top + 100.0));
        }
        7716 => {
            eprintln!(
                "history: refs double-clicked: {} — kept {}",
                shown(&shell),
                kept()
            );
            // The subject: a drag across its first words, then Ctrl+C.
            let p = panes();
            let y = p.detail_top + 8.0 + 9.0;
            drag(run, at(p.log_x + 9.0, y), at(p.log_x + 130.0, y));
            run.chord(&[Key::Control], "c");
        }
        7718 => {
            eprintln!("history: the subject dragged and Ctrl+C: {}", copied());
            run.snapshot("message-selected.png");
            let p = panes();
            let position = at(p.log_x + 40.0, p.detail_top + 8.0 + 9.0);
            run.click(position.x, position.y, PointerEventButton::Right);
        }
        7719 => {
            eprintln!("history: the subject's menu: {:?}", run.menu());
            run.snapshot("message-menu.png");
            run.pick("copy");
        }
        7720 => {
            eprintln!("history: its Copy: {}", copied());
            // The menu again, and its Copy message.
            let p = panes();
            run.click(
                p.log_x + 40.0,
                p.detail_top + 8.0 + 9.0,
                PointerEventButton::Right,
            );
            run.pick("copy-message");
        }
        7721 => {
            eprintln!("history: Copy message: {}", copied());
            // The left column: from forty pixels into the long line to a
            // hundred into the line three rows down, then Ctrl+C.
            let p = panes();
            let row = CHANGED.with(|c| *c.borrow());
            let x = p.left_text(true);
            drag(
                run,
                at(x + 40.0, p.row_y(row)),
                at(x + 100.0, p.row_y(row + 3)),
            );
            run.chord(&[Key::Control], "c");
        }
        7723 => {
            eprintln!(
                "history: the left column dragged and Ctrl+C: {}",
                copied_diff()
            );
            run.snapshot("diff-left-selected.png");
            let p = panes();
            let row = CHANGED.with(|c| *c.borrow());
            let x = p.right_text();
            drag(
                run,
                at(x + 40.0, p.row_y(row)),
                at(x + 100.0, p.row_y(row + 3)),
            );
            run.chord(&[Key::Control], "c");
        }
        7725 => {
            eprintln!(
                "history: the right column dragged and Ctrl+C: {}",
                copied_diff()
            );
            run.snapshot("diff-right-selected.png");
            // A word double-clicked: the first of the line two rows down.
            let p = panes();
            let row = CHANGED.with(|c| *c.borrow());
            double_click(run, at(p.right_text() + 10.0, p.row_y(row + 2)));
            run.chord(&[Key::Control], "c");
        }
        7727 => {
            eprintln!(
                "history: a word double-clicked and Ctrl+C: {}",
                copied_diff()
            );
            run.snapshot("diff-word.png");
            run.chord(&[Key::Control], "a");
            run.chord(&[Key::Control], "c");
        }
        7729 => {
            eprintln!("history: Ctrl+A and Ctrl+C on the right: {}", copied_diff());
            run.snapshot("diff-select-all.png");
            // The left column's menu: Copy, and Select all for that side.
            let p = panes();
            let row = CHANGED.with(|c| *c.borrow());
            run.click(
                p.left_text(true) + 20.0,
                p.row_y(row + 1),
                PointerEventButton::Right,
            );
        }
        7730 => {
            eprintln!("history: the diff's menu: {:?}", run.menu());
            run.snapshot("diff-menu.png");
            run.pick("select-all");
        }
        7731 => {
            run.chord(&[Key::Control], "c");
        }
        7732 => {
            eprintln!(
                "history: Select all on the left and Ctrl+C: {}",
                copied_diff()
            );
            run.chord(&[], Key::Escape);
            // Shift and the wheel over the diff: both columns' text sideways.
            let p = panes();
            let position = at(p.diff_x + 100.0, p.row_y(2));
            run.send(WindowEvent::PointerMoved { position });
            run.send(WindowEvent::KeyPressed {
                text: Key::Shift.into(),
            });
            for _ in 0..3 {
                run.send(WindowEvent::PointerScrolled {
                    position,
                    delta_x: 0.0,
                    delta_y: -120.0,
                });
            }
            run.send(WindowEvent::KeyReleased {
                text: Key::Shift.into(),
            });
        }
        7734 => {
            run.snapshot("diff-scrolled.png");
            // A selection made after the scroll starts where the text now is.
            let p = panes();
            let row = CHANGED.with(|c| *c.borrow());
            let x = p.left_text(true);
            drag(run, at(x + 10.0, p.row_y(row)), at(x + 90.0, p.row_y(row)));
            run.chord(&[Key::Control], "c");
        }
        7736 => {
            eprintln!(
                "history: scrolled 360 px sideways, then dragged: {}",
                copied_diff()
            );
            run.snapshot("diff-scrolled-selected.png");
            app.act("git.split");
        }
        7738 => {
            run.snapshot("unified.png");
            // One column: the long line removed, then added, then the line
            // after — copied without a marker.
            let p = panes();
            let row = shell
                .get_git_detail_rows()
                .iter()
                .position(|r| r.kind == 2)
                .unwrap_or(0);
            let x = p.left_text(false);
            drag(
                run,
                at(x + 30.0, p.row_y(row)),
                at(x + 200.0, p.row_y(row + 2)),
            );
            run.chord(&[Key::Control], "c");
        }
        7740 => {
            eprintln!("history: one column dragged and Ctrl+C: {}", copied_diff());
            run.snapshot("unified-selected.png");
            app.act("git.split");
            // Two groups: the chapter on the right, the History page on the
            // left beside it.
            let tabs = shell.get_tabs();
            if let Some(file) = tabs.iter().find(|t| t.title.ends_with(".md")) {
                shell.invoke_pick_tab(file.id);
            }
        }
        7742 => app.act("editor.split"),
        7744 => app.act("editor.split"),
        7746 => {
            let history = rusty_i18n::lookup("git.history");
            let tabs = shell.get_tabs();
            if let Some(page) = tabs.iter().find(|t| t.title.as_str() == history) {
                shell.invoke_pick_tab(page.id);
            }
        }
        7748 => group_to(run, 900.0),
        7750 => {
            eprintln!(
                "history: in the left group at {} px: {}",
                page(run).w,
                panes().said()
            );
            run.snapshot("narrow-900.png");
            group_to(run, 550.0);
        }
        7752 => {
            eprintln!(
                "history: in the left group at {} px: {}",
                page(run).w,
                panes().said()
            );
            run.snapshot("narrow-550.png");
            group_to(run, 300.0);
        }
        7754 => {
            eprintln!(
                "history: in the left group at {} px: {}",
                page(run).w,
                panes().said()
            );
            run.snapshot("narrow-300.png");
            group_to(run, 220.0);
        }
        7756 => {
            eprintln!(
                "history: in the left group at {} px: {}",
                page(run).w,
                panes().said()
            );
            run.snapshot("narrow-220.png");
        }
        7758 => {
            if let Some(text) = CLIPBOARD.with(|c| c.borrow_mut().take())
                && let Ok(mut clipboard) = arboard::Clipboard::new()
            {
                let _ = clipboard.set_text(text);
            }
            app.act("app.exit");
        }
        _ => {}
    }
}
