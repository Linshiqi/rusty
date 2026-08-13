//! The bottom dock.
//!
//! Every serious development tool has one, for a reason that only shows up in
//! use: a flash fails while you are reading the memory report, and you need to
//! see both. Panels that each own their output force you to leave the thing you
//! were doing to find out what went wrong.
//!
//! So output is shared state, not panel state, and this is the one place that
//! renders it.

use leptos::prelude::*;

use rusty_embed::{LogLevel, LogStream};
use rusty_lsp::DiagSeverity;

use crate::{
    controller,
    state::{AppState, Divider, DockTab},
    view::components::{ContextMenu, Dot, MenuItem, MenuSeparator, ProblemRow, Tone, copy_to_clipboard},
    view::icon::{Icon, IconView},
    view::loclink::{self, Piece},
};

#[component]
pub fn Dock() -> impl IntoView {
    let state = AppState::expect();

    view! {
        // The handle sits above the tab strip so the whole dock resizes, not
        // just its contents.
        <Show when=move || state.dock_open.get()>
            <crate::view::split::Handle divider=Divider::Dock />
        </Show>
        <section class="flex flex-none flex-col bg-window">
            <DockTabs />
            <Show when=move || state.dock_open.get()>
                // A column rather than one scrolling box: the terminal keeps its
                // prompt pinned below the scrollback, which only works if the
                // tab owns its own scroll region.
                <div
                    class="flex min-h-0 flex-col border-t border-line bg-content"
                    style=move || format!("height: {}px", state.dock_height.get())
                >
                    {move || match state.dock_tab.get() {
                        DockTab::Problems => view! { <ProblemsTab /> }.into_any(),
                        DockTab::Output => view! { <OutputTab /> }.into_any(),
                        DockTab::Terminal => {
                            view! { <crate::view::terminal::TerminalView /> }.into_any()
                        }
                        DockTab::Waves => {
                            view! { <crate::view::waves::WavesTab /> }.into_any()
                        }
                        DockTab::Devices => view! { <DevicesTab /> }.into_any(),
                    }}
                </div>
            </Show>
        </section>
    }
}

#[component]
fn DockTabs() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <div class="flex h-8 items-center gap-0.5 border-t border-line px-2">
            {DockTab::ALL
                .into_iter()
                .map(|tab| {
                    let selected = Signal::derive(move || {
                        state.dock_open.get() && state.dock_tab.get() == tab
                    });
                    view! {
                        <button
                            type="button"
                            on:click=move |_| {
                                // Clicking the tab you are already on collapses
                                // the dock. That is how every editor behaves,
                                // and it saves reaching for a separate control.
                                if selected.get() {
                                    state.dock_open.set(false);
                                } else {
                                    state.show_dock(tab);
                                }
                            }
                            class=move || {
                                let base = "flex h-[26px] items-center gap-1.5 rounded-[5px] px-2.5 \
                                            text-callout transition-colors";
                                if selected.get() {
                                    format!("{base} bg-sunken font-medium text-label")
                                } else {
                                    format!("{base} text-label-2 hover:text-label")
                                }
                            }
                        >
                            {tab.label()}
                            <DockCount tab=tab />
                        </button>
                    }
                })
                .collect_view()}

            <span class="flex-1" />

            <Show when=move || state.dock_tab.get() == DockTab::Output && state.dock_open.get()>
                <FollowToggle />
                <button
                    type="button"
                    class="rounded-[5px] px-2 py-1 text-footnote text-label-2 hover:text-label"
                    on:click=move |_| state.clear_log()
                >
                    "Clear"
                </button>
            </Show>

            <button
                type="button"
                title=move || {
                    if state.dock_open.get() { "Collapse panel" } else { "Expand panel" }
                }
                class="grid size-6 place-items-center rounded-[5px] text-label-2 hover:bg-sunken hover:text-label"
                on:click=move |_| state.dock_open.update(|open| *open = !*open)
            >
                <span class=move || {
                    if state.dock_open.get() {
                        "grid transition-transform"
                    } else {
                        "grid rotate-180 transition-transform"
                    }
                }>
                    <IconView icon=Icon::Chevron size=13 />
                </span>
            </button>
        </div>
    }
}

