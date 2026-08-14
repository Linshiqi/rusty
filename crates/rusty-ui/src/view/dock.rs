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

use rusty_embed::LogLevel;
use rusty_lsp::DiagSeverity;

use crate::{
    controller,
    state::{AppState, Divider, DockTab},
    view::components::{Button, ButtonKind, ContextMenu, Dot, MenuItem, MenuSeparator, ProblemRow, Tone, copy_to_clipboard},
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
                        DockTab::Debug => view! { <DebugTab /> }.into_any(),
                        DockTab::Registers => view! { <RegistersTab /> }.into_any(),
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
                        // The shell picker sits beside its tab, not at the
                        // far edge next to the collapse chevron — two
                        // unrelated dropdown arrows in one corner read as
                        // one broken control.
                        {(tab == DockTab::Terminal)
                            .then(|| {
                                view! {
                                    <Show when=move || selected.get()>
                                        <ShellPicker />
                                    </Show>
                                }
                            })}
                    }
                })
                .collect_view()}

            <span class="flex-1" />

            <Show when=move || state.dock_tab.get() == DockTab::Output && state.dock_open.get()>
                // VSCode's pair: which channel, then a text filter. The
                // channel list is fixed — it is the set of things rusty runs.
                <select
                    title="Show one output channel"
                    class="h-6 rounded-[5px] bg-sunken px-1 text-footnote text-label-2 outline-none"
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        let pick = CHANNELS
                            .iter()
                            .find(|c| **c == value)
                            .copied()
                            .unwrap_or("all");
                        state.log_pick.set(pick);
                    }
                >
                    {CHANNELS
                        .iter()
                        .map(|c| {
                            let channel = *c;
                            view! {
                                <option
                                    value=channel
                                    selected=move || state.log_pick.get() == channel
                                >
                                    {channel}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
                // No Follow or Clear buttons: following is the default (a
                // scroll up detaches, the bottom reattaches), and Clear
                // lives in the right-click menu with the other verbs.
                <input
                    placeholder="filter (!word excludes)"
                    title="Space-separated terms all must match; !term excludes"
                    class="h-6 w-40 rounded-[5px] bg-sunken px-1.5 text-footnote outline-none placeholder:text-label-4"
                    prop:value=move || state.log_filter.get()
                    on:input=move |event| state.log_filter.set(event_target_value(&event))
                />
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

#[component]
fn ShellPicker() -> impl IntoView {
    let state = AppState::expect();
    controller::load_shell_choices(state);
    controller::load_shell_info(state);

    view! {
        <select
            title="Which shell the terminal runs"
            class="h-6 rounded-[5px] bg-sunken px-1.5 text-footnote text-label-2 outline-none"
            prop:value=move || {
                state
                    .shell_info
                    .get()
                    .and_then(|info| info.preference)
                    .unwrap_or_else(|| "auto".to_string())
            }
            on:change=move |event| {
                controller::set_terminal_shell(state, Some(event_target_value(&event)));
            }
        >
            {move || {
                let mut choices = state.shell_choices.get();
                // A stored preference the list does not carry (an uninstalled
                // shell, an old bare-name value) still has to be visible —
                // a select whose value matches nothing renders blank.
                if let Some(preference) =
                    state.shell_info.get().and_then(|info| info.preference)
                    && !choices.iter().any(|c| c.value == preference)
                {
                    let short = preference
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(&preference)
                        .to_string();
                    choices.push(rusty_embed::ShellChoice {
                        label: format!("{short} (current)"),
                        value: preference,
                    });
                }
                choices
                    .into_iter()
                    .map(|choice| {
                        view! { <option value=choice.value>{choice.label}</option> }
                    })
                    .collect_view()
            }}
        </select>
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
            // The frame count while stopped: a badge that says how deep
            // the target is, without opening the tab.
            DockTab::Registers => (0, Tone::Neutral),
            DockTab::Debug => (
                state.debug.with(|d| {
                    d.as_ref().filter(|d| !d.running).map_or(0, |d| d.stack.len())
                }),
                Tone::Rust,
            ),
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

/// Every channel a line can carry. "all" is the view's word, not a tag.
const CHANNELS: [&str; 8] =
    ["all", "build", "flash", "monitor", "simulate", "commands", "tools", "app"];

/// True when the line passes the Output panel's filter box. Terms AND
/// together; a `!` prefix excludes. Case-insensitive, as every log filter
/// people actually use is.
fn passes_filter(text: &str, filter: &str) -> bool {
    let haystack = text.to_lowercase();
    filter.split_whitespace().all(|term| {
        if let Some(excluded) = term.strip_prefix('!') {
            excluded.is_empty() || !haystack.contains(&excluded.to_lowercase())
        } else {
            haystack.contains(&term.to_lowercase())
        }
    })
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
                let pick = state.log_pick.get();
                let filter = state.log_filter.get();
                let all = state.log.get();
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
                            {format!(
                                "All {total} lines are hidden by the current channel and \
                                 filter.",
                            )}
                        </p>
                    }
                        .into_any();
                }
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
                        {(hidden > 0)
                            .then(|| {
                                view! {
                                    <div class="text-caption text-label-4">
                                        {format!("… {hidden} lines hidden by the filter")}
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

    // One list of what is plugged in, one place to act on it.
    view! {
        <div
            class="min-h-0 flex-1 overflow-y-auto pb-2"
            on:contextmenu=move |event: leptos::ev::MouseEvent| {
                event.prevent_default();
                menu.set(Some((event.client_x() as f64, event.client_y() as f64)));
            }
        >
            // The whole device workspace — list, mode toggle, command, run.
            // It lived in a Flash panel once; every path to it was a detour
            // past this list, which is where the eye already was.
            <crate::view::panels::session::Session />
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

/// Where the target is stopped: the call stack, and what the selected
/// frame's variables hold.
///
/// Two columns rather than two panels: a frame and its variables are read
/// together — "what called this, and with what" is one question.
#[component]
fn DebugTab() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let Some(debug) = state.debug.get() else {
            return view! {
                <p class="min-h-0 flex-1 overflow-y-auto px-4 py-3 text-callout text-label-2">
                    "Nothing is being debugged. The Simulate panel's Debug button boots                      the firmware frozen and attaches here."
                </p>
            }
            .into_any();
        };
        if debug.running {
            return view! {
                <p class="min-h-0 flex-1 overflow-y-auto px-4 py-3 text-callout text-label-2">
                    "Running — pause or hit a breakpoint to read the stack."
                </p>
            }
            .into_any();
        }

        let frame = debug.frame;
        view! {
            <div class="flex min-h-0 flex-1">
                <div class="w-[46%] min-w-0 overflow-y-auto border-r border-line">
                    <div class="px-4 py-1.5 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        "Call stack"
                    </div>
                    {debug
                        .stack
                        .into_iter()
                        .map(|entry| {
                            let selected = entry.level == frame;
                            let level = entry.level;
                            let place = match (&entry.file, entry.line) {
                                (Some(file), Some(line)) => format!("{file}:{}", line + 1),
                                // A frame with no source is listed with its
                                // address rather than hidden: an interrupt
                                // vector between two of your functions is
                                // the answer sometimes.
                                _ => entry.address.clone(),
                            };
                            let jump = entry.file.clone().zip(entry.line);
                            view! {
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        controller::debug_frame(state, level);
                                        if let Some((file, line)) = jump.clone() {
                                            controller::open_at(state, file, line, 0);
                                        }
                                    }
                                    class=move || {
                                        let base = "flex w-full items-baseline gap-2 px-4 py-1                                                     text-left transition-colors";
                                        if selected {
                                            format!("{base} bg-selection text-rust")
                                        } else {
                                            format!("{base} text-label-2 hover:bg-sunken")
                                        }
                                    }
                                >
                                    <span class="w-[2ch] shrink-0 font-mono text-caption text-label-4">
                                        {entry.level.to_string()}
                                    </span>
                                    <span class="shrink-0 font-mono text-footnote">
                                        {entry.function}
                                    </span>
                                    <span class="min-w-0 truncate font-mono text-caption text-label-3">
                                        {place}
                                    </span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
                <div class="min-w-0 flex-1 overflow-y-auto">
                    <div class="px-4 py-1.5 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        "Variables"
                    </div>
                    {debug
                        .variables
                        .into_iter()
                        .map(|variable| {
                            view! {
                                <div class="flex items-baseline gap-2 px-4 py-1 font-mono text-footnote">
                                    <span class="shrink-0 text-label">{variable.name}</span>
                                    {variable
                                        .kind
                                        .map(|kind| {
                                            view! {
                                                <span class="shrink-0 text-caption text-label-4">
                                                    {kind}
                                                </span>
                                            }
                                        })}
                                    <span class="min-w-0 flex-1 truncate text-label-2 select-text">
                                        {variable.value}
                                    </span>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>
            </div>
        }
        .into_any()
    }
}

/// The chip's peripherals, as the target holds them right now.
///
/// The whole reason a debugger beats printf on embedded work: "did my GPIO
/// config actually take" is a question about a register, and the answer is
/// four bytes at a fixed address. The values come from one read of the
/// selected peripheral's whole block, refreshed on each stop.
#[component]
fn RegistersTab() -> impl IntoView {
    let state = AppState::expect();

    // Read the map once per project; the file does not change under us.
    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.registers.with(Option::is_none) && state.has_project() {
            controller::load_registers(state);
        }
    });

    // On every stop, re-read the selected peripheral: the values on screen
    // must be the ones the target holds *now*, not the ones from before the
    // last step.
    Effect::new(move |_| {
        let stopped = state
            .debug
            .with(|debug| debug.as_ref().is_some_and(|debug| !debug.running));
        let Some(name) = state.peripheral.get() else {
            return;
        };
        if !stopped {
            return;
        }
        let span = state.registers.with_untracked(|map| {
            map.as_ref()
                .and_then(Option::as_ref)
                .and_then(|map| map.peripherals.iter().find(|p| p.name == name))
                .map(|p| {
                    let end = p
                        .registers
                        .iter()
                        .filter(|r| r.readable)
                        .map(|r| r.offset + 4)
                        .max()
                        .unwrap_or(4);
                    (p.base, end.min(4096))
                })
        });
        if let Some((base, bytes)) = span {
            controller::read_peripheral(state, base, bytes);
        }
    });

    move || {
        let Some(loaded) = state.registers.get() else {
            return view! {
                <p class="px-4 py-3 text-callout text-label-2">"Reading the chip's SVD…"</p>
            }
            .into_any();
        };
        let Some(map) = loaded else {
            // Refused rather than guessed: register addresses invented from
            // memory would be the worst possible answer here.
            return view! {
                <div class="flex flex-col items-start gap-3 px-4 py-3">
                    <p class="max-w-[70ch] text-callout leading-relaxed text-label-2">
                        "No SVD for this chip on this machine. rusty will not guess register \
                         addresses — fetch the vendor's description, or drop one in the \
                         project at .rusty/svd/<chip>.svd."
                    </p>
                    <Button
                        label="Fetch the SVD"
                        kind=ButtonKind::Primary
                        on_click=Callback::new(move |_| controller::fetch_svd(state))
                    />
                </div>
            }
            .into_any();
        };

        let names: Vec<String> = map.peripherals.iter().map(|p| p.name.clone()).collect();
        let selected = state.peripheral.get().or_else(|| names.first().cloned());
        let peripheral = selected
            .as_ref()
            .and_then(|name| map.peripherals.iter().find(|p| &p.name == name).cloned());
        let dropped = map.dropped;

        view! {
            <div class="flex min-h-0 flex-1">
                <div class="w-[180px] flex-none overflow-y-auto border-r border-line">
                    {names
                        .into_iter()
                        .map(|name| {
                            let is_selected = selected.as_deref() == Some(name.as_str());
                            let pick = name.clone();
                            view! {
                                <button
                                    type="button"
                                    on:click=move |_| state.peripheral.set(Some(pick.clone()))
                                    class=if is_selected {
                                        "w-full bg-selection px-4 py-1 text-left font-mono text-footnote text-rust"
                                    } else {
                                        "w-full px-4 py-1 text-left font-mono text-footnote text-label-2 hover:bg-sunken"
                                    }
                                >
                                    {name}
                                </button>
                            }
                        })
                        .collect_view()}
                    {(dropped > 0)
                        .then(|| {
                            view! {
                                <p class="px-4 py-2 text-caption leading-snug text-label-4">
                                    {format!(
                                        "{dropped} more inherit from another peripheral, which rusty does not resolve yet.",
                                    )}
                                </p>
                            }
                        })}
                </div>
                <div class="min-w-0 flex-1 overflow-y-auto">
                    {peripheral
                        .map(|peripheral| {
                            let base = peripheral.base;
                            view! {
                                <div class="flex items-baseline gap-2 px-4 py-1.5">
                                    <span class="font-mono text-footnote text-label">
                                        {peripheral.name.clone()}
                                    </span>
                                    <span class="font-mono text-caption text-label-4">
                                        {format!("0x{base:08X}")}
                                    </span>
                                    <span class="min-w-0 truncate text-caption text-label-3">
                                        {peripheral.description.clone()}
                                    </span>
                                </div>
                                {peripheral
                                    .registers
                                    .into_iter()
                                    .map(|register| {
                                        view! { <RegisterRow base=base register=register /> }
                                    })
                                    .collect_view()}
                            }
                        })}
                </div>
            </div>
        }
        .into_any()
    }
}

/// One register: what it holds now, and what its bits mean.
#[component]
fn RegisterRow(base: u64, register: rusty_embed::Register) -> impl IntoView {
    let state = AppState::expect();
    let open = RwSignal::new(false);
    let address = base + u64::from(register.offset);
    let readable = register.readable;
    let fields = register.fields.clone();
    let offset = register.offset;
    let name = register.name.clone();
    let description = register.description.clone();

    // The value, assembled little-endian out of whichever span covers it.
    let value = Signal::derive(move || {
        if !readable {
            return None;
        }
        state.debug.with(|debug| {
            let debug = debug.as_ref()?;
            let read = debug.memory.iter().find(|read| {
                address >= read.begin && address + 4 <= read.begin + read.data.len() as u64
            })?;
            let at = (address - read.begin) as usize;
            let bytes = read.data.get(at..at + 4)?;
            Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        })
    });

    view! {
        <div class="border-b border-line last:border-b-0">
            <button
                type="button"
                on:click=move |_| open.update(|o| *o = !*o)
                class="flex w-full items-baseline gap-3 px-4 py-1 text-left hover:bg-sunken"
            >
                <span class="w-[7ch] shrink-0 font-mono text-caption text-label-4">
                    {format!("+0x{offset:03X}")}
                </span>
                <span class="w-[22ch] shrink-0 truncate font-mono text-footnote text-label">
                    {name}
                </span>
                <span class="w-[12ch] shrink-0 font-mono text-footnote text-rust select-text">
                    {move || match value.get() {
                        Some(value) => format!("0x{value:08X}"),
                        // Not a zero: a register never read and a register
                        // reading zero are different facts, and showing zero
                        // for both is how a debugger lies.
                        None if readable => "—".to_string(),
                        None => "write-only".to_string(),
                    }}
                </span>
                <span class="min-w-0 truncate text-caption text-label-3">{description}</span>
            </button>
            <Show when=move || open.get()>
                <div class="px-4 pb-2">
                    {fields
                        .iter()
                        .map(|field| {
                            let (bit, width) = (field.offset, field.width);
                            let field_name = field.name.clone();
                            let field_help = field.description.clone();
                            let bits = if width == 1 {
                                format!("[{bit}]")
                            } else {
                                format!("[{}:{bit}]", bit + width - 1)
                            };
                            view! {
                                <div class="flex items-baseline gap-3 py-0.5 pl-8">
                                    <span class="w-[8ch] shrink-0 font-mono text-caption text-label-4">
                                        {bits}
                                    </span>
                                    <span class="w-[20ch] shrink-0 truncate font-mono text-caption text-label-2">
                                        {field_name}
                                    </span>
                                    <span class="w-[8ch] shrink-0 font-mono text-caption text-rust">
                                        {move || {
                                            value
                                                .get()
                                                .map(|value| {
                                                    let mask = if width >= 32 {
                                                        u32::MAX
                                                    } else {
                                                        (1u32 << width) - 1
                                                    };
                                                    ((value >> bit) & mask).to_string()
                                                })
                                                .unwrap_or_default()
                                        }}
                                    </span>
                                    <span class="min-w-0 truncate text-caption text-label-4">
                                        {field_help}
                                    </span>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>
            </Show>
        </div>
    }
}
