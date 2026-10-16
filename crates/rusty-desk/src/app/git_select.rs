//! Selecting and copying on the History page and the Changes page: a diff's
//! text, one side at a time, and the opened commit's message.
//!
//! A diff is rows in a list that only makes the rows on screen, so its
//! selection is the window's own, as the editor's is: two places in one
//! side's lines (`crate::diff_select`, the pure half), drawn by the rows
//! from the two ends' rows and distances (`DiffSel`), every distance
//! measured here in the diff's own font (`AppWindow.diff-probe`, through the
//! editor's `Metrics`). The rows are kept while what they were made from is
//! the same, so a selection outlives every read of the repository that does
//! not change the patch under it — the status is read again after every
//! save anywhere.
//!
//! The message is Slint's own text (`DetailText`, read-only): a drag selects
//! and Ctrl+C copies there without anything here. What comes here is a
//! right-click, with the selection's two ends, so the menu's Copy copies
//! what the text has selected and its Select all selects it.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    rc::Rc,
};

use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::{
    App,
    git::{Target, copy_text},
    git_view::{detail_texts, diff_rows, message_of},
};
use crate::{
    AppWindow, DiffRow, DiffSel, Shell,
    diff_select::{DiffText, Selection},
    editor::Metrics,
};

/// Which page's diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Which {
    /// The History page's, of a file of the commit opened.
    History,
    /// The Changes page's, of a file in the working tree.
    Changes,
}

impl Which {
    pub(super) fn parse(name: &str) -> Option<Which> {
        match name {
            "history" => Some(Which::History),
            "changes" => Some(Which::Changes),
            _ => None,
        }
    }
}

/// One page's diff as the window holds it.
#[derive(Default)]
pub(super) struct View {
    /// What the rows were made from — the patch, or the note in its place,
    /// and the layout — hashed.
    made: Option<u64>,
    rows: ModelRc<DiffRow>,
    note: String,
    text: DiffText,
    selection: Option<Selection>,
    /// The widest line on each side as drawn, for how far it scrolls.
    wide: [f32; 2],
}

/// Both pages' diffs, and their font measured.
#[derive(Default)]
pub(super) struct Views {
    history: View,
    changes: View,
    metrics: Option<Metrics>,
}

impl Views {
    fn view(&mut self, which: Which) -> &mut View {
        match which {
            Which::History => &mut self.history,
            Which::Changes => &mut self.changes,
        }
    }
}

/// What a page's diff is made from.
enum Source<'a> {
    Patch(&'a str),
    /// Nothing to draw, and what to say instead.
    Note(String),
}

fn made_of(source: &Source, split: bool) -> u64 {
    let mut hash = DefaultHasher::new();
    split.hash(&mut hash);
    match source {
        Source::Patch(patch) => (0u8, patch).hash(&mut hash),
        Source::Note(note) => (1u8, note).hash(&mut hash),
    }
    hash.finish()
}

/// How wide a run of text is in the diff's font.
fn width_in(window: &AppWindow, text: &str) -> f32 {
    window.set_diff_probe_text(text.into());
    window.get_diff_probe_width()
}

/// The rows again, when what they are made from changed; the selection goes
/// with the rows it was in.
fn remake(view: &mut View, source: Source, split: bool, metrics: &mut Metrics, window: &AppWindow) {
    let made = made_of(&source, split);
    if view.made == Some(made) {
        return;
    }
    let (rows, note, texts) = match source {
        Source::Patch(patch) => diff_rows(patch, split),
        Source::Note(note) => (Vec::new(), note, Vec::new()),
    };
    let cells: Vec<[Option<&str>; 2]> = texts
        .iter()
        .map(|[left, right]| [left.as_deref(), right.as_deref()])
        .collect();
    view.text = DiffText::new(&cells);
    for side in 0..2 {
        let lines = view.text.side(side);
        let mut widest: f32 = 0.0;
        for line in 0..lines.len() {
            let drawn = lines.drawn(line);
            metrics.learn_by(|run| width_in(window, run), drawn);
            widest = widest.max(metrics.x_of(drawn, usize::MAX));
        }
        view.wide[side] = widest.ceil();
    }
    view.rows = ModelRc::new(VecModel::from(rows));
    view.note = note;
    view.selection = None;
    view.made = Some(made);
}

