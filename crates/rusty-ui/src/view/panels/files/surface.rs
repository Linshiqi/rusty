//! The editing surface itself: the `<pre>` and the `<textarea>` over it.
//!
//! Every keystroke the editor answers to arrives here and is routed on — to
//! the modal state machine, to completion, to find, to the language server.
//! The two layers must agree on font, size and line height exactly, or the
//! caret drifts from its glyph a column at a time across the line.

use leptos::{ev, html, prelude::*};
use wasm_bindgen::{JsCast, closure::Closure};

use rusty_edit::{Document, Line};
use rusty_lsp::{CompletionItem, FileDiagnostic};

use rusty_i18n::t;

use super::*;
use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, MenuItem, MenuSeparator},
    view::icon::{Icon, IconView},
};

/// One row of the margin, and everything its markup depends on — which is
/// also its key, so a row is rebuilt only when something it draws changed.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GutterRow {
    line: u32,
    chevron: bool,
    collapsed: bool,
    folds_column: bool,
    icon_px: u32,
}

/// One row of the echo, ready to draw. Keyed by its line and a hash of
/// everything drawn on it — its runs, the squiggles over it, its fold — so
/// the window's `<For>` rebuilds a row only when what it shows changed:
/// typing on a line redraws that line, not the screen.
#[derive(Clone)]
struct EchoRow {
    key: (u32, u64),
    index: u32,
    line: Line,
    diags: Vec<FileDiagnostic>,
    folded: Option<u32>,
}

