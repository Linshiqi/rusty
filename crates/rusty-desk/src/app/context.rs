//! Ctrl and the wheel over what is read, and the right-click menus of the
//! surfaces that had none.
//!
//! The text that is read rather than the chrome around it has a scale of its
//! own, VS Code's split: Ctrl+= is the whole window, Ctrl and the wheel over
//! the code are the editor's text (`editor.mouseWheelZoom`), over a Markdown
//! page the page's — prose and a listing are read at different sizes — and
//! over the terminal its font. Each is kept in `workbench.toml`.
//!
//! The menus are VS Code's for each surface, through the window's one menu
//! (`menu.rs`): what the surface was clicked on is kept here until a row is
//! picked, so a row acts on what was under the pointer and not on whatever
//! has moved there since.

use std::{
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use rusty_i18n::t;
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{PointerEventButton, WindowEvent},
};

use super::{App, Tab, git::copy_text};
use crate::{ProblemRow, Shell, docks::Dock};

/// What a surface's menu was opened on.
#[derive(Clone, Debug)]
pub(super) enum Target {
    /// A Markdown file read as a page.
    Page(String),
    /// A picture shown as one: a picture file, or an SVG's picture over its
    /// source.
    Picture(String),
    /// Output, and the line under the pointer when there was one.
    Output(Option<usize>),
    /// A row of Problems.
    Problem(ProblemRow),
    Terminal,
    /// A row of the search results: its file, and the hit when it is one.
    Search {
        path: String,
        hit: Option<usize>,
    },
    /// A panel's tab on a dock's strip.
    Dock(Dock, &'static str),
}

/// What the surfaces' menus and the wheel keep between events.
#[derive(Default)]
pub(super) struct Context {
    pub target: Option<Target>,
    /// A wheel's travel not yet a step, which surface it was over, and
    /// when it last turned: a pause or another surface starts afresh.
    carried: f32,
    over: String,
    turned: Option<Instant>,
}

/// A path the way the system writes one: what a file manager or a shell
/// pasted into takes.
fn native(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        path.to_string()
    }
}

/// A row: its id, its words, and whether it destroys something.
fn item(id: &str, label: String) -> (String, String, bool) {
    (id.to_string(), label, false)
}

fn divider() -> (String, String, bool) {
    (String::new(), String::new(), false)
}

/// A problem as VS Code's Problems panel copies one: the marker as JSON,
/// tab-indented, in an array.
fn marker_json(
    resource: Option<&str>,
    code: Option<&str>,
    severity: u8,
    message: &str,
    source: Option<&str>,
    span: (u32, u32, u32, u32),
) -> String {
    let quoted = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let mut fields = Vec::new();
    if let Some(resource) = resource {
        fields.push(format!("\"resource\": {}", quoted(resource)));
    }
    if let Some(code) = code {
        fields.push(format!("\"code\": {}", quoted(code)));
    }
    fields.push(format!("\"severity\": {severity}"));
    fields.push(format!("\"message\": {}", quoted(message)));
    if let Some(source) = source {
        fields.push(format!("\"source\": {}", quoted(source)));
    }
    let (start_line, start_col, end_line, end_col) = span;
    fields.push(format!("\"startLineNumber\": {start_line}"));
    fields.push(format!("\"startColumn\": {start_col}"));
    fields.push(format!("\"endLineNumber\": {end_line}"));
    fields.push(format!("\"endColumn\": {end_col}"));
    format!("[{{\n\t{}\n}}]", fields.join(",\n\t"))
}

impl App {
    // ---- the text's own scale --------------------------------------------

    /// Ctrl and the wheel over `what` — "editor", "page" or "terminal" —
    /// by how far, up being larger.
    pub(super) fn content_zoom(self: &Rc<Self>, what: &str, delta: f32) {
        let steps = {
            let mut state = self.state.borrow_mut();
            let wheel = &mut state.context;
            let pause = Duration::from_millis(500);
            if wheel.over != what || wheel.turned.is_none_or(|at| at.elapsed() > pause) {
                wheel.carried = 0.0;
                wheel.over = what.to_string();
            }
            wheel.turned = Some(Instant::now());
            crate::zoom::wheel_steps(&mut wheel.carried, delta)
        };
        if steps != 0 {
            self.text_zoom(what, Some(steps));
        }
    }

