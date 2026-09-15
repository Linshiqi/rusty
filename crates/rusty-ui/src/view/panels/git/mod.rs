//! The repository, the way Fork shows it.
//!
//! Down the left, **the refs** (`sidebar`): local branches with how far each
//! is ahead of and behind its upstream, the remotes' branches grouped by
//! remote, and the tags. A click goes to the commit, a double-click checks
//! a branch out, a right-click offers what can be done to it. Beside them,
//! three views behind one row of tabs. **History**: the log as a graph beside
//! the commits, labels on the commits that carry branches and tags, a commit
//! opened below with its files and each file's patch. **Changes**: the
//! working tree in two lists — what the next commit would carry and what it
//! would not — each file's diff, and the commit box. **Stashes**: what has
//! been put aside, with the three things one can do to a stash and, opened
//! below, what each one holds. Above everything: what is checked out and
//! how it stands against its upstream, a search over the log, and the
//! repository's verbs.
//!
//! Every write is a dock command (see `controller::git`), so the exact `git`
//! line and its answer are readable. Only staging is quiet.
//!
//! **Nothing is drawn twice.** The log draws the rows on screen and a few
//! more (`crate::gitlog`); a row's selection and its search match are its
//! own signals, so a click repaints two rows rather than every row; the
//! opened commit's frame, file list and patch are separate closures, so
//! picking a file redraws the patch alone. The panel is rebuilt each time it
//! is switched to, and a return to a project it has shown only asks whether
//! anything moved (`controller::open_git_panel`).
//!
//! One module per region — `sidebar`, `log`, `detail`, `diff`, `changes`,
//! `stashes`, `menu` — where there was one file of 1,750 lines.

mod changes;
mod detail;
mod diff;
mod log;
mod menu;
mod sidebar;
mod stashes;

use std::time::Duration;

use leptos::{ev, html, prelude::*};

use rusty_git::{GitOperation, RefNameProblem};
use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::view::split;
use crate::{
    controller,
    state::{AppState, Divider, GitMode, PromptKind},
    view::components::{Button, ButtonKind, Empty},
};

use detail::Detail;

/// How often the panel asks, while it is showing, whether the repository
/// moved: a commit, a checkout or a fetch made in a terminal touches nothing
/// the file watcher sees. The question costs no `git` at all.
const PROBE_EVERY: Duration = Duration::from_millis(2500);

#[component]
pub fn GitPanel() -> impl IntoView {
    let state = AppState::expect();

    // Keyed on the root: a project already shown is only probed, a new one
    // starts clean — see `controller::open_git_panel`. A keystroke elsewhere
    // re-renders nothing here.
    Effect::new(move |previous: Option<Option<String>>| {
        let root = state
            .project
            .detected
            .with(|project| project.as_ref().map(|p| p.root.clone()));
        if previous.as_ref() != Some(&root)
            && let Some(root) = root.clone()
        {
            controller::open_git_panel(state, root);
        }
        root
    });

    let probe = set_interval_with_handle(move || controller::probe_git(state), PROBE_EVERY).ok();
    on_cleanup(move || {
        if let Some(probe) = probe {
            probe.clear();
        }
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
            // Not a repository is the one refusal the panel can fix itself.
            let offer_init = state.git.not_a_repo.get();
            return view! {
                <Empty title=t!("git.unavailable-title") detail=why>
                    {offer_init
                        .then(|| {
                            view! {
                                <div class="flex flex-col items-center gap-2">
                                    <button
                                        type="button"
                                        on:click=move |_| controller::git_init(state)
                                        class="rounded-[6px] bg-rust px-3 py-1 text-footnote font-medium text-white hover:opacity-90"
                                    >
                                        {t!("git.init")}
                                    </button>
                                    <p class="max-w-[46ch] text-footnote text-label-3">
                                        {t!("git.init-hint")}
                                    </p>
                                </div>
                            }
                        })}
                </Empty>
            }
            .into_any();
        }
        view! {
            // The browser's own menu is never the answer here: rows offer
            // theirs, and everywhere else a right-click does nothing.
            <div
                class="flex min-h-0 flex-1 flex-col"
                on:contextmenu=move |event: ev::MouseEvent| event.prevent_default()
            >
                <TopBar />
                <PromptRow />
                <OperationBanner />
                <div class="flex min-h-0 flex-1">
                    <sidebar::Sidebar />
                    <split::Handle divider=Divider::GitSidebar />
                    <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                        <Modes />
                        {move || match state.git.mode.get() {
                            GitMode::History => view! { <log::Log /> <Detail /> }.into_any(),
                            GitMode::Changes => view! { <changes::Changes /> }.into_any(),
                            GitMode::Stashes => view! { <stashes::Stashes /> <Detail /> }.into_any(),
                        }}
                    </div>
                </div>
            </div>
            <menu::GitContextMenu />
        }
        .into_any()
    }
}