/// The count beside a tab name. Absent rather than zero — a badge showing "0"
/// is a badge drawing attention to nothing.
#[component]
fn DockCount(tab: DockTab) -> impl IntoView {
    let state = AppState::expect();

    move || {
        let (count, tone) = match tab {
            DockTab::Problems => {
                let blocking = state.blocking_count();
                let (errors, warnings) = state.diag_counts();
                let total = state.problems().len() + errors + warnings;
                (
                    total,
                    if blocking > 0 || errors > 0 {
                        Tone::Crimson
                    } else {
                        Tone::Amber
                    },
                )
            }
            DockTab::Output => (state.log.with(Vec::len), Tone::Neutral),
            DockTab::Terminal => (0, Tone::Neutral),
            DockTab::Waves => (0, Tone::Neutral),
            DockTab::Devices => (0, Tone::Neutral),
        };

        (count > 0).then(|| {
            let text = match tone {
                Tone::Crimson => "text-crimson",
                Tone::Amber => "text-amber",
                _ => "text-label-3",
            };
            view! {
                <span class=format!("tnum font-mono text-footnote {text}")>{count.to_string()}</span>
            }
        })
    }
}

#[component]
fn FollowToggle() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <button
            type="button"
            title="Stick to the newest line as output arrives"
            class=move || {
                let base = "rounded-[5px] px-2 py-1 text-footnote transition-colors";
                if state.log_follow.get() {
                    format!("{base} text-rust")
                } else {
                    format!("{base} text-label-2 hover:text-label")
                }
            }
            on:click=move |_| state.log_follow.update(|f| *f = !*f)
        >
            "Follow"
        </button>
    }
}

/// Where a right-click on a diagnostic row landed, and what the row named.
#[derive(Clone)]
struct DiagMenuAt {
    x: f64,
    y: f64,
    path: String,
    line: u32,
    col: u32,
    message: String,
}

#[component]
fn ProblemsTab() -> impl IntoView {
    let state = AppState::expect();
    let menu = RwSignal::new(None::<DiagMenuAt>);

    let rows = move || {
        let problems = state.problems();
        // Compiler diagnostics, flattened out of the per-file map. They join
        // the config problems here because "why does my project not build" has
        // one answer set, not two panels' worth.
        let mut diags: Vec<(String, rusty_lsp::FileDiagnostic)> = state
            .diagnostics
            .with(|by_file| {
                by_file
                    .iter()
                    .flat_map(|(path, items)| {
                        items.iter().map(|d| (path.clone(), d.clone()))
                    })
                    .collect()
            });
        diags.sort_by(|a, b| {
            (&a.0, a.1.start_line, a.1.severity).cmp(&(&b.0, b.1.start_line, b.1.severity))
        });

        if problems.is_empty() && diags.is_empty() {
            return view! {
                <p class="min-h-0 flex-1 overflow-y-auto px-4 py-3 text-callout text-label-2">
                    {if state.has_project() {
                        "Nothing wrong that rusty can see."
                    } else {
                        "Open a project to see what would stop it building."
                    }}
                </p>
            }
            .into_any();
        }
        view! {
            <div class="min-h-0 flex-1 overflow-y-auto">
                {problems
                    .into_iter()
                    .map(|problem| view! { <ProblemRow problem=problem /> })
                    .collect_view()}
                {diags
                    .into_iter()
                    .map(|(path, diagnostic)| {
                        view! { <DiagnosticRow path=path diagnostic=diagnostic menu=menu /> }
                    })
                    .collect_view()}
            </div>
        }
        .into_any()
    };

    view! {
        // The wrapper eats the browser's own context menu even over the empty
        // state — every dock surface answers a right-click itself or not at all.
        <div
            class="flex min-h-0 flex-1 flex-col"
            on:contextmenu=move |event: leptos::ev::MouseEvent| event.prevent_default()
        >
            {rows}
            {move || {
                let at = menu.get()?;
                let close = Callback::new(move |_| menu.set(None));
                let (path, line, col) = (at.path.clone(), at.line, at.col);
                let message = at.message.clone();
                Some(
                    view! {
                        <ContextMenu x=at.x y=at.y on_close=close>
                            <MenuItem
                                label="Open in the editor"
                                on_select=Callback::new(move |_| {
                                    controller::open_at(state, path.clone(), line, col);
                                    menu.set(None);
                                })
                            />
                            <MenuItem
                                label="Copy message"
                                on_select=Callback::new(move |_| {
                                    copy_to_clipboard(&message);
                                    menu.set(None);
                                })
                            />
                        </ContextMenu>
                    },
                )
            }}
        </div>
    }
}

