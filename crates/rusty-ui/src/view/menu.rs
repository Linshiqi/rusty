//! The menu bar, drawn in the title bar.
//!
//! The window has no OS decorations, so this is both the menu bar and the drag
//! handle. VSCode does the same thing on Windows for the same reason: the
//! platform's own title bar wastes a strip of height and cannot hold anything.
//!
//! Menus rather than a row of buttons because a button row only scales to about
//! three commands before it becomes clutter with no organising principle —
//! which is exactly what "Open project" and "Re-check" sitting in the corner had
//! become. Everything a menu can reach is an [`Action`], the same ones the
//! palette lists and the keyboard fires.

use leptos::{ev, prelude::*};

use crate::{
    command::{self, Chrome, Item},
    state::AppState,
};

#[component]
pub fn MenuBar(chrome: Chrome) -> impl IntoView {
    let state = AppState::expect();
    // Which menu is down, by index. Also the "menu mode" flag: once one is
    // open, hovering a sibling switches to it without a second click, which is
    // how every desktop menu bar has worked for thirty years.
    let open = RwSignal::new(None::<usize>);
    close_on_escape(open);

    let menus = command::menus(state);

    view! {
        <header
            data-tauri-drag-region
            class="relative z-40 flex h-9 flex-none items-center bg-window pl-2"
        >
            <div class="pointer-events-none flex items-center gap-1.5 pr-1 pl-1 text-rust">
                <crate::view::icon::Brandmark size=14 />
            </div>

            <nav class="flex items-center">
                {menus
                    .into_iter()
                    .enumerate()
                    .map(|(index, menu)| {
                        let is_open = Signal::derive(move || open.get() == Some(index));
                        view! {
                            <div class="relative">
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        open.update(|o| {
                                            *o = if *o == Some(index) { None } else { Some(index) }
                                        })
                                    }
                                    on:mouseenter=move |_| {
                                        if open.get_untracked().is_some() {
                                            open.set(Some(index));
                                        }
                                    }
                                    class=move || {
                                        let base = "rounded-[4px] px-2 py-[3px] text-callout \
                                                    transition-colors";
                                        if is_open.get() {
                                            format!("{base} bg-sunken text-label")
                                        } else {
                                            format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                        }
                                    }
                                >
                                    {menu.title}
                                </button>

                                <Show when=move || is_open.get()>
                                    <Dropdown
                                        items=command::menus(state)
                                            .into_iter()
                                            .nth(index)
                                            .map(|m| m.items)
                                            .unwrap_or_default()
                                        chrome=chrome
                                        close=Callback::new(move |_| open.set(None))
                                    />
                                </Show>
                            </div>
                        }
                    })
                    .collect_view()}
            </nav>

            <span data-tauri-drag-region class="flex-1 self-stretch" />

            // The open project, centred the way an editor centres the document
            // it is showing. It is the answer to "what am I looking at", which
            // is worth a glance and never worth a click.
            {move || {
                state
                    .project
                    .get()
                    .map(|project| {
                        let chip = project.chip.clone();
                        let name = project
                            .root
                            .rsplit(['/', '\\'])
                            .next()
                            .unwrap_or(&project.root)
                            .to_string();
                        view! {
                            <div class="pointer-events-none flex min-w-0 items-center gap-2 text-footnote">
                                <span class="truncate text-label-2">{name}</span>
                                {chip
                                    .map(|chip| {
                                        view! {
                                            <span class="rounded-full bg-sunken px-1.5 font-mono text-label-3">
                                                {chip}
                                            </span>
                                        }
                                    })}
                            </div>
                        }
                    })
            }}

            <span data-tauri-drag-region class="flex-1 self-stretch" />

            {
                let state = AppState::expect();
                view! {
                    <button
                        type="button"
                        aria-label="Assistant"
                        title="Assistant"
                        on:click=move |_| {
                            state.assistant_open.update(|open| *open = !*open)
                        }
                        class=move || {
                            let base = "grid h-9 w-[42px] place-items-center transition-colors \
                                        hover:bg-sunken";
                            if state.assistant_open.get() {
                                format!("{base} text-rust")
                            } else {
                                format!("{base} text-label-2 hover:text-label")
                            }
                        }
                    >
                        <crate::view::icon::IconView icon=crate::view::icon::Icon::Assistant size=15 />
                    </button>
                }
            }

            <WindowControls />

            // Clicking anywhere else closes the menu. A transparent sheet behind
            // the dropdown is the only way to catch that click without a
            // document listener that would fight the button's own handler.
            <Show when=move || open.get().is_some()>
                <div
                    class="fixed inset-0 z-30"
                    on:mousedown=move |_| open.set(None)
                />
            </Show>
        </header>
    }
}

