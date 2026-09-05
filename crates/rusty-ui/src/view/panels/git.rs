//! The repository, the way Fork shows it.
//!
//! Three views behind one branch strip. **History**: the log as a graph beside
//! the commits, labels on the commits that carry branches and tags, a commit
//! opened below with its files and each file's patch. **Changes**: the working
//! tree in two lists — what the next commit would carry and what it would not
//! — each file's diff, and the commit box. **Stashes**: what has been put
//! aside, with the three things one can do to a stash and, opened below, what
//! each one holds. The rail carries fetch, pull, push and a new branch; a
//! right-click on a commit or a file offers what is local to it.
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
//! **A diff is read once and laid out two ways.** `rusty_git::diff` turns
//! git's unified text into numbered rows — side by side, or one column in
//! git's own order — and this file only decides what a row looks like. The
//! toggle between the two is remembered.
//!
//! **Every boundary somebody would want to move, moves.** The log against
//! the opened commit, the message against the files, the files against the
//! patch, and the Changes view's two columns are `split::Handle`s over
//! `Divider`s, remembered like the sidebar's and the dock's.
//!
//! The lane colours are fixed hex, like the board sheet's: a commit graph is
//! the same colours in every editor that draws one, and a lane that changed
//! colour with the theme would look like a different branch.

use leptos::{ev, prelude::*};
use wasm_bindgen::JsCast;

use rusty_git::diff::{Cell, CellKind, Hunk};
use rusty_git::{Branch, ChangeKind, GraphRow, RefKind, StatusEntry};

