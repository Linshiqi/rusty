//! The Disk section of the Crates panel: where this project's builds went,
//! what of it is dead weight, and the two settings that keep it that way.
//!
//! Numbers first, then the removals, then the advice. Every removal button
//! sits on the row it acts on and says what it removes; the sweep of stale
//! artifacts has the table as its preview, the whole-tree and cache removals
//! ask with the size. Nothing here computes what is stale — the backend does,
//! and recomputes it when asked to remove.

use leptos::prelude::*;

use rusty_core::{BuildTree, DiskItem, DiskKind, DiskReport};
use rusty_i18n::t;

use crate::view::components::copy_to_clipboard;
use crate::view::icon::{Icon, IconView};
use crate::{controller, format, state::AppState};

const BUTTON: &str = "shrink-0 rounded-[6px] px-2 py-0.5 text-footnote ring-1 ring-line hover:bg-sunken disabled:pointer-events-none disabled:opacity-40";
const DANGER: &str = "shrink-0 rounded-[6px] px-2 py-0.5 text-footnote text-crimson ring-1 ring-line hover:bg-sunken disabled:pointer-events-none disabled:opacity-40";

#[component]
pub fn Disk() -> impl IntoView {
    let state = AppState::expect();
    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.project.disk.with(Option::is_none) {
            controller::load_disk_report(state);
            controller::load_disk_auto_sweep(state);
        }
    });
    let busy = state.project.disk_busy;

    view! {
        <div class="flex items-center gap-2 px-5 py-2">
            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                {t!("disk.title")}
            </span>
            <span class="flex-1" />
            <button
                type="button"
                title=t!("disk.rescan")
                disabled=move || busy.get()
                on:click=move |_| controller::load_disk_report(state)
                class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label disabled:opacity-40"
            >
                <IconView icon=Icon::Refresh size=13 />
            </button>
        </div>
        <div class="flex flex-col gap-4 px-5 pb-4">
            {move || {
                let Some(report) = state.project.disk.get() else {
                    return view! {
                        <p class="text-callout text-label-3">
                            {if busy.get() { t!("disk.scanning") } else { t!("disk.not-yet") }}
                        </p>
                    }
                        .into_any();
                };
                view! { <Report report=report /> }.into_any()
            }}
        </div>
    }
}

