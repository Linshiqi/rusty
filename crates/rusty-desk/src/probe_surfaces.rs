//! Two of the probe's runs, kept apart from `probe.rs`'s long list of steps:
//! with `RUSTY_PROBE_ZOOM` it turns the wheel with Ctrl held over the code,
//! a Markdown page and the terminal, and says what each text's size became,
//! what the file kept and that a plain wheel still scrolls; with
//! `RUSTY_PROBE_MENUS` it right-clicks Output, Problems, Search, the
//! terminal, a Markdown page and a dock's tab, photographs each menu, uses a
//! row of it and says what the row did. Both drive the window with the
//! events a pointer makes — Ctrl pressed, the pointer moved, the wheel
//! turned, the right button pressed — at places worked out from the
//! layout's own sizes; where a place cannot be worked out (a search hit
//! under fields of the panel's own height) the surface's callback is called
//! as the click would call it.
//!
//! Both need a project with a `README.md` whose first paragraph begins with
//! a link short enough to stay on one line (Slint hears a link wrapped onto
//! two only on its last), to `https://example.invalid/rusty-probe` — never
//! followed: the probe takes the window's link handler for its own — and
//! the menus run takes the clipboard for a while and puts back what it held.

use std::{cell::RefCell, path::Path};

use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};

use crate::{AppWindow, Code, Shell, work};

thread_local! {
    /// The link the page's click reached, which the probe hears instead of
    /// the browser.
    static LINK: RefCell<Option<String>> = const { RefCell::new(None) };
    /// What the clipboard held before the menus run took it.
    static CLIPBOARD: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// What a step needs of the probe.
pub struct Run<'a> {
    pub window: &'a AppWindow,
    pub dir: &'a Path,
    pub step: &'a std::cell::Cell<u32>,
    pub waited: &'a std::cell::Cell<u32>,
}

impl Run<'_> {
    fn send(&self, event: WindowEvent) {
        self.window.window().dispatch_event(event);
    }

    /// The size the window is laid out at, in the pixels events are sent
    /// in — not the system's size over its scale, which a window whose
    /// scale was set from outside need not share.
    fn size(&self) -> (f32, f32) {
        (self.window.get_laid_width(), self.window.get_laid_height())
    }

    /// Where the editor area starts across, past the left dock.
    fn editor_left(&self) -> f32 {
        let shell = self.window.global::<Shell>();
        if shell.get_left_open() {
            shell.get_left_size() + 1.0
        } else {
            0.0
        }
    }

    /// Where the editor area ends across, before the right dock.
    fn editor_right(&self) -> f32 {
        let shell = self.window.global::<Shell>();
        let (width, _) = self.size();
        if shell.get_right_open() {
            width - shell.get_right_size() - 1.0
        } else {
            width
        }
    }

    /// Where the bottom dock's panel starts down: under the title bar's
    /// forty pixels the area runs to the status bar's twenty-four, the dock
    /// at its foot and its strip of tabs thirty-five tall.
    fn bottom_panel_top(&self) -> f32 {
        let (_, height) = self.size();
        height - 24.0 - self.window.global::<Shell>().get_bottom_size() + 35.0
    }

    /// The wheel turned at a point, with Ctrl held when `control`.
    fn wheel(&self, x: f32, y: f32, delta: f32, control: bool) {
        let position = LogicalPosition::new(x, y);
        if control {
            self.send(WindowEvent::KeyPressed {
                text: Key::Control.into(),
            });
        }
        self.send(WindowEvent::PointerMoved { position });
        self.send(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: delta,
        });
        if control {
            self.send(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
        }
    }

    /// A button pressed and let go at a point.
    fn click(&self, x: f32, y: f32, button: PointerEventButton) {
        let position = LogicalPosition::new(x, y);
        self.send(WindowEvent::PointerMoved { position });
        self.send(WindowEvent::PointerPressed { position, button });
        self.send(WindowEvent::PointerReleased { position, button });
    }

    fn snapshot(&self, name: &str) {
        crate::probe::snapshot(self.window, &self.dir.join(name));
    }

    /// Wait at this step until `ready`, at most `ticks` times.
    fn until(&self, ready: bool, ticks: u32) -> bool {
        if ready || self.waited.get() >= ticks {
            self.waited.set(0);
            return true;
        }
        self.waited.set(self.waited.get() + 1);
        self.step.set(self.step.get() - 1);
        false
    }

    /// The open menu's rows, as they read.
    fn menu(&self) -> Vec<String> {
        let shell = self.window.global::<Shell>();
        if !shell.get_menu_open() {
            return vec!["(no menu)".to_string()];
        }
        shell
            .get_menu_items()
            .iter()
            .map(|item| {
                if item.id.is_empty() {
                    "—".to_string()
                } else {
                    format!("{} [{}]", item.label, item.id)
                }
            })
            .collect()
    }

    /// A row of the open menu, picked as its click picks it.
    fn pick(&self, id: &str) {
        self.window.global::<Shell>().invoke_menu_picked(id.into());
    }
}

