//! A tab dragged, VS Code's way: along its strip to move it, onto the other
//! group's strip or code to move it there, onto the right half of the only
//! group's code to split, beyond the window's edge into a window of its
//! own — and out of a window of its own back onto the main window's strips
//! or code. `crate::tab_drag` decides where it lands, pure and tested; this
//! is the drag as the strips and the windows of their own report it, what
//! the main window draws while it goes on, and the move once it is let go.
//!
//! The strips are measured here, not by the layout: every tab is drawn as
//! wide as [`App::tab_width`] says (`DocTab.width`), so the gap a pointer is
//! in, worked out from the widths, is the gap drawn under it. Everything
//! else — where the panes are, how far each strip is scrolled — is read off
//! the window as it is laid out (`EditorArea.tab-geometry`), at every move.
//!
//! Slint carries no drag from one window to another, so a tab dragged out
//! of a window of its own is followed through the screen: its window's
//! client corner plus the pointer, at that window's scale, less the main
//! window's corner, at the main window's (`App::torn_point_in_main`). The
//! windows' corners are winit's `inner_position`, which is where Slint's
//! coordinates begin; a window's own position counts its frame.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, VecModel};

use super::{App, Tab, strip};
use crate::{
    AppWindow, DocTab, Shell, TabDrag,
    tab_drag::{self, Area, Groups, Landing, Lane, Rect},
};

/// Room round a tab's words, as `TabStrip` in `ui/pages.slint` lays its row
/// out: the padding either side, the gap between each part, the box its ×
/// or its dot sits in — and no tab wider than the strip lets one be.
const PAD_LEFT: f32 = 12.0;
const PAD_RIGHT: f32 = 6.0;
const GAP: f32 = 6.0;
const CLOSE: f32 = 20.0;
const WIDEST: f32 = 240.0;

/// The two strips' models, changed in place: a model replaced is every
/// tab made again, and the one under a held pointer would take the drag
/// with it. And the words they hold, measured once.
pub(super) struct Strips {
    keys: Rc<VecModel<DocTab>>,
    other: Rc<VecModel<DocTab>>,
    /// What a tab's name (`false`) or hint (`true`) measures.
    measured: RefCell<HashMap<(bool, String), f32>>,
}

impl Strips {
    pub fn new(window: &AppWindow) -> Strips {
        let keys = Rc::new(VecModel::<DocTab>::default());
        let other = Rc::new(VecModel::<DocTab>::default());
        let shell = window.global::<Shell>();
        shell.set_tabs(ModelRc::from(keys.clone()));
        shell.set_shade_tabs(ModelRc::from(other.clone()));
        Strips {
            keys,
            other,
            measured: RefCell::default(),
        }
    }

    /// The strips as `tabs` and `other` say, changed where they differ.
    pub fn show(&self, tabs: Vec<DocTab>, other: Vec<DocTab>) {
        patch(&self.keys, tabs);
        patch(&self.other, other);
    }
}

/// `model` made to hold `rows`, changing only what differs: the rows
/// before and after a change keep their elements.
fn patch<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    let old = model.row_count();
    let same = |at: usize, row: &T| model.row_data(at).as_ref() == Some(row);
    let mut head = 0;
    while head < old.min(rows.len()) && same(head, &rows[head]) {
        head += 1;
    }
    let mut tail = 0;
    while tail < (old - head).min(rows.len() - head)
        && same(old - 1 - tail, &rows[rows.len() - 1 - tail])
    {
        tail += 1;
    }
    let (was, now) = (old - head - tail, rows.len() - head - tail);
    let both = was.min(now);
    for at in 0..both {
        model.set_row_data(head + at, rows[head + at].clone());
    }
    for at in both..now {
        model.insert(head + at, rows[head + at].clone());
    }
    for _ in both..was {
        model.remove(head + both);
    }
}

/// A tab being dragged.
pub(super) struct Drag {
    id: String,
    from: Source,
    /// Where the pointer is, in the main window.
    at: (f32, f32),
    /// Still over the window of its own it was dragged out of, which is in
    /// front of whatever is under it there.
    over_own: bool,
    /// Escape gave it up: nothing more until the button comes up.
    cancelled: bool,
}

/// Where a dragged tab comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// A group's strip: whether it is the other group's.
    Group(bool),
    /// A window of its own, by its surface's number.
    Torn(usize),
}

