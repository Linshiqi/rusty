//! The repository's history, the way Fork draws it.
//!
//! A branch strip, then the log as a graph beside the commits, then the
//! commit opened below: its message, the files it touched, and one file's
//! patch. Reading, mostly. The one thing it changes is which branch is
//! checked out, and that runs as a visible command in the dock like every
//! other change rusty makes to a checkout.
//!
//! **The graph is drawn, not computed, here.** Lanes and edges arrive from
//! the backend already laid out (`rusty_git::graph`, pure and tested), so
//! this file only turns a lane index into an x coordinate. One row is one
//! small SVG whose lines run past its own bottom edge into the row below —
//! `overflow: visible` — because each row knows only where its lines go,
//! and the row beneath draws none of the line arriving at it.
//!
//! The lane colours are fixed hex, like the board sheet's: a commit graph is
//! the same colours in every editor that draws one, and a lane that changed
//! colour with the theme would look like a different branch.

use leptos::prelude::*;

use rusty_git::{ChangeKind, GraphRow, RefKind};

use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::{
    controller, format,
    state::AppState,
    view::components::{Button, Empty, Pill, Tone, register_toolbar},
};

/// Fork's palette, near enough: distinct at 2px, legible on both themes.
const LANE_COLOURS: [&str; 8] = [
    "#4f8ef7", "#e0a33b", "#3fbf7f", "#d56fd0", "#f0715b", "#5cc8d6", "#a98cff", "#c7b34a",
];
const ROW_PX: u32 = 26;
const LANE_PX: u32 = 14;

#[component]
pub fn GitPanel() -> impl IntoView {
    let state = AppState::expect();

    let toolbar = Callback::new(move |_| {
        view! {
            <button
                type="button"
                title=t!("git.refresh")
                on:click=move |_| {
                    controller::load_history(state);
                    controller::load_branches(state);
                }
                class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label"
            >
                <IconView icon=Icon::Refresh size=15 />
            </button>
        }
        .into_any()
    });
    register_toolbar(state, toolbar);

    // Load for the project that is open, and again when a different one is:
    // keyed on the root, so a keystroke elsewhere re-renders nothing here and
    // a project switch drops the last project's selection with its history.
    Effect::new(move |previous: Option<Option<String>>| {
        let root = state
            .project
            .detected
            .with(|project| project.as_ref().map(|p| p.root.clone()));
        if previous.as_ref() != Some(&root) {
            state.git.selected.set(None);
            state.git.detail.set(None);
            state.git.file.set(None);
            state.git.rev.set(None);
            if root.is_some() {
                controller::load_history(state);
                controller::load_branches(state);
            }
        }
        root
    });

    move || {
        if !state.has_project() {
            return view! {
                <Empty title=t!("git.no-project-title") detail=t!("git.no-project-detail") />
            }
            .into_any();
        }
        if let Some(why) = state.git.unavailable.get() {
            return view! { <Empty title=t!("git.unavailable-title") detail=why /> }.into_any();
        }
        view! {
            <div class="flex min-h-0 flex-1 flex-col">
                <Branches />
                <Log />
                <Detail />
            </div>
        }
        .into_any()
    }
}

/// Every branch as a chip. The current one is marked; the selected one
/// filters the log; a selected local branch that is not current offers
/// checkout.
#[component]
fn Branches() -> impl IntoView {
    let state = AppState::expect();
    view! {
        <div class="flex flex-wrap items-center gap-1.5 border-b border-line px-3 py-2">
            {move || {
                let rev = state.git.rev.get();
                let all_on = rev.is_none();
                view! {
                    <button
                        type="button"
                        on:click=move |_| controller::show_rev(state, None)
                        class=move || chip_class(all_on, false)
                    >
                        {t!("git.all")}
                    </button>
                }
            }}
            {move || {
                let rev = state.git.rev.get();
                state
                    .git.branches
                    .get()
                    .into_iter()
                    .map(|branch| {
                        let selected = rev.as_deref() == Some(branch.name.as_str());
                        let name = branch.name.clone();
                        let pick = name.clone();
                        let title = if branch.current {
                            t!("git.current")
                        } else {
                            branch.tip.clone()
                        };
                        view! {
                            <button
                                type="button"
                                title=title
                                on:click=move |_| controller::show_rev(state, Some(pick.clone()))
                                class=move || chip_class(selected, branch.remote)
                            >
                                {branch.current.then(|| view! { <span class="mr-1 text-patina">"●"</span> })}
                                {name}
                            </button>
                        }
                    })
                    .collect_view()
            }}
            {move || {
                // Checkout, for the selected branch when it is local and not
                // already checked out. A remote branch is not checked out by
                // name — that would detach HEAD — and the current one is.
                let rev = state.git.rev.get()?;
                let branch = state
                    .git.branches
                    .with(|list| list.iter().find(|b| b.name == rev).cloned())?;
                (!branch.current && !branch.remote).then(|| {
                    let name = branch.name.clone();
                    view! {
                        <Button
                            label=t!("git.checkout")
                            on_click=Callback::new(move |_| controller::checkout(state, name.clone()))
                        />
                    }
                })
            }}
        </div>
    }
}

