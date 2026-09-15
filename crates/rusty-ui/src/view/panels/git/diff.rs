//! One file's difference, and the pictures that stand in for one.
//!
//! **A diff is read once and laid out two ways.** `rusty_git::diff` turns
//! git's unified text into numbered rows — side by side, or one column in
//! git's own order — and this file only decides what a row looks like. The
//! toggle between the two is remembered.
//!
//! **A long diff is drawn in part until asked.** Every row is several DOM
//! nodes, and a regenerated lock file or a vendored header is tens of
//! thousands of rows: drawn whole, opening it froze the window for as long
//! as that took. The first `DRAWN_ROWS` are drawn, with a line saying how
//! many there are and a button for the rest.

use leptos::{ev, prelude::*};

use rusty_git::ChangeKind;
use rusty_git::diff::{Cell, CellKind, Hunk};
use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::view::split;
use crate::{
    controller,
    state::{AppState, Divider, ImageSide},
};

/// How many rows of a diff are drawn before the rest waits for a click.
const DRAWN_ROWS: usize = 1500;

/// One file's difference: a header naming the file and offering the two
/// layouts, then the rows. Reads the layout signal, so the closure that
/// calls this re-renders when the toggle is pressed.
pub(super) fn diff_pane(state: AppState, path: String, text: &str) -> AnyView {
    // A picture is compared as pictures. The controller that picked the file
    // has already asked for both sides; this only draws what has arrived.
    if rusty_git::is_image_path(&path) {
        return image_pane(state, path);
    }
    let hunks = rusty_git::diff::hunks(text);
    let split = state.git.split.get();
    let whole = state.git.diff_whole.get();
    let total: usize = hunks
        .iter()
        .map(|hunk| {
            1 + if split {
                hunk.rows.len()
            } else {
                hunk.lines.len()
            }
        })
        .sum();
    let budget = (!whole && total > DRAWN_ROWS).then_some(DRAWN_ROWS);
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
        split_rows(state, &hunks, budget)
    } else {
        unified_rows(&hunks, budget)
    };
    let clipped = budget.map(|shown| {
        view! {
            <div class="flex items-center gap-3 border-t border-line px-3 py-2 font-sans text-footnote text-label-3">
                <span>{t!("git.diff-clipped", shown = shown, count = total)}</span>
                <button
                    type="button"
                    class="text-rust hover:underline"
                    on:click=move |_| state.git.diff_whole.set(true)
                >
                    {t!("git.diff-show-all", count = total)}
                </button>
            </div>
        }
    });
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
                {clipped}
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
fn split_rows(state: AppState, hunks: &[Hunk], budget: Option<usize>) -> AnyView {
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
    let mut remaining = budget.unwrap_or(usize::MAX);
    let mut cells: Vec<AnyView> = Vec::new();
    for hunk in hunks {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        // The hunk header once per side, as Fork draws it: one header across
        // both would cross the line between old and new, and a row that
        // crosses it reads as a layout that has come apart.
        let header = hunk.header.clone();
        cells.push(
            view! {
                <div class="col-span-2 min-w-0 bg-sunken px-3 py-0.5 text-slate break-words whitespace-pre-wrap">
                    {header.clone()}
                </div>
                <div class="col-span-2 min-w-0 bg-sunken px-3 py-0.5 text-slate break-words whitespace-pre-wrap">
                    {header}
                </div>
            }
            .into_any(),
        );
        let take = remaining.min(hunk.rows.len());
        for row in &hunk.rows[..take] {
            cells.push(view! { {side(row.left.as_ref())} {side(row.right.as_ref())} }.into_any());
        }
        remaining -= take;
    }
    view! {
        // At least the pane's height, so the centre line runs the whole way
        // down even when the diff is a few rows — a line that stops at the
        // last row leaves the two halves reading as one pane below it. And
        // `content-start`, because a grid taller than its rows stretches
        // them to fill it (`align-content: normal` is `stretch` in a grid):
        // v0.6.2 shipped short diffs double-spaced, an empty line under
        // every line, and the height came from exactly this rule.
        <div class="relative grid min-h-full content-start" style=columns node_ref=grid>
            {cells}
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
fn unified_rows(hunks: &[Hunk], budget: Option<usize>) -> AnyView {
    let mut remaining = budget.unwrap_or(usize::MAX);
    let mut cells: Vec<AnyView> = Vec::new();
    for hunk in hunks {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        cells.push(
            view! { <div class="col-span-3 bg-sunken px-3 py-0.5 text-slate">{hunk.header.clone()}</div> }
                .into_any(),
        );
        let take = remaining.min(hunk.lines.len());
        for line in &hunk.lines[..take] {
            let tint = tint(line.kind);
            let sign = match line.kind {
                CellKind::Added => "+",
                CellKind::Removed => "-",
                CellKind::Context | CellKind::Note => " ",
            };
            let old = line.old.map(|n| n.to_string()).unwrap_or_default();
            let new = line.new.map(|n| n.to_string()).unwrap_or_default();
            cells.push(
                view! {
                    <span class=format!("px-2 text-right text-label-4 tnum select-none {tint}")>{old}</span>
                    <span class=format!("px-2 text-right text-label-4 tnum select-none {tint}")>{new}</span>
                    <span class=format!("min-w-0 px-2 break-words whitespace-pre-wrap {tint} {}", ink(line.kind))>
                        {format!("{sign} {}", shown(&line.text))}
                    </span>
                }
                .into_any(),
            );
        }
        remaining -= take;
    }
    view! { <div class="grid grid-cols-[3.5rem_3.5rem_minmax(0,1fr)]">{cells}</div> }.into_any()
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
pub(super) fn change_glyph(
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
