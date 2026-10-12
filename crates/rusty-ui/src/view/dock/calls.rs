//! The Calls tab: who calls a function and what it calls, a level at a time
//! — VS Code's call hierarchy, in the dock beside the other lists rather
//! than in a sidebar this window does not have.
//!
//! A row is a function. Its chevron opens the calls into it or out of it;
//! a click goes where the call is made, and a double-click to the function
//! itself. A function called more than once from one place says how many
//! times, and the count lists every call in the finder.

use rusty_i18n::t;

use super::*;
use crate::{
    calls::{CallRow, CallTree, Openness},
    controller,
    state::{AppState, CallsView},
    view::icon::{Icon, IconView},
};

#[component]
pub(super) fn CallsTab() -> impl IntoView {
    let state = AppState::expect();

    let body = move || match state.layout.calls.get() {
        CallsView::Idle => note(t!("calls.idle")),
        CallsView::Asking(_) => note(t!("calls.asking")),
        CallsView::NoFunction(word) if word.is_empty() => note(t!("calls.no-function")),
        CallsView::NoFunction(word) => note(t!("calls.not-a-function", name = word)),
        CallsView::Tree(tree) => tree_view(state, tree),
    };

    view! {
        // Every dock surface answers a right-click itself or not at all.
        <div
            class="flex min-h-0 flex-1 flex-col"
            on:contextmenu=move |event: leptos::ev::MouseEvent| event.prevent_default()
        >
            {body}
        </div>
    }
}

fn note(text: String) -> AnyView {
    view! {
        <p class="min-h-0 flex-1 overflow-y-auto px-4 py-3 text-callout text-label-2">{text}</p>
    }
    .into_any()
}

/// The heading — which function, which way, and the switch between the two
/// ways — over the rows.
fn tree_view(state: AppState, tree: CallTree) -> AnyView {
    let root = tree.root().name.clone();
    let incoming = tree.incoming;
    let heading = if incoming {
        t!("calls.heading-in", name = root.clone())
    } else {
        t!("calls.heading-out", name = root.clone())
    };
    // Opened, and nothing came back: say so under the function, rather than
    // leaving a lone row that reads as a hierarchy still loading.
    let empty = tree.rows.len() == 1 && tree.rows[0].open == Openness::Open;
    let rows = (0..tree.rows.len())
        .map(|index| call_row(state, &tree, index))
        .collect_view();
    let direction = move |into: bool, label: String| {
        let class = if into == incoming {
            "h-[22px] rounded-[5px] bg-content px-2.5 text-footnote font-medium text-label shadow-sm"
        } else {
            "h-[22px] rounded-[5px] px-2.5 text-footnote text-label-2 hover:text-label"
        };
        view! {
            <button
                type="button"
                class=class
                on:click=move |_| controller::set_call_direction(state, into)
            >
                {label}
            </button>
        }
    };
    view! {
        <div class="flex h-9 flex-none items-center gap-3 border-b border-line px-3">
            <span class="min-w-0 flex-1 truncate text-footnote text-label-2">{heading}</span>
            <div class="inline-flex flex-none rounded-[7px] bg-sunken p-0.5">
                {direction(true, t!("calls.incoming"))}
                {direction(false, t!("calls.outgoing"))}
            </div>
        </div>
        <div class="min-h-0 flex-1 overflow-y-auto py-1">
            {rows}
            {empty
                .then(|| {
                    let text = if incoming {
                        t!("calls.none-in", name = root.clone())
                    } else {
                        t!("calls.none-out", name = root.clone())
                    };
                    view! { <p class="py-1 pr-3 pl-9 text-footnote text-label-3">{text}</p> }
                })}
        </div>
    }
    .into_any()
}

/// One function: the chevron, its name, what the server says of it, how
/// many calls, and the file and line a click goes to.
fn call_row(state: AppState, tree: &CallTree, index: usize) -> AnyView {
    let row: &CallRow = &tree.rows[index];
    let id = row.id;
    let open = row.open;
    let recursive = tree.recursive(index);
    let first = index == 0;
    // Where a click goes: the first call, or the function for the first row.
    let target = row
        .call
        .sites
        .first()
        .unwrap_or(&row.call.item.place)
        .location
        .clone();
    let file = target
        .path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&target.path)
        .to_string();
    let place = format!("{file}:{}", target.line + 1);
    let whole = format!("{}:{}", target.path, target.line + 1);
    let count = row.call.sites.len();
    let tip = if first {
        t!("calls.root-tip")
    } else {
        t!("calls.row-tip")
    };
    let indent = format!("padding-left: {}px", 6 + row.depth * 16);
    let chevron = match open {
        Openness::Closed => "-rotate-90 transition-transform",
        _ => "transition-transform",
    };
    let toggle_tip = if open == Openness::Closed {
        t!("calls.open-row")
    } else {
        t!("calls.close-row")
    };
    view! {
        <div
            class="flex h-[24px] cursor-pointer items-center gap-1.5 pr-3 hover:bg-sunken"
            style=indent
            title=tip
            on:click=move |_| controller::go_to_call(state, id)
            on:dblclick=move |_| controller::go_to_called(state, id)
        >
            <button
                type="button"
                title=toggle_tip
                class="flex h-5 w-5 flex-none items-center justify-center rounded text-label-3 hover:text-label"
                on:click=move |event| {
                    event.stop_propagation();
                    match open {
                        Openness::Closed => controller::open_call(state, id),
                        Openness::Open => controller::close_call(state, id),
                        Openness::Asking => {}
                    }
                }
                on:dblclick=move |event| event.stop_propagation()
            >
                {if open == Openness::Asking {
                    view! { <span class="text-caption">"…"</span> }.into_any()
                } else {
                    view! {
                        <span class=chevron>
                            <IconView icon=Icon::Chevron size=11 />
                        </span>
                    }
                        .into_any()
                }}
            </button>
            <span class="flex-none font-mono text-footnote text-label">{row.call.item.name.clone()}</span>
            {recursive
                .then(|| {
                    view! {
                        <span class="flex-none rounded bg-sunken px-1 text-caption text-label-3">
                            {t!("calls.recursive")}
                        </span>
                    }
                })}
            <span class="min-w-0 flex-1 truncate font-mono text-caption text-label-3">
                {row.call.item.detail.clone().unwrap_or_default()}
            </span>
            {(count > 1)
                .then(|| {
                    view! {
                        <button
                            type="button"
                            title=t!("calls.sites", count = count)
                            class="flex-none rounded px-1 text-caption text-label-2 tnum hover:bg-content hover:text-label"
                            on:click=move |event| {
                                event.stop_propagation();
                                controller::list_call_sites(state, id);
                            }
                            on:dblclick=move |event| event.stop_propagation()
                        >
                            {format!("×{count}")}
                        </button>
                    }
                })}
            <span class="flex-none font-mono text-caption text-label-3" title=whole>{place}</span>
        </div>
    }
    .into_any()
}
