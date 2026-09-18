//! The log: the graph beside the commits, a screen of it at a time.
//!
//! It drew every row it had — a thousand small SVGs with their labels — and
//! rebuilt all of them when the selection moved or the history was read
//! again. Every row is `ROW_PX` tall, so which rows are on screen is
//! division (`crate::gitlog::window`): a spacer as tall as the whole log
//! keeps the scrollbar honest, and only the rows in view and a dozen each
//! side exist. The ones above the view are not only for smooth scrolling —
//! a row draws the lines that *leave* it, so the lines arriving at the top
//! of the view belong to a row above it.
//!
//! Rows are keyed on what they draw (`gitlog::row_key`), so a history read
//! again redraws only the rows whose lane, lines or labels moved. Whether a
//! row is selected and whether it matches the search are its own memos: a
//! click repaints two rows, and a keystroke in the search box repaints no
//! graph at all.
//!
//! **One row is one SVG whose lines run past its bottom edge.** Lanes and
//! edges arrive laid out (`rusty_git::graph`, pure and tested); this only
//! turns a lane into an x. The SVG is `relative z-10`, because the next
//! row's hover or selection fill is painted after it and covered the part
//! of a line that had crossed into that row. The lane colours are fixed hex,
//! like the board sheet's: a commit graph is the same colours in every
//! client that draws one, and a lane that changed colour with the theme
//! would read as a different branch.

use std::time::Duration;

use leptos::{ev, html, prelude::*};
use wasm_bindgen::{JsCast, closure::Closure};

use rusty_git::{GraphRow, RefKind, RefLabel};
use rusty_i18n::t;

use crate::{
    controller, format, gitlog,
    state::{AppState, GitMenu, GitTarget},
    view::components::{Pill, Tone},
};

/// Fork's palette, near enough: distinct at 2px, legible on both themes.
const LANE_COLOURS: [&str; 8] = [
    "#4f8ef7", "#e0a33b", "#3fbf7f", "#d56fd0", "#f0715b", "#5cc8d6", "#a98cff", "#c7b34a",
];
const ROW_PX: u32 = 26;
const LANE_PX: u32 = 14;
/// Rows drawn beyond each edge of the view.
const SPARE_ROWS: usize = 12;
/// The line under the last row that offers older commits.
const FOOTER_PX: u32 = 36;
/// Labels drawn on a row before the rest are counted: a release commit
/// carrying six tags would otherwise push its subject off the row.
const MAX_LABELS: usize = 3;