#[component]
fn Dropdown(items: Vec<Item>, chrome: Chrome, close: Callback<()>) -> impl IntoView {
    view! {
        <div class="absolute top-full left-0 z-40 mt-0.5 min-w-[224px] rounded-[8px] bg-raised py-1 shadow-2xl ring-1 ring-line-strong">
            <Rows items=items chrome=chrome close=close />
        </div>
    }
}

/// The rows of one dropdown level; submenus recurse into a flyout.
#[component]
fn Rows(items: Vec<Item>, chrome: Chrome, close: Callback<()>) -> AnyView {
    let state = AppState::expect();

    items
        .into_iter()
        .map(|item| match item {
                    Item::Separator => {
                        view! { <div class="my-1 h-px bg-line" /> }.into_any()
                    }
                    Item::Submenu { label, items } => {
                        // Hover opens, like every menu bar; the flyout sits to
                        // the right, aligned with its parent row.
                        let hovering = RwSignal::new(false);
                        view! {
                            <div
                                class="relative"
                                on:mouseenter=move |_| hovering.set(true)
                                on:mouseleave=move |_| hovering.set(false)
                            >
                                <div class="flex w-full cursor-default items-center gap-6 px-3 py-[3px] text-left text-callout text-label-2 transition-colors hover:bg-selection hover:text-rust">
                                    <span class="flex-1 whitespace-nowrap">{label}</span>
                                    <span class="shrink-0 text-footnote text-label-3">"▸"</span>
                                </div>
                                <Show when=move || hovering.get()>
                                    <div class="absolute top-0 left-full z-50 min-w-[280px] max-w-[440px] rounded-[8px] bg-raised py-1 shadow-2xl ring-1 ring-line-strong">
                                        <Rows items=items.clone() chrome=chrome close=close />
                                    </div>
                                </Show>
                            </div>
                        }
                        .into_any()
                    }
                    Item::Entry { action, label, shortcut, needs_project } => {
                        let disabled = Signal::derive(move || {
                            needs_project && !state.has_project()
                        });
                        view! {
                            <button
                                type="button"
                                disabled=move || disabled.get()
                                on:click=move |_| {
                                    close.run(());
                                    command::run(action, state, chrome);
                                }
                                class="flex w-full items-center gap-6 px-3 py-[3px] text-left text-callout text-label-2 transition-colors hover:bg-selection hover:text-rust disabled:pointer-events-none disabled:opacity-35"
                            >
                                <span class="flex-1 whitespace-nowrap">{label}</span>
                                {shortcut
                                    .map(|keys| {
                                        view! {
                                            <span class="shrink-0 font-mono text-footnote text-label-3">
                                                {keys}
                                            </span>
                                        }
                                    })}
                            </button>
                        }
                        .into_any()
                    }
        })
        .collect_view()
        .into_any()
}

#[component]
fn WindowControls() -> impl IntoView {
    // Windows proportions, scaled to the shorter bar. Close goes red on hover
    // rather than grey — matching the platform matters more than matching the
    // other two, because the muscle memory is for the OS.
    let button = "grid h-9 w-[42px] place-items-center text-label-2 \
                  transition-colors hover:bg-sunken hover:text-label";

    view! {
        <div class="ml-1 flex self-stretch">
            <button
                type="button"
                aria-label="Minimise"
                class=button
                on:click=move |_| {
                    crate::controller::window_action(crate::ipc::cmd::window::MINIMIZE)
                }
            >
                <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
                    <path d="M0 5h10" stroke="currentColor" stroke-width="1" />
                </svg>
            </button>
            <button
                type="button"
                aria-label="Maximise"
                class=button
                on:click=move |_| {
                    crate::controller::window_action(crate::ipc::cmd::window::TOGGLE_MAXIMIZE)
                }
            >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
                    <rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" stroke-width="1" />
                </svg>
            </button>
            <button
                type="button"
                aria-label="Close"
                class=format!("{button} hover:!bg-crimson hover:!text-white")
                on:click=move |_| {
                    crate::controller::window_action(crate::ipc::cmd::window::CLOSE)
                }
            >
                <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
                    <path d="M0 0l10 10M10 0L0 10" stroke="currentColor" stroke-width="1" />
                </svg>
            </button>
        </div>
    }
}

/// Close any open menu on Escape.
///
/// Lives with the menu rather than in the palette's handler so the two overlays
/// do not have to know about each other's state.
fn close_on_escape(open: RwSignal<Option<usize>>) {
    let handle = window_event_listener(ev::keydown, move |event| {
        if event.key() == "Escape" && open.get_untracked().is_some() {
            open.set(None);
        }
    });
    std::mem::forget(handle);
}