/// One commit filling a window of its own — what `?gitdiff=<target>` boots.
pub fn commit_window() -> AnyView {
    view! { <Detail standalone=true /> }.into_any()
}

/// The shape of one action in the top row: an icon button, as VS Code's
/// view titles carry theirs.
const HEADER_BUTTON: &str = "flex h-7 min-w-7 items-center justify-center gap-1 rounded-[6px] px-1.5 text-label-2 hover:bg-sunken hover:text-label";

fn header_button(
    title: String,
    icon: Icon,
    on_click: impl Fn(ev::MouseEvent) + 'static,
) -> AnyView {
    view! {
        <button type="button" title=title on:click=on_click class=HEADER_BUTTON>
            <IconView icon=icon size=14 />
        </button>
    }
    .into_any()
}

/// What is checked out and how it stands, the log's filter and search, and
/// the repository's verbs. Pull and push carry the counts they would move,
/// so "is there anything to push" is read off the button.
#[component]
fn TopBar() -> impl IntoView {
    let state = AppState::expect();
    let counts = Memo::new(move |_| {
        state.git.status.with(|status| {
            status
                .as_ref()
                .map(|s| (s.ahead, s.behind, s.upstream.is_some()))
                .unwrap_or((0, 0, false))
        })
    });
    view! {
        <div class="flex flex-wrap items-center gap-2 border-b border-line px-3 py-1.5">
            <HeadChip />
            {move || {
                let rev = state.git.rev.get()?;
                let title = t!("git.filter-on", name = rev.clone());
                Some(view! {
                    <span
                        class="flex h-[24px] items-center gap-1 rounded-full bg-selection pr-1 pl-2.5 font-mono text-footnote text-rust"
                        title=title
                    >
                        <span class="max-w-[16rem] truncate">{rev}</span>
                        <button
                            type="button"
                            title=t!("git.filter-clear")
                            class="grid size-[18px] place-items-center rounded-full hover:bg-raised"
                            on:click=move |_| controller::show_rev(state, None)
                        >
                            <IconView icon=Icon::Close size=10 />
                        </button>
                    </span>
                })
            }}
            <SearchBox />
            <span class="flex-1" />
            {header_button(t!("git.refresh"), Icon::Refresh, move |_| controller::load_git(state))}
            {header_button(t!("git.fetch"), Icon::Fetch, move |_| controller::fetch(state))}
            <button
                type="button"
                title=move || {
                    let (_, behind, _) = counts.get();
                    if behind > 0 { t!("git.pull-count", count = behind) } else { t!("git.pull") }
                }
                on:click=move |_| controller::pull(state)
                class=HEADER_BUTTON
            >
                <IconView icon=Icon::Pull size=14 />
                {move || {
                    let (_, behind, _) = counts.get();
                    (behind > 0).then(|| view! { <span class="text-caption tnum">{behind}</span> })
                }}
            </button>
            <button
                type="button"
                title=move || {
                    let (ahead, _, tracked) = counts.get();
                    if !tracked {
                        t!("git.push-upstream")
                    } else if ahead > 0 {
                        t!("git.push-count", count = ahead)
                    } else {
                        t!("git.push")
                    }
                }
                on:click=move |_| controller::push(state)
                class=HEADER_BUTTON
            >
                <IconView icon=Icon::Push size=14 />
                {move || {
                    let (ahead, _, _) = counts.get();
                    (ahead > 0).then(|| view! { <span class="text-caption tnum">{ahead}</span> })
                }}
            </button>
            {header_button(t!("git.new-branch"), Icon::Plus, move |_| {
                controller::open_prompt(state, PromptKind::Branch { from: None })
            })}
        </div>
    }
}

