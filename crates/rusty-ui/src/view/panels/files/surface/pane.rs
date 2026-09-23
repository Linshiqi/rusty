//! The editing surface as every piece of it shares it: the two layers'
//! handles, the window of rows drawn, the caret's place, the popups' state —
//! and the few things those pieces ask of it.
//!
//! It was forty locals and eight closures at the top of one 2,500-line
//! component, captured by whichever part of the view needed them. `Pane` is
//! those locals as fields and those closures as methods, `Copy` like the
//! signals it holds, so each piece of the surface can be a component of its
//! own that takes one.

use super::*;

/// Everything the surface's pieces share. See the module header.
#[derive(Clone, Copy)]
pub(super) struct Pane {
    pub state: AppState,
    pub area: NodeRef<html::Textarea>,
    pub scroller: NodeRef<html::Div>,
    /// The document's path — a `String`, so held as a value the `Copy`
    /// struct can carry.
    pub path: StoredValue<String>,
    pub read_only: bool,
    /// Hover only means something where a language server is listening.
    pub is_rust: bool,
    /// In no crate's module tree.
    pub unlinked: Signal<bool>,
    /// The cell the mouse was last over.
    pub hover_cell: RwSignal<Option<(u32, u32)>>,
    /// Which hover is the newest, so only it asks the server.
    pub hover_gen: RwSignal<u64>,
    /// True while the pointer is over the hover card itself.
    pub on_card: RwSignal<bool>,
    pub editor_menu: RwSignal<Option<(f64, f64)>>,
    /// Bumped by every way the caret moves (`selectionchange`).
    pub selection_moves: RwSignal<u32>,
    /// A drag the editor is following (`pointer.rs`).
    pub drag: StoredValue<Option<Drag>>,
    /// The name drawn as a link while Ctrl is held: (line, from, to).
    pub link: RwSignal<Option<(u32, u32, u32)>>,
    pub link_turn: StoredValue<u64>,
    pub zoom: RwSignal<f64>,
    pub rows_total: Memo<u32>,
    /// The rows drawn.
    pub window: Memo<std::ops::Range<u32>>,
    /// The widest line as drawn, hints and all.
    pub widest: Memo<f64>,
    pub brackets: Memo<Option<(At, At)>>,
    pub runnables: Memo<Vec<rusty_edit::tests_in::Runnable>>,
    pub foldables: Memo<Vec<rusty_edit::Region>>,
    pub levels: Memo<Vec<u8>>,
    pub lit_guide: Memo<Option<(u8, usize, usize)>>,
    /// Which completion row the keyboard is on.
    pub picked: RwSignal<usize>,
    pub picked_action: RwSignal<usize>,
    /// The font, size and line height both layers carry verbatim.
    pub metrics: Signal<String>,
    pub gutter_style: Signal<String>,
}

impl Pane {
    /// The name drawn as a link goes back to plain text, and an answer still
    /// on its way about it is stale.
    pub(super) fn unlink(self) {
        self.link_turn.update_value(|turn| *turn += 1);
        if self.link.get_untracked().is_some() {
            self.link.set(None);
        }
    }

    /// Draw the name at `cell` as a link, once the server says it has a
    /// definition to go to — as VS Code's is.
    pub(super) fn probe_link(self, cell: Option<(u32, u32)>) {
        let Self {
            state,
            is_rust,
            link,
            link_turn,
            ..
        } = self;
        let path = self.path.get_value();
        let Some((line, col)) = cell.filter(|_| is_rust) else {
            self.unlink();
            return;
        };
        let span = state.editor.draft.with_untracked(|draft| {
            name_span(
                draft.split('\n').nth(line as usize).unwrap_or_default(),
                col,
            )
        });
        let Some((from, to)) = span else {
            self.unlink();
            return;
        };
        if link.get_untracked() == Some((line, from, to)) {
            return;
        }
        self.unlink();
        let turn = link_turn.get_value();
        controller::has_definition(path.clone(), line, col, move |found| {
            if found && link_turn.try_get_value() == Some(turn) {
                let _ = link.try_set(Some((line, from, to)));
            }
        });
    }

    /// The height of `rows` rows, as a spacer's style.
    pub(super) fn spacer(self, rows: u32) -> String {
        format!(
            "height: {}px",
            f64::from(rows) * row_height(self.zoom.get())
        )
    }

    /// The margin's rows in the window. The icons scale with the row: a fixed
    /// 13px chevron is taller than the row itself once the editor is zoomed
    /// out far enough, and a row that out-grows its line height pushes every
    /// number below it down — the gutter walks away from the code a row at a
    /// time.
    pub(super) fn gutter_rows(self) -> Vec<GutterRow> {
        let Self {
            state,
            zoom,
            window,
            foldables,
            ..
        } = self;
        let range = window.get();
        let icon_px = (row_height(zoom.get()) * 0.68).round().max(7.0) as u32;
        state.editor.folds.with(|folds| {
            foldables.with(|found| {
                range
                    .map(|row| {
                        let line = folds.doc_of_view(row);
                        GutterRow {
                            line,
                            chevron: found
                                .binary_search_by_key(&line, |region| region.header)
                                .is_ok(),
                            collapsed: folds.is_folded(line),
                            folds_column: !found.is_empty(),
                            icon_px,
                        }
                    })
                    .collect::<Vec<_>>()
            })
        })
    }