impl App {
    /// A strip's tabs as the strip draws them, each as wide as it is drawn.
    pub(super) fn doc_tabs(&self, tabs: &[Tab]) -> Vec<DocTab> {
        tabs.iter()
            .zip(strip::strip_hints(tabs))
            .map(|(tab, hint)| {
                let title = tab.title();
                DocTab {
                    id: tab.id().into(),
                    width: self.tab_width(&title, &hint),
                    title: title.into(),
                    hint: hint.into(),
                    dirty: matches!(tab, Tab::File(path) if self.editor.is_dirty(path)),
                    kind: tab.kind().into(),
                    unlinked: matches!(tab, Tab::File(path) if self.state.borrow().unlinked.is_dimmed(path)),
                }
            })
            .collect()
    }

    /// How wide a tab is drawn: its name, the hint beside it when it has
    /// one, and the room round them, in whole pixels — so the lines between
    /// tabs fall on the pixel grid — and no wider than [`WIDEST`].
    fn tab_width(&self, title: &str, hint: &str) -> f32 {
        let words = self.measure(false, title)
            + if hint.is_empty() {
                0.0
            } else {
                GAP + self.measure(true, hint)
            };
        (PAD_LEFT + words + GAP + CLOSE + PAD_RIGHT)
            .ceil()
            .min(WIDEST)
    }

    /// A tab's name, or a hint, as the strip's text measures it.
    fn measure(&self, hint: bool, text: &str) -> f32 {
        let key = (hint, text.to_string());
        if let Some(width) = self.strips.measured.borrow().get(&key) {
            return *width;
        }
        let Some(window) = self.window() else {
            return 0.0;
        };
        let width = if hint {
            window.set_tab_probe_hint(text.into());
            window.get_tab_probe_hint_width()
        } else {
            window.set_tab_probe_title(text.into());
            window.get_tab_probe_title_width()
        };
        let mut measured = self.strips.measured.borrow_mut();
        // Every name ever opened is a few bytes; a session that opened
        // thousands starts again rather than keep them all.
        if measured.len() > 2048 {
            measured.clear();
        }
        measured.insert(key, width);
        width
    }

    /// The editor area as the window has it laid out now, with each
    /// strip's tabs as wide as they are drawn.
    fn drag_area(&self) -> Option<Area> {
        let window = self.window()?;
        let laid = window.invoke_tab_geometry();
        let state = self.state.borrow();
        let widths = |tabs: &[Tab]| -> Vec<f32> {
            self.doc_tabs(tabs).iter().map(|tab| tab.width).collect()
        };
        Some(Area {
            window: (window.get_laid_width(), window.get_laid_height()),
            panes: Rect {
                x: laid.x,
                y: laid.y,
                w: laid.width,
                h: laid.height,
            },
            left: laid.left,
            strip: laid.strip,
            keys: Lane {
                widths: widths(&state.tabs),
                shown: laid.keys_lane,
                scroll: laid.keys_scroll,
            },
            other: Lane {
                widths: if state.split {
                    widths(&state.other)
                } else {
                    Vec::new()
                },
                shown: laid.other_lane,
                scroll: laid.other_scroll,
            },
            split: state.split,
            keys_right: state.split && state.focus == 1,
        })
    }

    /// The group a dragged tab is in and where in its strip, for a tab of
    /// a group.
    fn drag_from(&self, from: Source, id: &str) -> Option<(bool, usize)> {
        let Source::Group(other) = from else {
            return None;
        };
        let state = self.state.borrow();
        let tabs = if other { &state.other } else { &state.tabs };
        tabs.iter()
            .position(|tab| tab.id() == id)
            .map(|at| (other, at))
    }

    /// Where a tab let go of at `x`, `y` of the main window lands.
    fn landing(&self, from: Source, id: &str, x: f32, y: f32, over_own: bool) -> Landing {
        if over_own {
            return Landing::Nowhere;
        }
        let Some(area) = self.drag_area() else {
            return Landing::Nowhere;
        };
        // A tab its group no longer has — closed meanwhile — goes nowhere.
        let at = self.drag_from(from, id);
        if matches!(from, Source::Group(_)) && at.is_none() {
            return Landing::Nowhere;
        }
        tab_drag::land(&area, at, x, y)
    }

    /// A tab of a group carried past a few pixels from its press —
    /// `begun` on the first move — to `x`, `y` of the window.
    pub(super) fn tab_dragged(&self, id: &str, other: bool, begun: bool, x: f32, y: f32) {
        self.drag_to(Source::Group(other), id, begun, (x, y), false);
    }