/// One compiler finding. Clicking it opens the file — the squiggle is already
/// waiting on the line.
#[component]
fn DiagnosticRow(
    path: String,
    diagnostic: rusty_lsp::FileDiagnostic,
    menu: RwSignal<Option<DiagMenuAt>>,
) -> impl IntoView {
    let state = AppState::expect();
    let tone = match diagnostic.severity {
        DiagSeverity::Error => Tone::Crimson,
        DiagSeverity::Warning => Tone::Amber,
        _ => Tone::Slate,
    };
    let open_path = path.clone();
    let menu_path = path.clone();
    let menu_message = diagnostic.message.clone();
    let (line, col) = (diagnostic.start_line, diagnostic.start_col);
    let place = format!("{path}:{}", diagnostic.start_line + 1);
    let origin = match (&diagnostic.source, &diagnostic.code) {
        (Some(source), Some(code)) => format!("{source} · {code}"),
        (Some(source), None) => source.clone(),
        (None, Some(code)) => code.clone(),
        (None, None) => String::new(),
    };

    view! {
        <button
            type="button"
            on:click=move |_| {
                // open_at, not open_file: the row names a line, and landing at
                // the top of the file makes the click look broken — which is
                // exactly what it did before the editor had tabs.
                controller::open_at(state, open_path.clone(), line, col);
            }
            on:contextmenu=move |event: leptos::ev::MouseEvent| {
                event.prevent_default();
                menu.set(
                    Some(DiagMenuAt {
                        x: event.client_x() as f64,
                        y: event.client_y() as f64,
                        path: menu_path.clone(),
                        line,
                        col,
                        message: menu_message.clone(),
                    }),
                );
            }
            class="flex w-full items-start gap-2.5 border-b border-line px-4 py-2 text-left transition-colors last:border-b-0 hover:bg-sunken"
        >
            <div class="mt-[5px]">
                <Dot tone=tone />
            </div>
            <div class="min-w-0 flex-1">
                <div class="flex items-baseline gap-2">
                    <span class="shrink-0 font-mono text-footnote text-label-2">{place}</span>
                    {(!origin.is_empty())
                        .then(|| {
                            view! {
                                <span class="shrink-0 text-footnote text-label-3">{origin}</span>
                            }
                        })}
                </div>
                <p class="mt-0.5 max-w-[90ch] text-callout leading-relaxed text-label whitespace-pre-wrap select-text">
                    {diagnostic.message}
                </p>
            </div>
        </button>
    }
}

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
fn OutputTab() -> impl IntoView {
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
        let _ = state.log.with(Vec::len);
        if !state.log_follow.get() {
            return;
        }
        let Some(element) = scroller.get() else { return };
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
        if line.is_empty() || state.session_running.get_untracked() {
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
                    state.log_follow.set(false);
                    return;
                }
                let distance = element.scroll_height()
                    - top
                    - element.client_height();
                // A few pixels of slack: fractional scroll positions mean an
                // exact comparison is never true on a trackpad.
                if distance <= 4 {
                    state.log_follow.set(true);
                }
            }
            class="min-h-0 flex-1 overflow-y-auto"
        >
            {move || {
                let lines = state.log.get();
                if lines.is_empty() {
                    return view! {
                        <p class="px-4 py-3 text-callout text-label-2">
                            "Output from flashing, monitoring and anything you run below lands \
                             here, and stays while you work in other panels."
                        </p>
                    }
                        .into_any();
                }
                view! {
                    <div class="px-3 py-2 font-mono text-footnote leading-[1.6] select-text">
                        {lines
                            .into_iter()
                            .map(|line| {
                                // Level first, then stream: a defmt ERROR is an
                                // error whichever pipe it arrived on, but an
                                // unlevelled line on stderr is still worth marking.
                                let colour = match line.level {
                                    Some(LogLevel::Error) => "text-crimson",
                                    Some(LogLevel::Warn) => "text-amber",
                                    Some(LogLevel::Info) => "text-label",
                                    Some(LogLevel::Debug) | Some(LogLevel::Trace) => "text-label-3",
                                    None if line.stream == LogStream::Stderr => "text-amber",
                                    None => "text-label-2",
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
                                                            title="Open in the editor"
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
                        "cargo build --release".to_string()
                    } else {
                        "open a project first — commands run in its root".to_string()
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
                    .session_running
                    .get()
                    .then(|| {
                        view! {
                            <button
                                type="button"
                                class="shrink-0 rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:text-label"
                                on:click=move |_| controller::stop_session(state)
                            >
                                "Stop"
                            </button>
                        }
                    })
            }}
        </div>

        {move || {
            let (x, y, line) = menu.get()?;
            let close = Callback::new(move |_| menu.set(None));
            let follow_label = if state.log_follow.get_untracked() {
                "Stop following"
            } else {
                "Follow new output"
            };
            Some(
                view! {
                    <ContextMenu x=x y=y on_close=close>
                        {line
                            .map(|text| {
                                view! {
                                    <MenuItem
                                        label="Copy line"
                                        on_select=Callback::new(move |_| {
                                            copy_to_clipboard(&text);
                                            menu.set(None);
                                        })
                                    />
                                }
                            })}
                        <MenuItem
                            label="Copy all"
                            on_select=Callback::new(move |_| {
                                let all = state
                                    .log
                                    .with_untracked(|lines| {
                                        lines
                                            .iter()
                                            .map(|l| l.text.as_str())
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
                                state.log_follow.update(|f| *f = !*f);
                                menu.set(None);
                            })
                        />
                        <MenuItem
                            label="Clear"
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

#[component]
fn DevicesTab() -> impl IntoView {
    let state = AppState::expect();
    let menu = RwSignal::new(None::<(f64, f64)>);

    // The same picker the Flash and Monitor panels use, and the same state
    // behind it: choosing a device here selects it there. Two lists of what is
    // plugged in would eventually disagree, and the one showing a device that
    // is gone is the one the user happens to be looking at.
    view! {
        <div
            class="min-h-0 flex-1 overflow-y-auto pb-2"
            on:contextmenu=move |event: leptos::ev::MouseEvent| {
                event.prevent_default();
                menu.set(Some((event.client_x() as f64, event.client_y() as f64)));
            }
        >
            <crate::view::panels::Devices />
            <div class="mt-1 flex items-center gap-2 px-4">
                <Dot tone=Tone::Neutral />
                <span class="text-footnote text-label-3">
                    {move || {
                        format!(
                            "named against {} boards and {} chips",
                            state.boards.with(Vec::len),
                            state.chips.with(Vec::len),
                        )
                    }}
                </span>
                <button
                    type="button"
                    class="rounded-[5px] px-2 py-0.5 text-footnote text-rust hover:underline"
                    on:click=move |_| controller::load_catalog(state)
                >
                    "Reload catalogue"
                </button>
            </div>
        </div>

        {move || {
            let (x, y) = menu.get()?;
            let close = Callback::new(move |_| menu.set(None));
            Some(
                view! {
                    <ContextMenu x=x y=y on_close=close>
                        <MenuItem
                            label="Rescan devices"
                            on_select=Callback::new(move |_| {
                                controller::scan_devices(state);
                                menu.set(None);
                            })
                        />
                        <MenuItem
                            label="Reload catalogue"
                            on_select=Callback::new(move |_| {
                                controller::load_catalog(state);
                                menu.set(None);
                            })
                        />
                    </ContextMenu>
                },
            )
        }}
    }
}
