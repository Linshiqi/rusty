//! The repository, the way Fork shows it.
//!
//! Three views behind one branch strip. **History**: the log as a graph beside
//! the commits, labels on the commits that carry branches and tags, a commit
//! opened below with its files and each file's patch. **Changes**: the working
//! tree in two lists — what the next commit would carry and what it would not
//! — each file's diff, and the commit box. **Stashes**: what has been put
//! aside, with the three things one can do to a stash. The rail carries
//! fetch, pull, push and a new branch.
//!
//! Every write is a dock command (see `controller::git`), so the exact `git`
//! line and its answer are readable. Only staging is quiet.
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

use leptos::{ev, prelude::*};

use rusty_git::{ChangeKind, GraphRow, RefKind, StatusEntry};

use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::{
    controller, format,
    state::{AppState, GitMode},
    view::components::{Button, ButtonKind, Empty, Pill, Tone, register_toolbar},
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
        let push_title = move || {
            let has_upstream = state
                .git
                .status
                .with(|s| s.as_ref().is_some_and(|s| s.upstream.is_some()));
            if has_upstream {
                t!("git.push")
            } else {
                t!("git.push-upstream")
            }
        };
        view! {
            {rail_button(t!("git.refresh"), Icon::Refresh, move |_| controller::load_git(state))}
            {rail_button(t!("git.fetch"), Icon::Fetch, move |_| controller::fetch(state))}
            {rail_button(t!("git.pull"), Icon::Pull, move |_| controller::pull(state))}
            <button
                type="button"
                title=push_title
                on:click=move |_| controller::push(state)
                class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label"
            >
                <IconView icon=Icon::Push size=15 />
            </button>
            {rail_button(t!("git.new-branch"), Icon::Plus, move |_| {
                state.git.new_branch.set(Some(String::new()));
            })}
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
            state.git.diff.set(None);
            state.git.diff_for.set(None);
            state.git.new_branch.set(None);
            if root.is_some() {
                controller::load_git(state);
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
                <Modes />
                {move || match state.git.mode.get() {
                    GitMode::History => view! { <Log /> <Detail /> }.into_any(),
                    GitMode::Changes => view! { <Changes /> }.into_any(),
                    GitMode::Stashes => view! { <Stashes /> }.into_any(),
                }}
            </div>
        }
        .into_any()
    }
}

/// One rail action, in the shape every panel's are.
fn rail_button(title: String, icon: Icon, on_click: impl Fn(ev::MouseEvent) + 'static) -> AnyView {
    view! {
        <button
            type="button"
            title=title
            on:click=on_click
            class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label"
        >
            <IconView icon=icon size=15 />
        </button>
    }
    .into_any()
}