    /// The drag goes on, or begins: what the window draws for it follows.
    fn drag_to(&self, from: Source, id: &str, begun: bool, at: (f32, f32), over_own: bool) {
        {
            let mut state = self.state.borrow_mut();
            let fresh = state
                .tab_drag
                .as_ref()
                .is_none_or(|drag| drag.from != from || drag.id != id);
            if begun || fresh {
                state.tab_drag = Some(Drag {
                    id: id.to_string(),
                    from,
                    at,
                    over_own,
                    cancelled: false,
                });
            }
            let Some(drag) = state.tab_drag.as_mut() else {
                return;
            };
            if drag.cancelled {
                return;
            }
            drag.at = at;
            drag.over_own = over_own;
        }
        self.sync_drag();
    }

    /// A strip scrolled under the pointer: where the tab would land is
    /// worked out again where the pointer is.
    pub(super) fn tab_drag_again(&self) {
        self.sync_drag();
    }

    /// Escape: the drag is given up, and the button coming up does nothing.
    pub(super) fn tab_drag_cancel(&self) {
        if let Some(drag) = self.state.borrow_mut().tab_drag.as_mut() {
            drag.cancelled = true;
        }
        self.sync_drag();
    }

    /// The drag is over: whether it had been given up.
    fn drag_ends(&self) -> bool {
        let gone = self.state.borrow_mut().tab_drag.take();
        self.sync_drag();
        gone.is_some_and(|drag| drag.cancelled)
    }

    /// A tab of a group let go of after a drag, at `x`, `y` of the window.
    pub(super) fn tab_dropped(self: &Rc<Self>, id: &str, other: bool, x: f32, y: f32) {
        if self.drag_ends() {
            return;
        }
        match self.landing(Source::Group(other), id, x, y, false) {
            Landing::Nowhere => {}
            Landing::Out => {
                let Some(main) = self.window() else { return };
                let window = main.window();
                let scale = window.scale_factor();
                let corner = client_corner(window);
                let at = PhysicalPosition::new(
                    corner.x + (x * scale) as i32,
                    corner.y + (y * scale) as i32,
                );
                self.detach(Some((other, id.to_string())), Some(at));
            }
            landing => {
                let Some(tab) = self.tab_in(other, id) else {
                    return;
                };
                // Where its view was, when its group had it in front.
                let (keys_front, other_front) = self.fronts();
                let front = if other { other_front } else { keys_front };
                let view = (front.as_ref() == Some(&tab)).then(|| {
                    if other {
                        self.editor.shade_scroll()
                    } else {
                        self.editor.scroll()
                    }
                });
                self.tab_lands(&tab, Some(other), landing, view);
            }
        }
    }

    /// What each group has in front: the group with the keys', and the
    /// other's while split.
    fn fronts(&self) -> (Option<Tab>, Option<Tab>) {
        let state = self.state.borrow();
        (
            state.active.and_then(|at| state.tabs.get(at)).cloned(),
            state
                .other_active
                .and_then(|at| state.other.get(at))
                .filter(|_| state.split)
                .cloned(),
        )
    }

    /// `tab` into the groups at `landing`, from its group (`from`, whether
    /// it is the other) or from a window of its own (`None`); `view` is
    /// where its view was scrolled when one showed it. The group it lands
    /// in takes the keys with it in front, and every file in front of a
    /// view afterwards is where it was: the moved one where its own view
    /// had it, one still in front of the view it was in where that view
    /// was, any other where its document was left. The document is the
    /// same document — its text, its caret, its history — whatever group
    /// or window shows it. False when nothing moved.
    pub(super) fn tab_lands(
        self: &Rc<Self>,
        tab: &Tab,
        from: Option<bool>,
        landing: Landing,
        view: Option<(f32, f32)>,
    ) -> bool {
        let (keys_front, other_front) = self.fronts();
        let (code, shade) = (self.editor.scroll(), self.editor.shade_scroll());
        let mut groups = {
            let state = self.state.borrow();
            Groups {
                keys: state.tabs.clone(),
                keys_front: state.active,
                other: state.other.clone(),
                other_front: state.other_active,
                split: state.split,
                focus: state.focus,
            }
        };
        if !groups.land(tab, from, landing) {
            return false;
        }
        // The other group's file may leave its view: kept where it was.
        if let Some(Tab::File(path)) = &other_front {
            self.editor.keep_scroll(path, shade);
        }
        {
            let mut state = self.state.borrow_mut();
            state.tabs = groups.keys;
            state.active = groups.keys_front;
            state.other = groups.other;
            state.other_active = groups.other_front;
            state.split = groups.split;
            state.focus = groups.focus;
        }
        let place = |shown: &Option<Tab>| -> Option<(f32, f32)> {
            let shown = shown.as_ref()?;
            if shown == tab
                && let Some(view) = view
            {
                return Some(view);
            }
            if Some(shown) == keys_front.as_ref() {
                return Some(code);
            }
            if Some(shown) == other_front.as_ref() {
                return Some(shade);
            }
            match shown {
                Tab::File(path) => self.editor.kept_scroll(path),
                _ => None,
            }
        };
        let (keys_now, other_now) = self.fronts();
        self.editor.show_shade(
            self.shade_file().as_deref(),
            place(&other_now).unwrap_or_default(),
        );
        self.front_tab();
        if matches!(keys_now, Some(Tab::File(_)))
            && let Some(at) = place(&keys_now)
        {
            self.editor.set_scroll(at);
        }
        true
    }