#[component]
fn Report(report: DiskReport) -> impl IntoView {
    let state = AppState::expect();
    let busy = state.project.disk_busy;
    let auto = state.project.disk_auto_sweep;
    let idle = state.project.disk_idle_days;

    let stale_total: u64 = report
        .trees
        .iter()
        .flat_map(|t| t.groups.iter())
        .map(|g| g.stale_bytes)
        .sum();
    let any_locked = report.trees.iter().any(|t| t.locked);
    let low = report.volume.is_some_and(|v| {
        v.free_bytes < 5 * 1024 * 1024 * 1024 || v.free_bytes * 20 < v.total_bytes
    });
    let volume_line = report.volume.map(|v| {
        t!(
            "disk.volume",
            free = format::bytes(v.free_bytes),
            total = format::bytes(v.total_bytes)
        )
    });
    let shared = report.shared;
    let target_dir = report.target_dir.clone();
    let total = t!(
        "disk.total",
        size = format::bytes(report.total_bytes),
        files = report.files.to_string()
    );
    let trees = report.trees.clone();
    let extras = report.extras.clone();
    let caches = report.cargo_home.clone();
    let warnings = report.warnings.clone();
    let debuginfo = report.debuginfo_bytes;
    let share_snippet = report.cargo_home_dir.as_ref().map(|home| {
        let dir = format!(
            "{}{}target",
            home,
            if home.contains('\\') { "\\" } else { "/" }
        );
        (
            format!("[build]\ntarget-dir = \"{}\"", dir.replace('\\', "\\\\")),
            format!(
                "{}{}config.toml",
                home,
                if home.contains('\\') { "\\" } else { "/" }
            ),
        )
    });
    let debug_snippet = "[profile.dev]\ndebug = \"line-tables-only\"".to_string();

    view! {
        // The two numbers that decide whether anything below matters.
        <div class="flex flex-col gap-1">
            {volume_line.map(|line| {
                view! {
                    <p class=if low { "text-callout text-crimson" } else { "text-callout text-label" }>
                        {line}
                    </p>
                }
            })}
            <p class="flex flex-wrap items-baseline gap-x-2 text-footnote text-label-2">
                <span class="font-mono text-label-3">{target_dir}</span>
                <span>{total}</span>
                <span class=if shared { "text-patina" } else { "text-label-3" }>
                    {if shared { t!("disk.shared") } else { t!("disk.not-shared") }}
                </span>
            </p>
        </div>

        // One row per build tree.
        {(!trees.is_empty()).then(|| {
            view! {
                <div>
                    <div class="flex items-baseline gap-3 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        <span class="w-[30ch] shrink-0">{t!("disk.tree")}</span>
                        <span class="w-[9ch] shrink-0 text-right">{t!("disk.size")}</span>
                        <span class="w-[9ch] shrink-0 text-right">{t!("disk.stale")}</span>
                        <span class="flex-1" />
                    </div>
                    {trees.into_iter().map(|tree| view! { <TreeRow tree=tree /> }).collect_view()}
                    <div class="flex items-baseline gap-3 pt-2">
                        <span class="min-w-0 flex-1 text-footnote text-label-2">
                            {t!("disk.stale-total", size = format::bytes(stale_total))}
                        </span>
                        <label class="flex shrink-0 items-center gap-1 text-footnote text-label-3">
                            <span>{t!("disk.idle-after")}</span>
                            <select
                                on:change=move |event| {
                                    if let Ok(days) = event_target_value(&event).parse::<u32>() {
                                        controller::set_disk_idle_days(state, days);
                                    }
                                }
                                class="rounded-[6px] bg-sunken px-1.5 py-0.5 text-footnote text-label-2 ring-1 ring-line"
                            >
                                {[1u32, 3, 7, 30]
                                    .into_iter()
                                    .map(|days| {
                                        view! {
                                            <option value=days.to_string() selected=move || idle.get() == days>
                                                {t!("disk.days", days = days.to_string())}
                                            </option>
                                        }
                                    })
                                    .collect_view()}
                            </select>
                        </label>
                        <button
                            type="button"
                            disabled=move || busy.get() || stale_total == 0
                            on:click=move |_| controller::sweep_disk(state, None)
                            class="shrink-0 rounded-[6px] bg-rust px-2 py-0.5 text-footnote font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                        >
                            {t!("disk.sweep-all")}
                        </button>
                    </div>
                    {any_locked.then(|| view! {
                        <p class="pt-1 text-footnote text-amber">{t!("disk.locked-note")}</p>
                    })}
                </div>
            }
        })}

        {(!extras.is_empty()).then(|| {
            view! {
                <div>
                    <p class="pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        {t!("disk.extras")}
                    </p>
                    {extras
                        .into_iter()
                        .map(|item| {
                            let name = rusty_i18n::translate(&format!("disk.extra.{}", item.label))
                                .unwrap_or_else(|| item.label.clone());
                            view! { <ItemRow item=item name=name cache=false /> }
                        })
                        .collect_view()}
                </div>
            }
        })}

        {(!caches.is_empty()).then(|| {
            view! {
                <div>
                    <p class="pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        {t!("disk.caches")}
                    </p>
                    {caches
                        .into_iter()
                        .map(|item| {
                            let name = rusty_i18n::translate(&format!("disk.cache.{}", item.label))
                                .unwrap_or_else(|| item.label.clone());
                            view! { <ItemRow item=item name=name cache=true /> }
                        })
                        .collect_view()}
                </div>
            }
        })}

        // Prevention: the switch, and the two settings that shrink what a
        // build writes in the first place.
        <label class="flex items-center gap-2 text-footnote text-label-2">
            <input
                type="checkbox"
                prop:checked=move || auto.get()
                on:change=move |event| {
                    controller::set_disk_auto_sweep(state, event_target_checked(&event));
                }
                class="accent-rust"
            />
            <span>{t!("disk.auto-sweep")}</span>
        </label>

        {(!shared).then(|| {
            let (snippet, file) = share_snippet.clone().unwrap_or_default();
            let has_snippet = !snippet.is_empty();
            let to_copy = snippet.clone();
            view! {
                <div class="flex flex-col gap-2 rounded-[8px] bg-sunken p-3">
                    <p class="text-footnote text-label-2">{t!("disk.share-advice")}</p>
                    {has_snippet.then(|| view! {
                        <pre class="overflow-x-auto font-mono text-footnote text-label">{snippet.clone()}</pre>
                        <div class="flex items-center gap-2">
                            <button
                                type="button"
                                on:click=move |_| copy_to_clipboard(&to_copy)
                                class=BUTTON
                            >
                                {t!("disk.copy")}
                            </button>
                            <span class="min-w-0 truncate text-caption text-label-3">
                                {t!("disk.share-where", path = file.clone())}
                            </span>
                        </div>
                    })}
                </div>
            }
        })}

        {(debuginfo > 0).then(|| {
            let to_copy = debug_snippet.clone();
            view! {
                <div class="flex flex-col gap-2 rounded-[8px] bg-sunken p-3">
                    <p class="text-footnote text-label-2">
                        {t!("disk.debuginfo-advice", size = format::bytes(debuginfo))}
                    </p>
                    <pre class="overflow-x-auto font-mono text-footnote text-label">{debug_snippet.clone()}</pre>
                    <button
                        type="button"
                        on:click=move |_| copy_to_clipboard(&to_copy)
                        class=BUTTON
                    >
                        {t!("disk.copy")}
                    </button>
                </div>
            }
        })}

        {(!warnings.is_empty()).then(|| {
            view! {
                <div class="flex flex-col gap-1">
                    <p class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        {t!("disk.warnings")}
                    </p>
                    {warnings
                        .into_iter()
                        .map(|w| view! { <p class="text-caption text-label-3">{w}</p> })
                        .collect_view()}
                </div>
            }
        })}
    }
}