/// The selection as its rows draw it.
fn drawn(view: &View, metrics: &Metrics) -> DiffSel {
    let Some(selection) = view.selection.filter(|s| !s.is_empty()) else {
        return DiffSel::default();
    };
    let side = view.text.side(selection.side);
    let (from, to) = selection.ordered();
    DiffSel {
        active: true,
        side: selection.side as i32,
        from_row: side.row_of(from.line) as i32,
        from_x: metrics.x_of(side.drawn(from.line), from.col),
        to_row: side.row_of(to.line) as i32,
        to_x: metrics.x_of(side.drawn(to.line), to.col),
    }
}

impl App {
    /// Both pages' diffs onto the window, made again only where what they
    /// are made from changed.
    pub(super) fn sync_diff_views(&self, window: &AppWindow) {
        let mut state = self.state.borrow_mut();
        let git = &mut state.git;
        let history = match &git.detail {
            Some(detail) if git.selected.as_deref() == Some(detail.commit.id.as_str()) => {
                let change = git
                    .file
                    .as_ref()
                    .and_then(|f| detail.files.iter().find(|c| &c.path == f));
                match change {
                    Some(change) => Source::Patch(&change.patch),
                    None if detail.files.is_empty() => Source::Note(t!("git.no-files")),
                    None => Source::Note(String::new()),
                }
            }
            _ => Source::Note(String::new()),
        };
        let changes = match (&git.diff_for, &git.diff) {
            (Some(_), Some(patch)) => Source::Patch(patch),
            (Some(_), None) => Source::Note(t!("git.loading")),
            _ => Source::Note(t!("git.pick-change")),
        };
        let split = git.split;
        let views = &mut git.views;
        let metrics = views
            .metrics
            .get_or_insert_with(|| Metrics::measure_by(|run| width_in(window, run)));
        remake(&mut views.history, history, split, metrics, window);
        remake(&mut views.changes, changes, split, metrics, window);
        let shell = window.global::<Shell>();
        for which in [Which::History, Which::Changes] {
            publish(&shell, which, views, true);
        }
    }

    /// One page's selection drawn again.
    fn publish_diff(&self, which: Which) {
        let Some(window) = self.window() else { return };
        let state = self.state.borrow();
        publish(&window.global::<Shell>(), which, &state.git.views, false);
    }