    /// A text's scale moved by `steps` tenths, or back to its own size —
    /// the palette's reset — with none; drawn at once and kept.
    pub(super) fn text_zoom(self: &Rc<Self>, what: &str, steps: Option<i32>) {
        {
            let mut state = self.state.borrow_mut();
            let layout = &mut state.layout;
            let scale = match what {
                "editor" => &mut layout.editor_zoom,
                "page" => &mut layout.page_zoom,
                "terminal" => &mut layout.terminal_zoom,
                _ => return,
            };
            *scale = steps.map_or(1.0, |steps| crate::zoom::stepped(*scale, steps));
        }
        self.sync_settings();
        self.save_layout();
    }

    /// A click on the page, made again underneath with the page's own touch
    /// stepped aside, so the link under it answers: the text takes every
    /// press, so the touch over it is the only way the right button and
    /// the wheel are heard there. After this event, never inside it.
    pub(super) fn page_click(self: &Rc<Self>, x: f32, y: f32) {
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::ZERO, move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(window) = app.window() else { return };
            let shell = window.global::<Shell>();
            shell.set_page_through(true);
            let position = LogicalPosition::new(x, y);
            let button = PointerEventButton::Left;
            window
                .window()
                .dispatch_event(WindowEvent::PointerPressed { position, button });
            window
                .window()
                .dispatch_event(WindowEvent::PointerReleased { position, button });
            shell.set_page_through(false);
        });
    }

    // ---- the menus -------------------------------------------------------

    /// A right-click on `surface`, on `what` and `row` there, at a point of
    /// the window: its menu, or none where it would offer nothing.
    pub(super) fn surface_menu(
        self: &Rc<Self>,
        surface: &str,
        what: &str,
        row: i32,
        x: f32,
        y: f32,
    ) {
        let Some((target, items)) = self.surface_items(surface, what, row) else {
            return;
        };
        if items.is_empty() {
            return;
        }
        self.state.borrow_mut().context.target = Some(target);
        self.open_menu("surface", items, x, y);
    }

    #[allow(clippy::type_complexity)]
    fn surface_items(
        &self,
        surface: &str,
        what: &str,
        row: i32,
    ) -> Option<(Target, Vec<(String, String, bool)>)> {
        let row = usize::try_from(row).ok();
        match surface {
            // VS Code's preview has no selection here to copy: the page's
            // text cannot be selected, so its menu has no Copy.
            "page" => {
                let Some(Tab::File(path)) = self.active_tab() else {
                    return None;
                };
                let mut items = vec![
                    item("source", t!("markdown.show-source")),
                    divider(),
                    item("copy-path", t!("context.tree-copy-path")),
                    item("copy-relative", t!("context.tree-copy-relative-path")),
                ];
                if self.state.borrow().layout.page_zoom != 1.0 {
                    items.push(divider());
                    items.push(item("zoom-reset", t!("desk.page-zoom-reset")));
                }
                Some((Target::Page(path), items))
            }
            // VS Code's image preview: the file's own rows — its paths and
            // where it is — and an SVG's source, which it also is.
            "picture" => {
                let path = match self.active_tab() {
                    Some(Tab::Picture(path)) => path,
                    Some(Tab::File(path)) if crate::picture::is_svg(&path) => path,
                    _ => return None,
                };
                let mut items = Vec::new();
                if crate::picture::is_svg(&path) {
                    items.push(item("source", t!("markdown.show-source")));
                    items.push(divider());
                }
                items.push(item("copy-path", t!("context.tree-copy-path")));
                items.push(item("copy-relative", t!("context.tree-copy-relative-path")));
                items.push(divider());
                items.push(item(
                    "reveal",
                    rusty_i18n::lookup(crate::reveal::label_key()),
                ));
                Some((Target::Picture(path), items))
            }
            "output" => {
                let lines = self.window()?.global::<Shell>().get_output();
                if lines.row_count() == 0 {
                    return None;
                }
                let line = row.filter(|&at| at < lines.row_count());
                let mut items = Vec::new();
                if line.is_some() {
                    items.push(item("copy-line", t!("context.copy-line")));
                }
                items.push(item("copy-all", t!("context.copy-all")));
                items.push(divider());
                items.push(item("clear", t!("context.clear-output")));
                Some((Target::Output(line), items))
            }
            "problems" => {
                let rows = self.window()?.global::<Shell>().get_problems();
                let problem = rows.row_data(row?)?;
                let items = if problem.kind == 0 {
                    // A file's heading: where it is. The project's and the
                    // last build's headings name no file.
                    if problem.path.is_empty() {
                        return None;
                    }
                    vec![
                        item("open", t!("context.open")),
                        divider(),
                        item("copy-path", t!("context.tree-copy-path")),
                        item("copy-relative", t!("context.tree-copy-relative-path")),
                    ]
                } else {
                    vec![
                        item("go", t!("context.problem-go")),
                        divider(),
                        item("copy", t!("context.copy")),
                        item("copy-message", t!("context.copy-message")),
                    ]
                };
                Some((Target::Problem(problem), items))
            }
            "terminal" => {
                let mut items = Vec::new();
                if !self.term_selected().is_empty() {
                    items.push(item("copy", t!("context.copy")));
                }
                items.push(item("paste", t!("context.paste")));
                items.push(divider());
                items.push(item("select-all", t!("context.select-all")));
                items.push(divider());
                items.push(item("clear", t!("context.terminal-clear")));
                if self.term_alive() {
                    items.push(item("kill", t!("context.terminal-kill")));
                }
                items.push(item("restart", t!("context.terminal-restart")));
                if self.state.borrow().layout.terminal_zoom != 1.0 {
                    items.push(divider());
                    items.push(item("zoom-reset", t!("desk.terminal-zoom-reset")));
                }
                Some((Target::Terminal, items))
            }
            "search" => {
                let rows = self.window()?.global::<Shell>().get_search_rows();
                let found = rows.row_data(row?)?;
                let hit = (!found.file).then_some(found.index.max(0) as usize);
                let mut items = vec![item("open", t!("context.open")), divider()];
                if hit.is_some() {
                    items.push(item("copy-line", t!("context.copy-line")));
                }
                items.push(item("copy-path", t!("context.tree-copy-path")));
                items.push(item("copy-relative", t!("context.tree-copy-relative-path")));
                let path = found.path.to_string();
                Some((Target::Search { path, hit }, items))
            }
            "dock" => {
                let (dock, panel) = what.split_once(':')?;
                let dock = Dock::parse(dock)?;
                let panel = dock.panels().iter().find(|(id, ..)| *id == panel)?.0;
                let state = self.state.borrow();
                let layout = dock.of(&state.layout);
                let front = dock.front(&state.layout);
                let has = |id: &str| super::has_content(&state, id);
                let name = |id: &str| dock.title(id).map(rusty_i18n::lookup).unwrap_or_default();
                let listed: Vec<&str> = dock
                    .panels()
                    .iter()
                    .map(|(id, ..)| *id)
                    .filter(|id| crate::docks::listed(id, has, front, &layout.hidden))
                    .collect();
                let mut items = Vec::new();
                if listed.len() > 1 {
                    items.push(item("hide", t!("context.dock-hide", panel = name(panel))));
                }
                // What was taken off this strip and has something to show,
                // to put back: a hidden panel has no tab to be found by.
                let back: Vec<_> = layout
                    .hidden
                    .iter()
                    .filter(|id| crate::docks::listed(id, has, "", &[]))
                    .map(|id| {
                        item(
                            &format!("show:{id}"),
                            t!("context.dock-show", panel = name(id)),
                        )
                    })
                    .collect();
                if !back.is_empty() {
                    if !items.is_empty() {
                        items.push(divider());
                    }
                    items.extend(back);
                }
                Some((Target::Dock(dock, panel), items))
            }
            _ => None,
        }
    }

    /// A surface's menu put away with nothing picked: the keys back where
    /// they were.
    pub(super) fn surface_menu_dismissed(&self) {
        let target = self.state.borrow_mut().context.target.take();
        if matches!(target, Some(Target::Terminal)) {
            self.term_refocus();
        }
    }

    /// The keys back to the terminal, which its menu took them from.
    fn term_refocus(&self) {
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_term_focus(shell.get_term_focus().wrapping_add(1));
        }
    }

    /// A row of a surface's menu picked.
    pub(super) fn surface_menu_picked(self: &Rc<Self>, id: &str) {
        let Some(target) = self.state.borrow_mut().context.target.take() else {
            return;
        };
        let root = self.root();
        // A path the project names, as the system writes it whole.
        let full = |path: &str| {
            let path = match &root {
                Some(root) if !Path::new(path).is_absolute() => {
                    root.join(path).to_string_lossy().into_owned()
                }
                _ => path.to_string(),
            };
            native(&path)
        };
        match target {
            Target::Page(path) => match id {
                "source" => self.markdown_toggle(),
                "copy-path" => self.tree_copy_path(&path, false),
                "copy-relative" => self.tree_copy_path(&path, true),
                "zoom-reset" => self.text_zoom("page", None),
                _ => {}
            },
            Target::Picture(path) => match id {
                "source" => self.markdown_toggle(),
                "copy-path" => self.tree_copy_path(&path, false),
                "copy-relative" => self.tree_copy_path(&path, true),
                "reveal" => self.tree_reveal(path),
                _ => {}
            },
            Target::Output(line) => {
                let Some(window) = self.window() else { return };
                let lines = window.global::<Shell>().get_output();
                match id {
                    "copy-line" => {
                        if let Some(line) = line.and_then(|at| lines.row_data(at)) {
                            copy_text(&line.text);
                        }
                    }
                    "copy-all" => {
                        let all: Vec<String> = lines.iter().map(|l| l.text.to_string()).collect();
                        copy_text(&all.join("\n"));
                    }
                    "clear" => self.runner.clear(),
                    _ => {}
                }
            }
            Target::Problem(problem) => match id {
                "open" => self.open_file(problem.path.to_string()),
                "go" => {
                    if problem.path.is_empty() {
                        self.project_problem(problem.line.max(0) as usize);
                    } else {
                        self.open_place(
                            problem.path.to_string(),
                            (problem.line.max(0) as usize, problem.col.max(0) as usize),
                        );
                    }
                }
                "copy-path" => self.tree_copy_path(&problem.path, false),
                "copy-relative" => self.tree_copy_path(&problem.path, true),
                "copy" | "copy-message" => {
                    let (json, message) = self.problem_said(&problem, &full);
                    copy_text(if id == "copy" { &json } else { &message });
                }
                _ => {}
            },
            Target::Terminal => {
                match id {
                    "copy" => self.term_copy(),
                    "paste" => self.term_paste(),
                    "select-all" => self.term_select_all(),
                    "clear" => self.term_clear(),
                    "kill" => self.term_kill(),
                    "restart" => self.term_start(),
                    "zoom-reset" => self.text_zoom("terminal", None),
                    _ => {}
                }
                self.term_refocus();
            }
            Target::Search { path, hit } => match id {
                "open" => match hit {
                    Some(hit) => self.search_open(hit as i32),
                    None => self.open_file(path),
                },
                "copy-line" => {
                    let line = hit.and_then(|at| self.state.borrow().search.line_of(at));
                    if let Some(line) = line {
                        copy_text(&line);
                    }
                }
                "copy-path" => self.tree_copy_path(&path, false),
                "copy-relative" => self.tree_copy_path(&path, true),
                _ => {}
            },
            Target::Dock(dock, panel) => match id {
                "hide" => self.dock_hide(dock, panel),
                other => {
                    if let Some(shown) = other.strip_prefix("show:") {
                        self.show_panel(dock_name(dock), shown);
                    }
                }
            },
        }
    }

    /// A panel taken off its dock's strip; the one beside it in front.
    fn dock_hide(self: &Rc<Self>, dock: Dock, panel: &str) {
        {
            let mut state = self.state.borrow_mut();
            let front = dock.front(&state.layout);
            let listed: Vec<&str> = {
                let state = &*state;
                let layout = dock.of(&state.layout);
                dock.panels()
                    .iter()
                    .map(|(id, ..)| *id)
                    .filter(|id| {
                        crate::docks::listed(
                            id,
                            |id| super::has_content(state, id),
                            front,
                            &layout.hidden,
                        )
                    })
                    .collect()
            };
            let layout = dock.of_mut(&mut state.layout);
            // The front's own name, so the next in front is chosen from
            // where the strip stood.
            if layout.front.is_none() {
                layout.front = Some(front.to_string());
            }
            if !crate::docks::hide(layout, panel, &listed) {
                return;
            }
        }
        self.sync_docks();
        self.save_layout();
    }

    /// What a problem says in full: VS Code's JSON of it, and its message.
    fn problem_said(
        &self,
        problem: &ProblemRow,
        full: &dyn Fn(&str) -> String,
    ) -> (String, String) {
        // A file's problem the servers see now, found by where it is and
        // what its first line says.
        let path = problem.path.as_str();
        if !path.is_empty() {
            let said = self.problems.borrow().get(path).and_then(|items| {
                items
                    .iter()
                    .find(|d| {
                        d.start_line as i32 == problem.line
                            && d.start_col as i32 == problem.col
                            && d.message.lines().next().unwrap_or_default() == problem.text.as_str()
                    })
                    .cloned()
            });
            if let Some(d) = said {
                let severity = match d.severity {
                    rusty_lsp::DiagSeverity::Error => 8,
                    rusty_lsp::DiagSeverity::Warning => 4,
                    rusty_lsp::DiagSeverity::Info => 2,
                    _ => 1,
                };
                let json = marker_json(
                    Some(&full(path)),
                    d.code.as_deref(),
                    severity,
                    &d.message,
                    d.source.as_deref(),
                    (
                        d.start_line + 1,
                        d.start_col + 1,
                        d.end_line + 1,
                        d.end_col + 1,
                    ),
                );
                return (json, d.message);
            }
            // The last build's, which the compiler printed.
            let built = self
                .state
                .borrow()
                .build_log
                .errors
                .iter()
                .find(|e| {
                    e.path == path
                        && e.line as i32 - 1 == problem.line
                        && e.col as i32 - 1 == problem.col
                })
                .cloned();
            if let Some(e) = built {
                let json = marker_json(
                    Some(&full(path)),
                    None,
                    8,
                    &e.message,
                    None,
                    (e.line, e.col, e.line, e.col),
                );
                return (json, e.message);
            }
        } else {
            // The project's own, which names no file.
            let state = self.state.borrow();
            let found = state
                .project
                .as_ref()
                .and_then(|p| p.embedded.as_ref().ok())
                .and_then(|e| e.problems.get(problem.line.max(0) as usize));
            if let Some(found) = found {
                let (title, detail) = super::part::problem_text(found);
                let message = if detail.is_empty() {
                    title
                } else {
                    format!("{title}\n{detail}")
                };
                let severity = match problem.kind {
                    1 => 8,
                    2 => 4,
                    _ => 2,
                };
                let json = marker_json(
                    None,
                    (!found.kind.is_empty()).then_some(found.kind.as_str()),
                    severity,
                    &message,
                    None,
                    (1, 1, 1, 1),
                );
                return (json, message);
            }
        }
        // Gone since the menu opened: what the row says.
        let message = problem.text.to_string();
        let json = marker_json(
            (!path.is_empty()).then(|| full(path)).as_deref(),
            None,
            if problem.kind == 1 { 8 } else { 4 },
            &message,
            None,
            (
                problem.line as u32 + 1,
                problem.col as u32 + 1,
                problem.line as u32 + 1,
                problem.col as u32 + 1,
            ),
        );
        (json, message)
    }
}