#[component]
fn TreeRow(tree: BuildTree) -> impl IntoView {
    let state = AppState::expect();
    let busy = state.project.disk_busy;
    let stale: u64 = tree.groups.iter().map(|g| g.stale_bytes).sum();
    let label = match &tree.triple {
        Some(triple) => format!("{triple} / {}", tree.profile),
        None => format!("{} / {}", t!("disk.host"), tree.profile),
    };
    // What the tree is made of, for the tooltip: one figure per kind.
    let breakdown = tree
        .groups
        .iter()
        .map(|g| format!("{} {}", kind_name(g.kind), format::bytes(g.bytes)))
        .collect::<Vec<_>>()
        .join(" · ");
    // Why the stale bytes are stale, in the row itself.
    let mut by_reason: Vec<(String, u64)> = Vec::new();
    for group in &tree.groups {
        for summary in &group.stale_by_reason {
            match by_reason.iter_mut().find(|(r, _)| *r == summary.reason) {
                Some((_, bytes)) => *bytes += summary.bytes,
                None => by_reason.push((summary.reason.clone(), summary.bytes)),
            }
        }
    }
    by_reason.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let reasons = by_reason
        .iter()
        .map(|(reason, bytes)| {
            format!(
                "{} {}",
                rusty_i18n::translate(&format!("disk.reason.{reason}"))
                    .unwrap_or_else(|| reason.clone()),
                format::bytes(*bytes)
            )
        })
        .collect::<Vec<_>>()
        .join(" · ");
    // Bound above the macro: a `>` inside an attribute value ends the tag.
    let stale_class = if stale > 0 {
        "tnum w-[9ch] shrink-0 text-right font-mono text-footnote text-amber"
    } else {
        "tnum w-[9ch] shrink-0 text-right font-mono text-footnote text-label-3"
    };
    let locked = tree.locked;
    let (sweep_path, remove_path) = (tree.path.clone(), tree.path.clone());
    let incremental_bytes = tree
        .groups
        .iter()
        .find(|g| g.kind == DiskKind::Incremental)
        .map_or(0, |g| g.bytes);
    let incremental_path = tree.path.clone();
    let bytes = tree.bytes;

    view! {
        <div class="flex items-baseline gap-3 border-b border-line py-1.5 last:border-b-0" title=breakdown>
            <span class="w-[30ch] shrink-0 truncate font-mono text-footnote text-label">{label}</span>
            <span class="tnum w-[9ch] shrink-0 text-right font-mono text-footnote text-label-2">
                {format::bytes(tree.bytes)}
            </span>
            <span class=stale_class>
                {if stale > 0 { format::bytes(stale) } else { "—".to_string() }}
            </span>
            <span class="min-w-0 flex-1 truncate text-footnote text-label-3">{reasons}</span>
            {if locked {
                view! { <span class="shrink-0 text-footnote text-amber">{t!("disk.locked")}</span> }.into_any()
            } else {
                view! {
                    <button
                        type="button"
                        title=t!("disk.sweep-tree-hint")
                        disabled=move || busy.get() || stale == 0
                        on:click=move |_| controller::sweep_disk(state, Some(sweep_path.clone()))
                        class=BUTTON
                    >
                        {t!("disk.sweep-tree")}
                    </button>
                    {(incremental_bytes > 0).then(|| view! {
                        <button
                            type="button"
                            title=t!("disk.drop-incremental-hint")
                            disabled=move || busy.get()
                            on:click=move |_| {
                                controller::drop_incremental(
                                    state,
                                    incremental_path.clone(),
                                    incremental_bytes,
                                )
                            }
                            class=BUTTON
                        >
                            {t!("disk.drop-incremental", size = format::bytes(incremental_bytes))}
                        </button>
                    })}
                    <button
                        type="button"
                        title=t!("disk.remove-tree-hint")
                        disabled=move || busy.get()
                        on:click=move |_| controller::remove_disk_path(state, remove_path.clone(), bytes)
                        class=DANGER
                    >
                        {t!("disk.remove-tree")}
                    </button>
                }
                    .into_any()
            }}
        </div>
    }
}