    /// A window of its own's tab carried to `x`, `y` of that window.
    pub(super) fn torn_tab_dragged(&self, id: usize, path: &str, begun: bool, x: f32, y: f32) {
        let Some((at, over_own)) = self.torn_point_in_main(id, x, y) else {
            return;
        };
        let tab = Tab::File(path.to_string()).id();
        self.drag_to(Source::Torn(id), &tab, begun, at, over_own);
    }

    /// A window of its own's tab let go of at `x`, `y` of that window:
    /// where it lands in the main window, if anywhere.
    pub(super) fn torn_tab_landing(&self, id: usize, path: &str, x: f32, y: f32) -> Landing {
        if self.drag_ends() {
            return Landing::Nowhere;
        }
        let Some(((x, y), over_own)) = self.torn_point_in_main(id, x, y) else {
            return Landing::Nowhere;
        };
        let tab = Tab::File(path.to_string()).id();
        self.landing(Source::Torn(id), &tab, x, y, over_own)
    }

    /// A point of the window of its own `id`, in the main window — through
    /// the screen, each window's client corner and scale its own — and
    /// whether it is still over the window of its own.
    pub(super) fn torn_point_in_main(
        &self,
        id: usize,
        x: f32,
        y: f32,
    ) -> Option<((f32, f32), bool)> {
        let main = self.window()?;
        let torn = self.torn_windows();
        let (torn, _) = torn.iter().find(|(_, surface)| surface.id() == id)?;
        let (from, to) = (torn.window(), main.window());
        let (corner, scale) = (client_corner(from), from.scale_factor());
        let screen = (corner.x as f32 + x * scale, corner.y as f32 + y * scale);
        let (corner, scale) = (client_corner(to), to.scale_factor());
        let at = (
            (screen.0 - corner.x as f32) / scale,
            (screen.1 - corner.y as f32) / scale,
        );
        let size = from.size().to_logical(from.scale_factor());
        let over_own = (0.0..size.width).contains(&x) && (0.0..size.height).contains(&y);
        Some((at, over_own))
    }

    /// A point of the main window in the window of its own `index` — the
    /// other way through the screen, for the probe, which drags a tab out
    /// of one onto the other.
    pub fn main_point_in_torn(&self, index: usize, x: f32, y: f32) -> Option<(f32, f32)> {
        let main = self.window()?;
        let torn = self.torn_windows();
        let (torn, _) = torn.get(index)?;
        let (from, to) = (main.window(), torn.window());
        let (corner, scale) = (client_corner(from), from.scale_factor());
        let screen = (corner.x as f32 + x * scale, corner.y as f32 + y * scale);
        let (corner, scale) = (client_corner(to), to.scale_factor());
        Some((
            (screen.0 - corner.x as f32) / scale,
            (screen.1 - corner.y as f32) / scale,
        ))
    }

    /// The middle of a tab of a group, in the window — for the probe, which
    /// presses tabs where they are drawn.
    pub fn tab_point(&self, other: bool, index: usize) -> Option<(f32, f32)> {
        let area = self.drag_area()?;
        let pane = area.pane(other);
        let lane = if other { &area.other } else { &area.keys };
        let width = *lane.widths.get(index)?;
        let start: f32 = lane.widths[..index].iter().sum();
        Some((
            pane.x + start - lane.scroll + width / 2.0,
            pane.y + area.strip / 2.0,
        ))
    }