fn clipboard() -> String {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .unwrap_or_default()
}

/// The first problem's row in Problems: one in a file, or any.
fn problem_row(shell: &Shell, in_a_file: bool) -> Option<usize> {
    shell
        .get_problems()
        .iter()
        .position(|row| row.kind == 1 && (!in_a_file || !row.path.is_empty()))
}

fn code_size(code: &Code) -> String {
    format!(
        "font {} row {} gutter {}",
        code.get_font_size(),
        code.get_row_height(),
        code.get_gutter()
    )
}

/// `RUSTY_PROBE_ZOOM`, from step 6100.
pub fn zoom(n: u32, run: &Run) {
    let shell = run.window.global::<Shell>();
    let code = run.window.global::<Code>();
    let app = work::app();
    let left = run.editor_left();
    let (_, height) = run.size();
    let saved = || {
        let layout = rusty_embed::config::workbench().layout;
        (layout.editor_zoom, layout.page_zoom, layout.terminal_zoom)
    };
    match n {
        6100 => {
            let physical = run.window.window().size();
            eprintln!(
                "zoom: the window {}x{} pixels at a scale of {}, laid out at {:?}",
                physical.width,
                physical.height,
                run.window.window().scale_factor(),
                run.size()
            );
            eprintln!("zoom: the editor at first: {}", code_size(&code));
            // Two notches up, over the code.
            run.wheel(left + 300.0, 160.0, 120.0, true);
        }
        6102 => {
            eprintln!(
                "zoom: after Ctrl and two notches up: {}; the file keeps {:?}",
                code_size(&code),
                saved()
            );
            run.snapshot("zoom-editor.png");
            eprintln!(
                "zoom: scrolled {} before a plain wheel",
                code.get_scroll_y()
            );
            run.wheel(left + 300.0, 160.0, -120.0, false);
        }
        6110 => {
            eprintln!(
                "zoom: a plain wheel down: scrolled {}, {}",
                code.get_scroll_y(),
                code_size(&code)
            );
            // Five notches down, a touchpad's way: twenty small turns.
            for _ in 0..20 {
                run.wheel(left + 300.0, 160.0, -15.0, true);
            }
        }
        6112 => {
            eprintln!(
                "zoom: after five notches down in small turns: {}",
                code_size(&code)
            );
            run.snapshot("zoom-editor-small.png");
            app.act("editor.zoom-reset");
        }
        6114 => {
            eprintln!(
                "zoom: Reset editor font zoom: {}; the file keeps {:?}",
                code_size(&code),
                saved()
            );
            // Ctrl at once after a plain scroll, the pointer still: Slint's
            // list keeps a wheel gesture it began for a moment.
            run.wheel(left + 300.0, 200.0, -60.0, false);
            run.wheel(left + 300.0, 200.0, 60.0, true);
        }
        6116 => {
            eprintln!(
                "zoom: Ctrl straight after a plain scroll: {}",
                code_size(&code)
            );
            app.act("editor.zoom-reset");
            app.open_file("README.md".into());
        }
        6118 => {
            if !run.until(shell.get_md_page(), 250) {
                return;
            }
            let middle = (left + run.editor_right()) / 2.0;
            run.wheel(middle, 300.0, 120.0, true);
        }
        6120 => {
            eprintln!(
                "zoom: the page after two notches up: {}; the editor still {}",
                shell.get_page_zoom(),
                code_size(&code)
            );
            run.snapshot("zoom-page.png");
            // The page's links still answer through the touch over it: the
            // probe hears the link rather than a browser opening it.
            shell.on_open_link(|link| {
                LINK.with(|l| *l.borrow_mut() = Some(link.to_string()));
            });
        }
        6121 => {
            let heard = LINK.with(|l| l.borrow().clone());
            if heard.is_none() && run.waited.get() < 12 {
                // The first line of the first paragraph, a few heights in.
                let area = run.editor_right() - left;
                let column = left + f32::max(32.0, (area - 820.0 * shell.get_page_zoom()) / 2.0);
                let y = 41.0 + 36.0 + 28.0 + 6.0 + run.waited.get() as f32 * 4.0;
                eprintln!("zoom: a click on the page at {:.0},{y:.0}", column + 40.0);
                run.click(column + 40.0, y, PointerEventButton::Left);
                run.waited.set(run.waited.get() + 1);
                run.step.set(6121);
                return;
            }
            // Slint takes a link wrapped onto two lines for its last line
            // alone, so the probe's README begins with a short one.
            eprintln!("zoom: a click on the page's link reached {heard:?}");
            run.waited.set(0);
            run.snapshot("zoom-page-settled.png");
            let middle = (left + run.editor_right()) / 2.0;
            run.wheel(middle, 300.0, 180.0, true);
        }
        6123 => {
            eprintln!(
                "zoom: the page three notches further: {}",
                shell.get_page_zoom()
            );
            run.snapshot("zoom-page-larger.png");
            app.act("page.zoom-reset");
        }
        6124 => {
            eprintln!("zoom: Reset page zoom: {}", shell.get_page_zoom());
            shell.invoke_show_panel("bottom".into(), "terminal".into());
        }
        6125 => {
            let up = !crate::probe::screen_text(&shell).trim().is_empty();
            if !run.until(up, 500) {
                return;
            }
            eprintln!(
                "zoom: the terminal at first: {} px, {} rows",
                shell.get_term_font_size(),
                shell.get_term_rows().row_count()
            );
            run.wheel(left + 200.0, run.bottom_panel_top() + 60.0, 120.0, true);
        }
        6130 => {
            eprintln!(
                "zoom: the terminal after two notches up: {} px, {} rows; the file keeps {:?}",
                shell.get_term_font_size(),
                shell.get_term_rows().row_count(),
                saved()
            );
            run.snapshot("zoom-terminal.png");
            app.act("terminal.zoom-reset");
        }
        6134 => {
            eprintln!(
                "zoom: Reset terminal font zoom: {} px; the file keeps {:?}; height {height}",
                shell.get_term_font_size(),
                saved()
            );
            // Ctrl+= is still the whole window's, and the layout follows it.
            app.act("view.zoom-in");
        }
        6136 => {
            eprintln!(
                "zoom: Ctrl+=: the window at a scale of {}, laid out at {:?}; the editor still {}",
                run.window.window().scale_factor(),
                run.size(),
                code_size(&code)
            );
            run.snapshot("zoom-window.png");
            app.act("view.zoom-reset");
        }
        6138 => {
            eprintln!(
                "zoom: Ctrl+0: the window at a scale of {}, laid out at {:?}",
                run.window.window().scale_factor(),
                run.size()
            );
            app.act("app.exit");
        }
        _ => {}
    }
}

