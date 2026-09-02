//! The stack, the variables, and where execution is stopped.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, Divider},
    view::split,
};

/// Where the target is stopped: the call stack, and what the selected
/// frame's variables hold.
///
/// Two columns rather than two panels: a frame and its variables are read
/// together — "what called this, and with what" is one question.
#[component]
pub(super) fn DebugTab() -> impl IntoView {
    let state = AppState::expect();
    // Which long values are open, by name. Outside the render closure so a
    // stop — or a refreshed frame — does not fold everything back up.
    let expanded = RwSignal::new(std::collections::HashSet::<String>::new());

    move || {
        let Some(debug) = state.debug.session.get() else {
            return view! {
                <p class="min-h-0 flex-1 overflow-y-auto px-4 py-3 text-callout text-label-2">
                    {t!("dock.debug.idle")}
                </p>
            }
            .into_any();
        };
        if debug.running {
            return view! {
                <p class="min-h-0 flex-1 overflow-y-auto px-4 py-3 text-callout text-label-2">
                    {t!("dock.debug.running")}
                </p>
            }
            .into_any();
        }

        let frame = debug.frame;
        view! {
            <div class="flex min-h-0 flex-1">
                <div
                    class="min-w-0 shrink-0 overflow-y-auto"
                    style:width=move || format!("{}px", state.layout.debug_width.get())
                >
                    <div class="px-4 py-1.5 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        {t!("dock.debug.call-stack")}
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
                                        let base = "flex w-full items-baseline gap-2 px-4 \
                                                    py-1 text-left transition-colors";
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
                <split::Handle divider=Divider::DebugStack />
                <div class="min-w-0 flex-1 overflow-y-auto">
                    <div class="px-4 py-1.5 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        {t!("dock.debug.variables")}
                    </div>
                    {debug
                        .variables
                        .into_iter()
                        .map(|variable| {
                            // A HAL handle prints as its whole type structure —
                            // hundreds of characters of `Uart0(Inner(…))` that a
                            // truncated row turns into a row you cannot read and
                            // cannot widen. Long values get a chevron and open in
                            // place; short ones stay one line, which is most of
                            // them and the ones you are usually watching.
                            let long = variable.value.chars().count() > 64;
                            // Hovering shows the whole thing without opening it.
                            let tooltip = variable.value.clone();
                            let name = variable.name.clone();
                            let open = long && expanded.with(|set| set.contains(&name));
                            let toggle = name.clone();
                            let value_class = if open {
                                "min-w-0 flex-1 break-all whitespace-pre-wrap text-label-2 select-text"
                            } else {
                                "min-w-0 flex-1 truncate text-label-2 select-text"
                            };
                            // Opened, it is laid out rather than dumped: the
                            // structure gdb printed on one line is what makes
                            // the value answerable at all.
                            let shown = if open {
                                rusty_dbg::pretty::pretty(&variable.value, 72)
                            } else {
                                variable.value
                            };
                            view! {
                                <div class="flex items-baseline gap-2 px-4 py-1 font-mono text-footnote">
                                    <button
                                        type="button"
                                        title=if long { t!("dock.debug.show-whole") } else { String::new() }
                                        disabled=!long
                                        on:click=move |_| {
                                            expanded
                                                .update(|set| {
                                                    if !set.remove(&toggle) {
                                                        set.insert(toggle.clone());
                                                    }
                                                })
                                        }
                                        class=move || {
                                            let base = "w-[1ch] shrink-0 text-caption";
                                            if long {
                                                format!("{base} text-label-3 hover:text-rust")
                                            } else {
                                                format!("{base} text-transparent")
                                            }
                                        }
                                    >
                                        {if open { "⌄" } else { "›" }}
                                    </button>
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
                                    <span class=value_class title=tooltip>{shown}</span>
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
