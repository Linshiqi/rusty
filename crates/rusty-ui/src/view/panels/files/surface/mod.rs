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

mod completions;
mod cursor;
mod echo;
mod fixes;
mod gutter;
mod hover;
mod keys;
mod lenses;
mod menu;
mod mouse;
mod pane;
mod signature;
mod washes;

use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, MenuItem, MenuSeparator},
    view::icon::{Icon, IconView},
};
use completions::Completions;
use cursor::VimCursor;
use echo::Echo;
use fixes::QuickFixes;
use gutter::Gutter;
use hover::Hover;
use lenses::Lenses;
use menu::EditorMenu;
use pane::Pane;
use signature::SignatureCard;
use washes::BracketMarks;
use washes::FindWash;
use washes::OccurrenceWash;
use washes::StopLine;

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
    /// How many indent guides it draws (`guides.rs`).
    guides: u8,
    /// The inlay hints drawn in it (`hints.rs`).
    hints: Vec<Placed>,
    /// The name drawn as a link, while Ctrl is held over it.
    link: Option<(u32, u32)>,
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
    // A drag the editor is following (`pointer.rs`), and how far an input
    // method's composition shifts the textarea (`caret_shift`).
    let drag = StoredValue::new(None::<Drag>);
    follow_drags(state, area, scroller, drag);
    follow_caret(state, area, scroller, selection_moves);
    let ime_shift = RwSignal::new(0.0_f64);

    // The name under the pointer drawn as a link while Ctrl is held — only
    // one with a definition to go to, as VS Code's is: (line, from, to).
    let link = RwSignal::new(None::<(u32, u32, u32)>);
    let link_turn = StoredValue::new(0u64);
    let zoom = state.editor.zoom;

    // The scroller's viewport: how far down it is scrolled and how tall it
    // is. The two layers draw the rows in it and a margin (`window.rs`),
    // never the file. The height follows the dividers and the window; the
    // observer's callback is forgotten rather than kept, because one firing
    // after the view has gone would call a dropped closure, and this one reads
    // only through `try_` and finds nothing.
    let view_top = RwSignal::new(0.0_f64);
    // And how far across, for what is drawn over the text and not in it.
    let view_left = RwSignal::new(0.0_f64);
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
    // The widest line as drawn, hints and all, which the text column is at
    // least as wide as.
    let widest = {
        let path = path.clone();
        Memo::new(move |_| {
            let mut hinted = std::collections::HashMap::new();
            if state.editor.view.with(|view| view.inlay_hints) {
                state.editor.hints.with(|set| {
                    for hint in set
                        .iter()
                        .filter(|set| set.path == path)
                        .flat_map(|set| &set.hints)
                    {
                        hinted
                            .entry(hint.line)
                            .or_insert_with(|| placed_on(set.as_ref(), &path, hint.line));
                    }
                });
            }
            state.editor.draft.with(|text| {
                text.split('\n')
                    .enumerate()
                    .map(|(index, line)| match hinted.get(&(index as u32)) {
                        Some(hints) => HintedLine { text: line, hints }.width_px(&advance_of),
                        None => line_px(line),
                    })
                    .fold(0.0, f64::max)
            })
        })
    };
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
            let colours = controller::semantic_covers(state, drawn.0, drawn.1);
            let hints = controller::hints_cover(state, drawn.0, drawn.1);
            if !is_rust || (colours && hints) {
                return;
            }
            let turn = semantic_wait.get_value() + 1;
            semantic_wait.set_value(turn);
            let path = path.clone();
            set_timeout(
                move || {
                    if semantic_wait.try_get_value() == Some(turn) {
                        if !colours {
                            controller::request_semantic(state, path.clone());
                        }
                        if !hints {
                            controller::request_hints(state, path);
                        }
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

    // How many indent guides each line draws, when they are on (`guides.rs`):
    // one pass over the text per edit, like the fold scan beside it.
    let levels = Memo::new(move |_| {
        if !state.editor.view.with(|view| view.indent_guides) {
            return Vec::new();
        }
        state.editor.draft.with(|text| indent_levels(text))
    });
    // The guide of the block the caret is in, drawn brighter: each guide
    // reads it for itself, so a caret moving redraws no row.
    let lit_guide = Memo::new(move |_| {
        let (line, _) = caret_at.get()?;
        levels.with(|levels| active_guide(levels, line as usize))
    });

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

    // The margin's style, which sticky scroll's numbers take too so they stand
    // where the margin's do.
    let gutter_style = Signal::derive(move || {
        // Tailwind's border-box made a bare `width: 5ch` mean "5ch including
        // 20px of padding", which left 4-digit numbers 14px of room — they
        // clipped against the code column. The width now names the digits
        // and adds the padding explicitly.
        let digits = line_count.get().to_string().len().max(3);
        // Padding, the breakpoint dot and its gap, plus a column for the fold
        // chevron when the file has anything to fold. Reserving it
        // unconditionally would push the code right by two characters in
        // every flat file; reserving *nothing* was the bug that once made the
        // run arrows invisible — the row is `justify-end`, so anything that
        // does not fit overflows off the left edge rather than wrapping or
        // scrolling. (The arrows have since moved beside the item, as a lens.)
        let columns = usize::from(foldables.with(|found| !found.is_empty()));
        let extra = 32 + columns * 17;
        // The dot's column, then the digits, then the padding — a width that
        // only counted digits clipped the number the moment a dot appeared.
        format!("{}; width: calc({digits}ch + {extra}px)", metrics.get())
    });

    let pane = Pane {
        state,
        area,
        scroller,
        path: StoredValue::new(path.clone()),
        read_only,
        is_rust,
        unlinked,
        hover_cell,
        hover_gen,
        on_card,
        editor_menu,
        selection_moves,
        drag,
        link,
        link_turn,
        zoom,
        rows_total,
        window,
        widest,
        brackets,
        runnables,
        foldables,
        levels,
        lit_guide,
        picked,
        picked_action,
        metrics,
        gutter_style,
    };
    let on_input = move |event: ev::Event| pane.on_input(event);

    // Ctrl pressed or let go with the pointer still: the link follows the
    // key, not only the pointer.
    {
        let pressed = window_event_listener(ev::keydown, move |event| {
            if matches!(event.key().as_str(), "Control" | "Meta") && !event.repeat() {
                pane.probe_link(hover_cell.get_untracked());
            }
        });
        let released = window_event_listener(ev::keyup, move |event| {
            if matches!(event.key().as_str(), "Control" | "Meta") {
                pane.unlink();
            }
        });
        let blurred = window_event_listener(ev::blur, move |_| pane.unlink());
        let listeners = StoredValue::new_local(Some((pressed, released, blurred)));
        on_cleanup(move || {
            if let Some(Some((pressed, released, blurred))) =
                listeners.try_update_value(Option::take)
            {
                pressed.remove();
                released.remove();
                blurred.remove();
            }
        });
    }

    view! {
        <div class="relative flex min-h-0 flex-1 flex-col">
            <FindBar area=area scroller=scroller />
            <RenameBar />

            // The editor's own menu. It keeps the clipboard three every text
            // box has, and adds what only this editor knows: where a name is
            // defined, what rust-analyzer would fix, how the file formats.
            <EditorMenu pane=pane />
        // The scroller, and the minimap down its right-hand edge (`minimap.rs`).
        <div class="flex min-h-0 flex-1">
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
                    view_left.set(f64::from(element.scroll_left()));
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
            // The lines of the blocks the top of the view is in (`sticky.rs`).
            {sticky_lines(
                state,
                path.clone(),
                scroller,
                view_top,
                view_left,
                foldables,
                metrics,
                gutter_style,
            )}
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
                <Gutter pane=pane />

                <div class="relative min-w-0 flex-1">
                    // Find matches, washed under the text. Rectangles rather
                    // than woven spans: the wash must not disturb the span
                    // structure the caret math and diagnostics rely on. Only
                    // the matches in the window, their lines found in one walk
                    // down the text.
                    <FindWash pane=pane />
                    // The other places the name at the caret occurs, washed
                    // under the text as a find match is, in a colour of their
                    // own.
                    <OccurrenceWash pane=pane />
                    // The bracket beside the caret and the one it pairs with,
                    // outlined — so where a block ends is a glance, not a count.
                    <BracketMarks pane=pane />
                    // Every selection, washed behind the text (`selection.rs`).
                    {selections(state, path.clone(), area, window, selection_moves)}
                    <Echo pane=pane />

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
                        // Its own caret and selection hidden: it lays the
                        // text out without the hints the echo draws inside
                        // a line, so both are drawn where the text is
                        // (`selection.rs`).
                        class="editor-input absolute inset-0 m-0 resize-none overflow-hidden \
                               border-0 bg-transparent py-2 pr-4 pl-2 whitespace-pre \
                               text-transparent caret-transparent outline-none"
                        // Shifted while an input method composes, by what the
                        // hints before the caret push the drawn one along: the
                        // input method places its window at the textarea's own
                        // caret, which does not know about them.
                        style=move || {
                            let shift = ime_shift.get();
                            let mut style = metrics.get();
                            if shift != 0.0 {
                                style.push_str(&format!("; transform: translateX({shift}px)"));
                            }
                            // The hand over a link, as over any other.
                            if link.get().is_some() {
                                style.push_str("; cursor: pointer");
                            }
                            style
                        }
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
                                && (multi_paste(state, &element, &pasted, scroller)
                                    || paste_into(state, &element, &pasted, read_only))
                            {
                                event.prevent_default();
                            }
                        }
                        on:compositionstart=move |_| {
                            multi_compose(state);
                            if let Some(element) = area.get_untracked() {
                                ime_shift.set(caret_shift(state, &element));
                            }
                        }
                        on:compositionend=move |_| ime_shift.set(0.0)
                        on:mousedown=move |event: ev::MouseEvent| pane.on_mousedown(event)
                        on:mousemove=move |event: ev::MouseEvent| pane.on_mousemove(event)
                        on:mouseleave=move |_| pane.on_mouseleave()
                        on:keydown=move |event: ev::KeyboardEvent| pane.on_keydown(event)
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
                    <VimCursor pane=pane />

                    // A caret at every cursor (`selection.rs`). After the
                    // textarea, so they show while it has focus.
                    {carets(state, path.clone(), area, window, selection_moves)}

                    // The tests, offered where VS Code offers them — beside
                    // the item, not at the far edge of the margin — and with
                    // the half the margin never had room for: Debug. An
                    // overlay, not a row of its own: the textarea and the echo
                    // have to stay glyph for glyph, and a row that exists in
                    // one and not the other is a caret that drifts. So the
                    // lens sits on the attribute line above the item (the row
                    // VS Code draws its lens on), after that line's text, or
                    // on the item's own line when nothing is above it.
                    <Lenses pane=pane />

                    // Where the target is stopped. Drawn under the text like
                    // a find match rather than as a border, so it survives
                    // the caret and the selection sitting on the same line.
                    <StopLine pane=pane />

                    // What the server said about the token the mouse settled
                    // on. Interactive: long documentation scrolls inside it,
                    // and reading is not leaving.
                    <Hover pane=pane />

                    // The quick-fix popup, anchored under its line.
                    <QuickFixes pane=pane />

                    // The signature card, floated above the line whose call
                    // it describes, with the active parameter lit.
                    <SignatureCard pane=pane />

                    // The completion popup, anchored under the word it is
                    // completing.
                    <Completions pane=pane />
                </div>
            </div>
        </div>
        {move || {
            state
                .editor
                .view
                .with(|view| view.minimap)
                .then(|| {
                    view! {
                        <Minimap
                            scroller=scroller
                            view_top=view_top
                            view_height=view_height
                            rows_total=rows_total
                        />
                    }
                })
        }}
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