/// The branch checked out — or the commit, when HEAD is detached — with
/// its upstream and how far the two have drifted. A click goes to it.
#[component]
fn HeadChip() -> impl IntoView {
    let state = AppState::expect();
    let head = Memo::new(move |_| {
        state.git.status.with(|status| {
            status.as_ref().map(|s| {
                (
                    s.head.clone(),
                    s.detached,
                    s.upstream.clone(),
                    s.ahead,
                    s.behind,
                )
            })
        })
    });
    let reveal = move |_| {
        // The current branch's tip, or HEAD itself when detached — which the
        // log's decorations name.
        let current = state.git.branches.with_untracked(|branches| {
            branches
                .iter()
                .find(|b| b.current)
                .map(|b| (b.id.clone(), b.name.clone()))
        });
        let detached = state.git.history.with_untracked(|history| {
            history.as_ref().and_then(|h| {
                h.rows
                    .iter()
                    .find(|row| {
                        row.commit
                            .refs
                            .iter()
                            .any(|label| label.kind == rusty_git::RefKind::Head)
                    })
                    .map(|row| row.commit.id.clone())
            })
        });
        match (current, detached) {
            (Some((id, name)), _) => controller::reveal_ref(state, id, Some(name)),
            (None, Some(id)) => controller::reveal_ref(state, id, None),
            (None, None) => {}
        }
    };
    view! {
        <button
            type="button"
            title=t!("git.head-reveal")
            on:click=reveal
            class="flex h-[26px] max-w-[28rem] items-center gap-1.5 rounded-full bg-sunken px-2.5 font-mono text-footnote text-label ring-1 ring-line hover:ring-line-strong"
        >
            <IconView icon=Icon::Branch size=13 />
            {move || match head.get() {
                None => view! { <span class="text-label-3">"…"</span> }.into_any(),
                Some((_, true, ..)) => {
                    let short = state
                        .git
                        .history
                        .with(|h| h.as_ref().and_then(|h| h.head.clone()))
                        .unwrap_or_default();
                    view! {
                        <span class="text-amber">{t!("git.head-detached", hash = short)}</span>
                    }
                        .into_any()
                }
                Some((name, false, upstream, ahead, behind)) => {
                    view! {
                        <span class="text-patina">"●"</span>
                        <span class="truncate">{name.unwrap_or_default()}</span>
                        {(ahead > 0)
                            .then(|| view! { <span class="text-caption text-label-2 tnum">{format!("↑{ahead}")}</span> })}
                        {(behind > 0)
                            .then(|| view! { <span class="text-caption text-label-2 tnum">{format!("↓{behind}")}</span> })}
                        {upstream
                            .map(|upstream| {
                                view! { <span class="truncate text-label-4">{format!("→ {upstream}")}</span> }
                            })}
                    }
                        .into_any()
                }
            }}
        </button>
    }
}

