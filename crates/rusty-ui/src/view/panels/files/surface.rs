//! The editing surface itself: the `<pre>` and the `<textarea>` over it.
//!
//! Every keystroke the editor answers to arrives here and is routed on — to
//! the modal state machine, to completion, to find, to the language server.
//! The two layers must agree on font, size and line height exactly, or the
//! caret drifts from its glyph a column at a time across the line.

use leptos::{ev, html, prelude::*};

use rusty_edit::Document;
use rusty_lsp::CompletionItem;

use rusty_i18n::t;

use super::*;
use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, MenuItem, MenuSeparator},
    view::icon::{Icon, IconView},
};

/// The two stacked layers: highlighted text underneath, a transparent text
/// area on top taking every keystroke.
///
/// The painted layer follows `state.editor.highlighted`, not the document: on each
/// keystroke the edited lines are patched in plainly so the text under the
/// caret is never stale, and a debounced re-highlight restores the colours.
/// Without the immediate patch, typed characters are invisible for a quarter
/// of a second — the textarea's own glyphs are transparent by design.
#[component]
pub(super) fn Surface(document: Document) -> impl IntoView {
    let state = AppState::expect();
    let area: NodeRef<html::Textarea> = NodeRef::new();
    let scroller: NodeRef<html::Div> = NodeRef::new();
    let path = document.path.clone();
    let read_only = document.truncated || document.read_only;
    // Hover only means something where a language server is listening.
    let is_rust = path.ends_with(".rs");

    // The cell the mouse was last over, and a generation so only the newest
    // 400ms-old position asks the server. Hover is ambient: it must cost
    // nothing while the mouse is moving and only speak once it has settled.
    let hover_cell = RwSignal::new(None::<(u32, u32)>);
    let hover_gen = RwSignal::new(0u64);
    // True while the pointer is over the card itself. Reading the card —
    // scrolling it, selecting from it — must not count as leaving.
    let on_card = RwSignal::new(false);
    let editor_menu = RwSignal::new(None::<(f64, f64)>);

    let zoom = state.editor.zoom;

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

    // The coding toolbar: what a person editing firmware reaches for. Save
    // rides the same format-then-save path as Ctrl+S; Build shares the one
    // session slot; the last two are the places this work goes next.
    let toolbar = Callback::new(move |_| {
        let running = state.app.session_running;
        view! {
            <button
                type="button"
                title=t!("toolbar.save")
                disabled=read_only
                on:click=move |_| format_and_save(state, area)
                class="grid size-8 place-items-center rounded-[6px] text-rust hover:bg-sunken disabled:pointer-events-none disabled:opacity-40"
            >
                <IconView icon=Icon::Save size=15 />
            </button>
            <button
                type="button"
                title=t!("toolbar.build")
                disabled=move || running.get()
                on:click=move |_| controller::build_project(state)
                class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-40"
            >
                <IconView icon=Icon::Hammer size=15 />
            </button>
            <span class="my-1 h-px w-5 bg-line" />
            <button
                type="button"
                title=t!("toolbar.flash")
                on:click=move |_| state.show_dock(crate::state::DockTab::Devices)
                class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label"
            >
                <IconView icon=Icon::Flash size=15 />
            </button>
            // While a session is live the toolbar is the debugger's: the
            // transport controls belong where the eye already is, not in a
            // panel the stopped line just navigated away from.
            <crate::view::transport::DebugTransport />
            // Debug sits beside Run, because that is the pair: run it, or
            // run it and stop where you said. Hidden while a session is
            // live — the transport controls above are what it becomes.
            {move || {
                state.debug.session.with(Option::is_none).then(|| {
                    view! {
                        <button
                            type="button"
                            title=t!("toolbar.debug")
                            disabled=move || running.get()
                            on:click=move |_| {
                                state.layout.panel.set("simulate".to_string());
                                controller::run_simulation(state, true);
                            }
                            class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-40"
                        >
                            <IconView icon=Icon::Bug size=15 />
                        </button>
                    }
                })
            }}
            // A play icon runs — switching panels without running is the
            // mismatch that got this button reported. It also switches, so
            // the board is on screen while the build streams to the dock.
            <button
                type="button"
                title=t!("toolbar.run")
                disabled=move || running.get()
                on:click=move |_| {
                    state.layout.panel.set("simulate".to_string());
                    controller::run_simulation(state, false);
                }
                class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-40"
            >
                <IconView icon=Icon::Play size=15 />
            </button>
        }
        .into_any()
    });
    register_toolbar(state, toolbar);
    // Which completion row the keyboard is on. Reset when a new popup arrives.
    let picked = RwSignal::new(0usize);
    Effect::new(move |_| {
        let _ = state.editor.completion.get();
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

    // Both layers carry this verbatim. Any difference in font, size or line
    // height and the caret walks away from its glyph. Ctrl+wheel scales the
    // whole thing; every pixel computed below multiplies by the same factor.
    let metrics = Signal::derive(move || {
        let z = zoom.get();
        format!(
            "font-size: {}px; line-height: {}px; \
             font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; \
             tab-size: 4",
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

            // Completion triggers, judged by the character behind the caret.
            if !is_rust {
                return;
            }
            let Some(element) = area.get_untracked() else {
                return;
            };
            // The caret is a position in the screen text; the server wants
            // one in the document. Identical while nothing is folded.
            let Some((row, col)) = caret_line_col(&element, &screen_now) else {
                return;
            };
            let line = line_of_row(state, row);
            let line_text = new.split('\n').nth(line as usize).unwrap_or_default();
            let before: Vec<char> = line_text.chars().take(col as usize).collect();
            let last = before.last().copied();

            let popup_open = state.editor.completion.with_untracked(Option::is_some);
            match last {
                // `foo.` and `foo::` are the moments completion answers a
                // question the typist actually has.
                Some('.') => {
                    controller::request_completion(state, path.clone(), line, col, col);
                }
                Some(':') if before.len() >= 2 && before[before.len() - 2] == ':' => {
                    controller::request_completion(state, path.clone(), line, col, col);
                }
                // Inside a word. Once the popup is open the filter narrows it
                // reactively off the draft, so there is nothing to do — but
                // *opening* it was the gap: only `.` and `::` ever did, so
                // typing an identifier offered nothing at all, which reads as
                // an editor with no completion rather than one with a
                // deliberate trigger.
                //
                // On the second character, not the first: rust-analyzer
                // answers a one-letter prefix with the entire visible scope,
                // which is a thousand rows to draw and filter for a question
                // nobody has asked yet. One request per word, not per key —
                // after this the popup is open and this arm does nothing.
                Some(c) if c.is_alphanumeric() || c == '_' => {
                    let word = before
                        .iter()
                        .rev()
                        .take_while(|c| c.is_alphanumeric() || **c == '_')
                        .count();
                    if !popup_open && word == 2 {
                        let start = col - word as u32;
                        controller::request_completion(state, path.clone(), line, col, start);
                    }
                }
                // Anything else ends the word the popup was about.
                _ => {
                    if popup_open {
                        state.editor.completion.set(None);
                    }
                }
            }

            // The signature card follows the parentheses.
            match last {
                Some('(') | Some(',') => {
                    controller::request_signature(state, path.clone(), line, col);
                }
                Some(')') => state.editor.signature.set(None),
                _ => {}
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
                                    disabled=!has_selection || read_only
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked() {
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
                                    disabled=!has_selection
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked() {
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
            class="relative min-h-0 flex-1 overflow-auto"
            // Ctrl+wheel scales the editor font, as every editor since
            // forever. The browser's own page zoom is exactly what this
            // prevent_default suppresses.
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
                // long file's numbers stay beside their lines.
                {
                    let path_for_gutter = path.clone();
                    move || {
                        let count = state.editor.highlighted.with(Vec::len).max(1);
                        // Tailwind's border-box made a bare `width: 5ch` mean
                        // "5ch including 20px of padding", which left 4-digit
                        // numbers 14px of room — they clipped against the code
                        // column. The width now names the digits and adds the
                        // padding explicitly.
                        let digits = count.to_string().len().max(3);
                        // Padding, the breakpoint dot and its gap, plus a
                        // column for each margin affordance the file actually
                        // has. Reserving them unconditionally would push the
                        // code right by two characters in every file that has
                        // neither; reserving *neither* was the bug that made
                        // the run arrows invisible — the row is `justify-end`,
                        // so anything that does not fit overflows off the left
                        // edge rather than wrapping or scrolling.
                        let columns = usize::from(!runnables.get().is_empty())
                            + usize::from(!foldables.get().is_empty());
                        let extra = 32 + columns * 17;
                        // The icons scale with the row. A fixed 13px chevron
                        // is taller than the row itself once the editor is
                        // zoomed out far enough, and a row that out-grows its
                        // line height pushes every number below it down — the
                        // gutter walks away from the code a row at a time.
                        let icon_px = (row_height(zoom.get()) * 0.68).round().max(7.0) as u32;
                        // Each decoration gets a slot of its own on *every*
                        // row, occupied or not. The row is `justify-end`, so a
                        // line with no chevron lets its number slide right
                        // into the chevron's place — and one number out of
                        // step with its neighbours reads as the gutter having
                        // lost track of the file.
                        let slot = format!("width: {icon_px}px");
                        let runs_column = !runnables.get().is_empty();
                        let folds_column = !foldables.get().is_empty();
                        view! {
                            <div
                                class="flex-none py-2 pr-2 pl-3 text-right text-label-4 select-none"
                                style=format!(
                                    // The dot's column, then the digits, then the
                                    // padding — a width that only counted digits
                                    // clipped the number the moment a dot appeared.
                                    "{}; width: calc({digits}ch + {extra}px)",
                                    metrics.get(),
                                )
                            >
                                // Each number is a breakpoint target, as in
                                // every debugger since the first one with a
                                // mouse: click the margin, get a breakpoint.
                                // Only the lines on screen, each keeping its
                                // real number: a folded file whose numbers
                                // renumbered themselves would make every
                                // compiler error point at the wrong place.
                                {state
                                    .editor.folds
                                    .with(|f| f.visible(count as u32))
                                    .into_iter()
                                    .map(|line| {
                                        let n = line + 1;
                                        let file = path_for_gutter.clone();
                                        let toggle = file.clone();
                                        let marked = Signal::derive(move || {
                                            state.debug.breakpoints.with(|list| {
                                                list.iter().any(|(f, l)| f == &file && *l == line)
                                            })
                                        });
                                        // The test declared on this line, if
                                        // any, as its own click target rather
                                        // than a second meaning for the
                                        // margin. The margin already means
                                        // "breakpoint", and one glyph that did
                                        // two things depending on where you
                                        // hit it is how you set a breakpoint
                                        // when you meant to run a test.
                                        let run = runnables
                                            .get()
                                            .into_iter()
                                            .find(|r| r.line == line)
                                            .map(|r| {
                                                let label = match r.kind {
                                                    rusty_edit::RunnableKind::Module => {
                                                        t!("files.run-module", name = r.name)
                                                    }
                                                    rusty_edit::RunnableKind::Test => {
                                                        t!("files.run-test", name = r.name)
                                                    }
                                                };
                                                let filter = r.filter.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        title=label
                                                        on:click=move |event: ev::MouseEvent| {
                                                            event.stop_propagation();
                                                            controller::run_test(
                                                                state,
                                                                filter.clone(),
                                                            );
                                                        }
                                                        class="flex shrink-0 items-center text-accent/60 hover:text-accent"
                                                    >
                                                        <IconView icon=Icon::Play size=icon_px />
                                                    </button>
                                                }
                                            });
                                        // Fold control. Shown only where
                                        // something can collapse, and only on
                                        // hover unless it is already folded —
                                        // a chevron on every second line is a
                                        // margin nobody can read past.
                                        let collapsed = state
                                            .editor.folds
                                            .with(|f| f.is_folded(line));
                                        let chevron = foldables
                                            .get()
                                            .iter()
                                            .any(|r| r.header == line)
                                            .then(|| {
                                                // VSCode's shape: a stroked
                                                // chevron, down when the
                                                // region is open and turned a
                                                // quarter right when it is
                                                // collapsed. A filled triangle
                                                // reads as a disclosure widget
                                                // from a different decade and,
                                                // worse, as the run arrow's
                                                // sibling rather than as a
                                                // different kind of control.
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
                                        view! {
                                            // The dot sits *left of* the number, as every
                                            // editor with a breakpoint margin puts it:
                                            // replacing the number meant setting a
                                            // breakpoint cost you the line you were on.
                                            <div
                                                on:click=move |_| {
                                                    controller::debug_breakpoint(
                                                        state,
                                                        toggle.clone(),
                                                        line,
                                                    )
                                                }
                                                title=t!("files.breakpoint")
                                                class="group flex cursor-pointer items-center justify-end gap-1.5"
                                            >
                                                {runs_column
                                                    .then(|| {
                                                        view! {
                                                            <span
                                                                class="flex shrink-0 items-center justify-center"
                                                                style=slot.clone()
                                                            >
                                                                {run}
                                                            </span>
                                                        }
                                                    })}
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
                                                // Right of the number, hard
                                                // against the code, which is
                                                // where VSCode puts it — the
                                                // chevron belongs to the line
                                                // it opens, and on the far
                                                // side of the margin it reads
                                                // as another breakpoint
                                                // control.
                                                {folds_column
                                                    .then(|| {
                                                        view! {
                                                            <span
                                                                class="flex shrink-0 items-center justify-center"
                                                                style=slot.clone()
                                                            >
                                                                {chevron}
                                                            </span>
                                                        }
                                                    })}
                                            </div>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    }
                }

                <div class="relative min-w-0 flex-1">
                    // Find matches, washed under the text. Rectangles rather
                    // than woven spans: the wash must not disturb the span
                    // structure the caret math and diagnostics rely on.
                    {move || {
                        if !state.find.open.get() {
                            return ().into_any();
                        }
                        let text = state.editor.draft.get();
                        let query = state.find.query.get();
                        let case = state.find.case.get();
                        let matches = find_matches(&text, &query, case);
                        if matches.is_empty() {
                            return ().into_any();
                        }
                        let current = state.find.index.get().min(matches.len() - 1);
                        let z = zoom.get();
                        matches
                            .iter()
                            .take(500)
                            .enumerate()
                            .map(|(index, (from, to))| {
                                let (line, col) = line_col_of_byte(&text, *from);
                                let (_, end_col) = line_col_of_byte(&text, *to);
                                let x = col_left(&text, line, col, z);
                                let width = ((column_px(&text, line, end_col)
                                    - column_px(&text, line, col)) * z)
                                    .max(2.0);
                                let y = row_top(state, line, z);
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
                            .into_any()
                    }}
                    <pre
                        class="pointer-events-none m-0 overflow-visible py-2 pr-4 pl-2 whitespace-pre"
                        style=move || metrics.get()
                        aria-hidden="true"
                    >
                        {
                            let path = path.clone();
                            move || {
                                let diags = state
                                    .lsp.diagnostics
                                    .with(|by_file| by_file.get(&path).cloned())
                                    .unwrap_or_default();
                                // The compiler's colours, when they have
                                // arrived for this document.
                                let semantic = state
                                    .editor.semantic
                                    .with(|s| {
                                        s.as_ref()
                                            .filter(|(for_path, _)| for_path == &path)
                                            .map(|(_, spans)| spans.clone())
                                    })
                                    .unwrap_or_default();
                                let folds = state.editor.folds.get();
                                state
                                    .editor.highlighted
                                    .get()
                                    .into_iter()
                                    .enumerate()
                                    // Hidden lines are not drawn, and the
                                    // echo must drop exactly the lines the
                                    // textarea dropped: one row of
                                    // disagreement and every caret below it
                                    // sits on the wrong glyph.
                                    .filter(|(index, _)| !folds.hides(*index as u32))
                                    .map(|(index, line)| {
                                        let line = overlay_semantic(
                                            line,
                                            index as u32,
                                            &semantic,
                                        );
                                        // A collapsed header says how much is
                                        // underneath it. A bare `…` gives no
                                        // sense of whether unfolding costs
                                        // three lines or three hundred.
                                        let summary = folds
                                            .regions()
                                            .iter()
                                            .find(|r| r.header == index as u32)
                                            .map(|r| {
                                                let n = r.hidden();
                                                let unit = if n == 1 { "line" } else { "lines" };
                                                view! {
                                                    <span class="rounded-[3px] bg-selection px-1 text-label-3">
                                                        {format!(" ⋯ {n} {unit} ")}
                                                    </span>
                                                }
                                            });
                                        view! {
                                            <div>
                                                {decorate(line, index as u32, &diags)}
                                                {summary}
                                                // An empty line still occupies
                                                // one, or the caret above sits a
                                                // row too high for the rest of
                                                // the file.
                                                {"\u{200b}"}
                                            </div>
                                        }
                                    })
                                    .collect_view()
                            }
                        }
                    </pre>

                    <textarea
                        node_ref=area
                        id="editor-area"
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
                        prop:readonly=move || {
                            state.editor.vim_on.get()
                                && state.editor.vim.with(|vim| vim.mode != crate::vim::Mode::Insert)
                        }
                        class=move || {
                            let base = "absolute inset-0 m-0 resize-none overflow-hidden \
                                        border-0 bg-transparent py-2 pr-4 pl-2 whitespace-pre \
                                        text-transparent caret-rust outline-none";
                            // Normal mode only. Visual mode keeps the ordinary
                            // selection tint, because there the selection is a
                            // range the user chose rather than the cursor.
                            let block = state.editor.vim_on.get()
                                && state.editor.vim.with(|vim| vim.mode != crate::vim::Mode::Insert);
                            if block { format!("{base} vim-block") } else { base.to_string() }
                        }
                        style=move || metrics.get()
                        // What the textarea holds is the *screen* text, which
                        // is the draft minus every folded region. Identical to
                        // the draft while nothing is collapsed, so this is a
                        // no-op for a file nobody has folded.
                        prop:value=move || screen_tracked(state)
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
                        on:mousedown={
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                // Ctrl+Click asks where this is defined — the
                                // gesture every editor has taught.
                                if !(event.ctrl_key() || event.meta_key()) || !is_rust {
                                    state.editor.completion.set(None);
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
                                    h.as_ref().is_some_and(|(_, range, _)| {
                                        cell.is_some_and(|(l, c)| within(range, l, c))
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
                                            if hover_gen.get_untracked() == generation
                                                && !on_card.get_untracked()
                                            {
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
                                        if hover_gen.get_untracked() == generation
                                            && hover_cell.get_untracked() == Some((line, col))
                                        {
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
                                    if hover_gen.get_untracked() == generation
                                        && !on_card.get_untracked()
                                    {
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
                            if state.editor.vim_on.get_untracked()
                                && let Some(element) = area.get_untracked()
                                && vim_key(state, &element, scroller, &event)
                            {
                                event.prevent_default();
                                event.stop_propagation();
                                return;
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
                            // The popup owns its keys while it is up.
                            if state.editor.completion.with_untracked(Option::is_some) {
                                match event.key().as_str() {
                                    "ArrowDown" => {
                                        event.prevent_default();
                                        picked.update(|i| *i += 1);
                                        return;
                                    }
                                    "ArrowUp" => {
                                        event.prevent_default();
                                        picked.update(|i| *i = i.saturating_sub(1));
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
                                    "Escape" => {
                                        event.prevent_default();
                                        // Swallowed here so the window's own
                                        // Escape handling does not also close
                                        // an overlay behind the editor.
                                        event.stop_propagation();
                                        state.editor.completion.set(None);
                                        return;
                                    }
                                    _ => {}
                                }
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
                            if event.key() == "Enter" {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    let caret = caret_byte(&element, state);
                                    let insert =
                                        newline_indent(&state.editor.draft.get_untracked(), caret);
                                    insert_at_caret(&element, state, &insert);
                                }
                                return;
                            }
                            // A text area would move focus on Tab. In an editor
                            // that is never what was meant.
                            if event.key() == "Tab" {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    insert_at_caret(&element, state, "    ");
                                }
                            }
                        }}
                    />

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
                            let Some((for_path, range, text)) = state.editor.hover.get() else {
                                return ().into_any();
                            };
                            if for_path != path {
                                return ().into_any();
                            }
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
                                </div>
                            }
                            .into_any()
                        }
                    }

                    // The quick-fix popup, anchored under its line.
                    {
                        let path = path.clone();
                        move || {
                            let Some((for_path, line, fixes)) = state.editor.actions.get() else {
                                return ().into_any();
                            };
                            if for_path != path {
                                return ().into_any();
                            }
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
                                            let kind = fix.kind.clone().unwrap_or_default();
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
                            let word = typed_word(&draft, popup.line, popup.word_start);
                            let shown: Vec<(usize, CompletionItem)> = popup
                                .items
                                .iter()
                                .filter(|item| {
                                    word.is_empty()
                                        || item
                                            .label
                                            .to_lowercase()
                                            .starts_with(&word.to_lowercase())
                                })
                                .take(50)
                                .cloned()
                                .enumerate()
                                .collect();
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
                                            let detail = item.detail.clone().unwrap_or_default();
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