use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::view::split;
use crate::{
    controller, format,
    state::{AppState, Divider, GitMenu, GitMode, GitTarget, ImageSide},
    view::components::{
        Button, ButtonKind, ContextMenu, Empty, MenuItem, MenuSeparator, Pill, Tone,
        copy_to_clipboard, register_toolbar,
    },
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
                state.git.branch_from.set(None);
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
            state.git.branch_from.set(None);
            state.git.amend.set(false);
            state.git.menu.set(None);
            if root.is_some() {
                controller::load_git(state);
            }
        }
        root
    });

    move || {
        if !state.has_project() {
            // No project is also how a project starts: the clone lives here
            // as well as in the File menu.
            return view! {
                <div class="flex min-h-0 flex-1 flex-col">
                    <Empty title=t!("git.no-project-title") detail=t!("git.no-project-detail") />
                    <div class="flex justify-center pb-6">
                        <Button
                            label=t!("git.clone-button")
                            on_click=Callback::new(move |_| controller::open_clone_dialog(state))
                        />
                    </div>
                </div>
            }
            .into_any();
        }
        if let Some(why) = state.git.unavailable.get() {
            return view! { <Empty title=t!("git.unavailable-title") detail=why /> }.into_any();
        }
        view! {
            // The browser's own menu is never the answer here: rows offer
            // theirs, and everywhere else a right-click does nothing.
            <div
                class="flex min-h-0 flex-1 flex-col"
                on:contextmenu=move |event: ev::MouseEvent| event.prevent_default()
            >
                <Branches />
                <Modes />
                {move || match state.git.mode.get() {
                    GitMode::History => view! { <Log /> <Detail /> }.into_any(),
                    GitMode::Changes => view! { <Changes /> }.into_any(),
                    GitMode::Stashes => view! { <Stashes /> <Detail /> }.into_any(),
                }}
            </div>
            <GitContextMenu />
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
    // Where the branch menu opens while it is open: just under the picker.
    let picker = RwSignal::new(None::<(f64, f64)>);
    // One row of the menu. Copy, because the local and the remote list both
    // map through it.
    let item = move |branch: Branch| {
        let name = branch.name.clone();
        let label = if branch.current {
            format!("● {}", branch.name)
        } else {
            branch.name.clone()
        };
        let tip: String = branch.tip.chars().take(7).collect();
        view! {
            <MenuItem
                label=label
                shortcut=tip
                on_select=Callback::new(move |_| {
                    controller::show_rev(state, Some(name.clone()));
                    picker.set(None);
                })
            />
        }
    };
    view! {
        <div class="flex flex-wrap items-center gap-1.5 border-b border-line px-3 py-2">
            // A picker, not a row of chips: a repository with thirty branches
            // is ordinary, and thirty chips are a paragraph nobody reads.
            <button
                type="button"
                title=t!("git.pick-branch")
                class="flex h-[26px] items-center gap-1.5 rounded-full bg-sunken px-2.5 font-mono text-footnote text-label ring-1 ring-line hover:ring-line-strong"
                on:click=move |event: ev::MouseEvent| {
                    let under = event
                        .current_target()
                        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                        .map(|el| {
                            let rect = el.get_bounding_client_rect();
                            (rect.left(), rect.bottom() + 4.0)
                        })
                        .unwrap_or((f64::from(event.client_x()), f64::from(event.client_y())));
                    picker.set(Some(under));
                }
            >
                <IconView icon=Icon::Branch size=13 />
                <span>{move || state.git.rev.get().unwrap_or_else(|| t!("git.all"))}</span>
                <IconView icon=Icon::Chevron size=11 />
            </button>
            {move || {
                // Where HEAD is, when the log is not filtered to it: the
                // picker says what is shown, this says what is checked out.
                let head = state
                    .git
                    .branches
                    .with(|list| list.iter().find(|b| b.current).map(|b| b.name.clone()))?;
                (state.git.rev.get().as_deref() != Some(head.as_str())).then(|| {
                    view! {
                        <span
                            class="flex items-center gap-1 font-mono text-footnote text-label-3"
                            title=t!("git.current")
                        >
                            <span class="text-patina">"●"</span>
                            {head}
                        </span>
                    }
                })
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
                // From the commit the field was opened on, else the branch
                // selected in the strip, else HEAD.
                let from = state.git.branch_from.get();
                let placeholder = match &from {
                    Some(id) => id.chars().take(7).collect::<String>(),
                    None => t!("git.new-branch-placeholder"),
                };
                Some(view! {
                    <input
                        type="text"
                        autofocus
                        placeholder=placeholder
                        prop:value=draft
                        class="h-[26px] w-[26rem] max-w-full rounded-full bg-sunken px-3 font-mono text-footnote outline-none ring-1 ring-rust placeholder:text-label-3"
                        on:input=move |event| {
                            state.git.new_branch.set(Some(event_target_value(&event)));
                        }
                        on:keydown=move |event: ev::KeyboardEvent| {
                            if event.key() == "Enter" {
                                let name = state.git.new_branch.get_untracked().unwrap_or_default();
                                let from = state
                                    .git
                                    .branch_from
                                    .get_untracked()
                                    .or_else(|| state.git.rev.get_untracked());
                                controller::branch_create(state, name, from);
                            } else if event.key() == "Escape" {
                                state.git.new_branch.set(None);
                                state.git.branch_from.set(None);
                            }
                        }
                    />
                })
            }}
        </div>
        {move || {
            let (x, y) = picker.get()?;
            let close = Callback::new(move |_| picker.set(None));
            let (local, remote): (Vec<Branch>, Vec<Branch>) = state
                .git
                .branches
                .get()
                .into_iter()
                .partition(|b| !b.remote);
            Some(view! {
                <ContextMenu x=x y=y on_close=close>
                    <div class="max-h-[60vh] min-w-[16rem] overflow-y-auto">
                        <MenuItem
                            label=t!("git.all")
                            on_select=Callback::new(move |_| {
                                controller::show_rev(state, None);
                                picker.set(None);
                            })
                        />
                        <MenuSeparator />
                        {local.into_iter().map(item).collect_view()}
                        {(!remote.is_empty()).then(|| view! { <MenuSeparator /> })}
                        {remote.into_iter().map(item).collect_view()}
                    </div>
                </ContextMenu>
            })
        }}
    }
}

/// History, Changes, Stashes — with the counts that say whether the second
/// two are worth a look. Switching drops the opened commit: History and
/// Stashes share the pane below, and a stash's files under the log — or a
/// commit's under the stashes — would be about something no longer listed.
#[component]
fn Modes() -> impl IntoView {
    let state = AppState::expect();
    let tab = move |mode: GitMode, label: Signal<String>| {
        let on = Signal::derive(move || state.git.mode.get() == mode);
        view! {
            <button
                type="button"
                on:click=move |_| {
                    if state.git.mode.get_untracked() != mode {
                        state.git.selected.set(None);
                        state.git.detail.set(None);
                        state.git.file.set(None);
                        state.git.mode.set(mode);
                    }
                }
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

/// The log: graph, decorations, subject, author, when — and, when there is
/// more, a way to ask for it.
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
                let shown = history.rows.len();
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
                                    <div class="flex items-center gap-3 px-4 py-2 text-footnote text-label-4">
                                        <span>{t!("git.truncated", count = shown)}</span>
                                        <button
                                            type="button"
                                            class="text-rust hover:underline"
                                            on:click=move |_| controller::show_more(state)
                                        >
                                            {t!("git.show-more")}
                                        </button>
                                    </div>
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
    let menu_id = row.commit.id.clone();
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
            on:contextmenu=move |event: ev::MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
                state.git.menu.set(Some(GitMenu {
                    x: event.client_x() as f64,
                    y: event.client_y() as f64,
                    target: GitTarget::Commit { id: menu_id.clone() },
                }));
            }
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
            // Above the row backgrounds: a row's lines run into the next row,
            // and that row's hover or selection fill painted over them, so
            // the graph looked cut at whichever row the pointer was on.
            class="relative z-10 shrink-0 overflow-visible"
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

// ─── diffs ───────────────────────────────────────────────────────────────────

/// One file's difference: a header naming the file and offering the two
/// layouts, then the rows. Reads the layout signal, so the closure that
/// calls this re-renders when the toggle is pressed.
fn diff_pane(state: AppState, path: String, text: &str) -> AnyView {
    // A picture is compared as pictures. The controller that picked the file
    // has already asked for both sides; this only draws what has arrived.
    if rusty_git::is_image_path(&path) {
        return image_pane(state, path);
    }
    let hunks = rusty_git::diff::hunks(text);
    let split = state.git.split.get();
    let body = if hunks.is_empty() {
        // Nothing to lay out — a binary file, or a file with no hunks. Whatever
        // git said is shown as it is, muted, rather than an empty pane.
        view! {
            <pre class="m-0 px-3 py-2 font-mono text-footnote text-label-4 whitespace-pre-wrap">
                {text.trim().to_string()}
            </pre>
        }
        .into_any()
    } else if split {
        split_rows(state, &hunks)
    } else {
        unified_rows(&hunks)
    };
    view! {
        <div class="flex min-h-0 min-w-0 flex-1 flex-col">
            <div class="flex shrink-0 items-center gap-1 border-b border-line px-3 py-1">
                <span class="min-w-0 flex-1 truncate font-mono text-footnote text-label-2 select-text">
                    {path}
                </span>
                {layout_button(state, false, Icon::Rows, t!("git.unified"))}
                {layout_button(state, true, Icon::Columns, t!("git.split"))}
            </div>
            <div class="min-h-0 min-w-0 flex-1 overflow-auto font-mono text-footnote leading-relaxed select-text">
                {body}
            </div>
        </div>
    }
    .into_any()
}

/// An image's two sides, before and after, each on a checkerboard so a
/// transparent PNG shows its edges. A side the file does not have — the
/// old of an added picture, the new of a deleted one — says so rather than
/// showing an empty box that reads as a broken load.
fn image_pane(state: AppState, path: String) -> AnyView {
    let pair = state.git.images.get().filter(|pair| pair.path == path);
    let (old, new) = match pair {
        Some(pair) => (pair.old, pair.new),
        None => (ImageSide::Loading, ImageSide::Loading),
    };
    view! {
        <div class="flex min-h-0 min-w-0 flex-1 flex-col">
            <div class="flex shrink-0 items-center gap-1 border-b border-line px-3 py-1">
                <span class="min-w-0 flex-1 truncate font-mono text-footnote text-label-2 select-text">
                    {path}
                </span>
            </div>
            <div class="grid min-h-0 flex-1 grid-cols-2 gap-px overflow-auto bg-line">
                {image_side(t!("git.image-old"), old)}
                {image_side(t!("git.image-new"), new)}
            </div>
        </div>
    }
    .into_any()
}

fn image_side(label: String, side: ImageSide) -> AnyView {
    let body = match side {
        ImageSide::Absent => view! {
            <p class="text-footnote text-label-4">{t!("git.image-none")}</p>
        }
        .into_any(),
        ImageSide::Loading => view! {
            <p class="text-footnote text-label-4">{t!("git.image-loading")}</p>
        }
        .into_any(),
        ImageSide::Failed(why) => view! { <p class="text-footnote text-crimson">{why}</p> }.into_any(),
        ImageSide::Ready { url, bytes } => view! {
            <img
                src=url
                alt=""
                class="max-h-full max-w-full object-contain"
                style="background: repeating-conic-gradient(rgba(127,127,127,.18) 0 25%, transparent 0 50%) 0 0 / 16px 16px"
            />
            <span class="text-caption text-label-4 tnum">{t!("git.image-size", bytes = bytes)}</span>
        }
        .into_any(),
    };
    view! {
        <div class="flex min-h-0 flex-col items-center gap-2 bg-content p-3">
            <span class="self-start text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                {label}
            </span>
            <div class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2">{body}</div>
        </div>
    }
    .into_any()
}

/// One of the two layout buttons; the one in force is filled.
fn layout_button(state: AppState, split: bool, icon: Icon, title: String) -> AnyView {
    let on = state.git.split.get() == split;
    let class = if on {
        "grid size-6 place-items-center rounded-[5px] bg-sunken text-label"
    } else {
        "grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label"
    };
    view! {
        <button
            type="button"
            title=title
            class=class
            on:click=move |_| controller::set_split(state, split)
        >
            <IconView icon=icon size=13 />
        </button>
    }
    .into_any()
}

/// Side by side: old on the left, new on the right, each with its numbers.
/// One grid for the whole file, so the gutters line up across hunks and a
/// row is one row on both sides however either wraps.
///
/// The text columns are `minmax(0, …fr)`, never a bare `1fr`: a `1fr` track
/// is at least as wide as its longest line, so one long README paragraph
/// grew the left half to the whole pane and pushed the right half past the
/// horizontal scrollbar — a side-by-side view that looked exactly like the
/// one-column view. Long lines wrap instead, as GitHub's split view does,
/// and because both sides share a grid row they stay aligned when they do.
///
/// Where old meets new is a divider (`Divider::GitSplit`), in permille of
/// the text width. The grip measures the grid on grab so a pixel of travel
/// becomes the right fraction, and `split::grab` hands the rest to the same
/// window listeners every other divider uses.
fn split_rows(state: AppState, hunks: &[Hunk]) -> AnyView {
    let grid = NodeRef::<leptos::html::Div>::new();
    let columns = move || {
        let left = state.layout.git_split.get();
        format!(
            "grid-template-columns: 3.5rem minmax(0, {left}fr) 3.5rem minmax(0, {}fr)",
            1000.0 - left
        )
    };
    let grip_left = move || {
        format!(
            "left: calc(3.5rem + (100% - 7rem) * {})",
            state.layout.git_split.get() / 1000.0
        )
    };
    let grip_class = move || {
        let base = "pointer-events-auto relative w-px cursor-col-resize transition-colors \
                    before:absolute before:-left-[3px] before:top-0 before:h-full \
                    before:w-[7px] before:content-['']";
        if state.layout.dragging.get() == Some(Divider::GitSplit) {
            format!("{base} bg-rust")
        } else {
            format!("{base} bg-line hover:bg-rust")
        }
    };
    let on_grab = move |event: ev::MouseEvent| {
        event.prevent_default();
        // Pixels into permille of the text: the grid less its two gutters,
        // the first number cell being one gutter's width.
        let per_px = grid
            .get_untracked()
            .map(|el| {
                let gutter = el
                    .query_selector("span")
                    .ok()
                    .flatten()
                    .map(|cell| f64::from(cell.client_width()))
                    .unwrap_or(56.0);
                let text = (f64::from(el.client_width()) - 2.0 * gutter).max(1.0);
                1000.0 / text
            })
            .unwrap_or(1.0);
        split::grab(
            state,
            Divider::GitSplit,
            f64::from(event.client_x()),
            per_px,
        );
    };
    view! {
        <div class="relative grid" style=columns node_ref=grid>
            {hunks
                .iter()
                .map(|hunk| {
                    // The hunk header once per side, as Fork draws it: one
                    // header across both would cross the line between old
                    // and new, and a row that crosses it reads as a layout
                    // that has come apart rather than as a heading.
                    let header = hunk.header.clone();
                    view! {
                        <div class="col-span-2 min-w-0 bg-sunken px-3 py-0.5 text-slate break-words whitespace-pre-wrap">
                            {header.clone()}
                        </div>
                        <div class="col-span-2 min-w-0 bg-sunken px-3 py-0.5 text-slate break-words whitespace-pre-wrap">
                            {header}
                        </div>
                        {hunk
                            .rows
                            .iter()
                            .map(|row| view! { {side(row.left.as_ref())} {side(row.right.as_ref())} })
                            .collect_view()}
                    }
                })
                .collect_view()}
            // Out of the grid's flow: an absolutely positioned child takes no
            // cell, so the line can run the full height at the split.
            <div class="pointer-events-none absolute inset-y-0 z-10 flex" style=grip_left>
                <div
                    role="separator"
                    aria-orientation="vertical"
                    class=grip_class
                    on:mousedown=on_grab
                />
            </div>
        </div>
    }
    .into_any()
}

/// One side of a two-column row: its number and its text — or two blank
/// cells where the other side has a line with nothing to face.
fn side(cell: Option<&Cell>) -> AnyView {
    let Some(cell) = cell else {
        return view! {
            <span class="bg-sunken" />
            <span class="bg-sunken" />
        }
        .into_any();
    };
    let tint = tint(cell.kind);
    let number = cell.number.map(|n| n.to_string()).unwrap_or_default();
    view! {
        <span class=format!("px-2 text-right text-label-4 tnum select-none {tint}")>{number}</span>
        <span class=format!("min-w-0 px-2 break-words whitespace-pre-wrap {tint} {}", ink(cell.kind))>
            {shown(&cell.text)}
        </span>
    }
    .into_any()
}

/// One column, in git's own order, with the old and new numbers beside it.
fn unified_rows(hunks: &[Hunk]) -> AnyView {
    view! {
        <div class="grid grid-cols-[3.5rem_3.5rem_minmax(0,1fr)]">
            {hunks
                .iter()
                .map(|hunk| {
                    view! {
                        <div class="col-span-3 bg-sunken px-3 py-0.5 text-slate">{hunk.header.clone()}</div>
                        {hunk
                            .lines
                            .iter()
                            .map(|line| {
                                let tint = tint(line.kind);
                                let sign = match line.kind {
                                    CellKind::Added => "+",
                                    CellKind::Removed => "-",
                                    CellKind::Context | CellKind::Note => " ",
                                };
                                let old = line.old.map(|n| n.to_string()).unwrap_or_default();
                                let new = line.new.map(|n| n.to_string()).unwrap_or_default();
                                view! {
                                    <span class=format!("px-2 text-right text-label-4 tnum select-none {tint}")>{old}</span>
                                    <span class=format!("px-2 text-right text-label-4 tnum select-none {tint}")>{new}</span>
                                    <span class=format!("min-w-0 px-2 break-words whitespace-pre-wrap {tint} {}", ink(line.kind))>
                                        {format!("{sign} {}", shown(&line.text))}
                                    </span>
                                }
                            })
                            .collect_view()}
                    }
                })
                .collect_view()}
        </div>
    }
    .into_any()
}

/// The theme's own tinted fills, so a changed line reads the same way a
/// changed badge does.
fn tint(kind: CellKind) -> &'static str {
    match kind {
        CellKind::Added => "bg-patina-fill",
        CellKind::Removed => "bg-crimson-fill",
        CellKind::Context | CellKind::Note => "",
    }
}

fn ink(kind: CellKind) -> &'static str {
    match kind {
        CellKind::Note => "text-label-4 italic",
        CellKind::Added | CellKind::Removed => "text-label",
        CellKind::Context => "text-label-2",
    }
}

/// An empty line still has a height.
fn shown(text: &str) -> String {
    if text.is_empty() {
        " ".to_string()
    } else {
        text.to_string()
    }
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

/// Open a right-click menu about a path — a file in a commit or in the
/// working tree.
fn path_menu(state: AppState, event: &ev::MouseEvent, path: &str) {
    event.prevent_default();
    event.stop_propagation();
    state.git.menu.set(Some(GitMenu {
        x: event.client_x() as f64,
        y: event.client_y() as f64,
        target: GitTarget::Path {
            path: path.to_string(),
        },
    }));
}

// ─── the opened commit ───────────────────────────────────────────────────────

/// The opened commit (or stash): message, files, one file's patch — under
/// the log or the stash list, behind a divider, with two more inside.
#[component]
fn Detail(#[prop(default = false)] standalone: bool) -> impl IntoView {
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
        let commit = detail.commit.clone();
        // Folded to a strip: the hash and the summary, and the way back.
        // Fork's hide, so a wide graph can have the whole panel.
        if !standalone && state.git.detail_hidden.get() {
            let summary = commit.summary.clone();
            return Some(
                view! {
                    <div class="flex shrink-0 items-center gap-3 border-t border-line bg-sunken px-4 py-1.5">
                        <span class="font-mono text-footnote text-label-2">{commit.short.clone()}</span>
                        <span class="min-w-0 flex-1 truncate text-footnote text-label-3">{summary}</span>
                        <button
                            type="button"
                            title=t!("git.detail-show")
                            class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-raised hover:text-label"
                            on:click=move |_| controller::toggle_detail(state)
                        >
                            <span class="-rotate-180"><IconView icon=Icon::Chevron size=13 /></span>
                        </button>
                    </div>
                }
                .into_any(),
            );
        }
        let chosen = state.git.file.get();
        let when = format::since(commit.time);
        let files = detail.files.clone();
        let patch = chosen
            .as_deref()
            .and_then(|path| files.iter().find(|f| f.path == path))
            .map(|f| f.patch.clone())
            .unwrap_or_default();
        let diff = match chosen.clone() {
            Some(path) => diff_pane(state, path, &patch),
            None => view! {
                <p class="px-4 py-3 text-footnote text-label-4">{t!("git.pick-change")}</p>
            }
            .into_any(),
        };
        let target = selected.clone();
        // The pane's frame: in the panel, a divider above and a dragged
        // height capped at most of the panel; in a window of its own, the
        // whole window.
        let frame = if standalone {
            "flex min-h-0 flex-1 flex-col overflow-hidden"
        } else {
            "flex max-h-[80%] min-h-0 shrink-0 flex-col overflow-hidden"
        };
        let height = move || {
            if standalone {
                String::new()
            } else {
                format!("height: {}px", state.layout.git_detail_height.get())
            }
        };
        Some(
            view! {
                {(!standalone).then(|| view! { <split::Handle divider=Divider::GitDetail /> })}
                // Never more than most of the panel, whatever the divider was
                // dragged to, and *everything* inside it bounded and scrolling
                // on its own: a header that took an essay-length message's
                // natural height once pushed this pane straight over the dock.
                // The three regions wear three grounds — message, files, patch
                // — so the eye finds the boundaries without reading for them.
                <div class=frame style=height>
                    // The message block scrolls *itself*: it is the element
                    // the cap is on. A version that capped this box and put the
                    // scrolling on a child inside a flex row let the child take
                    // its content height — a flex item's cross size is the
                    // line's, not the clamped container's — and an essay-length
                    // message painted straight over the files and the patch
                    // below. The buttons sit in the first row, so they are at
                    // hand until the message is scrolled.
                    <div
                        class="shrink-0 overflow-y-auto bg-sunken px-4 py-2"
                        style=move || format!("max-height: {}px", state.layout.git_message_height.get())
                    >
                        <div class="flex items-center gap-2 text-footnote text-label-3">
                            <span class="font-mono text-label-2 select-text">{commit.short.clone()}</span>
                            <span>{commit.author.clone()}</span>
                            <span class="text-label-4">{when}</span>
                            <span class="flex-1" />
                            {(!standalone).then(|| view! {
                                <div class="flex shrink-0 items-center gap-0.5">
                                    <button
                                        type="button"
                                        title=t!("git.detail-window")
                                        class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-raised hover:text-label"
                                        on:click=move |_| controller::open_commit_window(state, target.clone())
                                    >
                                        <IconView icon=Icon::External size=13 />
                                    </button>
                                    <button
                                        type="button"
                                        title=t!("git.detail-hide")
                                        class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-raised hover:text-label"
                                        on:click=move |_| controller::toggle_detail(state)
                                    >
                                        <IconView icon=Icon::Close size=13 />
                                    </button>
                                </div>
                            })}
                        </div>
                        <p class="mt-1 text-body whitespace-pre-wrap select-text">{detail.body.clone()}</p>
                    </div>
                    <split::Handle divider=Divider::GitMessage />
                    <div class="flex min-h-0 flex-1">
                        <div
                            class="shrink-0 overflow-y-auto bg-sidebar py-1"
                            style=move || format!("width: {}px", state.layout.git_files_width.get())
                        >
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
                                        let open = path.clone();
                                        let menu = path.clone();
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
                                                on:click=move |_| controller::show_commit_file(state, pick.clone())
                                                on:dblclick=move |_| controller::open_file(state, open.clone())
                                                on:contextmenu=move |event: ev::MouseEvent| path_menu(state, &event, &menu)
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
                        <split::Handle divider=Divider::GitFiles />
                        {diff}
                    </div>
                </div>
            }
            .into_any(),
        )
    }
}

/// One commit filling a window of its own — what `?gitdiff=<target>` boots.
pub fn commit_window() -> AnyView {
    view! { <Detail standalone=true /> }.into_any()
}

// ─── the working tree ────────────────────────────────────────────────────────

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
                <div
                    class="flex min-h-0 shrink-0 flex-col bg-sidebar"
                    style=move || format!("width: {}px", state.layout.git_changes_width.get())
                >
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
                <split::Handle divider=Divider::GitChanges />
                <div class="flex min-h-0 min-w-0 flex-1">
                    {move || {
                        let path = state.git.diff_for.get().map(|(path, _)| path);
                        match (path, state.git.diff.get()) {
                            (Some(path), Some(text)) => diff_pane(state, path, &text),
                            _ => view! {
                                <p class="px-4 py-3 text-footnote text-label-4">{t!("git.pick-change")}</p>
                            }
                                .into_any(),
                        }
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
                let open = path.clone();
                let menu = path.clone();
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
                    <div
                        class=class
                        on:contextmenu=move |event: ev::MouseEvent| path_menu(state, &event, &menu)
                    >
                        <span class=format!("w-3 shrink-0 {ink}") title=hint>{glyph}</span>
                        <button
                            type="button"
                            class="min-w-0 flex-1 truncate text-left text-label-2"
                            on:click=move |_| controller::load_diff(state, show.clone(), staged, untracked)
                            on:dblclick=move |_| controller::open_file(state, open.clone())
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
/// message box does; a checkbox turns the commit into an amend.
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
    // An amend may go without a message (it keeps the one it has) and
    // without anything staged (it only rewords); a commit may do neither.
    let blocked = Signal::derive(move || {
        !state.git.amend.get()
            && (staged_count.get() == 0 || state.git.message.with(|m| m.trim().is_empty()))
    });
    view! {
        <div class="shrink-0 border-t border-line bg-sunken px-4 py-3">
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
                {move || {
                    let label = if state.git.amend.get() {
                        t!("git.amend-button")
                    } else {
                        t!("git.commit")
                    };
                    view! {
                        <Button
                            label=label
                            kind=ButtonKind::Primary
                            disabled=blocked
                            on_click=Callback::new(move |_| controller::commit(state))
                        />
                    }
                }}
                <label class="flex items-center gap-1.5 text-footnote text-label-3 select-none">
                    <input
                        type="checkbox"
                        prop:checked=move || state.git.amend.get()
                        on:change=move |event| controller::amend_toggle(state, event_target_checked(&event))
                    />
                    {t!("git.amend")}
                </label>
            </div>
        </div>
    }
}

// ─── stashes ─────────────────────────────────────────────────────────────────

/// What has been put aside, and the three things one can do to each. A
/// stash clicked opens below like a commit — it is one, and `git show` on
/// `stash@{n}` against its first parent is exactly the working tree it holds.
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
                    let selected = state.git.selected.get();
                    stashes
                        .into_iter()
                        .map(|stash| {
                            let when = format::since(stash.time);
                            let name = format!("stash@{{{}}}", stash.index);
                            let on = selected.as_deref() == Some(name.as_str());
                            let class = if on {
                                "flex cursor-pointer items-center gap-3 bg-selection px-4 py-1.5"
                            } else {
                                "flex cursor-pointer items-center gap-3 px-4 py-1.5 hover:bg-sunken"
                            };
                            let (apply, pop, drop) = (stash.index, stash.index, stash.index);
                            view! {
                                <div
                                    class=class
                                    on:click=move |_| controller::select_commit(state, name.clone())
                                >
                                    <span class="shrink-0 font-mono text-footnote text-label-3">{stash.label}</span>
                                    <span class="min-w-0 flex-1 truncate text-body">{stash.message}</span>
                                    <span class="shrink-0 text-footnote text-label-4">{when}</span>
                                    // The buttons act on the stash without also opening it.
                                    <div
                                        class="flex items-center gap-2"
                                        on:click=move |event: ev::MouseEvent| event.stop_propagation()
                                    >
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

// ─── the right-click menu ────────────────────────────────────────────────────

/// What a right-click on a commit or a file offers. Local to the thing under
/// the pointer, as a context menu should be; every write in it is the same
/// visible dock command a button would run.
#[component]
fn GitContextMenu() -> impl IntoView {
    let state = AppState::expect();
    move || {
        let menu = state.git.menu.get()?;
        let close = Callback::new(move |_| state.git.menu.set(None));
        let items = match menu.target {
            GitTarget::Commit { id } => {
                let (copy, branch, checkout, pick, revert) =
                    (id.clone(), id.clone(), id.clone(), id.clone(), id);
                view! {
                    <MenuItem
                        label=t!("git.copy-hash")
                        on_select=Callback::new(move |_| {
                            copy_to_clipboard(&copy);
                            state.git.menu.set(None);
                        })
                    />
                    <MenuSeparator />
                    <MenuItem
                        label=t!("git.branch-here")
                        on_select=Callback::new(move |_| {
                            state.git.branch_from.set(Some(branch.clone()));
                            state.git.new_branch.set(Some(String::new()));
                            state.git.menu.set(None);
                        })
                    />
                    <MenuItem
                        label=t!("git.checkout-commit")
                        on_select=Callback::new(move |_| {
                            controller::checkout_commit(state, checkout.clone());
                            state.git.menu.set(None);
                        })
                    />
                    <MenuSeparator />
                    <MenuItem
                        label=t!("git.cherry-pick")
                        on_select=Callback::new(move |_| {
                            controller::cherry_pick(state, pick.clone());
                            state.git.menu.set(None);
                        })
                    />
                    <MenuItem
                        label=t!("git.revert")
                        on_select=Callback::new(move |_| {
                            controller::revert_commit(state, revert.clone());
                            state.git.menu.set(None);
                        })
                    />
                }
                .into_any()
            }
            GitTarget::Path { path } => {
                let (open, copy) = (path.clone(), path);
                view! {
                    <MenuItem
                        label=t!("git.open-file")
                        on_select=Callback::new(move |_| {
                            controller::open_file(state, open.clone());
                            state.git.menu.set(None);
                        })
                    />
                    <MenuItem
                        label=t!("git.copy-path")
                        on_select=Callback::new(move |_| {
                            copy_to_clipboard(&copy);
                            state.git.menu.set(None);
                        })
                    />
                }
                .into_any()
            }
        };
        Some(view! {
            <ContextMenu x=menu.x y=menu.y on_close=close>
                {items}
            </ContextMenu>
        })
    }
}