/// A search over the loaded log: matching rows stay bright and the rest
/// dim, Enter goes to the next match and Shift+Enter to the one before, and
/// the box says where in the matches the selection is.
#[component]
fn SearchBox() -> impl IntoView {
    let state = AppState::expect();
    let found = Memo::new(move |_| {
        let query = state.git.query.get();
        if query.trim().is_empty() {
            return None;
        }
        let hits = state.git.history.with(|h| {
            h.as_ref()
                .map(|h| crate::gitlog::hits(&h.rows, &query))
                .unwrap_or_default()
        });
        let at = state.git.selected.with(|selected| {
            state.git.history.with(|h| {
                let rows = &h.as_ref()?.rows;
                let index = rows
                    .iter()
                    .position(|row| Some(&row.commit.id) == selected.as_ref())?;
                hits.iter().position(|hit| *hit == index)
            })
        });
        Some((at, hits.len()))
    });
    view! {
        <div class="flex h-[26px] min-w-[14rem] max-w-[26rem] flex-1 items-center gap-1.5 rounded-[6px] bg-sunken px-2 ring-1 ring-line focus-within:ring-rust">
            <IconView icon=Icon::Search size=12 />
            <input
                type="text"
                spellcheck="false"
                placeholder=t!("git.search-placeholder")
                class="min-w-0 flex-1 bg-transparent text-footnote outline-none placeholder:text-label-3"
                prop:value=move || state.git.query.get()
                on:input=move |event| state.git.query.set(event_target_value(&event))
                on:keydown=move |event: ev::KeyboardEvent| match event.key().as_str() {
                    "Enter" => {
                        event.prevent_default();
                        controller::step_search(state, !event.shift_key());
                    }
                    "Escape" => state.git.query.set(String::new()),
                    _ => {}
                }
            />
            {move || {
                found
                    .get()
                    .map(|(at, count)| {
                        let text = match (at, count) {
                            (_, 0) => t!("git.search-none"),
                            (Some(at), count) => t!("git.search-count", at = at + 1, count = count),
                            (None, count) => t!("git.search-total", count = count),
                        };
                        view! { <span class="shrink-0 text-caption text-label-3 tnum">{text}</span> }
                    })
            }}
        </div>
    }
}

/// The name field for a new branch, a rename or a new tag. It says what git
/// would object to while the name is typed, and will not send a name git
/// would refuse.
#[component]
fn PromptRow() -> impl IntoView {
    let state = AppState::expect();
    // What the field is for, alone: the value changes on every keystroke,
    // and a closure keyed on it rebuilt the input under the caret.
    let kind = Memo::new(move |_| {
        state
            .git
            .prompt
            .with(|p| p.as_ref().map(|p| p.kind.clone()))
    });
    let input: NodeRef<html::Input> = NodeRef::new();
    Effect::new(move |_| {
        if kind.with(Option::is_some)
            && let Some(input) = input.get()
        {
            let _ = input.focus();
            input.select();
        }
    });
    let problem = Memo::new(move |_| {
        state.git.prompt.with(|p| {
            p.as_ref()
                .and_then(|p| rusty_git::ref_name_problem(p.value.trim()))
        })
    });
    let short = |id: &str| -> String {
        if id.len() == 40 {
            id.chars().take(7).collect()
        } else {
            id.to_string()
        }
    };
    move || {
        let kind = kind.get()?;
        let label = match &kind {
            PromptKind::Branch { from: Some(from) } => {
                t!("git.prompt-branch-from", from = short(from))
            }
            PromptKind::Branch { from: None } => t!("git.prompt-branch"),
            PromptKind::Rename { from } => t!("git.prompt-rename", from = from.clone()),
            PromptKind::Tag { at } => t!("git.prompt-tag", at = short(at)),
        };
        let cancel = move || state.git.prompt.set(None);
        Some(view! {
            <div class="flex flex-wrap items-center gap-2 border-b border-line bg-sunken px-3 py-1.5">
                <span class="text-footnote text-label-2">{label}</span>
                <input
                    node_ref=input
                    type="text"
                    spellcheck="false"
                    class="h-[26px] w-[22rem] max-w-full rounded-[6px] bg-content px-2.5 font-mono text-footnote outline-none ring-1 ring-rust"
                    prop:value=move || state.git.prompt.with(|p| p.as_ref().map(|p| p.value.clone()).unwrap_or_default())
                    on:input=move |event| {
                        let value = event_target_value(&event);
                        state.git.prompt.update(|p| {
                            if let Some(p) = p {
                                p.value = value;
                            }
                        });
                    }
                    on:keydown=move |event: ev::KeyboardEvent| match event.key().as_str() {
                        "Enter" => controller::submit_prompt(state),
                        "Escape" => cancel(),
                        _ => {}
                    }
                />
                <span class="text-footnote text-crimson">
                    {move || {
                        let typed = state.git.prompt.with(|p| p.as_ref().is_some_and(|p| !p.value.trim().is_empty()));
                        problem.get().filter(|_| typed).map(name_problem)
                    }}
                </span>
                <span class="flex-1" />
                <Button
                    label=t!("git.prompt-ok")
                    kind=ButtonKind::Primary
                    disabled=Signal::derive(move || problem.get().is_some())
                    on_click=Callback::new(move |_| controller::submit_prompt(state))
                />
                <Button label=t!("git.cancel") on_click=Callback::new(move |_| cancel()) />
            </div>
        })
    }
}

