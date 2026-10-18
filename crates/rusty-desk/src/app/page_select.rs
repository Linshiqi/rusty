//! Selecting and copying on a Markdown page read as a page.
//!
//! The page's text is Slint's and cannot be selected there, so the
//! selection is the window's own, as the diff's is (`git_select.rs`): every
//! paragraph, heading, caption and cell is laid out into lines by rusty
//! (`markdown::flow`) and every code block is lines of its own, each block
//! reporting where it stands on the page (`Shell.md-placed`, from the
//! page's top left, so scrolling moves nothing). A press on the page is a
//! point in those coordinates; the block under it is found among the
//! boxes, the line and the atom among its lines, and the character by
//! measuring the word's prefixes with the measure that placed it
//! (`markdown::select`, the pure half). The selection is two places in
//! atoms, which outlive a reflow; it is drawn by each line's own
//! `sel-x`/`sel-w`, changed in the rows the blocks are drawn from rather
//! than by making the blocks again — a block made again is a press lost
//! under the pointer.
//!
//! A click that did not become a drag opens the link under it, which is
//! why the page no longer sends the click through to the text underneath.

use std::rc::Rc;

use slint::{ComponentHandle, Model};

use super::{
    App,
    markdown_view::{MonoProbe, Page, Probe},
};
use crate::{
    AppWindow, Shell,
    markdown::{
        Block,
        flow::{self, Line, Measure, Placed, Style},
        select::{self, Place, Selection, Spot},
    },
};

/// The inset of a code block's lines from its corner, as `CodeBox` draws
/// them, and how tall each is against the page's text size.
const CODE_PAD: f32 = 8.0;
const CODE_LINE: f32 = 1.45;
/// The code block's font against the page's.
const CODE_SCALE: f32 = 0.95;

/// How far a press travels before it is a drag rather than a click.
const DRAG_SLOP: f32 = 3.0;

/// Where a block stands on the page, as its view reported: its box from
/// the page's top left, and the text size it is drawn at.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PageBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub size: f32,
}

/// A press on the page not let go of yet: where it landed, the link under
/// it, whether it has travelled far enough to be a drag, and whether a
/// double-click has already made a selection of it.
#[derive(Clone, Debug)]
pub(super) struct Press {
    at: (f32, f32),
    link: Option<String>,
    dragging: bool,
    doubled: bool,
}

/// Whether the key's block is a code block, whose lines are inset and
/// measured in the code font.
fn is_code(read: &[Block], key: i32) -> bool {
    matches!(read.get((key / 65536) as usize), Some(Block::Code { .. }))
}