#[component]
pub(super) fn Log() -> impl IntoView {
    let state = AppState::expect();
    let scroller: NodeRef<html::Div> = NodeRef::new();
    let scroll_top = RwSignal::new(0.0_f64);
    let view_height = RwSignal::new(0.0_f64);

    // The scroller's height follows the dividers and the window. The
    // callback is forgotten rather than kept: an observer firing after the
    // view has gone would call a dropped closure, and this one reads only
    // through `try_` and finds nothing.
    let observer = StoredValue::new_local(None::<web_sys::ResizeObserver>);
    Effect::new(move |_| {
        let Some(element) = scroller.get() else {
            return;
        };
        view_height.set(f64::from(element.client_height()));
        let measure = Closure::<dyn FnMut()>::new(move || {
            if let Some(Some(element)) = scroller.try_get_untracked() {
                let _ = view_height.try_set(f64::from(element.client_height()));
            }
        });
        if let Ok(watch) = web_sys::ResizeObserver::new(measure.as_ref().unchecked_ref()) {
            watch.observe(&element);
            observer.update_value(|slot| {
                if let Some(old) = slot.replace(watch) {
                    old.disconnect();
                }
            });
        }
        measure.forget();
    });
    on_cleanup(move || {
        observer.try_update_value(|slot| {
            if let Some(watch) = slot.take() {
                watch.disconnect();
            }
        });
    });

    let total = Memo::new(move |_| {
        state
            .git
            .history
            .with(|history| history.as_ref().map_or(0, |h| h.rows.len()))
    });
    let span = Memo::new(move |_| {
        gitlog::window(
            scroll_top.get(),
            view_height.get(),
            f64::from(ROW_PX),
            total.get(),
            SPARE_ROWS,
        )
    });
    let loaded = Memo::new(move |_| state.git.history.with(Option::is_some));
    let truncated = Memo::new(move |_| {
        state
            .git
            .history
            .with(|history| history.as_ref().is_some_and(|h| h.truncated))
    });
    let needle = Memo::new(move |_| state.git.query.with(|query| query.trim().to_lowercase()));

    // A shorter log clamps the scroller without asking; read where it
    // landed once the spacer has shrunk.
    Effect::new(move |_| {
        total.track();
        set_timeout(
            move || {
                if let Some(Some(element)) = scroller.try_get_untracked() {
                    let _ = scroll_top.try_set(f64::from(element.scroll_top()));
                }
            },
            Duration::ZERO,
        );
    });

    // Another branch's history starts at its top.
    Effect::new(move |previous: Option<Option<String>>| {
        let rev = state.git.rev.get();
        if previous.is_some_and(|previous| previous != rev) {
            if let Some(element) = scroller.get_untracked() {
                element.set_scroll_top(0);
            }
            scroll_top.set(0.0);
        }
        rev
    });

    // Scroll a commit into view, once — a branch clicked in the sidebar, a
    // search hit, an arrow key. A commit not in the log yet is waited for
    // while a filter brings it; a log that arrives without it ends the wait,
    // so a later read cannot scroll the page from under somebody.
    Effect::new(move |waiting: Option<Option<String>>| {
        let id = state.git.reveal.get()?;
        let index = state.git.history.with(|history| {
            history
                .as_ref()
                .and_then(|h| h.rows.iter().position(|row| row.commit.id == id))
        });
        let Some(index) = index else {
            if waiting.flatten().as_deref() == Some(id.as_str()) {
                state.git.reveal.set(None);
                return None;
            }
            return Some(id);
        };
        state.git.reveal.set(None);
        // After the spacer has grown to the log it may just have received.
        set_timeout(
            move || {
                let Some(Some(element)) = scroller.try_get_untracked() else {
                    return;
                };
                let top = f64::from(element.scroll_top());
                let height = f64::from(element.client_height());
                if let Some(target) = gitlog::scroll_for(index, top, height, f64::from(ROW_PX)) {
                    element.set_scroll_top(target.round() as i32);
                    let _ = scroll_top.try_set(target);
                }
            },
            Duration::ZERO,
        );
        None
    });

    let on_scroll = move |_: ev::Event| {
        if let Some(element) = scroller.get_untracked() {
            scroll_top.set(f64::from(element.scroll_top()));
        }
    };
    // The arrows walk the log as they walk any list; the commit opens once
    // they stop (`controller::select_step`).
    let on_key = move |event: ev::KeyboardEvent| {
        let page = scroller.get_untracked().map_or(10, |element| {
            (i64::from(element.client_height()) / i64::from(ROW_PX) - 1).max(1)
        });
        let step = match event.key().as_str() {
            "ArrowDown" => 1,
            "ArrowUp" => -1,
            "PageDown" => page,
            "PageUp" => -page,
            "Home" => -i64::from(u32::MAX),
            "End" => i64::from(u32::MAX),
            _ => return,
        };
        event.prevent_default();
        controller::select_step(state, step);
    };

    let rows = move || {
        let range = span.get();
        state.git.history.with(|history| {
            let Some(history) = history else {
                return Vec::new();
            };
            let end = range.end.min(history.rows.len());
            let start = range.start.min(end);
            // Each row with the lane of the commit under it, which decides
            // where the lines leaving it turn (`gitlog::edge_line`).
            (start..end)
                .map(|at| {
                    let row = &history.rows[at];
                    let next = history.rows.get(at + 1).map(|below| below.lane);
                    (
                        gitlog::row_key(row, history.lanes, next),
                        row.clone(),
                        history.lanes,
                        next,
                    )
                })
                .collect::<Vec<_>>()
        })
    };

    view! {
        <div
            node_ref=scroller
            tabindex="0"
            class="min-h-0 flex-1 overflow-y-auto outline-none"
            on:scroll=on_scroll
            on:keydown=on_key
        >
            {move || {
                if !loaded.get() {
                    return view! {
                        <p class="px-4 py-3 text-callout text-label-3">{t!("git.loading")}</p>
                    }
                    .into_any();
                }
                view! {
                    <div
                        class="relative"
                        style=move || {
                            let footer = if truncated.get() { FOOTER_PX } else { 0 };
                            format!("height: {}px", total.get() as u32 * ROW_PX + footer)
                        }
                    >
                        <div
                            class="absolute inset-x-0 flex flex-col"
                            style=move || format!("top: {}px", span.get().start as u32 * ROW_PX)
                        >
                            <For
                                each=rows
                                key=|(key, ..)| *key
                                children=move |(_, row, lanes, next)| {
                                    view! { <Row row=row lanes=lanes next=next needle=needle /> }
                                }
                            />
                        </div>
                        {move || {
                            truncated
                                .get()
                                .then(|| {
                                    view! {
                                        <div
                                            class="absolute inset-x-0 flex items-center gap-3 px-4 text-footnote text-label-4"
                                            style=move || {
                                                format!(
                                                    "top: {}px; height: {FOOTER_PX}px",
                                                    total.get() as u32 * ROW_PX,
                                                )
                                            }
                                        >
                                            <span>{move || t!("git.truncated", count = total.get())}</span>
                                            <button
                                                type="button"
                                                class="text-rust hover:underline"
                                                on:click=move |_| controller::show_more(state)
                                            >
                                                {t!("git.show-more")}
                                            </button>
                                        </div>
                                    }
                                })
                        }}
                    </div>
                }
                .into_any()
            }}
        </div>
    }
}