/// `RUSTY_PROBE_MENUS`, from step 6200.
pub fn menus(n: u32, run: &Run) {
    let shell = run.window.global::<Shell>();
    let app = work::app();
    let left = run.editor_left();
    match n {
        6200 => {
            CLIPBOARD.with(|c| *c.borrow_mut() = Some(clipboard()));
            app.runner().say("rusty-probe-output-one", 0);
            app.runner().say("rusty-probe-output-two", 2);
            shell.invoke_show_panel("bottom".into(), "output".into());
        }
        6202 => {
            // The second line, by the button a hand uses.
            run.click(
                left + 120.0,
                run.bottom_panel_top() + 18.0 + 9.0,
                PointerEventButton::Right,
            );
        }
        6203 => {
            eprintln!("menus: Output, on its second line: {:?}", run.menu());
            run.snapshot("menu-output.png");
            run.pick("copy-line");
        }
        6204 => {
            eprintln!("menus: Copy line put {:?} on the clipboard", clipboard());
            run.click(
                left + 120.0,
                run.bottom_panel_top() + 9.0,
                PointerEventButton::Right,
            );
        }
        6205 => {
            run.pick("copy-all");
        }
        6206 => {
            eprintln!("menus: Copy all put {:?} on the clipboard", clipboard());
            run.click(
                left + 120.0,
                run.bottom_panel_top() + 9.0,
                PointerEventButton::Right,
            );
        }
        6207 => run.pick("clear"),
        6208 => {
            eprintln!(
                "menus: Clear output left {} lines",
                shell.get_output().row_count()
            );
            // A mistake the language server will name, at the file's end.
            let editor = app.editor();
            if let Some(path) = editor.current_path() {
                let text = editor.draft_of(&path).map(|(t, ..)| t).unwrap_or_default();
                let version = editor.version_of(&path).unwrap_or(0);
                editor.replace(
                    &path,
                    version,
                    format!("{}\n\nfn probe_broken() {{ let = ; }}\n", text.trim_end()),
                );
            }
            shell.invoke_show_panel("bottom".into(), "problems".into());
        }
        6209 => {
            // One the language server names in a file, when it says one in
            // time; the project's own otherwise.
            let found = problem_row(&shell, true);
            if !run.until(found.is_some(), 1500) {
                return;
            }
            let found = found.or_else(|| problem_row(&shell, false));
            let Some(at) = found else {
                eprintln!("menus: no problem to right-click");
                run.step.set(6220);
                return;
            };
            let y = run.bottom_panel_top() + at as f32 * 24.0 + 12.0;
            run.click(left + 200.0, y, PointerEventButton::Right);
        }
        6210 => {
            eprintln!("menus: Problems, on a problem: {:?}", run.menu());
            run.snapshot("menu-problems.png");
            run.pick("copy");
        }
        6211 => {
            eprintln!("menus: Copy put this on the clipboard:\n{}", clipboard());
            let at = problem_row(&shell, true)
                .or_else(|| problem_row(&shell, false))
                .unwrap_or(0);
            let y = run.bottom_panel_top() + at as f32 * 24.0 + 12.0;
            run.click(left + 200.0, y, PointerEventButton::Right);
        }
        6212 => run.pick("copy-message"),
        6213 => {
            eprintln!("menus: Copy message put {:?} on the clipboard", clipboard());
            let at = problem_row(&shell, true)
                .or_else(|| problem_row(&shell, false))
                .unwrap_or(0);
            let y = run.bottom_panel_top() + at as f32 * 24.0 + 12.0;
            run.click(left + 200.0, y, PointerEventButton::Right);
        }
        6214 => run.pick("go"),
        6216 => {
            eprintln!(
                "menus: Go to problem put the caret at {:?}",
                app.editor().status().map(|s| (s.0, s.1))
            );
        }
        6220 => {
            shell.invoke_show_panel("left".into(), "search".into());
            shell.invoke_search_edited("text".into(), "fn ".into());
        }
        6222 => {
            let rows = shell.get_search_rows();
            if !run.until(rows.row_count() > 1, 500) {
                return;
            }
            // A hit's row, under the fields of the panel's own height:
            // called as the click calls it.
            shell.invoke_surface_menu("search".into(), "".into(), 1, 140.0, 260.0);
        }
        6223 => {
            eprintln!("menus: Search, on a hit: {:?}", run.menu());
            run.snapshot("menu-search.png");
            run.pick("copy-line");
        }
        6224 => {
            eprintln!("menus: Copy line put {:?} on the clipboard", clipboard());
            shell.invoke_surface_menu("search".into(), "".into(), 0, 140.0, 240.0);
        }
        6225 => {
            eprintln!("menus: Search, on a file: {:?}", run.menu());
            run.pick("copy-path");
        }
        6226 => {
            eprintln!("menus: Copy path put {:?} on the clipboard", clipboard());
            shell.invoke_surface_menu("search".into(), "".into(), 1, 140.0, 260.0);
        }
        6227 => run.pick("open"),
        6229 => {
            eprintln!(
                "menus: Open went to {:?} at {:?}",
                app.editor().current_path(),
                app.editor().status().map(|s| (s.0, s.1))
            );
            shell.invoke_show_panel("bottom".into(), "terminal".into());
        }
        6230 => {
            let up = !crate::probe::screen_text(&shell).trim().is_empty();
            if !run.until(up, 500) {
                return;
            }
            for c in "echo rusty-menu-probe".chars() {
                run.send(WindowEvent::KeyPressed {
                    text: c.to_string().into(),
                });
                run.send(WindowEvent::KeyReleased {
                    text: c.to_string().into(),
                });
            }
            run.send(WindowEvent::KeyPressed {
                text: Key::Return.into(),
            });
            run.send(WindowEvent::KeyReleased {
                text: Key::Return.into(),
            });
        }
        6231 => {
            let said = crate::probe::screen_text(&shell)
                .lines()
                .filter(|l| l.contains("rusty-menu-probe"))
                .count();
            if !run.until(said >= 2, 150) {
                return;
            }
            run.click(
                left + 200.0,
                run.bottom_panel_top() + 60.0,
                PointerEventButton::Right,
            );
        }
        6232 => {
            eprintln!("menus: the terminal, nothing selected: {:?}", run.menu());
            run.pick("select-all");
        }
        6233 => {
            run.click(
                left + 200.0,
                run.bottom_panel_top() + 60.0,
                PointerEventButton::Right,
            );
        }
        6234 => {
            eprintln!("menus: the terminal, all selected: {:?}", run.menu());
            run.snapshot("menu-terminal.png");
            run.pick("copy");
        }
        6235 => {
            let copied = clipboard();
            eprintln!(
                "menus: Copy took {} lines, the marker {}; the first {:?}",
                copied.lines().count(),
                if copied.contains("rusty-menu-probe") {
                    "among them"
                } else {
                    "missing"
                },
                copied.lines().next()
            );
            run.click(
                left + 200.0,
                run.bottom_panel_top() + 60.0,
                PointerEventButton::Right,
            );
        }
        6236 => run.pick("clear"),
        6260 => {
            let text = crate::probe::screen_text(&shell);
            eprintln!(
                "menus: Clear terminal left the marker {}; on screen: {:?}",
                if text.contains("rusty-menu-probe") {
                    "there"
                } else {
                    "gone"
                },
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .collect::<Vec<_>>()
            );
            run.snapshot("menu-terminal-cleared.png");
            run.click(
                left + 200.0,
                run.bottom_panel_top() + 60.0,
                PointerEventButton::Right,
            );
        }
        6261 => run.pick("kill"),
        6280 => {
            eprintln!("menus: Kill terminal: {:?}", shell.get_term_exited());
            run.click(
                left + 200.0,
                run.bottom_panel_top() + 60.0,
                PointerEventButton::Right,
            );
        }
        6281 => {
            eprintln!("menus: the terminal, its shell gone: {:?}", run.menu());
            run.pick("restart");
        }
        6300 => {
            eprintln!(
                "menus: Restart terminal: exited {:?}, {} rows of text",
                shell.get_term_exited(),
                crate::probe::screen_text(&shell)
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .count()
            );
            app.open_file("README.md".into());
        }
        6301 => {
            if !run.until(shell.get_md_page(), 250) {
                return;
            }
            let middle = (left + run.editor_right()) / 2.0;
            run.click(middle, 300.0, PointerEventButton::Right);
        }
        6302 => {
            eprintln!("menus: the Markdown page: {:?}", run.menu());
            run.snapshot("menu-page.png");
            run.pick("copy-path");
        }
        6303 => {
            eprintln!("menus: Copy path put {:?} on the clipboard", clipboard());
            let middle = (left + run.editor_right()) / 2.0;
            run.click(middle, 300.0, PointerEventButton::Right);
        }
        6304 => run.pick("source"),
        6306 => {
            eprintln!(
                "menus: Show the source: the page is shown {}",
                shell.get_md_page()
            );
            // The bottom dock's first tab, Output, ten pixels in.
            let (_, height) = run.size();
            let tab = height - 24.0 - shell.get_bottom_size() + 17.0;
            run.click(left + 16.0, tab, PointerEventButton::Right);
        }
        6307 => {
            eprintln!("menus: the bottom dock's first tab: {:?}", run.menu());
            run.snapshot("menu-dock.png");
            run.pick("hide");
        }
        6308 => {
            let tabs: Vec<String> = shell
                .get_bottom_panels()
                .iter()
                .map(|p| p.id.to_string())
                .collect();
            eprintln!(
                "menus: Hide left the strip {tabs:?}, in front {:?}, the file hiding {:?}",
                shell.get_bottom_front(),
                rusty_embed::config::workbench().layout.bottom.hidden
            );
            run.snapshot("menu-dock-hidden.png");
            let (_, height) = run.size();
            let tab = height - 24.0 - shell.get_bottom_size() + 17.0;
            run.click(left + 16.0, tab, PointerEventButton::Right);
        }
        6309 => {
            eprintln!("menus: the first tab now: {:?}", run.menu());
            run.pick("show:output");
        }
        6310 => {
            let tabs: Vec<String> = shell
                .get_bottom_panels()
                .iter()
                .map(|p| p.id.to_string())
                .collect();
            eprintln!(
                "menus: Show put the strip back to {tabs:?}, the file hiding {:?}",
                rusty_embed::config::workbench().layout.bottom.hidden
            );
            if let Some(held) = CLIPBOARD.with(|c| c.borrow_mut().take())
                && let Ok(mut clipboard) = arboard::Clipboard::new()
            {
                let _ = clipboard.set_text(held);
            }
            app.act("app.exit");
        }
        _ => {}
    }
}