/// The two stacked layers: highlighted text underneath, a transparent text
/// area on top taking every keystroke.
///
/// The painted layer follows `state.editor.highlighted`, not the document: on each
/// keystroke the edited lines are patched in plainly so the text under the
/// caret is never stale, and a debounced re-highlight restores the colours.
/// Without the immediate patch, typed characters are invisible for a quarter
/// of a second — the textarea's own glyphs are transparent by design.
#[component]
pub(super) fn Surface(document: Document, area: NodeRef<html::Textarea>) -> impl IntoView {
    let state = AppState::expect();
    let scroller: NodeRef<html::Div> = NodeRef::new();
    let path = document.path.clone();
    let read_only = document.read_only;
    // Hover only means something where a language server is listening.
    let is_rust = path.ends_with(".rs");
    // In no crate's module tree — one rule, shared with the tree and the tab
    // strip, so the three cannot dim different files.
    let unlinked = {
        let path = path.clone();
        Signal::derive(move || state.is_unlinked(&path))
    };

    // The cell the mouse was last over, and a generation so only the newest
    // 400ms-old position asks the server. Hover is ambient: it must cost
    // nothing while the mouse is moving and only speak once it has settled.
    let hover_cell = RwSignal::new(None::<(u32, u32)>);
    let hover_gen = RwSignal::new(0u64);
    // True while the pointer is over the card itself. Reading the card —
    // scrolling it, selecting from it — must not count as leaving.
    let on_card = RwSignal::new(false);
    let editor_menu = RwSignal::new(None::<(f64, f64)>);
    // Bumped by the textarea's `selectionchange`, which is the one event every
    // way of moving the caret fires — a key, a click, a find, undo, a reveal —
    // so the drawn Vim cursor follows all of them without a list of sites to
    // keep in step. And the blink restart: which of two identical animations
    // the cursor runs flips on every move (`input.css`).
    let selection_moves = RwSignal::new(0u32);

    let zoom = state.editor.zoom;

    // The scroller's viewport: how far down it is scrolled and how tall it
    // is. The two layers draw the rows in it and a margin (`window.rs`),
    // never the file. The height follows the dividers and the window; the
    // observer's callback is forgotten rather than kept, because one firing
    // after the view has gone would call a dropped closure, and this one reads
    // only through `try_` and finds nothing.
    let view_top = RwSignal::new(0.0_f64);
    let view_height = RwSignal::new(0.0_f64);
    let observer = StoredValue::new_local(None::<web_sys::ResizeObserver>);
    Effect::new(move |_| {
        let Some(element) = scroller.get() else {
            return;
        };
        view_height.set(f64::from(element.client_height()));
        let measure = Closure::<dyn FnMut()>::new(move || {
            if let Some(Some(element)) = scroller.try_get_untracked() {
                let _ = view_height.try_set(f64::from(element.client_height()));
                let _ = view_top.try_set(f64::from(element.scroll_top()));
            }
        });
        if let Ok(watch) = web_sys::ResizeObserver::new(measure.as_ref().unchecked_ref()) {
            watch.observe(&element);
            observer.update_value(|slot| {
                if let Some(old) = slot.replace(watch) {
                    old.disconnect();
                }
            });
        }
        measure.forget();
    });
    on_cleanup(move || {
        observer.try_update_value(|slot| {
            if let Some(watch) = slot.take() {
                watch.disconnect();
            }
        });
    });
    // How many lines the document has, how many rows they take with the folds
    // collapsed, and which of those rows are drawn.
    let line_count = Memo::new(move |_| state.editor.highlighted.with(Vec::len).max(1) as u32);
    let rows_total = Memo::new(move |_| {
        let lines = line_count.get();
        state.editor.folds.with(|folds| folds.rows(lines))
    });
    let window = Memo::new(move |_| {
        rows_to_draw(
            view_top.get(),
            view_height.get(),
            row_height(zoom.get()),
            rows_total.get(),
        )
    });
    // The height of `rows` rows, as a spacer's style.
    let spacer = move |rows: u32| format!("height: {}px", f64::from(rows) * row_height(zoom.get()));
    // The widest line, which the text column is at least as wide as.
    let widest = Memo::new(move |_| {
        state
            .editor
            .draft
            .with(|text| text.split('\n').map(line_px).fold(0.0, f64::max))
    });
    // Where the caret is in the document while nothing is selected: what the
    // bracket beside it and the name under it are marked from. It follows
    // every way the caret moves (`selection_moves`) and every edit.
    let caret_at = Memo::new(move |_| {
        selection_moves.track();
        state.editor.draft.track();
        let element = area.get()?;
        let (from, to) = doc_selection(&element, state);
        (from == to).then(|| {
            state
                .editor
                .draft
                .with_untracked(|text| line_col_of_byte(text, from))
        })
    });
    let brackets = Memo::new(move |_| {
        let at = caret_at.get()?;
        state
            .editor
            .highlighted
            .with(|lines| bracket_pair(lines, at))
    });
    // The other places the name under the caret occurs, asked for once the
    // caret has rested — later than the edit pulse, so the server has the
    // text the position is in.
    let occurrence_wait = StoredValue::new(0u64);
    {
        let path = path.clone();
        Effect::new(move |_| {
            let at = caret_at.get();
            let turn = occurrence_wait.get_value() + 1;
            occurrence_wait.set_value(turn);
            let Some((line, col)) = at.filter(|_| is_rust) else {
                return;
            };
            let path = path.clone();
            set_timeout(
                move || {
                    if occurrence_wait.try_get_value() == Some(turn) {
                        controller::request_highlights(state, path, line, col);
                    }
                },
                std::time::Duration::from_millis(400),
            );
        });
    }
    // The document lines drawn, which a long file's semantic colours are
    // asked for around (`controller::request_semantic`) — and asked for again
    // once a scroll settles outside what the last answer covered.
    let semantic_wait = StoredValue::new(0u64);
    {
        let path = path.clone();
        Effect::new(move |_| {
            let range = window.get();
            let drawn = state.editor.folds.with_untracked(|folds| {
                (folds.doc_of_view(range.start), folds.doc_of_view(range.end))
            });
            state.editor.drawn_lines.set_value(drawn);
            if !is_rust || controller::semantic_covers(state, drawn.0, drawn.1) {
                return;
            }
            let turn = semantic_wait.get_value() + 1;
            semantic_wait.set_value(turn);
            let path = path.clone();
            set_timeout(
                move || {
                    if semantic_wait.try_get_value() == Some(turn) {
                        controller::request_semantic(state, path);
                    }
                },
                std::time::Duration::from_millis(150),
            );
        });
    }

    // Which lines can be run, keyed by line. Derived from the *draft* rather
    // than from the document, so an arrow appears beside a test the moment it
    // is typed rather than on the next save, and goes with it when it is
    // deleted. The scan is lexical and cheap; a `Memo` keeps it to once per
    // edit rather than once per gutter row.
    let runnables = Memo::new(move |_| {
        if !is_rust {
            return Vec::new();
        }
        rusty_edit::tests_in::runnables(&state.editor.draft.get())
    });

    // Which lines head a foldable region. Memoised for the same reason and a
    // sharper one: the scan walks forward from every line, so asking it once
    // per gutter row made drawing a thousand-line file quadratic in the
    // number of rows on screen.
    let foldables = Memo::new(move |_| rusty_edit::fold::regions(&state.editor.draft.get()));

    // The margin's rows in the window. The icons scale with the row: a fixed
    // 13px chevron is taller than the row itself once the editor is zoomed
    // out far enough, and a row that out-grows its line height pushes every
    // number below it down — the gutter walks away from the code a row at a
    // time.
    let gutter_rows = move || {
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
    };

    // The echo's rows in the window, each with what is drawn on it. Hidden
    // lines are not among them: the echo must drop exactly the lines the
    // textarea dropped, or every caret below sits on the wrong glyph.
    let echo_rows = {
        let path = path.clone();
        move || {
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
                                Some(EchoRow {
                                    key: (index, row_hash(&line, &diags, folded)),
                                    index,
                                    line,
                                    diags,
                                    folded,
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                })
            })
        }
    };

    // Where `line` sits in the scroller's visible box: (pixels from the top
    // of the view, view height). The overlays decide their direction with
    // this — a card that always opens downward is unreadable for exactly the
    // lines nearest the dock, which is where the eye spends half its time.
    let line_in_view = move |line: u32| {
        scroller.get_untracked().map(|el| {
            (
                row_top(state, line, zoom.get()) - f64::from(el.scroll_top()),
                f64::from(el.client_height()),
            )
        })
    };
    // Downward-opening overlays flip up past ~55% of the view.
    let opens_up =
        move |line: u32| line_in_view(line).is_some_and(|(top, height)| top > height * 0.55);

    // Which completion row the keyboard is on: the first, whenever the list
    // changes — a new answer, or a letter that narrows it. Kept across a
    // narrowing, the row under the keyboard became another item, which Enter
    // then accepted.
    let picked = RwSignal::new(0usize);
    Effect::new(move |_| {
        state.editor.completion.track();
        state.editor.draft.track();
        picked.set(0);
    });
    let picked_action = RwSignal::new(0usize);
    Effect::new(move |_| {
        let _ = state.editor.actions.get();
        picked_action.set(0);
    });
    // The strip is remembered whenever it changes, so a crash loses nothing.
    // Keyed on the paths, not the documents: `document` is replaced by every
    // save's re-read, and each of those was a `workbench.toml` write about a
    // strip that had not changed.
    Effect::new(move |previous: Option<(Vec<String>, Option<String>)>| {
        let key = (state.editor.tabs.get(), state.active_path());
        if previous.as_ref() != Some(&key) {
            controller::remember_tabs(state);
        }
        key
    });

    // Apply a pending goto once this document is the one on screen.
    {
        let path = path.clone();
        Effect::new(move |_| {
            let Some(target) = state.editor.reveal.get() else {
                return;
            };
            if target.path != path || state.editor.highlighted.with(Vec::is_empty) {
                return;
            }
            state.editor.reveal.set(None);
            // A jump beats a parked viewport: the tab may have been fronted
            // with one pending, and a goto into it lands on the target.
            state.editor.viewport.set(None);
            // Deferred one tick: on a freshly mounted editor the textarea's
            // value lands after this effect runs, and a selection set before
            // the value is snapped to the end when the text arrives — the
            // caret ended at EOF instead of the target line, every time a
            // goto opened a file that was not already on screen.
            set_timeout(
                move || {
                    let Some(element) = area.get_untracked() else {
                        return;
                    };
                    let offset = utf16_offset_of(
                        &state.editor.draft.get_untracked(),
                        target.line,
                        target.col,
                    );
                    // preventScroll, because the browser's own focus scroll
                    // lands asynchronously and overwrote the deliberate one
                    // below — the jump ended wherever Chrome felt like.
                    let options = web_sys::FocusOptions::new();
                    options.set_prevent_scroll(true);
                    let _ = element.focus_with_options(&options);
                    let _ = element.set_selection_start(Some(offset));
                    let _ = element.set_selection_end(Some(offset));
                    if let Some(scroller) = scroller.get_untracked() {
                        // A third of the viewport above the target line, so
                        // the jump lands in context rather than at the top
                        // edge.
                        let top = f64::from(row_for(state, target.line))
                            * row_height(zoom.get_untracked())
                            - 120.0;
                        scroller.set_scroll_top(top.max(0.0) as i32);
                    }
                },
                std::time::Duration::ZERO,
            );
        });
    }

    // Put a fronted tab back where it was left: the caret first, without
    // scrolling, then the scroller — so what decides where the eye lands is
    // what was on screen, not where the caret happened to be. Deferred one
    // tick for the reason the goto above is, and re-read when the tick
    // fires: a jump that arrived in between has cleared it and must win.
    {
        let path = path.clone();
        Effect::new(move |_| {
            let Some(pending) = state.editor.viewport.get() else {
                return;
            };
            if pending.path != path || state.editor.highlighted.with(Vec::is_empty) {
                return;
            }
            let mine = path.clone();
            set_timeout(
                move || {
                    let still = state.editor.viewport.get_untracked();
                    if still.as_ref().is_none_or(|it| it.path != mine) {
                        return;
                    }
                    state.editor.viewport.set(None);
                    if let (Some(element), Some((line, col))) =
                        (area.get_untracked(), pending.caret)
                    {
                        let offset =
                            utf16_offset_of(&state.editor.draft.get_untracked(), line, col);
                        let options = web_sys::FocusOptions::new();
                        options.set_prevent_scroll(true);
                        let _ = element.focus_with_options(&options);
                        let _ = element.set_selection_start(Some(offset));
                        let _ = element.set_selection_end(Some(offset));
                    }
                    if let Some(scroller) = scroller.get_untracked() {
                        scroller.set_scroll_top(pending.top);
                        scroller.set_scroll_left(pending.left);
                    }
                },
                std::time::Duration::ZERO,
            );
        });
    }

    // Both layers carry this verbatim. Any difference in font, size or line
    // height and the caret walks away from its glyph. Ctrl+wheel scales the
    // whole thing; every pixel computed below multiplies by the same factor.
    let metrics = Signal::derive(move || {
        let z = zoom.get();
        format!(
            "font-size: {}px; line-height: {}px; \
             font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; \
             tab-size: {TAB_SIZE}",
            FONT_SIZE * z,
            row_height(z),
        )
    });

    let on_input = {
        let path = path.clone();
        move |event: ev::Event| {
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
    };

    view! {
        <div class="relative flex min-h-0 flex-1 flex-col">
            <FindBar area=area scroller=scroller />
            <RenameBar />

            // The editor's own menu. It keeps the clipboard three every text
            // box has, and adds what only this editor knows: where a name is
            // defined, what rust-analyzer would fix, how the file formats.
            {
                let path = path.clone();
                move || {
                    let (x, y) = editor_menu.get()?;
                    let close = Callback::new(move |_| editor_menu.set(None));
                    let path = path.clone();
                    let has_selection = area
                        .get_untracked()
                        .zip(Some(state.editor.draft.get_untracked()))
                        .and_then(|(element, text)| selection_of(&element, &text))
                        .is_some();
                    let (goto_path, fix_path) = (path.clone(), path.clone());
                    Some(
                        view! {
                            <ContextMenu x=x y=y on_close=close>
                                <MenuItem
                                    label=t!("context.editor-cut")
                                    shortcut="Ctrl+X"
                                    disabled=read_only
                                    on_select=Callback::new(move |_| {
                                        // With nothing selected, the line — the
                                        // same rule as the key.
                                        if let Some(element) = area.get_untracked()
                                            && !has_selection
                                        {
                                            clipboard_key(state, &element, true, read_only);
                                        } else if let Some(element) = area.get_untracked() {
                                            let text = state.editor.draft.get_untracked();
                                            if let Some((from, to, picked)) =
                                                selection_of(&element, &text)
                                            {
                                                copy_to_clipboard(&picked);
                                                record_edit(state);
                                                let mut next = text.clone();
                                                next.replace_range(from..to, "");
                                                echo_edit(state, &next);
                                                set_buffer(state, &element, &next);
                                                let caret = utf16_len(&next[..from]);
                                                let _ = element.set_selection_start(Some(caret));
                                                let _ = element.set_selection_end(Some(caret));
                                                controller::schedule_pulse(state);
                                            }
                                        }
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("context.editor-copy")
                                    shortcut="Ctrl+C"
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked()
                                            && !has_selection
                                        {
                                            clipboard_key(state, &element, false, read_only);
                                        } else if let Some(element) = area.get_untracked() {
                                            let text = state.editor.draft.get_untracked();
                                            if let Some((_, _, picked)) =
                                                selection_of(&element, &text)
                                            {
                                                copy_to_clipboard(&picked);
                                            }
                                        }
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("context.editor-paste")
                                    shortcut="Ctrl+V"
                                    disabled=read_only
                                    on_select=Callback::new(move |_| {
                                        paste_at_caret(state, area);
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                                <MenuItem
                                    label=t!("context.editor-definition")
                                    shortcut="Ctrl+Click"
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked()
                                            && let Some((row, col)) =
                                                caret_line_col(&element, &screen(state))
                                        {
                                            controller::goto_definition(
                                                state,
                                                goto_path.clone(),
                                                line_of_row(state, row),
                                                col,
                                            );
                                        }
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("menu.view.references")
                                    shortcut="Shift+F12"
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        editor_menu.set(None);
                                        controller::find_places(state, controller::PlaceQuery::References);
                                    })
                                />
                                <MenuItem
                                    label=t!("menu.view.implementations")
                                    shortcut="Ctrl+F12"
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        editor_menu.set(None);
                                        controller::find_places(
                                            state,
                                            controller::PlaceQuery::Implementations,
                                        );
                                    })
                                />
                                <MenuItem
                                    label=t!("menu.view.type-definition")
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        editor_menu.set(None);
                                        controller::find_places(
                                            state,
                                            controller::PlaceQuery::TypeDefinition,
                                        );
                                    })
                                />
                                <MenuItem
                                    label=t!("menu.view.call-hierarchy")
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        editor_menu.set(None);
                                        controller::show_call_hierarchy(state);
                                    })
                                />
                                <MenuItem
                                    label=t!("menu.view.expand-macro")
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        editor_menu.set(None);
                                        controller::expand_macro(state);
                                    })
                                />
                                <MenuItem
                                    label=t!("context.editor-quick-fix")
                                    shortcut="Ctrl+."
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked()
                                            && let Some((row, col)) =
                                                caret_line_col(&element, &screen(state))
                                        {
                                            controller::request_actions(
                                                state,
                                                fix_path.clone(),
                                                line_of_row(state, row),
                                                col,
                                            );
                                        }
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                                <MenuItem
                                    label=t!("context.editor-fold-all")
                                    on_select=Callback::new(move |_| {
                                        fold_all(state);
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("context.editor-unfold-all")
                                    on_select=Callback::new(move |_| {
                                        unfold_all(state);
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                                <MenuItem
                                    label=t!("context.editor-save")
                                    shortcut="Ctrl+S"
                                    disabled=read_only
                                    on_select=Callback::new(move |_| {
                                        format_and_save(state, area);
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("context.editor-find")
                                    shortcut="Ctrl+F"
                                    on_select=Callback::new(move |_| {
                                        state.find.open.set(true);
                                        editor_menu.set(None);
                                    })
                                />
                            </ContextMenu>
                        },
                    )
                }
            }
        <div
            node_ref=scroller
            // Tagged with the group, so parking reads this group's offset
            // and never the other's.
            data-scroller=state.group.index().to_string()
            class="relative min-h-0 flex-1 overflow-auto"
            // Ctrl+wheel scales the editor font, as every editor since
            // forever. The browser's own page zoom is exactly what this
            // prevent_default suppresses.
            on:scroll=move |_| {
                if let Some(element) = scroller.get_untracked() {
                    view_top.set(f64::from(element.scroll_top()));
                }
            }
            on:wheel=move |event: ev::WheelEvent| {
                if !event.ctrl_key() {
                    return;
                }
                event.prevent_default();
                let step = if event.delta_y() < 0.0 { 1.1 } else { 1.0 / 1.1 };
                let (min, max) = crate::state::EDITOR_ZOOM_RANGE;
                zoom.update(|z| *z = (*z * step).clamp(min, max));
                crate::state::remember_zoom(zoom.get_untracked());
            }
        >
            // w-max: the row is as wide as the longest line, so the textarea
            // overlay (inset-0 in the column beside the gutter) covers every
            // glyph. At viewport width, a long line overflowed the column and
            // the caret inside it lived in the textarea's own hidden scroll —
            // drifting away from the echoed text.
            <div class="flex min-h-full w-max min-w-full">
                // Line numbers scroll with the text rather than floating, so a
                // long file's numbers stay beside their lines. Only the rows in
                // the window are drawn, between spacers as tall as the rows
                // above and below it — the echo beside it draws the same rows.
                <div
                    class="flex-none py-2 pr-2 pl-3 text-right text-label-4 select-none"
                    style=move || {
                        // Tailwind's border-box made a bare `width: 5ch` mean
                        // "5ch including 20px of padding", which left 4-digit
                        // numbers 14px of room — they clipped against the code
                        // column. The width now names the digits and adds the
                        // padding explicitly.
                        let digits = line_count.get().to_string().len().max(3);
                        // Padding, the breakpoint dot and its gap, plus a
                        // column for the fold chevron when the file has
                        // anything to fold. Reserving it unconditionally would
                        // push the code right by two characters in every flat
                        // file; reserving *nothing* was the bug that once made
                        // the run arrows invisible — the row is `justify-end`,
                        // so anything that does not fit overflows off the left
                        // edge rather than wrapping or scrolling. (The arrows
                        // have since moved beside the item, as a lens.)
                        let columns = usize::from(foldables.with(|found| !found.is_empty()));
                        let extra = 32 + columns * 17;
                        // The dot's column, then the digits, then the padding —
                        // a width that only counted digits clipped the number
                        // the moment a dot appeared.
                        format!("{}; width: calc({digits}ch + {extra}px)", metrics.get())
                    }
                >
                    <div style=move || spacer(window.get().start)></div>
                    <For
                        each=gutter_rows
                        key=|row| *row
                        children={
                            let path = path.clone();
                            move |row: GutterRow| {
                                let GutterRow { line, chevron, collapsed, folds_column, icon_px } = row;
                                let n = line + 1;
                                let file = path.clone();
                                let toggle = file.clone();
                                let marked = Signal::derive(move || {
                                    state.debug.breakpoints.with(|list| {
                                        list.iter().any(|(f, l)| f == &file && *l == line)
                                    })
                                });
                                // Fold control. Shown only where something can
                                // collapse, and only on hover unless it is
                                // already folded — a chevron on every second
                                // line is a margin nobody can read past.
                                let chevron = chevron.then(|| {
                                    // VSCode's shape: a stroked chevron, down
                                    // when the region is open and turned a
                                    // quarter right when it is collapsed. A
                                    // filled triangle reads as a disclosure
                                    // widget from a different decade and,
                                    // worse, as the run arrow's sibling rather
                                    // than as a different kind of control.
                                    let class = if collapsed {
                                        "flex shrink-0 -rotate-90 items-center text-label-2"
                                    } else {
                                        "flex shrink-0 items-center text-transparent \
                                         group-hover:text-label-3"
                                    };
                                    let title = if collapsed {
                                        t!("files.unfold")
                                    } else {
                                        t!("files.fold")
                                    };
                                    view! {
                                        <button
                                            type="button"
                                            title=title
                                            on:click=move |event: ev::MouseEvent| {
                                                event.stop_propagation();
                                                toggle_fold(state, line);
                                            }
                                            class=class
                                        >
                                            <IconView icon=Icon::Chevron size=icon_px />
                                        </button>
                                    }
                                });
                                // Each decoration gets a slot of its own on
                                // *every* row, occupied or not. The row is
                                // `justify-end`, so a line with no chevron lets
                                // its number slide right into the chevron's
                                // place — and one number out of step with its
                                // neighbours reads as the gutter having lost
                                // track of the file.
                                let slot = format!("width: {icon_px}px");
                                view! {
                                    // The dot sits *left of* the number, as every
                                    // editor with a breakpoint margin puts it:
                                    // replacing the number meant setting a
                                    // breakpoint cost you the line you were on.
                                    // Each number is a breakpoint target, as in
                                    // every debugger since the first one with a
                                    // mouse, and keeps its real number: a folded
                                    // file whose numbers renumbered themselves
                                    // would make every compiler error point at
                                    // the wrong place.
                                    <div
                                        on:click=move |_| {
                                            controller::debug_breakpoint(state, toggle.clone(), line)
                                        }
                                        title=t!("files.breakpoint")
                                        class="group flex cursor-pointer items-center justify-end gap-1.5"
                                    >
                                        <span class=move || {
                                            if marked.get() {
                                                "text-crimson"
                                            } else {
                                                // Faint under the pointer, invisible
                                                // otherwise: a margin that looks
                                                // inert is a margin nobody clicks.
                                                "text-transparent group-hover:text-crimson/50"
                                            }
                                        }>
                                            "●"
                                        </span>
                                        <span>{n.to_string()}</span>
                                        // Right of the number, hard against the
                                        // code, which is where VSCode puts it —
                                        // the chevron belongs to the line it
                                        // opens, and on the far side of the
                                        // margin it reads as another breakpoint
                                        // control.
                                        {folds_column
                                            .then(|| {
                                                view! {
                                                    <span
                                                        class="flex shrink-0 items-center justify-center"
                                                        style=slot
                                                    >
                                                        {chevron}
                                                    </span>
                                                }
                                            })}
                                    </div>
                                }
                            }
                        }
                    />
                    <div style=move || spacer(rows_total.get().saturating_sub(window.get().end))></div>
                </div>

                <div class="relative min-w-0 flex-1">
                    // Find matches, washed under the text. Rectangles rather
                    // than woven spans: the wash must not disturb the span
                    // structure the caret math and diagnostics rely on. Only
                    // the matches in the window, their lines found in one walk
                    // down the text.
                    {move || {
                        if !state.find.open.get() {
                            return ().into_any();
                        }
                        let query = state.find.query.get();
                        let case = state.find.case.get();
                        let chosen = state.find.index.get();
                        let range = window.get();
                        let z = zoom.get();
                        state
                            .editor
                            .draft
                            .with(|text| {
                                let matches = find_matches(text, &query, case);
                                let current = chosen.min(matches.len().saturating_sub(1));
                                match_lines(text, &matches)
                                    .into_iter()
                                    .enumerate()
                                    .filter(|(_, found)| {
                                        !state.editor.folds.with(|folds| folds.hides(found.line))
                                            && range.contains(&row_for(state, found.line))
                                    })
                                    .map(|(index, found)| {
                                        let x = col_left(found.text, 0, found.col, z);
                                        let width = ((column_px(found.text, 0, found.end_col)
                                            - column_px(found.text, 0, found.col))
                                            * z)
                                            .max(2.0);
                                        let y = row_top(state, found.line, z);
                                        let wash = if index == current {
                                            "pointer-events-none absolute rounded-[3px] bg-amber-fill"
                                        } else {
                                            "pointer-events-none absolute rounded-[3px] bg-selection"
                                        };
                                        view! {
                                            <div
                                                class=wash
                                                style=format!(
                                                    "left: {x}px; top: {y}px; width: {width}px; height: {h}px",
                                                    h = row_height(z),
                                                )
                                            />
                                        }
                                    })
                                    .collect_view()
                            })
                            .into_any()
                    }}
                    // The other places the name at the caret occurs, washed
                    // under the text as a find match is, in a colour of their
                    // own.
                    {
                        let path = path.clone();
                        move || {
                            let range = window.get();
                            let z = zoom.get();
                            state
                                .editor
                                .occurrences
                                .with(|found| {
                                    let Some((_, ranges)) = found
                                        .as_ref()
                                        .filter(|(for_path, _)| for_path == &path)
                                    else {
                                        return ().into_any();
                                    };
                                    state
                                        .editor
                                        .draft
                                        .with(|text| {
                                            ranges
                                                .iter()
                                                .filter(|r| {
                                                    r.start_line == r.end_line
                                                        && !state.editor.folds.with(|f| f.hides(r.start_line))
                                                        && range.contains(&row_for(state, r.start_line))
                                                })
                                                .filter_map(|r| {
                                                    let content = text.split('\n').nth(r.start_line as usize)?;
                                                    let x = col_left(content, 0, r.start_col, z);
                                                    let width = ((column_px(content, 0, r.end_col)
                                                        - column_px(content, 0, r.start_col))
                                                        * z)
                                                        .max(2.0);
                                                    let y = row_top(state, r.start_line, z);
                                                    Some(view! {
                                                        <div
                                                            class="pointer-events-none absolute rounded-[3px] bg-slate-fill"
                                                            style=format!(
                                                                "left: {x}px; top: {y}px; width: {width}px; height: {h}px",
                                                                h = row_height(z),
                                                            )
                                                        />
                                                    })
                                                })
                                                .collect_view()
                                        })
                                        .into_any()
                                })
                        }
                    }
                    // The bracket beside the caret and the one it pairs with,
                    // outlined — so where a block ends is a glance, not a count.
                    {move || {
                        let Some((open, close)) = brackets.get() else {
                            return ().into_any();
                        };
                        let range = window.get();
                        let z = zoom.get();
                        state
                            .editor
                            .draft
                            .with(|text| {
                                [open, close]
                                    .into_iter()
                                    .filter(|(line, _)| {
                                        !state.editor.folds.with(|f| f.hides(*line))
                                            && range.contains(&row_for(state, *line))
                                    })
                                    .filter_map(|(line, col)| {
                                        let content = text.split('\n').nth(line as usize)?;
                                        let x = col_left(content, 0, col, z);
                                        let width = (column_px(content, 0, col + 1)
                                            - column_px(content, 0, col))
                                            * z;
                                        let y = row_top(state, line, z);
                                        Some(view! {
                                            <div
                                                class="pointer-events-none absolute rounded-[2px] ring-1 ring-label-3"
                                                style=format!(
                                                    "left: {x}px; top: {y}px; width: {width}px; height: {h}px",
                                                    h = row_height(z),
                                                )
                                            />
                                        })
                                    })
                                    .collect_view()
                            })
                            .into_any()
                    }}
                    <pre
                        class=move || {
                            let base = "pointer-events-none m-0 overflow-visible py-2 pr-4 \
                                        pl-2 whitespace-pre";
                            // Drained when no `mod` declares the file, because
                            // rust-analyzer is not analysing a word of it. The
                            // name being dim in the tree, the tab and the header
                            // is missable — on a selected row it is a shade
                            // against a highlight — and the code is where the
                            // eye actually is.
                            //
                            // This goes past VS Code deliberately, and the
                            // protocol is why: `unlinked-file` arrives as a
                            // Hint over *two characters* with no `Unnecessary`
                            // tag, so there is nothing for VS Code's
                            // `editorUnnecessaryCode.opacity` to act on and it
                            // dims nothing. rusty knows more than the
                            // diagnostic does — it reads the `mod` lines
                            // itself, before the file is ever opened.
                            //
                            // The squiggle dims with the text rather than being
                            // exempted: opacity compounds through a parent, and
                            // a two-character mark at 60% is still plainly a
                            // mark. Hovering it is unaffected — that is the
                            // textarea's job, and the textarea is not dimmed.
                            if unlinked.get() {
                                format!("{base} opacity-60")
                            } else {
                                base.to_string()
                            }
                        }
                        // At least as wide as the widest line, drawn or not:
                        // the textarea over this column holds every line, and
                        // one wider than the column scrolls inside itself —
                        // the caret drifting off its glyph.
                        style=move || {
                            format!(
                                "{}; min-width: {}px",
                                metrics.get(),
                                widest.get() * zoom.get() + PAD_PX + 16.0,
                            )
                        }
                        aria-hidden="true"
                    >
                        <div style=move || spacer(window.get().start)></div>
                        // Keyed by the line and everything drawn on it, so a
                        // keystroke rebuilds the row it changed and a scroll
                        // the rows it brought in (`EchoRow`).
                        <For
                            each=echo_rows
                            key=|row| row.key
                            children=move |row: EchoRow| {
                                // A collapsed header says how much is
                                // underneath it. A bare `…` gives no sense of
                                // whether unfolding costs three lines or three
                                // hundred.
                                let summary = row
                                    .folded
                                    .map(|n| {
                                        let unit = if n == 1 { "line" } else { "lines" };
                                        view! {
                                            <span class="rounded-[3px] bg-selection px-1 text-label-3">
                                                {format!(" ⋯ {n} {unit} ")}
                                            </span>
                                        }
                                    });
                                view! {
                                    <div>
                                        {decorate(row.line, row.index, &row.diags)}
                                        {summary}
                                        // An empty line still occupies one, or
                                        // the caret above sits a row too high
                                        // for the rest of the file.
                                        {"\u{200b}"}
                                    </div>
                                }
                            }
                        />
                        <div style=move || spacer(rows_total.get().saturating_sub(window.get().end))></div>
                    </pre>

                    <textarea
                        node_ref=area
                        // Which group this is, for `controller::editor_area`.
                        // Not an id: there are two of these once the editor
                        // splits, and a lookup by id always finds the left.
                        data-editor=state.group.index().to_string()
                        spellcheck="false"
                        autocapitalize="off"
                        autocomplete="off"
                        disabled=read_only
                        // Normal and visual mode cannot type, and this is
                        // what guarantees it — not `preventDefault` on every
                        // key, which only covers the keys we thought of.
                        //
                        // An IME is the one that got through: `is_composing`
                        // returns before Vim is consulted, so with Chinese
                        // input active a `j` in normal mode composed and
                        // replaced the character the block cursor was on. A
                        // read-only textarea cannot be typed into by anything
                        // — IME, dictation, paste, a key nobody enumerated —
                        // while Vim's own edits go through `set_value`, which
                        // read-only does not touch.
                        //
                        // On the textarea, and it has to be: the `files.rs`
                        // split once left this attribute on the context
                        // menu's Paste row, where Leptos spread it onto a
                        // button and the guard silently guarded nothing.
                        //
                        // And as the attribute, not `prop:readonly`, which is
                        // what this was until the clipboard work measured it:
                        // a `prop:` name is a JavaScript property name, those
                        // are case-sensitive, and the DOM's is `readOnly`. The
                        // lowercase one set an expando nothing reads, so the
                        // guard above was never on — `t.readOnly` was false in
                        // normal mode the whole time.
                        readonly=move || {
                            state.editor.vim_on.get()
                                && state.editor.vim.with(|vim| vim.mode != crate::vim::Mode::Insert)
                        }
                        class=move || {
                            let base = "absolute inset-0 m-0 resize-none overflow-hidden \
                                        border-0 bg-transparent py-2 pr-4 pl-2 whitespace-pre \
                                        text-transparent caret-rust outline-none";
                            // Outside insert mode the cursor is drawn below,
                            // and the textarea's own caret is hidden for the
                            // one moment a paste makes it writable.
                            let modal = state.editor.vim_on.get()
                                && state.editor.vim.with(|vim| vim.mode != crate::vim::Mode::Insert);
                            if modal { format!("{base} vim-modal") } else { base.to_string() }
                        }
                        style=move || metrics.get()
                        on:selectionchange=move |_| selection_moves.update(|n| *n = n.wrapping_add(1))
                        // What the textarea holds is the *screen* text, which
                        // is the draft minus every folded region. Identical to
                        // the draft while nothing is collapsed, so this is a
                        // no-op for a file nobody has folded. Left as it is
                        // while it lags the other view of its file, which
                        // `controller::catch_up` writes when this one is used.
                        prop:value=move || {
                            if state.editor.lagging.get() {
                                return area.get_untracked().map(|a| a.value()).unwrap_or_default();
                            }
                            screen_tracked(state)
                        }
                        on:contextmenu=move |event: ev::MouseEvent| {
                            event.prevent_default();
                            editor_menu
                                .set(Some((
                                    f64::from(event.client_x()),
                                    f64::from(event.client_y()),
                                )));
                        }
                        on:scroll=move |_| {
                            let Some(element) = area.get_untracked() else {
                                return;
                            };
                            let (top, left) = (element.scroll_top(), element.scroll_left());
                            if top != 0 || left != 0 {
                                if let Some(outer) = scroller.get_untracked() {
                                    outer.set_scroll_top(outer.scroll_top() + top);
                                    outer.set_scroll_left(outer.scroll_left() + left);
                                }
                                element.set_scroll_top(0);
                                element.set_scroll_left(0);
                            }
                        }
                        on:input=on_input
                        // A line copied with nothing selected pastes as a line,
                        // and Vim's read-only modes take every paste here
                        // (`edits::paste_into`). The event, not the async
                        // clipboard API: the event carries the text with the
                        // key press, where a read of the clipboard waits on a
                        // permission this WebView never answers.
                        on:paste=move |event: web_sys::ClipboardEvent| {
                            let Some(pasted) = event
                                .clipboard_data()
                                .and_then(|data| data.get_data("text/plain").ok())
                            else {
                                return;
                            };
                            if let Some(element) = area.get_untracked()
                                && paste_into(state, &element, &pasted, read_only)
                            {
                                event.prevent_default();
                            }
                        }
                        // The word, not the word and the space after it —
                        // VS Code's double-click, not Windows' (see
                        // `word_selection_overhang`).
                        on:dblclick=move |_| {
                            let Some(element) = area.get_untracked() else {
                                return;
                            };
                            let (Ok(Some(start)), Ok(Some(end))) =
                                (element.selection_start(), element.selection_end())
                            else {
                                return;
                            };
                            if end <= start {
                                return;
                            }
                            let value = element.value();
                            let picked = &value[byte_of_utf16(&value, start as usize)
                                ..byte_of_utf16(&value, end as usize)];
                            let overhang = word_selection_overhang(picked);
                            if overhang > 0 {
                                let _ = element.set_selection_end(Some(end - overhang));
                            }
                        }
                        on:mousedown={
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                // Ctrl+Click asks where this is defined — the
                                // gesture every editor has taught.
                                if !(event.ctrl_key() || event.meta_key()) || !is_rust {
                                    controller::dismiss_completion(state);
                                    state.editor.signature.set(None);
                                    state.editor.actions.set(None);
                                    return;
                                }
                                event.prevent_default();
                                // A pixel names a *row*; the server wants a
                                // document line. `screen` and `line_of_row`
                                // are both the identity while nothing is
                                // folded.
                                if let Some((row, col)) = cell_under(
                                    &screen(state),
                                    event.offset_x() as f64,
                                    event.offset_y() as f64,
                                    zoom.get_untracked(),
                                ) {
                                    controller::goto_definition(
                                        state,
                                        path.clone(),
                                        line_of_row(state, row),
                                        col,
                                    );
                                }
                            }
                        }
                        on:mousemove={
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                if !is_rust {
                                    return;
                                }
                                let cell = cell_under(
                                    &screen(state),
                                    event.offset_x() as f64,
                                    event.offset_y() as f64,
                                    zoom.get_untracked(),
                                )
                                .map(|(row, col)| (line_of_row(state, row), col));
                                if hover_cell.get_untracked() == cell {
                                    return;
                                }
                                hover_cell.set(cell);

                                // Inside the shown token, there is nothing to
                                // dismiss and nothing to re-request.
                                let inside = state.editor.hover.with_untracked(|h| {
                                    h.as_ref().is_some_and(|card| {
                                        cell.is_some_and(|(l, c)| within(&card.range, l, c))
                                    })
                                });
                                if inside {
                                    return;
                                }

                                // Outside it: a short grace before the card
                                // goes, so the pointer can cross the gap onto
                                // the card without killing it en route.
                                let generation = hover_gen.get_untracked() + 1;
                                hover_gen.set(generation);
                                if state.editor.hover.with_untracked(Option::is_some) {
                                    set_timeout(
                                        move || {
                                            // The surface can be disposed inside the
                                            // grace period — the tab closed, the project
                                            // switched, the file replaced — and these are
                                            // *its* signals, not the app's. Reading a
                                            // disposed one panics, and a panic in wasm
                                            // aborts the module: the window keeps its
                                            // last paint and every handler in it is dead,
                                            // right down to the close button. `try_` is
                                            // None once the owner is gone.
                                            let (Some(current), Some(on_card)) = (
                                                hover_gen.try_get_untracked(),
                                                on_card.try_get_untracked(),
                                            ) else {
                                                return;
                                            };
                                            if current == generation && !on_card {
                                                state.editor.hover.set(None);
                                            }
                                        },
                                        std::time::Duration::from_millis(300),
                                    );
                                }

                                let Some((line, col)) = cell else { return };
                                let path = path.clone();
                                set_timeout(
                                    move || {
                                        // Disposed-safe, as above.
                                        let (Some(current), Some(cell)) = (
                                            hover_gen.try_get_untracked(),
                                            hover_cell.try_get_untracked(),
                                        ) else {
                                            return;
                                        };
                                        if current == generation && cell == Some((line, col)) {
                                            controller::request_hover(state, path, line, col);
                                        }
                                    },
                                    std::time::Duration::from_millis(400),
                                );
                            }
                        }
                        on:mouseleave=move |_| {
                            hover_cell.set(None);
                            let generation = hover_gen.get_untracked() + 1;
                            hover_gen.set(generation);
                            set_timeout(
                                move || {
                                    // Disposed-safe, as above. Leaving the surface is
                                    // exactly when it is most likely to go away.
                                    let (Some(current), Some(on_card)) = (
                                        hover_gen.try_get_untracked(),
                                        on_card.try_get_untracked(),
                                    ) else {
                                        return;
                                    };
                                    if current == generation && !on_card {
                                        state.editor.hover.set(None);
                                    }
                                },
                                std::time::Duration::from_millis(300),
                            );
                        }
                        on:keydown={
                            let path = path.clone();
                            move |event: ev::KeyboardEvent| {
                            // While an IME is composing, Enter confirms the
                            // candidate and Tab moves through them. Stealing
                            // either would break Chinese input entirely.
                            if event.is_composing() {
                                return;
                            }
                            // Modal editing gets the key first, and takes only
                            // what it wants. Everything it passes through —
                            // every chord but five, and the whole of insert
                            // mode — carries on to the handling below and
                            // then to the global bindings, which is why
                            // Ctrl+S, Ctrl+K and the clipboard are unchanged
                            // by turning Vim on.
                            //
                            // `stop_propagation` on the taken ones is the
                            // half that matters: without it a `d` in normal
                            // mode would also reach the window listener.
                            // The popup's Escape comes before Vim's, as the
                            // suggest widget's does in VS Code: the first
                            // press closes the list, the second leaves insert
                            // mode. Vim taking it first left the popup drawn,
                            // with Tab still accepting into it. An ask still
                            // on its way is forgotten either way.
                            if event.key() == "Escape" {
                                let showing =
                                    state.editor.completion.with_untracked(|popup| {
                                        popup.as_ref().is_some_and(|popup| {
                                            !visible_items(
                                                popup,
                                                &state.editor.draft.get_untracked(),
                                            )
                                            .is_empty()
                                        })
                                    });
                                controller::dismiss_completion(state);
                                if showing {
                                    event.prevent_default();
                                    event.stop_propagation();
                                    return;
                                }
                            }
                            if state.editor.vim_on.get_untracked()
                                && let Some(element) = area.get_untracked()
                                && vim_key(state, &element, scroller, &event)
                            {
                                event.prevent_default();
                                event.stop_propagation();
                                return;
                            }
                            // Copy and cut take the selection when there is one
                            // and the whole line when there is not, as VS
                            // Code's do — in Vim's modes as well, since normal
                            // mode's cursor is drawn rather than selected.
                            // Paste is the paste event's, below — Vim's
                            // read-only textarea only has to be let in first.
                            if (event.ctrl_key() || event.meta_key())
                                && !event.alt_key()
                                && !event.shift_key()
                            {
                                match event.key().to_ascii_lowercase().as_str() {
                                    key @ ("c" | "x") => {
                                        if let Some(element) = area.get_untracked()
                                            && clipboard_key(state, &element, key == "x", read_only)
                                        {
                                            event.prevent_default();
                                            return;
                                        }
                                    }
                                    "v" => open_for_paste(state, area, read_only),
                                    _ => {}
                                }
                            }
                            // The actions popup owns its keys while it is up.
                            if state.editor.actions.with_untracked(Option::is_some) {
                                match event.key().as_str() {
                                    "ArrowDown" => {
                                        event.prevent_default();
                                        picked_action.update(|i| *i += 1);
                                        return;
                                    }
                                    "ArrowUp" => {
                                        event.prevent_default();
                                        picked_action.update(|i| *i = i.saturating_sub(1));
                                        return;
                                    }
                                    "Enter" => {
                                        event.prevent_default();
                                        if let Some(element) = area.get_untracked() {
                                            apply_action(
                                                state,
                                                &element,
                                                picked_action.get_untracked(),
                                            );
                                        }
                                        return;
                                    }
                                    "Escape" => {
                                        event.prevent_default();
                                        event.stop_propagation();
                                        state.editor.actions.set(None);
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            // F2 renames the symbol under the caret, the
                            // key every editor uses for it. rust-analyzer has
                            // always been able to do this; rusty never asked.
                            if event.key() == "F2" && !event.ctrl_key() && is_rust {
                                event.prevent_default();
                                // Taken here, so the window's binding for the
                                // same key does not send it back a second time.
                                event.stop_propagation();
                                if let Some(element) = area.get_untracked() {
                                    // The word under the caret is read off the
                                    // screen text, because that is what the
                                    // selection indexes; the line it is on is
                                    // then a document line, because that is
                                    // what the server renames by.
                                    let text = screen(state);
                                    let cursor = scalar_of_units(
                                        &text,
                                        element.selection_start().ok().flatten().unwrap_or(0)
                                            as usize,
                                    );
                                    if let (Some(word), Some((row, col))) =
                                        (word_at(&text, cursor), caret_line_col(&element, &text))
                                    {
                                        let line = line_of_row(state, row);
                                        state
                                            .editor.rename
                                            .set(Some((path.clone(), line, col, word)));
                                    }
                                }
                                return;
                            }
                            // Comment or uncomment, for everyone — Vim's `gc`
                            // reaches the same function. This editor had no
                            // comment toggle at all before, in any mode, and
                            // commenting out a block of pin setup is the most
                            // ordinary thing anyone does while bringing a
                            // board up.
                            if (event.ctrl_key() || event.meta_key()) && event.key() == "/" {
                                event.prevent_default();
                                // And kept from the window, whose `editor.comment`
                                // binding answers Ctrl+/ by sending it to this
                                // textarea again: the comment went on and came
                                // straight back off, so the key did nothing on the
                                // caret's line — and a selection lost the marker
                                // from its first line only.
                                event.stop_propagation();
                                if let Some(element) = area.get_untracked() {
                                    comment_selection(state, &element);
                                }
                                return;
                            }
                            // Ctrl+. asks what the server can fix here.
                            if (event.ctrl_key() || event.meta_key())
                                && event.key() == "."
                                && is_rust
                            {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    let text = screen(state);
                                    if let Some((row, col)) = caret_line_col(&element, &text) {
                                        state.editor.completion.set(None);
                                        controller::request_actions(
                                            state,
                                            path.clone(),
                                            line_of_row(state, row),
                                            col,
                                        );
                                    }
                                }
                                return;
                            }
                            // The popup owns its keys while it *shows*
                            // something. `Some` alone was the test, and a
                            // popup narrowed to nothing — `v.xyz` — was
                            // invisible yet still ate Enter and Tab.
                            let shown = state.editor.completion.with_untracked(|popup| {
                                popup.as_ref().map_or(0, |popup| {
                                    visible_items(popup, &state.editor.draft.get_untracked())
                                        .len()
                                })
                            });
                            if shown > 0 {
                                match event.key().as_str() {
                                    // Round the ends, as VS Code's list does.
                                    "ArrowDown" => {
                                        event.prevent_default();
                                        picked.update(|i| *i = (*i + 1) % shown);
                                        return;
                                    }
                                    "ArrowUp" => {
                                        event.prevent_default();
                                        picked.update(|i| *i = (*i + shown - 1) % shown);
                                        return;
                                    }
                                    "Enter" | "Tab" => {
                                        event.prevent_default();
                                        if let Some(element) = area.get_untracked() {
                                            accept_completion(
                                                state,
                                                &element,
                                                picked.get_untracked(),
                                            );
                                        }
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            // Moving the caret leaves the word the popup is
                            // about, and an answer still on its way would
                            // open it wherever the caret had gone.
                            if matches!(
                                event.key().as_str(),
                                "ArrowLeft"
                                    | "ArrowRight"
                                    | "ArrowUp"
                                    | "ArrowDown"
                                    | "Home"
                                    | "End"
                                    | "PageUp"
                                    | "PageDown"
                            ) {
                                controller::dismiss_completion(state);
                            }
                            if event.key() == "Escape"
                                && state.editor.signature.with_untracked(Option::is_some)
                            {
                                event.prevent_default();
                                event.stop_propagation();
                                state.editor.signature.set(None);
                                return;
                            }
                            if (event.ctrl_key() || event.meta_key())
                                && (event.key().eq_ignore_ascii_case("f")
                                    || event.key().eq_ignore_ascii_case("h"))
                            {
                                event.prevent_default();
                                // Prefill from the selection, as every editor
                                // does — finding the thing under the cursor is
                                // the whole gesture.
                                if let Some(element) = area.get_untracked() {
                                    let text = state.editor.draft.get_untracked();
                                    let from = element
                                        .selection_start()
                                        .ok()
                                        .flatten()
                                        .unwrap_or(0) as usize;
                                    let to = element
                                        .selection_end()
                                        .ok()
                                        .flatten()
                                        .unwrap_or(0) as usize;
                                    if to > from {
                                        let picked = text
                                            [byte_of_utf16(&text, from)..byte_of_utf16(&text, to)]
                                            .to_string();
                                        if !picked.contains('\n') && !picked.is_empty() {
                                            state.find.query.set(picked);
                                            state.find.index.set(0);
                                        }
                                    }
                                }
                                state.find.open.set(true);
                                if event.key().eq_ignore_ascii_case("h") {
                                    state.find.replace_open.set(true);
                                }
                                return;
                            }
                            if event.key() == "F3" && state.find.open.get_untracked() {
                                event.prevent_default();
                                find_jump(state, scroller, if event.shift_key() { -1 } else { 1 });
                                return;
                            }
                            if event.key() == "Escape"
                                && state.find.open.get_untracked()
                                && state
                                    .editor.completion
                                    .with_untracked(Option::is_none)
                                && state.editor.signature.with_untracked(Option::is_none)
                            {
                                event.prevent_default();
                                state.find.open.set(false);
                                state.find.replace_open.set(false);
                                return;
                            }
                            if (event.ctrl_key() || event.meta_key())
                                && event.key().eq_ignore_ascii_case("z")
                                && !event.shift_key()
                            {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    apply_history(&element, state, scroller, true);
                                }
                                return;
                            }
                            if (event.ctrl_key() || event.meta_key())
                                && (event.key().eq_ignore_ascii_case("y")
                                    || (event.key().eq_ignore_ascii_case("z")
                                        && event.shift_key()))
                            {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    apply_history(&element, state, scroller, false);
                                }
                                return;
                            }
                            // Ctrl+Space asks without a trigger character.
                            if event.ctrl_key() && event.key() == " " {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    let text = screen(state);
                                    if let Some((row, col)) = caret_line_col(&element, &text) {
                                        let start = word_start_before(&text, row, col);
                                        let line = line_of_row(state, row);
                                        controller::request_completion(
                                            state,
                                            path.clone(),
                                            line,
                                            col,
                                            start,
                                            true,
                                        );
                                    }
                                }
                                return;
                            }
                            if (event.ctrl_key() || event.meta_key())
                                && event.key().eq_ignore_ascii_case("s")
                            {
                                event.prevent_default();
                                format_and_save(state, area);
                                return;
                            }
                            if read_only {
                                return;
                            }
                            if event.key() == "Enter" {
                                event.prevent_default();
                                // A new line is not the word an answer on its
                                // way was asked for.
                                controller::dismiss_completion(state);
                                if let Some(element) = area.get_untracked() {
                                    let (from, to) = doc_selection(&element, state);
                                    // A selection is replaced by the newline,
                                    // as the browser would have done.
                                    let edit = if from < to {
                                        pairs::Edit {
                                            range: (from, to),
                                            text: "\n".to_string(),
                                            caret: from + 1,
                                            select: None,
                                        }
                                    } else {
                                        pairs::on_enter(&state.editor.draft.get_untracked(), from)
                                    };
                                    apply_edit(&element, state, &edit);
                                    keep_caret_in_view(&element, state, scroller);
                                }
                                return;
                            }
                            // Bracket pairs — the four things every editor
                            // does that a textarea does not: an opener brings
                            // its closer, a closer typed against one steps
                            // over it, a closer on a blank line takes its
                            // opener's indentation, and Backspace inside an
                            // empty pair removes both. The rules are `pairs`,
                            // pure and tested; this applies what they return
                            // and then asks the server what the input event
                            // would have — that event never fires for a key
                            // that was prevented, and a `(` that opened a pair
                            // without asking for the signature would be a
                            // feature taken away by adding one.
                            if !event.ctrl_key() && !event.alt_key() && !event.meta_key() {
                                let key = event.key();
                                let mut chars = key.chars();
                                if let (Some(ch), None) = (chars.next(), chars.next())
                                    && let Some(element) = area.get_untracked()
                                {
                                    let (from, to) = doc_selection(&element, state);
                                    let draft = state.editor.draft.get_untracked();
                                    if let Some(edit) = pairs::on_type(&draft, from, to, ch) {
                                        event.prevent_default();
                                        apply_edit(&element, state, &edit);
                                        typed_triggers(state, &path, is_rust, &element, true);
                                        return;
                                    }
                                }
                                if key == "Backspace"
                                    && let Some(element) = area.get_untracked()
                                {
                                    let (from, to) = doc_selection(&element, state);
                                    if from == to
                                        && let Some(edit) = pairs::on_backspace(
                                            &state.editor.draft.get_untracked(),
                                            from,
                                        )
                                    {
                                        event.prevent_default();
                                        apply_edit(&element, state, &edit);
                                        typed_triggers(state, &path, is_rust, &element, false);
                                        return;
                                    }
                                }
                            }
                            // A text area would move focus on Tab. In an editor
                            // that is never what was meant. A Tab with Ctrl or
                            // Alt is somebody else's — Ctrl+Tab switches files,
                            // and in a window with no switcher (a detached
                            // editor) it must not indent either.
                            if event.key() == "Tab"
                                && !event.ctrl_key()
                                && !event.alt_key()
                                && !event.meta_key()
                            {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    insert_at_caret(&element, state, "    ");
                                }
                            }
                        }}
                    />

                    // Vim's cursor, outside insert mode. The textarea is
                    // read-only there — the guard above — and a browser
                    // paints no caret in a read-only field, so the block the
                    // caret used to be (`caret-shape: block`) was gone from
                    // the release that guard started working. Drawn, it
                    // stands on the cursor Vim's next key starts from
                    // (`vim_cursor`) — in visual mode too, where the
                    // selection alone does not say which end moves — and on
                    // an empty line or past a line's end as well, where there
                    // is no character to select. Like a caret, it shows only
                    // while the textarea has focus (`input.css`).
                    {move || {
                        let modal = state.editor.vim_on.get()
                            && state.editor.vim.with(|vim| vim.mode != crate::vim::Mode::Insert);
                        if !modal {
                            return ().into_any();
                        }
                        let Some(element) = area.get() else {
                            return ().into_any();
                        };
                        let moves = selection_moves.get();
                        let z = zoom.get();
                        let path = state.active_path_now();
                        let (line, x, width) = state.editor.draft.with(|text| {
                            let cursor = vim_cursor(state, &element, path.as_deref(), text);
                            let (line, col, under) = cursor_cell(text, cursor);
                            // As wide as what it covers — a tab up to its
                            // stop — and a space wide on a line break or at
                            // the end of the text, where there is nothing.
                            let width = match under {
                                Some(_) => column_px(text, line, col + 1) - column_px(text, line, col),
                                None => column_px(" ", 0, 1),
                            } * z;
                            (line, col_left(text, line, col, z), width)
                        });
                        let y = row_top(state, line, z);
                        let blink = if moves.is_multiple_of(2) { "vim-cursor-a" } else { "vim-cursor-b" };
                        view! {
                            <div
                                class="vim-cursor"
                                style=format!(
                                    "left: {x}px; top: {y}px; width: {width}px; height: {h}px; \
                                     animation-name: {blink}",
                                    h = row_height(z),
                                )
                            />
                        }
                        .into_any()
                    }}

                    // The tests, offered where VS Code offers them — beside
                    // the item, not at the far edge of the margin — and with
                    // the half the margin never had room for: Debug. An
                    // overlay, not a row of its own: the textarea and the echo
                    // have to stay glyph for glyph, and a row that exists in
                    // one and not the other is a caret that drifts. So the
                    // lens sits on the attribute line above the item (the row
                    // VS Code draws its lens on), after that line's text, or
                    // on the item's own line when nothing is above it.
                    {move || {
                        let found = runnables.get();
                        if found.is_empty() {
                            return ().into_any();
                        }
                        let draft = state.editor.draft.get();
                        let lines: Vec<&str> = draft.lines().collect();
                        let z = zoom.get();
                        let height = row_height(z);
                        let icon_px = (height * 0.6).round().max(7.0) as u32;
                        let range = window.get();
                        found
                            .into_iter()
                            .filter_map(|r| {
                                let (line, col) = lens_anchor(&lines, r.line)?;
                                // Inside a collapsed region the header stands
                                // for the line, and a stack of lenses on one
                                // header would say nothing readable.
                                let row = row_for(state, line);
                                if line_of_row(state, row) != line || !range.contains(&row) {
                                    return None;
                                }
                                let x = col_left(&draft, line, col, z);
                                let y = row_top(state, line, z);
                                let (run_label, run_title) = match r.kind {
                                    rusty_edit::RunnableKind::Module => (
                                        t!("files.lens-run-tests"),
                                        t!("files.run-module", name = r.name.clone()),
                                    ),
                                    rusty_edit::RunnableKind::Test => (
                                        t!("files.lens-run-test"),
                                        t!("files.run-test", name = r.name.clone()),
                                    ),
                                };
                                let debug_title = t!("files.debug-test", name = r.name.clone());
                                let run_filter = r.filter.clone();
                                let debug_filter = r.filter.clone();
                                Some(view! {
                                    <div
                                        class="pointer-events-none absolute z-10 flex items-center gap-2 font-sans text-footnote leading-none text-label-3 select-none"
                                        style=format!("left: {x}px; top: {y}px; height: {height}px")
                                    >
                                        <button
                                            type="button"
                                            title=run_title
                                            // The editor keeps focus: a lens is a
                                            // command, not a place to be.
                                            on:mousedown=|event: ev::MouseEvent| event.prevent_default()
                                            on:click=move |event: ev::MouseEvent| {
                                                event.stop_propagation();
                                                controller::run_test(state, run_filter.clone());
                                            }
                                            class="pointer-events-auto flex items-center gap-1 hover:text-label"
                                        >
                                            <IconView icon=Icon::Play size=icon_px />
                                            {run_label}
                                        </button>
                                        <span class="text-label-4">"|"</span>
                                        <button
                                            type="button"
                                            title=debug_title
                                            on:mousedown=|event: ev::MouseEvent| event.prevent_default()
                                            on:click=move |event: ev::MouseEvent| {
                                                event.stop_propagation();
                                                controller::debug_test(state, debug_filter.clone());
                                            }
                                            class="pointer-events-auto flex items-center gap-1 hover:text-label"
                                        >
                                            <IconView icon=Icon::Bug size=icon_px />
                                            {t!("files.lens-debug")}
                                        </button>
                                    </div>
                                })
                            })
                            .collect_view()
                            .into_any()
                    }}

                    // Where the target is stopped. Drawn under the text like
                    // a find match rather than as a border, so it survives
                    // the caret and the selection sitting on the same line.
                    {
                        let path = path.clone();
                        move || {
                            let debug = state.debug.session.get()?;
                            if debug.running {
                                return None;
                            }
                            let frame = debug.stack.get(debug.frame as usize)?;
                            if frame.file.as_deref() != Some(path.as_str()) {
                                return None;
                            }
                            let line = frame.line?;
                            let y = row_top(state, line, zoom.get());
                            let height = row_height(zoom.get());
                            Some(view! {
                                <div
                                    class="pointer-events-none absolute left-0 w-full bg-amber-fill"
                                    style=format!("top: {y}px; height: {height}px")
                                />
                            })
                        }
                    }

                    // What the server said about the token the mouse settled
                    // on. Interactive: long documentation scrolls inside it,
                    // and reading is not leaving.
                    {
                        let path = path.clone();
                        move || {
                            let Some(card) = state.editor.hover.get() else {
                                return ().into_any();
                            };
                            if card.path != path {
                                return ().into_any();
                            }
                            let (range, text) = (card.range, card.text);
                            let x = 8.0
                                + column_px(
                                    &state.editor.draft.get_untracked(),
                                    range.start_line,
                                    range.start_col,
                                ) * zoom.get();
                            // Above the token when the token is low in the
                            // view — a card clipped by the dock reads as no
                            // card at all.
                            let place = if opens_up(range.start_line) {
                                let y = 8.0
                                    + f64::from(row_for(state, range.start_line)) * row_height(zoom.get())
                                    - 4.0;
                                format!("top: {y}px; transform: translateY(-100%)")
                            } else {
                                let y = 8.0
                                    + f64::from(row_for(state, range.end_line) + 1) * row_height(zoom.get())
                                    + 2.0;
                                format!("top: {y}px")
                            };
                            // The card reads at the editor's own scale: a
                            // zoomed-in buffer with an 11px tooltip under it
                            // reads as two unrelated programs.
                            let font = 11.0 * zoom.get();
                            view! {
                                <div
                                    class="absolute z-20 max-w-[70ch] overflow-y-auto rounded-[8px] bg-raised px-3 py-2 font-mono leading-relaxed whitespace-pre-wrap shadow-2xl ring-1 ring-line-strong select-text"
                                    style=format!(
                                        "left: {x}px; {place}; max-height: 40vh; font-size: {font}px",
                                    )
                                    on:mouseenter=move |_| on_card.set(true)
                                    on:mouseleave=move |_| {
                                        on_card.set(false);
                                        state.editor.hover.set(None);
                                    }
                                >
                                    {hover_parts(&text)}
                                    // What the server offers to do about it,
                                    // one click from the pointer that is
                                    // already there. Only a squiggle has
                                    // these — an `impl Trait for T {}` with
                                    // no members is the case they exist for
                                    // — and until now the only way to them
                                    // was to click into the line and press
                                    // Ctrl+. The card is interactive
                                    // already, so the buttons cost no new
                                    // behaviour: the pointer crossing onto
                                    // it keeps it up.
                                    {(!card.fixes.fixes.is_empty())
                                        .then(|| {
                                            let answer = card.fixes.clone();
                                            let fix_path = card.path.clone();
                                            view! {
                                                <div class="mt-2 flex flex-wrap gap-1.5 border-t border-line pt-2">
                                                    {answer
                                                        .fixes
                                                        .iter()
                                                        .enumerate()
                                                        .map(|(index, fix)| {
                                                            let title = fix.title.clone();
                                                            let hint = if fix.elsewhere.is_empty() {
                                                                title.clone()
                                                            } else {
                                                                format!(
                                                                    "{title} → {}",
                                                                    fix.elsewhere.join(", "),
                                                                )
                                                            };
                                                            let answer = answer.clone();
                                                            let fix_path = fix_path.clone();
                                                            view! {
                                                                <button
                                                                    type="button"
                                                                    title=hint
                                                                    on:mousedown=move |
                                                                        event: ev::MouseEvent,
                                                                    | {
                                                                        event.prevent_default();
                                                                        event.stop_propagation();
                                                                        on_card.set(false);
                                                                        state.editor.hover.set(None);
                                                                        if let Some(element) =
                                                                            area.get_untracked()
                                                                        {
                                                                            apply_fix(
                                                                                state,
                                                                                &element,
                                                                                &fix_path,
                                                                                &answer,
                                                                                index,
                                                                            );
                                                                        }
                                                                    }
                                                                    class="max-w-full truncate rounded-[5px] bg-rust/15 px-2 py-0.5 text-left text-rust hover:bg-rust/25"
                                                                >
                                                                    {title}
                                                                </button>
                                                            }
                                                        })
                                                        .collect_view()}
                                                </div>
                                            }
                                        })}
                                </div>
                            }
                            .into_any()
                        }
                    }

                    // The quick-fix popup, anchored under its line.
                    {
                        let path = path.clone();
                        move || {
                            let Some((for_path, line, answer)) = state.editor.actions.get()
                            else {
                                return ().into_any();
                            };
                            if for_path != path {
                                return ().into_any();
                            }
                            let fixes = answer.fixes;
                            let chosen = picked_action.get().min(fixes.len().saturating_sub(1));
                            let place = card_place(state, line, zoom.get(), opens_up(line));
                            view! {
                                <div
                                    class="absolute z-20 min-w-[280px] rounded-[8px] bg-raised py-1 font-mono text-footnote shadow-2xl ring-1 ring-line-strong"
                                    style=format!("left: 48px; {place}")
                                >
                                    {fixes
                                        .into_iter()
                                        .enumerate()
                                        .map(|(index, fix)| {
                                            let selected = index == chosen;
                                            // What else it changes, or its kind:
                                            // a fix that writes another file
                                            // says which before it is taken.
                                            let kind = if fix.elsewhere.is_empty() {
                                                fix.kind.clone().unwrap_or_default()
                                            } else {
                                                format!("→ {}", fix.elsewhere.join(", "))
                                            };
                                            view! {
                                                <button
                                                    type="button"
                                                    on:mousedown=move |event: ev::MouseEvent| {
                                                        event.prevent_default();
                                                        event.stop_propagation();
                                                        if let Some(element) =
                                                            area.get_untracked()
                                                        {
                                                            apply_action(
                                                                state, &element, index,
                                                            );
                                                        }
                                                    }
                                                    class=if selected {
                                                        "flex w-full items-baseline gap-2 bg-selection px-2.5 py-0.5 text-left text-rust"
                                                    } else {
                                                        "flex w-full items-baseline gap-2 px-2.5 py-0.5 text-left text-label-2"
                                                    }
                                                >
                                                    <span class="shrink-0">{fix.title.clone()}</span>
                                                    <span class="min-w-0 flex-1 truncate text-right text-label-3">
                                                        {kind}
                                                    </span>
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                            .into_any()
                        }
                    }

                    // The signature card, floated above the line whose call
                    // it describes, with the active parameter lit.
                    {
                        let path = path.clone();
                        move || {
                            let Some((for_path, line, info)) = state.editor.signature.get() else {
                                return ().into_any();
                            };
                            if for_path != path {
                                return ().into_any();
                            }
            // Above by nature — it describes the call being typed — but
                            // near the top of the view "above" is off screen,
                            // so it flips below the line there.
                            let near_top = line_in_view(line).is_some_and(|(top, _)| top < 96.0);
                            let place = card_place(state, line, zoom.get(), !near_top);
                            let label = info.label;
                            let split = match (info.param_start, info.param_end) {
                                (Some(start), Some(end)) => {
                                    let start = start as usize;
                                    let end = (end as usize).min(label.len());
                                    if start <= end
                                        && label.is_char_boundary(start)
                                        && label.is_char_boundary(end)
                                    {
                                        Some((start, end))
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            };
                            let (before, active, after) = match split {
                                Some((start, end)) => (
                                    label[..start].to_string(),
                                    label[start..end].to_string(),
                                    label[end..].to_string(),
                                ),
                                None => (label, String::new(), String::new()),
                            };
                            // One line of docs, not the essay — hover exists.
                            let doc = info
                                .doc
                                .as_deref()
                                .and_then(|d| d.lines().find(|l| !l.trim().is_empty()))
                                .map(str::to_string);
                            view! {
                                <div
                                    class="absolute z-10 max-w-[76ch] rounded-[8px] bg-raised px-3 py-1.5 font-mono text-footnote shadow-xl ring-1 ring-line-strong"
                                    style=format!(
                                        "left: 8px; {place}",
                                    )
                                >
                                    <div class="whitespace-pre-wrap select-text">
                                        <span class="text-label-2">{before}</span>
                                        <span class="font-semibold text-rust">{active}</span>
                                        <span class="text-label-2">{after}</span>
                                    </div>
                                    {doc
                                        .map(|text| {
                                            view! {
                                                <div class="mt-0.5 max-w-[70ch] truncate font-sans text-caption text-label-3">
                                                    {text}
                                                </div>
                                            }
                                        })}
                                </div>
                            }
                            .into_any()
                        }
                    }

                    // The completion popup, anchored under the word it is
                    // completing.
                    {
                        let path = path.clone();
                        move || {
                            let Some(popup) = state.editor.completion.get() else {
                                return ().into_any();
                            };
                            if popup.path != path {
                                return ().into_any();
                            }
                            let draft = state.editor.draft.get();
                            let shown: Vec<(usize, CompletionItem)> = visible_items(&popup, &draft);
                            if shown.is_empty() {
                                return ().into_any();
                            }
                            let chosen = picked.get().min(shown.len() - 1);
                            let x = col_left(&draft, popup.line, popup.word_start, zoom.get());
                            let place =
                                card_place(state, popup.line, zoom.get(), opens_up(popup.line));
                            // A window around the selection rather than a
                            // scrollbar: nine rows is what the eye takes in,
                            // and the arrows walk the rest into view.
                            let from = chosen.saturating_sub(4).min(shown.len().saturating_sub(9));
                            view! {
                                <div
                                    class="absolute z-20 min-w-[260px] rounded-[8px] bg-raised py-1 font-mono text-footnote shadow-2xl ring-1 ring-line-strong"
                                    style=format!("left: {x}px; {place}")
                                >
                                    {shown
                                        .into_iter()
                                        .skip(from)
                                        .take(9)
                                        .map(|(index, item)| {
                                            let selected = index == chosen;
                                            let kind = item.kind.clone().unwrap_or_default();
                                            // The type or signature, as the
                                            // server shows it beside the name.
                                            let detail = item
                                                .description
                                                .clone()
                                                .or_else(|| item.detail.clone())
                                                .unwrap_or_default();
                                            view! {
                                                <button
                                                    type="button"
                                                    on:mousedown=move |event: ev::MouseEvent| {
                                                        // Before the textarea's
                                                        // own mousedown closes
                                                        // the popup.
                                                        event.prevent_default();
                                                        event.stop_propagation();
                                                        if let Some(element) =
                                                            area.get_untracked()
                                                        {
                                                            accept_completion(
                                                                state, &element, index,
                                                            );
                                                        }
                                                    }
                                                    class=if selected {
                                                        "flex w-full items-baseline gap-2 bg-selection px-2.5 py-0.5 text-left text-rust"
                                                    } else {
                                                        "flex w-full items-baseline gap-2 px-2.5 py-0.5 text-left text-label-2"
                                                    }
                                                >
                                                    <span class="w-[7ch] shrink-0 truncate text-label-3">
                                                        {kind}
                                                    </span>
                                                    <span class="shrink-0">{item.label.clone()}</span>
                                                    <span class="shrink-0 text-label-4">
                                                        {item.label_detail.clone().unwrap_or_default()}
                                                    </span>
                                                    <span class="min-w-0 flex-1 truncate text-label-3">
                                                        {detail}
                                                    </span>
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                            .into_any()
                        }
                    }
                </div>
            </div>
        </div>
        </div>
    }
}

/// What a change to the text asks the server for, judged by what now sits
/// behind the caret: completion by [`ask_for`]'s rules — the first letter of
/// a word, `.` and `::`, the word again while its answer is incomplete, and
/// a close when the caret has left the word — and the signature card by the
/// parentheses, `(` and `,` asking and `)` dropping it.
///
/// Shared by the input event and by the keys the editor types on the
/// browser's behalf — a bracket pair, a step over a closer, a Backspace
/// inside an empty pair — which never reach the input event because they
/// were `preventDefault`ed. `typing` is false for a deletion, which widens a
/// popup that is open and never opens one.
fn typed_triggers(
    state: AppState,
    path: &str,
    is_rust: bool,
    element: &web_sys::HtmlTextAreaElement,
    typing: bool,
) {
    if !is_rust {
        return;
    }
    // The caret is a position in the screen text; the server wants one in
    // the document. Identical while nothing is folded.
    let screen_now = screen(state);
    let Some((row, col)) = caret_line_col(element, &screen_now) else {
        return;
    };
    let line = line_of_row(state, row);
    let draft = state.editor.draft.get_untracked();
    let line_text = draft.split('\n').nth(line as usize).unwrap_or_default();
    let before: Vec<char> = line_text.chars().take(col as usize).collect();
    let last = before.last().copied();

    // What the popup is about: the one showing, or the word still waiting
    // for its first answer — which counts as incomplete, since nobody has
    // said otherwise yet.
    let showing = state
        .editor
        .completion
        .with_untracked(|popup| {
            popup
                .as_ref()
                .filter(|popup| popup.path == path)
                .map(|popup| Showing {
                    line: popup.line,
                    word_start: popup.word_start,
                    incomplete: popup.incomplete,
                })
        })
        .or_else(|| {
            state.editor.completion_ask.with_value(|ask| {
                ask.anchor.as_ref().filter(|(asked, ..)| asked == path).map(
                    |(_, line, word_start)| Showing {
                        line: *line,
                        word_start: *word_start,
                        incomplete: true,
                    },
                )
            })
        });
    match ask_for(&before, line, showing, typing) {
        Ask::Keep => {}
        Ask::Close => controller::dismiss_completion(state),
        Ask::Complete { word_start, now } => {
            controller::request_completion(state, path.to_string(), line, col, word_start, now);
        }
    }

    // The signature card follows the parentheses.
    match last {
        Some('(') | Some(',') => {
            controller::request_signature(state, path.to_string(), line, col);
        }
        Some(')') => state.editor.signature.set(None),
        _ => {}
    }
}