    /// A press (0), a drag (1) or a double-click (2) on a page's diff, on
    /// `row`, `x` into the text of `side`. A press starts a selection there
    /// — or with Shift moves its end, on the same side — a drag carries its
    /// end on the side it started on whatever row it crosses, and a
    /// double-click takes the word.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn diff_pointer(
        self: &Rc<Self>,
        view: &str,
        phase: i32,
        row: i32,
        side: i32,
        x: f32,
        shift: bool,
    ) {
        let Some(which) = Which::parse(view) else {
            return;
        };
        let Some(window) = self.window() else { return };
        {
            let mut state = self.state.borrow_mut();
            let views = &mut state.git.views;
            let metrics = views
                .metrics
                .get_or_insert_with(|| Metrics::measure_by(|run| width_in(&window, run)));
            let view = match which {
                Which::History => &mut views.history,
                Which::Changes => &mut views.changes,
            };
            let side = side.clamp(0, 1) as usize;
            let row = i64::from(row);
            let column = |line: &str| metrics.column_at(line, x);
            match phase {
                0 => {
                    let at = view.text.side(side).place_at(row, column);
                    view.selection = match (at, view.selection) {
                        (Some(at), Some(mut held)) if shift && held.side == side => {
                            held.head = at;
                            Some(held)
                        }
                        (Some(at), _) => Some(Selection {
                            side,
                            anchor: at,
                            head: at,
                        }),
                        (None, _) => None,
                    };
                }
                1 => {
                    if let Some(held) = view.selection.as_mut()
                        && let Some(at) = view.text.side(held.side).place_at(row, column)
                    {
                        held.head = at;
                    }
                }
                2 => {
                    let lines = view.text.side(side);
                    if let Some(at) = lines.place_at(row, column) {
                        let (anchor, head) = lines.word(at);
                        view.selection = Some(Selection { side, anchor, head });
                    }
                }
                _ => return,
            }
        }
        self.publish_diff(which);
    }

    /// A key in a page's diff: "copy" puts the selection on the clipboard,
    /// "all" selects the whole of `side`, "clear" lets the selection go.
    /// Whether it did anything, so a key it did nothing with goes on.
    pub(super) fn diff_key(self: &Rc<Self>, view: &str, verb: &str, side: i32) -> bool {
        let Some(which) = Which::parse(view) else {
            return false;
        };
        let (done, copied) = {
            let mut state = self.state.borrow_mut();
            let view = state.git.views.view(which);
            match verb {
                "copy" => (
                    true,
                    view.selection
                        .filter(|s| !s.is_empty())
                        .map(|s| s.text(&view.text)),
                ),
                "all" => {
                    let side = view.selection.map_or(side.clamp(0, 1) as usize, |s| s.side);
                    view.selection = Selection::all(&view.text, side);
                    (true, None)
                }
                "clear" => (view.selection.take().is_some(), None),
                _ => (false, None),
            }
        };
        if let Some(text) = copied {
            copy_text(&text);
        }
        if done {
            self.publish_diff(which);
        }
        done
    }

    /// What is selected in a page's diff — "history" or "changes" — as the
    /// text a copy puts on the clipboard: for the probe, which reads the
    /// clipboard too, a thing every program on the machine shares.
    pub fn diff_selection_text(&self, view: &str) -> String {
        Which::parse(view).map_or_else(String::new, |which| self.diff_selected(which))
    }

    /// What is selected in a page's diff, as text: what Copy copies.
    pub(super) fn diff_selected(&self, which: Which) -> String {
        let mut state = self.state.borrow_mut();
        let view = state.git.views.view(which);
        view.selection
            .filter(|s| !s.is_empty())
            .map(|s| s.text(&view.text))
            .unwrap_or_default()
    }

    /// A right-click in a page's diff: Copy while something is selected,
    /// and Select all for the side clicked.
    pub(super) fn diff_menu(self: &Rc<Self>, view: &str, side: i32, x: f32, y: f32) {
        if let Some(which) = Which::parse(view) {
            let side = side.clamp(0, 1) as usize;
            self.git_menu(Target::Diff { which, side }, x, y);
        }
    }

    pub(super) fn diff_menu_items(&self, which: Which, side: usize) -> Vec<(String, String, bool)> {
        let mut state = self.state.borrow_mut();
        let view = state.git.views.view(which);
        let mut items = Vec::new();
        if view.selection.is_some_and(|s| !s.is_empty()) {
            items.push(("copy".to_string(), t!("context.copy"), false));
        }
        if !view.text.side(side).is_empty() {
            items.push(("select-all".to_string(), t!("context.select-all"), false));
        }
        items
    }

    pub(super) fn diff_menu_picked(self: &Rc<Self>, which: Which, side: usize, id: &str) {
        match id {
            "copy" => {
                let text = self.diff_selected(which);
                if !text.is_empty() {
                    copy_text(&text);
                }
            }
            "select-all" => {
                {
                    let mut state = self.state.borrow_mut();
                    let view = state.git.views.view(which);
                    view.selection = Selection::all(&view.text, side);
                }
                self.publish_diff(which);
            }
            _ => {}
        }
    }

    /// The keys back to a page's diff, which its menu took: Ctrl+C after
    /// the menu's Select all copies there.
    pub(super) fn diff_refocus(&self, which: Which) {
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_diff_focus_view(
                match which {
                    Which::History => "history",
                    Which::Changes => "changes",
                }
                .into(),
            );
            shell.set_diff_focus(shell.get_diff_focus().wrapping_add(1));
        }
    }

    // ---- the message -----------------------------------------------------

    /// A right-click on a piece of the opened commit's message — "summary",
    /// "meta" or "body" — with the selection's two ends in it, as the
    /// text's byte offsets.
    pub(super) fn git_detail_text_menu(
        self: &Rc<Self>,
        which: &str,
        anchor: i32,
        cursor: i32,
        x: f32,
        y: f32,
    ) {
        let target = {
            let state = self.state.borrow();
            let git = &state.git;
            let Some(detail) = git
                .detail
                .as_ref()
                .filter(|d| git.selected.as_deref() == Some(d.commit.id.as_str()))
            else {
                return;
            };
            let [summary, meta, body] = detail_texts(detail);
            let text = match which {
                "summary" => summary,
                "meta" => meta,
                "body" => body,
                _ => return,
            };
            Target::Text {
                which: which.to_string(),
                selected: selected_in(&text, anchor, cursor),
                id: detail.commit.id.clone(),
                short: detail.commit.short.clone(),
                message: message_of(detail),
            }
        };
        self.git_menu(target, x, y);
    }

    pub(super) fn text_menu_items(selected: &str) -> Vec<(String, String, bool)> {
        let item = |id: &str, label: String| (id.to_string(), label, false);
        let mut items = Vec::new();
        if !selected.is_empty() {
            items.push(item("copy", t!("context.copy")));
        }
        items.push(item("select-all", t!("context.select-all")));
        items.push((String::new(), String::new(), false));
        items.push(item("copy-message", t!("git.copy-message")));
        items.push(item("copy-hash", t!("git.copy-hash")));
        items.push(item("copy-short", t!("git.copy-short-hash")));
        items
    }

    pub(super) fn text_menu_picked(&self, id: &str, which: &str, selected: &str) {
        if id == "select-all"
            && let Some(window) = self.window()
        {
            let shell = window.global::<Shell>();
            shell.set_git_detail_select_which(which.into());
            shell.set_git_detail_select_all(shell.get_git_detail_select_all().wrapping_add(1));
        } else if id == "copy" && !selected.is_empty() {
            copy_text(selected);
        }
    }
}