#[component]
fn ItemRow(item: DiskItem, name: String, cache: bool) -> impl IntoView {
    let state = AppState::expect();
    let busy = state.project.disk_busy;
    let (label, path, bytes, removable) = (item.label, item.path, item.bytes, item.removable);
    let shown_path = path.clone();
    view! {
        <div class="flex items-baseline gap-3 border-b border-line py-1.5 last:border-b-0" title=shown_path>
            <span class="w-[30ch] shrink-0 truncate text-footnote text-label">{name}</span>
            <span class="tnum w-[9ch] shrink-0 text-right font-mono text-footnote text-label-2">
                {format::bytes(bytes)}
            </span>
            <span class="flex-1" />
            {removable.then(|| view! {
                <button
                    type="button"
                    disabled=move || busy.get()
                    on:click=move |_| {
                        if cache {
                            controller::remove_disk_cache(state, label.clone(), bytes);
                        } else {
                            controller::remove_disk_path(state, path.clone(), bytes);
                        }
                    }
                    class=DANGER
                >
                    {t!("disk.remove")}
                </button>
            })}
        </div>
    }
}

fn kind_name(kind: DiskKind) -> String {
    match kind {
        DiskKind::Deps => t!("disk.kind.deps"),
        DiskKind::Incremental => t!("disk.kind.incremental"),
        DiskKind::BuildScripts => t!("disk.kind.build-scripts"),
        DiskKind::Fingerprints => t!("disk.kind.fingerprints"),
        DiskKind::Binaries => t!("disk.kind.binaries"),
        DiskKind::Other => t!("disk.kind.other"),
    }
}