impl Page {
    /// What each atom of the block under `key` is copied as: a paragraph's
    /// words, spaces and formulas, a code block's lines with their breaks.
    pub(super) fn atom_texts(&self, key: i32) -> Option<Vec<String>> {
        let block = self.read.get((key / 65536) as usize)?;
        let cell = (key % 65536) as usize;
        match block {
            Block::Text { flow, .. } | Block::Heading { flow, .. } if cell == 0 => {
                Some(flow::atom_texts(flow))
            }
            Block::Table { rows } if cell > 0 => rows
                .iter()
                .flatten()
                .nth(cell - 1)
                .map(|cell| flow::atom_texts(&cell.flow)),
            Block::Code { lines, .. } if cell == 0 => Some(
                lines
                    .iter()
                    .enumerate()
                    .map(|(at, line)| {
                        if at + 1 < lines.len() {
                            format!("{line}\n")
                        } else {
                            line.clone()
                        }
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    /// The block's lines: a paragraph's as laid out for its width, a code
    /// block's measured once for the size its box reported. `None` before
    /// either is known.
    pub(super) fn lines_of(&mut self, window: &AppWindow, key: i32) -> Option<Rc<Vec<Line>>> {
        if !is_code(&self.read, key) {
            return self.laid_lines.get(&key).cloned();
        }
        let size = self.boxes.get(&key)?.size;
        if let Some((at, lines)) = self.code_lines.get(&key)
            && (at - size).abs() < 0.01
        {
            return Some(lines.clone());
        }
        let Some(Block::Code { lines, .. }) = self.read.get((key / 65536) as usize) else {
            return None;
        };
        let code = Style {
            code: true,
            ..Style::default()
        };
        let mut probe = MonoProbe {
            window,
            size: size * CODE_SCALE,
            measures: &mut self.mono,
        };
        let laid: Vec<Line> = lines
            .iter()
            .enumerate()
            .map(|(at, text)| Line {
                height: size * CODE_LINE,
                pieces: Vec::new(),
                placed: vec![Placed {
                    atom: at,
                    x: 0.0,
                    width: probe.width(&text.replace('\t', "    "), &code),
                    text: text.clone(),
                    style: code.clone(),
                    formula: false,
                }],
            })
            .collect();
        let laid = Rc::new(laid);
        self.code_lines.insert(key, (size, laid.clone()));
        Some(laid)
    }

    /// `measure` run with the measure the block under `key` is measured by.
    fn measured<R>(
        &mut self,
        window: &AppWindow,
        key: i32,
        size: f32,
        measure: impl FnOnce(&mut dyn Measure) -> R,
    ) -> R {
        if is_code(&self.read, key) {
            measure(&mut MonoProbe {
                window,
                size: size * CODE_SCALE,
                measures: &mut self.mono,
            })
        } else {
            measure(&mut Probe {
                window,
                size,
                measures: &mut self.measures,
            })
        }
    }

    /// The block a point of the page is in: the one whose box holds it, or
    /// the nearest one above or below when it is between blocks — a drag
    /// into the gap below a paragraph reaches the paragraph's end.
    fn box_under(&self, x: f32, y: f32, strict: bool) -> Option<i32> {
        let on_row: Vec<(&i32, &PageBox)> = self
            .boxes
            .iter()
            .filter(|(_, b)| y >= b.y && y < b.y + b.height)
            .collect();
        if !on_row.is_empty() {
            // Cells of a table stand beside each other: the one under the
            // point, or the nearest across.
            let nearest = on_row
                .iter()
                .min_by(|(_, a), (_, b)| {
                    let across = |b: &PageBox| {
                        if x < b.x {
                            b.x - x
                        } else if x >= b.x + b.width {
                            x - (b.x + b.width)
                        } else {
                            0.0
                        }
                    };
                    across(a).total_cmp(&across(b))
                })
                .map(|(key, _)| **key);
            return nearest;
        }
        if strict {
            return None;
        }
        self.boxes
            .iter()
            .min_by(|(_, a), (_, b)| {
                let away = |b: &PageBox| {
                    if y < b.y {
                        b.y - y
                    } else {
                        y - (b.y + b.height)
                    }
                };
                away(a).total_cmp(&away(b))
            })
            .map(|(key, _)| *key)
    }

    /// A point of the page as a point of the block under `key`'s lines.
    fn local(&self, key: i32, x: f32, y: f32) -> Option<(f32, f32)> {
        let b = self.boxes.get(&key)?;
        let inset = if is_code(&self.read, key) {
            CODE_PAD
        } else {
            0.0
        };
        Some((x - b.x - inset, y - b.y - inset))
    }

    /// The place a point of the page falls on, and the link there if any.
    pub(super) fn place_at(
        &mut self,
        window: &AppWindow,
        x: f32,
        y: f32,
    ) -> Option<(Place, Option<String>)> {
        let key = self.box_under(x, y, false)?;
        let size = self.boxes.get(&key)?.size;
        let (lx, ly) = self.local(key, x, y)?;
        let lines = self.lines_of(window, key)?;
        let spot = self.measured(window, key, size, |m| select::hit(&lines, lx, ly, m))?;
        let link = select::under(&lines, lx, ly).and_then(|p| p.style.link.clone());
        Some((Place { key, spot }, link))
    }

    /// What is under a point of the page, for the pointer's shape: 2 a
    /// link, 1 text, 0 nothing.
    pub(super) fn cursor_at(&mut self, window: &AppWindow, x: f32, y: f32) -> i32 {
        let Some(key) = self.box_under(x, y, true) else {
            return 0;
        };
        let Some((lx, ly)) = self.local(key, x, y) else {
            return 0;
        };
        let Some(lines) = self.lines_of(window, key) else {
            return 0;
        };
        match select::under(&lines, lx, ly) {
            Some(p) if p.style.link.is_some() => 2,
            Some(_) => 1,
            None => 0,
        }
    }

    /// Where the selection is drawn on each of `lines`, the block under
    /// `key`'s, at `size`.
    pub(super) fn selection_marks(
        &mut self,
        window: &AppWindow,
        key: i32,
        size: f32,
        lines: &[Line],
    ) -> Vec<Option<(f32, f32)>> {
        let Some((from, to)) = self
            .selection
            .filter(|s| !s.is_empty())
            .and_then(|s| s.in_block(key))
        else {
            return vec![None; lines.len()];
        };
        self.measured(window, key, size, |m| select::drawn(lines, from, to, m))
    }

    /// The place at the end of the block under `key`.
    fn end_of(&self, key: i32) -> Option<Place> {
        Some(Place {
            key,
            spot: select::end_of(&self.atom_texts(key)?),
        })
    }

    /// Every block with text, in reading order.
    fn text_keys(&self) -> Vec<i32> {
        let mut keys = Vec::new();
        for (index, block) in self.read.iter().enumerate() {
            match block {
                Block::Text { .. } | Block::Heading { .. } | Block::Code { .. } => {
                    keys.push(index as i32 * 65536);
                }
                Block::Table { rows } => {
                    for at in 1..=rows.iter().flatten().count() {
                        keys.push(index as i32 * 65536 + at as i32);
                    }
                }
                _ => {}
            }
        }
        keys
    }

    /// What goes between two blocks when both are copied: a tab between
    /// two cells of one row, a line break between two rows of one table,
    /// a blank line otherwise.
    fn separator(&self, before: i32, after: i32) -> &'static str {
        if before / 65536 == after / 65536
            && let Some(Block::Table { rows }) = self.read.get((before / 65536) as usize)
        {
            let columns = rows.first().map_or(1, |row| row.len().max(1)) as i32;
            let row_of = |key: i32| (key % 65536 - 1) / columns;
            return if row_of(before) == row_of(after) {
                "\t"
            } else {
                "\n"
            };
        }
        "\n\n"
    }

    /// The selection's text, block by block.
    pub(super) fn selected_text(&self) -> String {
        let Some(selection) = self.selection.filter(|s| !s.is_empty()) else {
            return String::new();
        };
        let mut out = String::new();
        let mut last: Option<i32> = None;
        for key in self.text_keys() {
            let Some((from, to)) = selection.in_block(key) else {
                continue;
            };
            let Some(texts) = self.atom_texts(key) else {
                continue;
            };
            if let Some(before) = last {
                out.push_str(self.separator(before, key));
            }
            out.push_str(&select::text(&texts, from, to));
            last = Some(key);
        }
        out
    }

    /// The whole page selected.
    fn select_all(&mut self) -> bool {
        let keys = self.text_keys();
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
            return false;
        };
        let Some(end) = self.end_of(*last) else {
            return false;
        };
        self.selection = Some(Selection {
            anchor: Place {
                key: *first,
                spot: Spot::default(),
            },
            head: end,
        });
        true
    }
}

impl App {
    /// A block's view says where it stands on the page.
    pub(super) fn md_placed(&self, key: i32, x: f32, y: f32, width: f32, height: f32, size: f32) {
        let placed = PageBox {
            x,
            y,
            width,
            height,
            size,
        };
        let mut state = self.state.borrow_mut();
        let page = &mut state.page;
        if let Some(known) = page.boxes.get(&key)
            && (known.x - x).abs() < 0.5
            && (known.y - y).abs() < 0.5
            && (known.width - width).abs() < 0.5
            && (known.height - height).abs() < 0.5
            && (known.size - size).abs() < 0.01
        {
            return;
        }
        page.boxes.insert(key, placed);
    }

    /// The selection drawn as it is now: every block's rows told where it
    /// crosses them, and only the rows whose share changed.
    pub(super) fn publish_page_selection(&self) {
        let Some(window) = self.window() else { return };
        let mut state = self.state.borrow_mut();
        let page = &mut state.page;
        let keys: Vec<i32> = page
            .line_models
            .keys()
            .chain(page.code_models.keys())
            .copied()
            .collect();
        for key in keys {
            let Some(size) = page.boxes.get(&key).map(|b| b.size) else {
                continue;
            };
            let Some(lines) = page.lines_of(&window, key) else {
                continue;
            };
            let marks = page.selection_marks(&window, key, size, &lines);
            if page.drawn.get(&key).is_some_and(|was| *was == marks) {
                continue;
            }
            let changed: Vec<(usize, Option<(f32, f32)>)> = marks
                .iter()
                .enumerate()
                .filter(|(at, mark)| page.drawn.get(&key).and_then(|w| w.get(*at)) != Some(mark))
                .map(|(at, mark)| (at, *mark))
                .collect();
            if let Some(model) = page.line_models.get(&key) {
                for (at, mark) in &changed {
                    if let Some(mut row) = model.row_data(*at) {
                        (row.sel_x, row.sel_w) = sel_of(*mark);
                        model.set_row_data(*at, row);
                    }
                }
            } else if let Some(model) = page.code_models.get(&key) {
                for (at, mark) in &changed {
                    if let Some(mut row) = model.row_data(*at) {
                        (row.sel_x, row.sel_w) = sel_of(*mark);
                        model.set_row_data(*at, row);
                    }
                }
            }
            page.drawn.insert(key, marks);
        }
    }

    /// The left button down on the page: the selection begins there, or
    /// with Shift reaches there from where it was.
    pub(super) fn page_press(self: &Rc<Self>, x: f32, y: f32, shift: bool) {
        let Some(window) = self.window() else { return };
        {
            let mut state = self.state.borrow_mut();
            let page = &mut state.page;
            let Some((place, link)) = page.place_at(&window, x, y) else {
                page.press = None;
                return;
            };
            page.selection = match page.selection {
                Some(mut selection) if shift => {
                    selection.head = place;
                    Some(selection)
                }
                _ => Some(Selection {
                    anchor: place,
                    head: place,
                }),
            };
            page.press = Some(Press {
                at: (x, y),
                link,
                dragging: false,
                doubled: false,
            });
        }
        self.publish_page_selection();
    }

    /// The pointer moving with the button down: past the slop, the
    /// selection's head follows it.
    pub(super) fn page_drag(self: &Rc<Self>, x: f32, y: f32) {
        let Some(window) = self.window() else { return };
        {
            let mut state = self.state.borrow_mut();
            let page = &mut state.page;
            let Some(press) = page.press.as_mut() else {
                return;
            };
            if !press.dragging {
                let (px, py) = press.at;
                if (x - px).abs() < DRAG_SLOP && (y - py).abs() < DRAG_SLOP {
                    return;
                }
                press.dragging = true;
            }
            let Some((place, _)) = page.place_at(&window, x, y) else {
                return;
            };
            if let Some(selection) = page.selection.as_mut() {
                selection.head = place;
            }
        }
        self.publish_page_selection();
    }

    /// The button up: a press that never became a drag is a click, which
    /// opens the link it landed on and leaves nothing selected.
    pub(super) fn page_release(self: &Rc<Self>, _x: f32, _y: f32) {
        let link = {
            let mut state = self.state.borrow_mut();
            let page = &mut state.page;
            let Some(press) = page.press.take() else {
                return;
            };
            if press.dragging || press.doubled {
                return;
            }
            if page.selection.is_some_and(|s| s.is_empty()) {
                page.selection = None;
            }
            press.link
        };
        // Through the page's own callback, which opens it in the browser —
        // and which the probe listens at instead.
        if let Some(link) = link
            && let Some(window) = self.window()
        {
            window.global::<Shell>().invoke_open_link(link.into());
        }
    }

    /// A double-click selects the word under it.
    pub(super) fn page_double(self: &Rc<Self>, x: f32, y: f32) {
        let Some(window) = self.window() else { return };
        {
            let mut state = self.state.borrow_mut();
            let page = &mut state.page;
            let Some((place, _)) = page.place_at(&window, x, y) else {
                return;
            };
            let Some(lines) = page.lines_of(&window, place.key) else {
                return;
            };
            let (from, to) = select::word_at(&lines, place.spot);
            page.selection = Some(Selection {
                anchor: Place {
                    key: place.key,
                    spot: from,
                },
                head: Place {
                    key: place.key,
                    spot: to,
                },
            });
            if let Some(press) = page.press.as_mut() {
                press.doubled = true;
            }
        }
        self.publish_page_selection();
    }

    /// The pointer resting over the page: its shape says what is under it.
    pub(super) fn page_hover(self: &Rc<Self>, x: f32, y: f32) {
        let Some(window) = self.window() else { return };
        let cursor = self.state.borrow_mut().page.cursor_at(&window, x, y);
        let shell = window.global::<Shell>();
        if shell.get_page_cursor() != cursor {
            shell.set_page_cursor(cursor);
        }
    }

    /// A key on the page: "copy" puts the selection on the clipboard,
    /// "all" selects the page, "clear" lets the selection go. Whether it
    /// did anything, so a key it did nothing with goes on.
    pub fn page_key(self: &Rc<Self>, verb: &str) -> bool {
        let (done, copied) = {
            let mut state = self.state.borrow_mut();
            let page = &mut state.page;
            match verb {
                "copy" => {
                    let text = page.selected_text();
                    (true, (!text.is_empty()).then_some(text))
                }
                "all" => (page.select_all(), None),
                "clear" => (page.selection.take().is_some(), None),
                _ => (false, None),
            }
        };
        if let Some(text) = copied {
            super::git::copy_text(&text);
        }
        if done {
            self.publish_page_selection();
        }
        done
    }

    /// Whether the page has anything selected, for its menu.
    pub(super) fn page_has_selection(&self) -> bool {
        self.state
            .borrow()
            .page
            .selection
            .is_some_and(|s| !s.is_empty())
    }

    /// What is selected on the page, as a copy puts it on the clipboard:
    /// for the probe.
    pub fn page_selected_text(&self) -> String {
        self.state.borrow().page.selected_text()
    }
}

/// A mark as a row carries it: nothing is a width of zero.
fn sel_of(mark: Option<(f32, f32)>) -> (f32, f32) {
    mark.unwrap_or((0.0, 0.0))
}