    /// How wide a tab of a strip is drawn, for the probe.
    pub fn doc_tab_width(&self, other: bool, index: usize) -> Option<f32> {
        let state = self.state.borrow();
        let tabs = if other { &state.other } else { &state.tabs };
        self.doc_tabs(tabs).get(index).map(|tab| tab.width)
    }

    /// A group's pane, in the window, for the probe.
    pub fn group_pane(&self, other: bool) -> Option<(f32, f32, f32, f32)> {
        let area = self.drag_area()?;
        let pane = area.pane(other);
        Some((pane.x, pane.y, pane.w, pane.h))
    }

    /// How much of each strip's lane shows and how far it is scrolled, for
    /// the probe: the group with the keys' and the other's.
    pub fn strip_lanes(&self) -> Option<[(f32, f32); 2]> {
        let laid = self.window()?.invoke_tab_geometry();
        Some([
            (laid.keys_lane, laid.keys_scroll),
            (laid.other_lane, laid.other_scroll),
        ])
    }

    /// What the window draws for the drag: nothing once it is over or
    /// given up; else the marker or the overlay where it would land, its
    /// name beside the pointer, and a strip to scroll.
    fn sync_drag(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let drag = self
            .state
            .borrow()
            .tab_drag
            .as_ref()
            .filter(|drag| !drag.cancelled)
            .map(|drag| (drag.from, drag.id.clone(), drag.at, drag.over_own));
        let Some((from, id, (x, y), over_own)) = drag else {
            if shell.get_tab_drag().on {
                shell.set_tab_drag(TabDrag::default());
            }
            return;
        };
        let Some(area) = self.drag_area() else { return };
        let landing = self.landing(from, &id, x, y, over_own);
        let shown = tab_drag::shown(&area, landing);
        let scroll = tab_drag::edge_scroll(&area, x, y).filter(|_| !over_own);
        let title = {
            let state = self.state.borrow();
            state
                .tabs
                .iter()
                .chain(&state.other)
                .find(|tab| tab.id() == id)
                .map(Tab::title)
                .unwrap_or_else(|| {
                    let path = id.strip_prefix("file:").unwrap_or(&id);
                    path.rsplit('/').next().unwrap_or(path).to_string()
                })
        };
        let (width, height) = area.window;
        let marker = shown.marker.unwrap_or_default();
        let overlay = shown.overlay.unwrap_or_default();
        let (group, other) = match from {
            Source::Group(other) => (id, other),
            Source::Torn(_) => (String::new(), false),
        };
        shell.set_tab_drag(TabDrag {
            on: true,
            title: title.into(),
            x,
            y,
            inside: !over_own && (0.0..width).contains(&x) && (0.0..height).contains(&y),
            marker: shown.marker.is_some(),
            marker_x: marker.x,
            marker_y: marker.y,
            marker_h: marker.h,
            overlay: shown.overlay.is_some(),
            overlay_x: overlay.x,
            overlay_y: overlay.y,
            overlay_w: overlay.w,
            overlay_h: overlay.h,
            scroll_strip: match scroll {
                Some((true, _)) => 2,
                Some((false, _)) => 1,
                None => 0,
            },
            scroll_dir: scroll.map_or(0, |(_, dir)| dir),
            id: group.into(),
            other,
        });
    }
}

/// Where a window's client area begins on the screen, in physical pixels:
/// where its coordinates begin. A window's own position counts its frame.
fn client_corner(window: &slint::Window) -> PhysicalPosition {
    use slint::winit_030::WinitWindowAccessor;
    window
        .with_winit_window(|winit| winit.inner_position().ok())
        .flatten()
        .map(|at| PhysicalPosition::new(at.x, at.y))
        .unwrap_or_else(|| window.position())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patched_model_holds_the_rows_and_keeps_what_did_not_change() {
        let model = VecModel::from(vec![1, 2, 3, 4]);
        patch(&model, vec![1, 3, 2, 4]);
        assert_eq!(model.iter().collect::<Vec<_>>(), [1, 3, 2, 4]);
        patch(&model, vec![1, 3, 2, 4, 5]);
        assert_eq!(model.iter().collect::<Vec<_>>(), [1, 3, 2, 4, 5]);
        patch(&model, vec![3, 4]);
        assert_eq!(model.iter().collect::<Vec<_>>(), [3, 4]);
        patch(&model, vec![]);
        assert_eq!(model.row_count(), 0);
    }
}
