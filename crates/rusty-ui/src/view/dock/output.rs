//! What every tool has said, on one scrollback.
//!
//! Shared rather than per-panel: a flash fails while you are reading the
//! memory report, and both have to be visible at once. The channel each line
//! came from is kept so the filter can narrow without losing the ordering.

use rusty_embed::LogLevel;

use super::*;
use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, MenuItem, MenuSeparator, copy_to_clipboard},
    view::loclink::{self, Piece},
};

/// What flashing and monitoring printed.
///
/// Not a terminal emulator: there is no pty, so anything that wants a prompt or
/// paints with cursor movement will not behave. That is stated in the placeholder
/// rather than discovered — but the commands this workbench is *about* are cargo,
/// espflash, probe-rs and git, and none of them needs one.
///
/// One scrollback for every source, because a flash that fails during a build is
/// one story and two panes would tell it in halves.
#[component]
pub(super) fn OutputTab() -> impl IntoView {
    let state = AppState::expect();
    let draft = RwSignal::new(String::new());
    let input: NodeRef<leptos::html::Input> = NodeRef::new();
    let scroller: NodeRef<leptos::html::Div> = NodeRef::new();
    // Where the view was last time, so an upward scroll can be told from the
    // follow's own downward one.
    let last_top = RwSignal::new(0);
    // Right-click: position, plus the clicked line's text when there was one.
    let menu = RwSignal::new(None::<(f64, f64, Option<String>)>);

    // Stick to the newest line. The Follow toggle existed from the start and
    // flipped a flag nothing read, so a build scrolled past while the view sat
    // where it was — which is the one thing a log pane must not do.
    Effect::new(move |_| {
        // Read the log so this runs on every new line.
        let _ = state.dock.lines.with(Vec::len);
        if !state.dock.follow.get() {
            return;
        }
        let Some(element) = scroller.get() else {
            return;
        };
        // Deferred, because Leptos flushes to the DOM in a microtask: the rows
        // being scrolled past do not exist yet at this point and `scrollHeight`
        // would still be the old one.
        //
        // A timer rather than `request_animation_frame`, which does not fire at
        // all while the page is hidden — so output arriving behind a minimised
        // window would land unscrolled and still be sitting at the top when the
        // window came back.
        set_timeout(
            move || element.set_scroll_top(element.scroll_height()),
            std::time::Duration::ZERO,
        );
    });

    let send = move || {
        let line = draft.get_untracked().trim().to_string();
        if line.is_empty() || state.app.session_running.get_untracked() {
            return;
        }
        draft.set(String::new());
        if let Some(element) = input.get_untracked() {
            element.set_value("");
        }
        controller::run_command(state, line);
    };

    view! {
        <div
            node_ref=scroller
            on:contextmenu=move |event: leptos::ev::MouseEvent| {
                event.prevent_default();
                menu.set(Some((event.client_x() as f64, event.client_y() as f64, None)));
            }
            on:scroll=move |_| {
                let Some(element) = scroller.get_untracked() else { return };
                let top = element.scroll_top();
                let went_up = top < last_top.get_untracked();
                last_top.set(top);

                // Only an *upward* scroll stops following. Judging it by
                // distance-from-bottom instead looks right and is not: the
                // follow itself fires this handler, and by the time it runs
                // more output has arrived, so the view reads as "far from the
                // bottom" and switches itself off after the first burst. That
                // is why it stopped 352 pixels short.
                if went_up {
                    state.dock.follow.set(false);
                    return;
                }
                let distance = element.scroll_height()
                    - top
                    - element.client_height();
                // A few pixels of slack: fractional scroll positions mean an
                // exact comparison is never true on a trackpad.
                if distance <= 4 {
                    state.dock.follow.set(true);
                }
            }
            class="min-h-0 flex-1 overflow-y-auto"
        >
            {move || {
                let pick = state.dock.pick.get();
                let filter = state.dock.filter.get();
                let all = state.dock.lines.get();
                let total = all.len();
                let lines: Vec<_> = all
                    .into_iter()
                    .filter(|(source, line)| {
                        (pick == "all" || *source == pick)
                            && (filter.is_empty() || passes_filter(&line.text, &filter))
                    })
                    .map(|(_, line)| line)
                    .collect();
                let hidden = total - lines.len();
                if lines.is_empty() && total > 0 {
                    return view! {
                        <p class="px-4 py-3 text-callout text-label-3">
                            {t!("dock.output.all-hidden", total = total)}
                        </p>
                    }
                        .into_any();
                }
                if lines.is_empty() {
                    return view! {
                        <p class="px-4 py-3 text-callout text-label-2">
                            {t!("dock.output.empty")}
                        </p>
                    }
                        .into_any();
                }
                view! {
                    <div class="px-3 py-2 font-mono text-footnote leading-[1.6] select-text">
                        {(hidden > 0)
                            .then(|| {
                                view! {
                                    <div class="text-caption text-label-4">
                                        {t!("dock.output.hidden", count = hidden)}
                                    </div>
                                }
                            })}
                        {lines
                            .into_iter()
                            .map(|line| {
                                // Level first; failing that, read the line.
                                // The stream is NOT a severity: cargo prints
                                // its ordinary progress on stderr, and painting
                                // all of it amber made a clean build look like
                                // a wall of warnings.
                                let colour = match line.level {
                                    Some(LogLevel::Error) => "text-crimson",
                                    Some(LogLevel::Warn) => "text-amber",
                                    Some(LogLevel::Info) => "text-label",
                                    Some(LogLevel::Debug) | Some(LogLevel::Trace) => "text-label-3",
                                    None => {
                                        let text = line.text.trim_start();
                                        if text.starts_with("error:")
                                            || text.starts_with("error[")
                                        {
                                            "text-crimson"
                                        } else if text.starts_with("warning:") {
                                            "text-amber"
                                        } else {
                                            "text-label-2"
                                        }
                                    }
                                };
                                let for_menu = line.text.clone();
                                view! {
                                    <div
                                        class=format!("whitespace-pre-wrap {colour}")
                                        on:contextmenu=move |event: leptos::ev::MouseEvent| {
                                            event.prevent_default();
                                            event.stop_propagation();
                                            menu.set(
                                                Some((
                                                    event.client_x() as f64,
                                                    event.client_y() as f64,
                                                    Some(for_menu.clone()),
                                                )),
                                            );
                                        }
                                    >
                                        {loclink::split_locations(&line.text)
                                            .into_iter()
                                            .map(|piece| match piece {
                                                Piece::Text(text) => text.into_any(),
                                                Piece::Loc { display, path, line, col } => {
                                                    view! {
                                                        <button
                                                            type="button"
                                                            title=t!("dock.output.open-in-editor")
                                                            class="cursor-pointer underline decoration-dotted underline-offset-2 hover:text-rust"
                                                            on:click=move |_| {
                                                                controller::open_at(
                                                                    state,
                                                                    path.clone(),
                                                                    line.saturating_sub(1),
                                                                    col.saturating_sub(1),
                                                                );
                                                            }
                                                        >
                                                            {display}
                                                        </button>
                                                    }
                                                        .into_any()
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                }
                            })
                            .collect_view()}
                    </div>
                }
                    .into_any()
            }}
        </div>

        <div class="flex flex-none items-center gap-2 border-t border-line px-3 py-1.5">
            <span class="shrink-0 font-mono text-footnote text-label-3">"$"</span>
            <input
                node_ref=input
                class="min-w-0 flex-1 bg-transparent font-mono text-footnote outline-none placeholder:text-label-3"
                placeholder=move || {
                    if state.has_project() {
                        t!("dock.chrome.command-placeholder")
                    } else {
                        t!("dock.chrome.command-needs-project")
                    }
                }
                disabled=move || !state.has_project()
                on:input=move |event| draft.set(event_target_value(&event))
                on:keydown=move |event: leptos::ev::KeyboardEvent| {
                    if event.key() == "Enter" {
                        event.prevent_default();
                        send();
                    }
                }
            />
            {move || {
                state
                    .app.session_running
                    .get()
                    .then(|| {
                        view! {
                            <button
                                type="button"
                                class="shrink-0 rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:text-label"
                                on:click=move |_| controller::stop_session(state)
                            >
                                {t!("dock.chrome.stop")}
                            </button>
                        }
                    })
            }}
        </div>

        {move || {
            let (x, y, line) = menu.get()?;
            let close = Callback::new(move |_| menu.set(None));
            let follow_label = if state.dock.follow.get_untracked() {
                t!("context.output-stop-following")
            } else {
                t!("context.output-follow")
            };
            Some(
                view! {
                    <ContextMenu x=x y=y on_close=close>
                        {line
                            .map(|text| {
                                view! {
                                    <MenuItem
                                        label=t!("context.output-copy-line")
                                        on_select=Callback::new(move |_| {
                                            copy_to_clipboard(&text);
                                            menu.set(None);
                                        })
                                    />
                                }
                            })}
                        <MenuItem
                            label=t!("context.output-copy-all")
                            on_select=Callback::new(move |_| {
                                let all = state
                                    .dock.lines
                                    .with_untracked(|lines| {
                                        lines
                                            .iter()
                                            .map(|(_, l)| l.text.as_str())
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    });
                                copy_to_clipboard(&all);
                                menu.set(None);
                            })
                        />
                        <MenuSeparator />
                        <MenuItem
                            label=follow_label
                            on_select=Callback::new(move |_| {
                                state.dock.follow.update(|f| *f = !*f);
                                menu.set(None);
                            })
                        />
                        <MenuItem
                            label=t!("context.output-clear")
                            on_select=Callback::new(move |_| {
                                state.clear_log();
                                menu.set(None);
                            })
                        />
                    </ContextMenu>
                },
            )
        }}
    }
}