/// Why git would refuse a name, in words.
fn name_problem(problem: RefNameProblem) -> String {
    match problem {
        RefNameProblem::Empty => t!("git.name-empty"),
        RefNameProblem::Whitespace => t!("git.name-whitespace"),
        RefNameProblem::Character(c) => t!("git.name-character", char = c.to_string()),
        RefNameProblem::Dash => t!("git.name-dash"),
        RefNameProblem::Dot => t!("git.name-dot"),
        RefNameProblem::Slash => t!("git.name-slash"),
        RefNameProblem::Lock => t!("git.name-lock"),
        RefNameProblem::Reserved => t!("git.name-reserved"),
    }
}

/// A merge, rebase, cherry-pick or revert that stopped half way, said above
/// everything else with the way on and the way back. A repository in that
/// state refuses most of what the panel offers, and git's refusal names the
/// state rather than the way out.
#[component]
fn OperationBanner() -> impl IntoView {
    let state = AppState::expect();
    let stopped = Memo::new(move |_| {
        state.git.status.with(|status| {
            status.as_ref().and_then(|s| {
                s.operation.map(|op| {
                    let conflicts = s.entries.iter().filter(|e| e.conflicted).count();
                    (op, conflicts)
                })
            })
        })
    });
    move || {
        let (op, conflicts) = stopped.get()?;
        let title = match op {
            GitOperation::Merge => t!("git.op-merge"),
            GitOperation::Rebase => t!("git.op-rebase"),
            GitOperation::CherryPick => t!("git.op-cherry-pick"),
            GitOperation::Revert => t!("git.op-revert"),
        };
        let detail = if conflicts > 0 {
            t!("git.op-conflicts", count = conflicts)
        } else {
            t!("git.op-resolved")
        };
        Some(view! {
            <div class="flex flex-wrap items-center gap-3 border-b border-line bg-amber-fill px-3 py-2">
                <div class="min-w-0 flex-1">
                    <p class="text-callout font-medium">{title}</p>
                    <p class="text-footnote text-label-2">{detail}</p>
                </div>
                <Button
                    label=t!("git.op-changes")
                    on_click=Callback::new(move |_| state.git.mode.set(GitMode::Changes))
                />
                <Button
                    label=t!("git.op-continue")
                    kind=ButtonKind::Primary
                    disabled=Signal::derive(move || conflicts > 0)
                    on_click=Callback::new(move |_| controller::continue_operation(state))
                />
                <Button
                    label=t!("git.op-abort")
                    on_click=Callback::new(move |_| controller::abort_operation(state))
                />
            </div>
        })
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