/// A dock as `show_panel` names it.
fn dock_name(dock: Dock) -> &'static str {
    match dock {
        Dock::Left => "left",
        Dock::Bottom => "bottom",
        Dock::Right => "right",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copied problem is the JSON VS Code's Problems panel copies, its
    /// message escaped whole — quotes, a backslash and a line break — so
    /// it pastes into an issue and parses.
    #[test]
    fn a_problem_copies_as_vs_code_copies_one() {
        let json = marker_json(
            Some("E:\\proj\\src\\main.rs"),
            Some("E0425"),
            8,
            "cannot find value `x`\nnot found in \"this\" scope",
            Some("rustc"),
            (4, 5, 4, 6),
        );
        assert!(json.starts_with("[{\n\t\"resource\": \"E:\\\\proj\\\\src\\\\main.rs\",\n"));
        assert!(json.contains("\n\t\"severity\": 8,\n"));
        assert!(
            json.contains(
                "\"message\": \"cannot find value `x`\\nnot found in \\\"this\\\" scope\""
            )
        );
        assert!(json.ends_with("\"endColumn\": 6\n}]"));
        // A field nobody gave is left out, not written empty.
        let bare = marker_json(None, None, 4, "m", None, (1, 1, 1, 1));
        assert!(!bare.contains("resource") && !bare.contains("code") && !bare.contains("source"));
    }
}