fn chip_class(selected: bool, remote: bool) -> String {
    let base = "rounded-full px-2 py-0.5 font-mono text-footnote transition-colors";
    let ink = if remote {
        "text-label-4"
    } else {
        "text-label-2"
    };
    if selected {
        format!("{base} {ink} bg-sunken ring-1 ring-rust")
    } else {
        format!("{base} {ink} ring-1 ring-line hover:bg-sunken hover:text-label")
    }
}

/// The log: graph, decorations, subject, author, when.
#[component]
fn Log() -> impl IntoView {
    let state = AppState::expect();
    view! {
        <div class="min-h-0 flex-1 overflow-y-auto">
            {move || {
                let Some(history) = state.git.history.get() else {
                    return view! {
                        <p class="px-4 py-3 text-callout text-label-3">{t!("git.loading")}</p>
                    }
                    .into_any();
                };
                let lanes = history.lanes;
                let selected = state.git.selected.get();
                view! {
                    <div class="flex flex-col">
                        {history
                            .rows
                            .into_iter()
                            .map(|row| {
                                let picked = selected.as_deref() == Some(row.commit.id.as_str());
                                view! { <Row row=row lanes=lanes picked=picked /> }
                            })
                            .collect_view()}
                        {history
                            .truncated
                            .then(|| {
                                view! {
                                    <p class="px-4 py-2 text-footnote text-label-4">
                                        {t!("git.truncated", count = rusty_git::LIMIT)}
                                    </p>
                                }
                            })}
                    </div>
                }
                .into_any()
            }}
        </div>
    }
}

#[component]
fn Row(row: GraphRow, lanes: u32, picked: bool) -> impl IntoView {
    let state = AppState::expect();
    let id = row.commit.id.clone();
    let class = if picked {
        "flex cursor-pointer items-center gap-2 bg-selection pr-3"
    } else {
        "flex cursor-pointer items-center gap-2 pr-3 hover:bg-sunken"
    };
    let when = format::since(row.commit.time);
    let author = row.commit.author.clone();
    let summary = row.commit.summary.clone();
    let refs = row.commit.refs.clone();
    let cell = graph_cell(&row, lanes);
    view! {
        <div
            class=class
            style=format!("height: {ROW_PX}px")
            on:click=move |_| controller::select_commit(state, id.clone())
        >
            {cell}
            <div class="flex min-w-0 flex-1 items-center gap-1.5">
                {refs
                    .into_iter()
                    .map(|label| {
                        let (tone, text) = match label.kind {
                            RefKind::Head if label.name.is_empty() => (Tone::Rust, t!("git.head")),
                            RefKind::Head => (Tone::Rust, label.name),
                            RefKind::Branch => (Tone::Patina, label.name),
                            RefKind::Remote => (Tone::Slate, label.name),
                            RefKind::Tag => (Tone::Amber, label.name),
                        };
                        // As spelled: a branch or tag is an identifier the
                        // user types, and `MASTER` names nothing.
                        view! { <Pill label=text tone=tone uppercase=false /> }
                    })
                    .collect_view()}
                <span class="min-w-0 flex-1 truncate text-body">{summary}</span>
            </div>
            <span class="w-[9rem] shrink-0 truncate text-footnote text-label-3">{author}</span>
            <span class="w-[5.5rem] shrink-0 text-right text-footnote text-label-4 tnum">{when}</span>
        </div>
    }
}

/// One row's slice of the graph. Lines run from this row's centre to the
/// next row's centre, so they spill past the bottom edge on purpose.
fn graph_cell(row: &GraphRow, lanes: u32) -> AnyView {
    let width = lanes.max(1) * LANE_PX;
    let x = |lane: u32| f64::from(lane * LANE_PX + LANE_PX / 2);
    let mid = f64::from(ROW_PX) / 2.0;
    let colour = |lane: u32| LANE_COLOURS[(lane as usize) % LANE_COLOURS.len()];
    let lines: Vec<_> = row
        .edges
        .iter()
        .map(|edge| {
            // A line takes the colour of the lane it arrives in, so a branch
            // leaving a merge is coloured as the branch it becomes.
            let stroke = colour(edge.to);
            view! {
                <line
                    x1=x(edge.from)
                    y1=mid
                    x2=x(edge.to)
                    y2=mid + f64::from(ROW_PX)
                    stroke=stroke
                    stroke-width="2"
                    stroke-linecap="round"
                />
            }
        })
        .collect();
    let dot = colour(row.lane);
    view! {
        <svg
            class="shrink-0 overflow-visible"
            width=width
            height=ROW_PX
            viewBox=format!("0 0 {width} {ROW_PX}")
            aria-hidden="true"
        >
            {lines}
            <circle cx=x(row.lane) cy=mid r="3.5" fill=dot />
        </svg>
    }
    .into_any()
}