/// One commit: its slice of the graph, its labels, its subject, hash,
/// author and date. Dimmed, all but the graph, when a search is on and the
/// commit does not match it — dimmed rather than hidden, so the lines still
/// join up.
#[component]
fn Row(row: GraphRow, lanes: u32, next: Option<u32>, needle: Memo<String>) -> impl IntoView {
    let state = AppState::expect();
    let commit = row.commit.clone();
    let picked = {
        let id = commit.id.clone();
        Memo::new(move |_| {
            state
                .git
                .selected
                .with(|selected| selected.as_deref() == Some(id.as_str()))
        })
    };
    let dim = {
        let row = row.clone();
        Memo::new(move |_| needle.with(|needle| !gitlog::matches(&row, needle)))
    };
    let class = move || {
        if picked.get() {
            "flex shrink-0 cursor-pointer items-center gap-2 bg-selection pr-3"
        } else {
            "flex shrink-0 cursor-pointer items-center gap-2 pr-3 hover:bg-sunken"
        }
    };
    let text = move || {
        if dim.get() {
            "flex min-w-0 flex-1 items-center gap-2 opacity-40"
        } else {
            "flex min-w-0 flex-1 items-center gap-2"
        }
    };
    // Branches before tags. git decorates in its own order, and four tags
    // on a release commit pushed the branch that also sat there into "+2".
    let mut refs = commit.refs.clone();
    refs.sort_by_key(|label| match label.kind {
        RefKind::Head => 0,
        RefKind::Branch => 1,
        RefKind::Remote => 2,
        RefKind::Tag => 3,
    });
    let lane = row.lane;
    let labels = refs
        .iter()
        .take(MAX_LABELS)
        .map(|label| label_view(label, lane))
        .collect_view();
    let more = (refs.len() > MAX_LABELS).then(|| {
        let names = refs[MAX_LABELS..]
            .iter()
            .map(|label| label.name.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let count = format!("+{}", refs.len() - MAX_LABELS);
        view! {
            <span class="shrink-0" title=names>
                <Pill label=count uppercase=false />
            </span>
        }
    });
    let when = format::commit_when(commit.time);
    let exact = format::full_time(commit.time);
    let who = format!("{} <{}>", commit.author, commit.email);
    let (open, menu) = (commit.id.clone(), commit.id.clone());
    view! {
        <div
            class=class
            style=format!("height: {ROW_PX}px")
            on:click=move |_| controller::select_commit(state, open.clone())
            on:contextmenu=move |event: ev::MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
                state.git.menu.set(Some(GitMenu {
                    x: f64::from(event.client_x()),
                    y: f64::from(event.client_y()),
                    target: GitTarget::Commit { id: menu.clone() },
                }));
            }
        >
            {graph_cell(&row, lanes, next)}
            <div class=text>
                {labels}
                {more}
                <span class="min-w-0 flex-1 truncate text-body">{commit.summary}</span>
                <span class="shrink-0 font-mono text-footnote text-label-4">{commit.short}</span>
                <span class="w-[9rem] shrink-0 truncate text-footnote text-label-3" title=who>
                    {commit.author}
                </span>
                <span class="w-[5.5rem] shrink-0 text-right text-footnote text-label-4 tnum" title=exact>
                    {when}
                </span>
            </div>
        </div>
    }
}