    /// The echo's rows in the window, each with what is drawn on it. Hidden
    /// lines are not among them: the echo must drop exactly the lines the
    /// textarea dropped, or every caret below sits on the wrong glyph.
    pub(super) fn echo_rows(self) -> Vec<EchoRow> {
        let Self {
            state,
            link,
            window,
            levels,
            ..
        } = self;
        let path = self.path.get_value();
        let range = window.get();
        let diagnostics = state
            .lsp
            .diagnostics
            .with(|by_file| by_file.get(&path).cloned())
            .unwrap_or_default();
        state.editor.folds.with(|folds| {
            // The compiler's colours, when they have arrived for this
            // document.
            state.editor.semantic.with(|semantic| {
                let semantic = semantic
                    .as_ref()
                    .filter(|(for_path, _)| for_path == &path)
                    .map_or(&[][..], |(_, spans)| spans.as_slice());
                state.editor.highlighted.with(|lines| {
                    range
                        .filter_map(|row| {
                            let index = folds.doc_of_view(row);
                            let line = overlay_semantic(
                                lines.get(index as usize)?.clone(),
                                index,
                                semantic_on(semantic, index),
                            );
                            let diags: Vec<FileDiagnostic> = diagnostics
                                .iter()
                                .filter(|d| d.start_line <= index && index <= d.end_line)
                                .cloned()
                                .collect();
                            let folded = folds
                                .regions()
                                .iter()
                                .find(|region| region.header == index)
                                .map(rusty_edit::Region::hidden);
                            let guides = levels
                                .with(|levels| levels.get(index as usize).copied())
                                .unwrap_or(0);
                            let hints = hints_on(state, &path, index);
                            let link = link
                                .get()
                                .filter(|(line, ..)| *line == index)
                                .map(|(_, from, to)| (from, to));
                            Some(EchoRow {
                                key: (index, row_hash(&line, &diags, &hints, link, folded, guides)),
                                index,
                                line,
                                diags,
                                folded,
                                guides,
                                hints,
                                link,
                            })
                        })
                        .collect::<Vec<_>>()
                })
            })
        })
    }

    /// Where `line` sits in the scroller's visible box: (pixels from the top
    /// of the view, view height). The overlays decide their direction with
    /// this — a card that always opens downward is unreadable for exactly the
    /// lines nearest the dock, which is where the eye spends half its time.
    pub(super) fn line_in_view(self, line: u32) -> Option<(f64, f64)> {
        let Self {
            state,
            scroller,
            zoom,
            ..
        } = self;
        scroller.get_untracked().map(|el| {
            (
                row_top(state, line, zoom.get()) - f64::from(el.scroll_top()),
                f64::from(el.client_height()),
            )
        })
    }

    /// Downward-opening overlays flip up past ~55% of the view.
    pub(super) fn opens_up(self, line: u32) -> bool {
        self.line_in_view(line)
            .is_some_and(|(top, height)| top > height * 0.55)
    }

    /// What was typed, or deleted, in the textarea itself.
    pub(super) fn on_input(self, event: ev::Event) {
        let Self {
            state,
            area,
            scroller,
            is_rust,
            ..
        } = self;
        let path = self.path.get_value();
        // Anything that reaches the textarea itself went in at its own
        // selection alone — an input method's, a dropped text — so the
        // other cursors go.
        multi_compose(state);
        // The textarea holds the screen text, so what comes out of an
        // input event is the screen *after* the edit. Turning that back
        // into a document edit is the one write path folding introduces,
        // and it is the reason `fold::splice` is a pure function with its
        // own tests rather than something written inline here.
        let screen_now = event_target_value(&event);
        // The screen as it was: the draft and the folds have not moved
        // yet, so re-deriving it is exact and needs no second signal to
        // keep in step with every programmatic `set_value`.
        let screen_was = screen(state);
        // Typing, or deleting: a deletion widens a popup that is open and
        // never opens one.
        let typing = screen_now.len() >= screen_was.len();
        let (new, folds) = rusty_edit::fold::splice(
            &state.editor.draft.get_untracked(),
            &state.editor.folds.get_untracked(),
            &screen_was,
            &screen_now,
        );
        state.editor.folds.set(folds);
        record_edit(state);
        echo_edit(state, &new);
        state.editor.draft.set(new.clone());
        controller::schedule_pulse(state);
        if let Some(element) = area.get_untracked() {
            keep_caret_in_view(&element, state, scroller);
        }

        if let Some(element) = area.get_untracked() {
            typed_triggers(state, &path, is_rust, &element, typing);
        }
    }
}