/// What lies between a text's two byte offsets, in either order: nothing
/// where either is not a character boundary of it.
fn selected_in(text: &str, anchor: i32, cursor: i32) -> String {
    let (a, b) = (anchor.max(0) as usize, cursor.max(0) as usize);
    let (from, to) = (a.min(b), a.max(b));
    text.get(from..to).unwrap_or_default().to_string()
}

/// One page's diff onto the window: its rows and note too when `whole`,
/// its selection always.
fn publish(shell: &Shell, which: Which, views: &Views, whole: bool) {
    let Some(metrics) = views.metrics.as_ref() else {
        return;
    };
    let view = match which {
        Which::History => &views.history,
        Which::Changes => &views.changes,
    };
    let selection = drawn(view, metrics);
    match which {
        Which::History => {
            if whole {
                shell.set_git_detail_rows(view.rows.clone());
                shell.set_git_detail_note(view.note.as_str().into());
                shell.set_git_detail_wide_left(view.wide[0]);
                shell.set_git_detail_wide_right(view.wide[1]);
            }
            shell.set_git_detail_sel(selection);
        }
        Which::Changes => {
            if whole {
                shell.set_git_diff_rows(view.rows.clone());
                shell.set_git_diff_note(view.note.as_str().into());
                shell.set_git_diff_wide_left(view.wide[0]);
                shell.set_git_diff_wide_right(view.wide[1]);
            }
            shell.set_git_diff_sel(selection);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::selected_in;

    /// The menu's Copy copies what the text has selected, dragged either
    /// way, and nothing for offsets that would cut a character.
    #[test]
    fn the_message_copies_what_its_text_selected() {
        let text = "修复 the reader";
        assert_eq!(selected_in(text, 7, 10), "the");
        assert_eq!(selected_in(text, 10, 7), "the");
        assert_eq!(selected_in(text, 0, 6), "修复");
        assert_eq!(selected_in(text, 1, 6), "", "inside 修");
        assert_eq!(selected_in(text, 3, 3), "");
    }
}
