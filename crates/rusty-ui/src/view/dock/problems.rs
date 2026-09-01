//! Everything that would stop this project building, in one list.
//!
//! The project's own cross-checks and the language server's diagnostics
//! together — they are the same question asked of different files, and a
//! reader who has to look in two places to answer it will look in one.

use leptos::prelude::*;

use rusty_lsp::DiagSeverity;

use super::*;
use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, Dot, MenuItem, ProblemRow, Tone, copy_to_clipboard},
};

#[component]
pub(super) fn ProblemsTab() -> impl IntoView {
    let state = AppState::expect();
    let menu = RwSignal::new(None::<DiagMenuAt>);

    let rows = move || {
        let problems = state.problems();
        // Compiler diagnostics, flattened out of the per-file map. They join
        // the config problems here because "why does my project not build" has
        // one answer set, not two panels' worth.
        let mut diags: Vec<(String, rusty_lsp::FileDiagnostic)> =
            state.lsp.diagnostics.with(|by_file| {
                by_file
                    .iter()
                    .flat_map(|(path, items)| items.iter().map(|d| (path.clone(), d.clone())))
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