/// The opened commit: message, files, one file's patch.
#[component]
fn Detail() -> impl IntoView {
    let state = AppState::expect();
    move || {
        let selected = state.git.selected.get()?;
        let Some(detail) = state.git.detail.get() else {
            return Some(
                view! {
                    <div class="border-t border-line px-4 py-2 text-footnote text-label-4">
                        <span class="font-mono">{selected.chars().take(7).collect::<String>()}</span>
                    </div>
                }
                .into_any(),
            );
        };
        let chosen = state.git.file.get();
        let commit = detail.commit.clone();
        let when = format::since(commit.time);
        let files = detail.files.clone();
        let patch = chosen
            .as_deref()
            .and_then(|path| files.iter().find(|f| f.path == path))
            .map(|f| f.patch.clone())
            .unwrap_or_default();
        Some(
            view! {
                // Half the panel at most, and *everything* inside it bounded
                // and scrolling on its own. A commit message is a paragraph
                // or two — or, as this project's are, an essay — and a header
                // that took the message's natural height pushed the pane past
                // its cap and painted straight over the dock below.
                <div class="flex h-[48%] min-h-0 shrink-0 flex-col overflow-hidden border-t border-line">
                    <div class="max-h-[45%] shrink-0 overflow-y-auto border-b border-line px-4 py-2">
                        <div class="flex items-center gap-2 text-footnote text-label-3">
                            <span class="font-mono text-label-2 select-text">{commit.short.clone()}</span>
                            <span>{commit.author.clone()}</span>
                            <span class="text-label-4">{when}</span>
                        </div>
                        <p class="mt-1 text-body whitespace-pre-wrap select-text">{detail.body.clone()}</p>
                    </div>
                    <div class="flex min-h-0 flex-1">
                        <div class="w-[38%] shrink-0 overflow-y-auto border-r border-line py-1">
                            {if files.is_empty() {
                                view! {
                                    <p class="px-3 py-1 text-footnote text-label-4">{t!("git.no-files")}</p>
                                }
                                    .into_any()
                            } else {
                                files
                                    .iter()
                                    .map(|file| {
                                        let path = file.path.clone();
                                        let pick = path.clone();
                                        let on = chosen.as_deref() == Some(path.as_str());
                                        let (glyph, ink) = match file.kind {
                                            ChangeKind::Added => ("A", "text-patina"),
                                            ChangeKind::Modified => ("M", "text-amber"),
                                            ChangeKind::Deleted => ("D", "text-crimson"),
                                            ChangeKind::Renamed => ("R", "text-slate"),
                                            ChangeKind::Other => ("·", "text-label-4"),
                                        };
                                        let counts = match (file.added, file.removed) {
                                            (Some(a), Some(r)) => format!("+{a} −{r}"),
                                            _ => t!("git.binary"),
                                        };
                                        let class = if on {
                                            "flex w-full items-center gap-2 bg-selection px-3 py-0.5 text-left font-mono text-footnote"
                                        } else {
                                            "flex w-full items-center gap-2 px-3 py-0.5 text-left font-mono text-footnote hover:bg-sunken"
                                        };
                                        view! {
                                            <button
                                                type="button"
                                                class=class
                                                on:click=move |_| state.git.file.set(Some(pick.clone()))
                                            >
                                                <span class=format!("w-3 shrink-0 {ink}")>{glyph}</span>
                                                <span class="min-w-0 flex-1 truncate text-label-2">{path}</span>
                                                <span class="shrink-0 text-label-4 tnum">{counts}</span>
                                            </button>
                                        }
                                    })
                                    .collect_view()
                                    .into_any()
                            }}
                        </div>
                        <pre class="m-0 min-w-0 flex-1 overflow-auto px-3 py-1 font-mono text-footnote leading-relaxed whitespace-pre select-text">
                            {patch
                                .lines()
                                .map(|line| {
                                    let ink = if line.starts_with("+++") || line.starts_with("---") {
                                        "text-label-4"
                                    } else if line.starts_with('+') {
                                        "text-patina"
                                    } else if line.starts_with('-') {
                                        "text-crimson"
                                    } else if line.starts_with("@@") {
                                        "text-slate"
                                    } else if line.starts_with("diff ") || line.starts_with("index ") {
                                        "text-label-4"
                                    } else {
                                        "text-label-2"
                                    };
                                    view! { <span class=ink>{format!("{line}\n")}</span> }
                                })
                                .collect_view()}
                        </pre>
                    </div>
                </div>
            }
            .into_any(),
        )
    }
}