/// A lane's colour.
fn lane_colour(lane: u32) -> &'static str {
    LANE_COLOURS[(lane as usize) % LANE_COLOURS.len()]
}

/// A label on a commit. A branch is the colour of the line it sits on, as
/// Fork draws it — a label in a colour of its own said "branch" and nothing
/// about which line it was; filled, with dark words, which read on every one
/// of the lane colours in both themes. The branch checked out carries a
/// tick; a remote's is the same colour as a tint with an edge, since it is a
/// copy of a branch rather than one; a tag keeps its own colour, because it
/// is not a line. Names as spelled: a branch or a tag is an identifier the
/// user types, and `MASTER` names nothing.
fn label_view(label: &RefLabel, lane: u32) -> AnyView {
    let colour = lane_colour(lane);
    let shape = "inline-flex h-[18px] shrink-0 items-center rounded-full px-2 font-mono \
                 text-caption font-semibold whitespace-nowrap";
    match label.kind {
        RefKind::Tag => {
            view! { <Pill label=label.name.clone() tone=Tone::Amber uppercase=false /> }.into_any()
        }
        RefKind::Remote => view! {
            <span
                class=format!("{shape} text-label")
                style=format!("background: {colour}33; box-shadow: inset 0 0 0 1px {colour}")
            >
                {label.name.clone()}
            </span>
        }
        .into_any(),
        RefKind::Head | RefKind::Branch => {
            let text = match label.kind {
                RefKind::Head if label.name.is_empty() => format!("✓ {}", t!("git.head")),
                RefKind::Head => format!("✓ {}", label.name),
                _ => label.name.clone(),
            };
            view! {
                <span class=shape style=format!("background: {colour}; color: #15130f")>
                    {text}
                </span>
            }
            .into_any()
        }
    }
}

/// One row's slice of the graph. Lines run from this row's centre to the
/// next row's centre, so they spill past the bottom edge on purpose. Their
/// shapes are `gitlog::edge_line`'s — straight down a lane, a quarter circle
/// into a commit, Fork's rather than a slant — and turning into a commit
/// depends on where the next row's commit is, which is `next`.
fn graph_cell(row: &GraphRow, lanes: u32, next: Option<u32>) -> AnyView {
    let width = lanes.max(1) * LANE_PX;
    let x = |lane: u32| f64::from(lane * LANE_PX + LANE_PX / 2);
    let mid = f64::from(ROW_PX) / 2.0;
    let lines: Vec<_> = row
        .edges
        .iter()
        .map(|edge| {
            let line = gitlog::edge_line(
                edge.from,
                edge.to,
                edge.from == row.lane,
                next == Some(edge.to),
                f64::from(LANE_PX),
                f64::from(ROW_PX),
            );
            view! {
                <path
                    d=line.d
                    stroke=lane_colour(line.lane)
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    fill="none"
                />
            }
        })
        .collect();
    let dot = lane_colour(row.lane);
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