/// Every branch as a chip. The current one is marked; the selected one
/// filters the log; a selected local branch that is not current offers
/// checkout and delete; the plus in the rail opens a field for a new one.
#[component]
fn Branches() -> impl IntoView {
    let state = AppState::expect();
    view! {
        <div class="flex flex-wrap items-center gap-1.5 border-b border-line px-3 py-2">
            {move || {
                let all_on = state.git.rev.get().is_none();
                view! {
                    <button
                        type="button"
                        on:click=move |_| controller::show_rev(state, None)
                        class=chip_class(all_on, false)
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
                        let class = chip_class(selected, branch.remote);
                        let current = branch.current;
                        view! {
                            <button
                                type="button"
                                title=title
                                on:click=move |_| controller::show_rev(state, Some(pick.clone()))
                                class=class
                            >
                                {current.then(|| view! { <span class="mr-1 text-patina">"●"</span> })}
                                {name}
                            </button>
                        }
                    })
                    .collect_view()
            }}
            {move || {
                // Checkout and delete, for the selected branch when it is local
                // and not already checked out. A remote branch is not checked
                // out by name — that would detach HEAD — and the current one
                // can be neither.
                let rev = state.git.rev.get()?;
                let branch = state
                    .git.branches
                    .with(|list| list.iter().find(|b| b.name == rev).cloned())?;
                (!branch.current && !branch.remote).then(|| {
                    let checkout = branch.name.clone();
                    let delete = branch.name.clone();
                    view! {
                        <Button
                            label=t!("git.checkout")
                            on_click=Callback::new(move |_| controller::checkout(state, checkout.clone()))
                        />
                        <Button
                            label=t!("git.delete-branch")
                            on_click=Callback::new(move |_| controller::branch_delete(state, delete.clone()))
                        />
                    }
                })
            }}
            {move || {
                let draft = state.git.new_branch.get()?;
                Some(view! {
                    <input
                        type="text"
                        autofocus
                        placeholder=t!("git.new-branch-placeholder")
                        prop:value=draft
                        class="h-[26px] w-[26rem] max-w-full rounded-full bg-sunken px-3 font-mono text-footnote outline-none ring-1 ring-rust placeholder:text-label-3"
                        on:input=move |event| {
                            state.git.new_branch.set(Some(event_target_value(&event)));
                        }
                        on:keydown=move |event: ev::KeyboardEvent| {
                            if event.key() == "Enter" {
                                let name = state.git.new_branch.get_untracked().unwrap_or_default();
                                controller::branch_create(state, name, state.git.rev.get_untracked());
                            } else if event.key() == "Escape" {
                                state.git.new_branch.set(None);
                            }
                        }
                    />
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

/// History, Changes, Stashes — with the counts that say whether the second
/// two are worth a look.
#[component]
fn Modes() -> impl IntoView {
    let state = AppState::expect();
    let tab = move |mode: GitMode, label: Signal<String>| {
        let on = Signal::derive(move || state.git.mode.get() == mode);
        view! {
            <button
                type="button"
                on:click=move |_| state.git.mode.set(mode)
                class=move || {
                    if on.get() {
                        "border-b-2 border-rust px-3 py-1.5 text-callout text-label"
                    } else {
                        "border-b-2 border-transparent px-3 py-1.5 text-callout text-label-3 hover:text-label"
                    }
                }
            >
                {move || label.get()}
            </button>
        }
    };
    let changes = Signal::derive(move || {
        let count = state
            .git
            .status
            .with(|s| s.as_ref().map(|s| s.entries.len()).unwrap_or(0));
        if count == 0 {
            t!("git.changes")
        } else {
            format!("{} {count}", t!("git.changes"))
        }
    });
    let stashes = Signal::derive(move || {
        let count = state.git.stashes.with(Vec::len);
        if count == 0 {
            t!("git.stashes-tab")
        } else {
            format!("{} {count}", t!("git.stashes-tab"))
        }
    });
    view! {
        <div class="flex items-center border-b border-line px-1">
            {tab(GitMode::History, Signal::derive(|| t!("git.history")))}
            {tab(GitMode::Changes, changes)}
            {tab(GitMode::Stashes, stashes)}
        </div>
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

/// A unified diff, coloured by line.
fn diff_view(text: &str) -> AnyView {
    view! {
        <pre class="m-0 min-w-0 flex-1 overflow-auto px-3 py-1 font-mono text-footnote leading-relaxed whitespace-pre select-text">
            {text
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
    }
    .into_any()
}

/// The letter and colour a change wears in a file list.
fn change_glyph(
    kind: Option<ChangeKind>,
    untracked: bool,
    conflicted: bool,
) -> (&'static str, &'static str) {
    if conflicted {
        return ("!", "text-crimson");
    }
    if untracked {
        return ("?", "text-label-3");
    }
    match kind {
        Some(ChangeKind::Added) => ("A", "text-patina"),
        Some(ChangeKind::Modified) => ("M", "text-amber"),
        Some(ChangeKind::Deleted) => ("D", "text-crimson"),
        Some(ChangeKind::Renamed) => ("R", "text-slate"),
        Some(ChangeKind::Other) | None => ("·", "text-label-4"),
    }
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
                                        let (glyph, ink) = change_glyph(Some(file.kind), false, false);
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
                        {diff_view(&patch)}
                    </div>
                </div>
            }
            .into_any(),
        )
    }
}

/// The working tree: staged and not, one file's diff, and the commit box.
#[component]
fn Changes() -> impl IntoView {
    let state = AppState::expect();
    // Two columns, as Fork and VS Code both lay it out: the files and the
    // commit box on the left, the diff on the right at full height. Stacked,
    // the diff and the commit box took the height between them and the file
    // lists — the thing this view is for — were left three rows tall in a
    // panel two thousand pixels wide.
    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            {move || {
                let Some(status) = state.git.status.get() else {
                    return view! {
                        <p class="px-4 py-3 text-callout text-label-3">{t!("git.loading")}</p>
                    }
                    .into_any();
                };
                let head = if status.detached {
                    t!("git.detached")
                } else {
                    status.head.clone().unwrap_or_default()
                };
                let tracking = match &status.upstream {
                    Some(upstream) => t!(
                        "git.ahead-behind",
                        ahead = status.ahead,
                        behind = status.behind,
                        upstream = upstream.clone()
                    ),
                    None => t!("git.no-upstream"),
                };
                view! {
                    <div class="flex items-center gap-2 border-b border-line px-4 py-1.5 text-footnote text-label-3">
                        <span class="font-mono text-label-2">{head}</span>
                        <span class="text-label-4">{tracking}</span>
                    </div>
                }
                .into_any()
            }}
            <div class="flex min-h-0 flex-1">
                <div class="flex min-h-0 w-[38%] max-w-[36rem] min-w-[18rem] shrink-0 flex-col border-r border-line">
                    <div class="min-h-0 flex-1 overflow-y-auto py-1">
                        {move || {
                            let Some(status) = state.git.status.get() else {
                                return ().into_any();
                            };
                            if status.entries.is_empty() {
                                return view! {
                                    <p class="px-4 py-3 text-callout text-label-3">{t!("git.clean")}</p>
                                }
                                .into_any();
                            }
                            let staged: Vec<StatusEntry> = status
                                .entries
                                .iter()
                                .filter(|e| e.staged.is_some())
                                .cloned()
                                .collect();
                            let unstaged: Vec<StatusEntry> = status
                                .entries
                                .iter()
                                .filter(|e| e.unstaged.is_some())
                                .cloned()
                                .collect();
                            view! {
                                {change_list(state, t!("git.staged"), staged, true)}
                                {change_list(state, t!("git.unstaged"), unstaged, false)}
                            }
                            .into_any()
                        }}
                    </div>
                    <CommitBox />
                </div>
                <div class="flex min-h-0 min-w-0 flex-1">
                    {move || match state.git.diff.get() {
                        Some(text) => diff_view(&text),
                        None => view! {
                            <p class="px-4 py-3 text-footnote text-label-4">{t!("git.pick-change")}</p>
                        }
                            .into_any(),
                    }}
                </div>
            </div>
        </div>
    }
}

/// One side of the working tree: a heading with an all-or-nothing action,
/// then a row per file with the other action beside it.
fn change_list(state: AppState, title: String, entries: Vec<StatusEntry>, staged: bool) -> AnyView {
    if entries.is_empty() {
        return ().into_any();
    }
    let all: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
    let count = entries.len();
    let all_label = if staged {
        t!("git.unstage-all")
    } else {
        t!("git.stage-all")
    };
    let row_title = if staged {
        t!("git.unstage")
    } else {
        t!("git.stage")
    };
    let chosen = state.git.diff_for.get();
    view! {
        <div class="flex items-center gap-2 px-4 pt-2 pb-1">
            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">{title}</span>
            <span class="text-caption text-label-4 tnum">{count}</span>
            <span class="flex-1" />
            <button
                type="button"
                class="text-footnote text-label-3 hover:text-label"
                on:click=move |_| controller::stage(state, all.clone(), !staged)
            >
                {all_label}
            </button>
        </div>
        {entries
            .into_iter()
            .map(|entry| {
                let kind = if staged { entry.staged } else { entry.unstaged };
                let (glyph, ink) = change_glyph(kind, entry.untracked && !staged, entry.conflicted);
                // The two glyphs that are not a letter get a word on hover.
                let hint = if entry.conflicted {
                    Some(t!("git.conflicted"))
                } else if entry.untracked && !staged {
                    Some(t!("git.untracked"))
                } else {
                    None
                };
                let path = entry.path.clone();
                let show = path.clone();
                let toggle = path.clone();
                let untracked = entry.untracked;
                let on = chosen.as_ref().is_some_and(|(p, s)| *p == path && *s == staged);
                let class = if on {
                    "group flex w-full items-center gap-2 bg-selection px-4 py-0.5 text-left font-mono text-footnote"
                } else {
                    "group flex w-full items-center gap-2 px-4 py-0.5 text-left font-mono text-footnote hover:bg-sunken"
                };
                let title = row_title.clone();
                view! {
                    <div class=class>
                        <span class=format!("w-3 shrink-0 {ink}") title=hint>{glyph}</span>
                        <button
                            type="button"
                            class="min-w-0 flex-1 truncate text-left text-label-2"
                            on:click=move |_| controller::load_diff(state, show.clone(), staged, untracked)
                        >
                            {path}
                        </button>
                        <button
                            type="button"
                            title=title
                            class="shrink-0 rounded-[4px] px-1.5 text-label-4 ring-1 ring-line hover:bg-raised hover:text-label"
                            on:click=move |_| controller::stage(state, vec![toggle.clone()], !staged)
                        >
                            {if staged { "−" } else { "+" }}
                        </button>
                    </div>
                }
            })
            .collect_view()}
    }
    .into_any()
}

/// The message and the button. Ctrl+Enter commits, as every git client's
/// message box does.
#[component]
fn CommitBox() -> impl IntoView {
    let state = AppState::expect();
    let staged_count = Signal::derive(move || {
        state.git.status.with(|s| {
            s.as_ref()
                .map(|s| s.entries.iter().filter(|e| e.staged.is_some()).count())
                .unwrap_or(0)
        })
    });
    let blocked = Signal::derive(move || {
        staged_count.get() == 0 || state.git.message.with(|m| m.trim().is_empty())
    });
    view! {
        <div class="shrink-0 border-t border-line px-4 py-3">
            <textarea
                rows="3"
                placeholder=t!("git.commit-placeholder")
                class="w-full resize-none rounded-[8px] bg-sunken px-3 py-2 text-body outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                prop:value=move || state.git.message.get()
                on:input=move |event| state.git.message.set(event_target_value(&event))
                on:keydown=move |event: ev::KeyboardEvent| {
                    if event.key() == "Enter" && event.ctrl_key() && !blocked.get_untracked() {
                        event.prevent_default();
                        controller::commit(state);
                    }
                }
            />
            <div class="mt-2 flex items-center gap-3">
                <Button
                    label=t!("git.commit")
                    kind=ButtonKind::Primary
                    disabled=blocked
                    on_click=Callback::new(move |_| controller::commit(state))
                />
                {move || {
                    (staged_count.get() == 0).then(|| {
                        view! { <span class="text-footnote text-label-4">{t!("git.nothing-staged")}</span> }
                    })
                }}
            </div>
        </div>
    }
}

/// What has been put aside, and the three things one can do to each.
#[component]
fn Stashes() -> impl IntoView {
    let state = AppState::expect();
    let dirty = Signal::derive(move || {
        state
            .git
            .status
            .with(|s| s.as_ref().is_some_and(|s| !s.entries.is_empty()))
    });
    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex items-center gap-2 border-b border-line px-4 py-2">
                <input
                    type="text"
                    placeholder=t!("git.stash-placeholder")
                    class="h-[28px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2.5 text-footnote outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                    prop:value=move || state.git.stash_note.get()
                    on:input=move |event| state.git.stash_note.set(event_target_value(&event))
                />
                <Button
                    label=t!("git.stash-save")
                    disabled=Signal::derive(move || !dirty.get())
                    on_click=Callback::new(move |_| controller::stash_save(state))
                />
            </div>
            <div class="min-h-0 flex-1 overflow-y-auto py-1">
                {move || {
                    let stashes = state.git.stashes.get();
                    if stashes.is_empty() {
                        return view! {
                            <p class="px-4 py-3 text-callout text-label-3">{t!("git.no-stashes")}</p>
                        }
                        .into_any();
                    }
                    stashes
                        .into_iter()
                        .map(|stash| {
                            let when = format::since(stash.time);
                            let (apply, pop, drop) = (stash.index, stash.index, stash.index);
                            view! {
                                <div class="flex items-center gap-3 px-4 py-1.5 hover:bg-sunken">
                                    <span class="shrink-0 font-mono text-footnote text-label-3">{stash.label}</span>
                                    <span class="min-w-0 flex-1 truncate text-body">{stash.message}</span>
                                    <span class="shrink-0 text-footnote text-label-4">{when}</span>
                                    <Button
                                        label=t!("git.apply")
                                        on_click=Callback::new(move |_| controller::stash_apply(state, apply))
                                    />
                                    <Button
                                        label=t!("git.pop")
                                        on_click=Callback::new(move |_| controller::stash_pop(state, pop))
                                    />
                                    <Button
                                        label=t!("git.drop")
                                        on_click=Callback::new(move |_| controller::stash_drop(state, drop))
                                    />
                                </div>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </div>
    }
}
