//! The status bar: one line of the facts checked without looking away —
//! what runs and what it said, the language server, the chip and what the
//! project builds for, the pins — and the chip switch it opens.

use super::*;

/// One segment of the status bar.
#[component]
pub(super) fn Status(
    #[prop(into)] text: String,
    /// A dim prefix naming what the value is.
    #[prop(optional, into)]
    label: Option<String>,
    #[prop(optional)] tone: Option<Tone>,
    #[prop(optional, into)] title: Option<String>,
    #[prop(optional)] on_click: Option<Callback<()>>,
    /// Text whose length nobody here controls — what a server says it is
    /// doing. Cut with an ellipsis at a bounded width, and the first item to
    /// give way when the bar runs out of room; the caller puts the whole of
    /// it in `title`.
    #[prop(optional)]
    clip: bool,
) -> impl IntoView {
    let colour = match tone {
        Some(Tone::Crimson) => "text-crimson",
        Some(Tone::Amber) => "text-amber",
        Some(Tone::Patina) => "text-patina",
        Some(Tone::Rust) => "text-rust",
        _ => "",
    };
    let interactive = if on_click.is_some() {
        "cursor-default hover:bg-sunken hover:text-label"
    } else {
        ""
    };
    // One line, always: the bar is 26 px tall, and a status that wraps
    // spills out of it. Everything else keeps its width, so what gives way
    // when the window is narrow is the clipped item and nothing beside it.
    let width = if clip {
        "min-w-0 max-w-[22rem]"
    } else {
        "shrink-0"
    };
    view! {
        <button
            type="button"
            disabled=on_click.is_none()
            title=title
            on:click=move |_| {
                if let Some(cb) = on_click {
                    cb.run(());
                }
            }
            class=format!(
                "flex h-full items-center gap-1.5 whitespace-nowrap border-r border-line px-3 \
                 transition-colors disabled:pointer-events-none {width} {colour} {interactive}",
            )
        >
            {label.map(|label| view! { <span class="shrink-0 text-label-3">{label}</span> })}
            <span class="min-w-0 truncate">{text}</span>
        </button>
    }
}

/// What this project is built for: the chip in the bar, the rest on click.
///
/// The popover opens upwards because the bar is the last row on screen — a
/// menu that renders below it is a menu nobody sees.
#[component]
pub(super) fn BuiltFor(chip: String, target: String, toolchain: String) -> impl IntoView {
    let open = RwSignal::new(false);
    // The proposed switch, once one has been planned. Held here rather than
    // applied on click: what a chip switch touches is exactly what somebody
    // needs to read before it happens.
    let proposal = RwSignal::new(None::<rusty_embed::Migration>);
    let picking = RwSignal::new(false);
    let current = chip.clone();
    let row = |label: String, value: String, note: String| {
        view! {
            <div class="flex flex-col gap-0.5 px-3 py-1.5">
                <div class="flex items-baseline gap-2">
                    <span class="w-[4.5rem] shrink-0 text-label-3">{label}</span>
                    <span class="min-w-0 break-all text-label select-text">{value}</span>
                </div>
                <span class="pl-[calc(4.5rem+0.5rem)] text-caption text-label-4">{note}</span>
            </div>
        }
    };

    view! {
        // Full width, one line, like every item in the bar (`Status`).
        <div class="relative h-full shrink-0">
            <button
                type="button"
                title=t!("status.built-for-hint")
                on:click=move |_| open.update(|it| *it = !*it)
                class="flex h-full items-center gap-1.5 whitespace-nowrap border-r border-line px-3 transition-colors hover:bg-sunken hover:text-label"
            >
                <span class="text-label-3">{t!("status.chip")}</span>
                {chip}
                <span class="text-label-4">"▴"</span>
            </button>
            {move || {
                open.get()
                    .then(|| {
                        let dismiss = move |_| {
                            open.set(false);
                            picking.set(false);
                            proposal.set(None);
                        };
                        let current = current.clone();
                        view! {
                            // Full-screen catcher, so clicking anywhere else
                            // closes it — the behaviour every menu in here has.
                            <div class="fixed inset-0 z-40" on:click=dismiss />
                            // Sized to what is in it, capped so a migration's
                            // notes wrap instead of running off. A fixed width
                            // wide enough for the plan left two short rows
                            // sitting in an otherwise empty box.
                            <div class="absolute bottom-full left-0 z-50 mb-px max-h-[70vh] w-max max-w-[34rem] min-w-[14rem] overflow-y-auto rounded-t-[8px] border border-line bg-raised py-1.5 shadow-lg">
                                {row(
                                    t!("status.target"),
                                    target.clone(),
                                    t!("status.target-note"),
                                )}
                                {row(
                                    t!("status.toolchain"),
                                    toolchain.clone(),
                                    t!("status.toolchain-note"),
                                )}
                                <div class="my-1 h-px bg-line" />
                                <SwitchChip current=current picking=picking proposal=proposal />
                            </div>
                        }
                    })
            }}
        </div>
    }
}

/// Moving the project to another chip, in three steps that are all reversible
/// until the last one: pick, read what it would do, apply.
///
/// The offer lives beside the chip because that is the fact it changes. The
/// answer to "must I recreate the project" is no for the configuration and
/// yes for anything naming a pin, and the plan says which is which rather
/// than implying it did everything.
#[component]
pub(super) fn SwitchChip(
    current: String,
    picking: RwSignal<bool>,
    proposal: RwSignal<Option<rusty_embed::Migration>>,
) -> impl IntoView {
    let state = AppState::expect();

    view! {
        {move || {
            if let Some(plan) = proposal.get() {
                let blocker = plan.blocker.clone();
                let heading = format!("{} → {}", plan.from, plan.to);
                let files = plan.files.clone();
                let notes = plan.notes.clone();
                let runnable = plan.clone();
                return view! {
                    <div class="px-3 py-1.5">
                        <div class="mb-1.5 font-mono text-footnote text-label">{heading}</div>
                        {blocker
                            .clone()
                            .map(|why| {
                                view! {
                                    <p class="rounded-[6px] bg-amber-fill px-2.5 py-2 text-caption leading-relaxed text-amber select-text">
                                        {why}
                                    </p>
                                }
                            })}
                        {(blocker.is_none())
                            .then(|| {
                                view! {
                                    <div class="mb-1.5 flex flex-col gap-0.5">
                                        {files
                                            .into_iter()
                                            .map(|file| {
                                                let count = file.edits.len();
                                                view! {
                                                    <div class="flex items-baseline justify-between gap-2 font-mono text-caption">
                                                        <span class="text-label-2">{file.path}</span>
                                                        <span class="shrink-0 text-label-4">
                                                            {format!(
                                                                "{count} change{}",
                                                                if count == 1 { "" } else { "s" },
                                                            )}
                                                        </span>
                                                    </div>
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                    <ul class="mb-2 flex flex-col gap-1">
                                        {notes
                                            .into_iter()
                                            .map(|note| {
                                                view! {
                                                    <li class="text-caption leading-relaxed text-label-3 select-text">
                                                        "— "{note}
                                                    </li>
                                                }
                                            })
                                            .collect_view()}
                                    </ul>
                                    <div class="flex gap-2">
                                        <button
                                            type="button"
                                            on:click=move |_| {
                                                controller::apply_migration(
                                                    state,
                                                    runnable.clone(),
                                                    proposal,
                                                )
                                            }
                                            class="rounded-[6px] bg-rust px-2.5 py-1 text-caption text-window transition-opacity hover:opacity-90"
                                        >
                                            {t!("migrate.switch")}
                                        </button>
                                        <button
                                            type="button"
                                            on:click=move |_| proposal.set(None)
                                            class="rounded-[6px] px-2.5 py-1 text-caption text-label-2 transition-colors hover:bg-sunken hover:text-label"
                                        >
                                            {t!("migrate.cancel")}
                                        </button>
                                    </div>
                                }
                            })}
                    </div>
                }
                    .into_any();
            }
            if !picking.get() {
                return view! {
                    <button
                        type="button"
                        on:click=move |_| picking.set(true)
                        class="flex w-full items-center px-3 py-1.5 text-left text-footnote text-label-2 transition-colors hover:bg-sunken hover:text-label"
                    >
                        {t!("migrate.pick")}
                    </button>
                }
                    .into_any();
            }
            let current = current.clone();
            // Which HAL this project's part sits behind. A switch is only
            // mechanical within one, so the list says which rows are a switch
            // and which are a new project — before the click, not after it.
            let ours = state
                .project.chips
                .get()
                .into_iter()
                .find(|chip| chip.id == current)
                .and_then(|chip| chip.hal);
            view! {
                <div class="max-h-56 overflow-y-auto py-0.5">
                    {state
                        .project.chips
                        .get()
                        .into_iter()
                        .filter(|chip| chip.id != current)
                        .map(|chip| {
                            let id = chip.id.clone();
                            let same_hal = chip.hal.is_some() && chip.hal == ours;
                            let detail = if same_hal {
                                format!("{} · {}", chip.arch.label(), chip.bare_metal_target)
                            } else {
                                t!("migrate.different-hal", arch = chip.arch.label())
                            };
                            let tone = if same_hal { "text-label-4" } else { "text-amber" };
                            view! {
                                <button
                                    type="button"
                                    disabled=!same_hal
                                    title=if same_hal {
                                        String::new()
                                    } else {
                                        t!("migrate.different-hal-hint")
                                    }
                                    on:click=move |_| {
                                        controller::plan_migration(state, id.clone(), proposal)
                                    }
                                    class="flex w-full flex-col items-start px-3 py-1 text-left transition-colors hover:bg-sunken disabled:pointer-events-none disabled:opacity-55"
                                >
                                    <span class="font-mono text-footnote text-label">{chip.name}</span>
                                    <span class=format!("font-mono text-caption {tone}")>{detail}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            }
                .into_any()
        }}
    }
}

/// The facts you check without looking away from what you are doing.
///
/// A status bar that only says "idle" is a decoration. This one carries the
/// answers to the questions asked most often while working — what am I
/// targeting, what is stopping the build, what is plugged in — and the problem
/// count opens the dock rather than merely reporting a number.
#[component]
pub(super) fn StatusBar() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <footer class="flex h-[26px] flex-none items-center border-t border-line bg-window font-mono text-footnote text-label-2">
            // What is running, or how the last run went — Xcode's activity
            // view. "Working" for everything was a status bar that said
            // nothing a spinner could not.
            <activity::ActivityStatus />

            {move || {
                let (errors, _) = state.diag_counts();
                // Absence explains itself: no chip means the status has nothing
                // to say, but a missing language server looks like "the editor
                // is broken" unless something names it.
                let lsp = state.lsp.status.get();
                (state.has_project() && lsp != crate::state::LspStatus::Off)
                    .then(|| {
                        // What the server said about itself, and why: the
                        // reason takes the tooltip's place, since a failure
                        // nobody can read is a colour and nothing more.
                        let health = state.lsp.health.get();
                        let title = match &health {
                            Some((_, Some(why))) => why.clone(),
                            _ => t!("status.lsp-hint"),
                        };
                        let (text, tone) = match lsp {
                            crate::state::LspStatus::Starting => {
                                (t!("status.lsp-starting"), Tone::Neutral)
                            }
                            // A server that did not load the workspace comes
                            // before everything else it might be doing: it
                            // still parses, so the squiggles arrive and every
                            // completion, hover and jump is empty for ever,
                            // which is the one broken state that reads as a
                            // working one.
                            _ if matches!(health, Some((rusty_lsp::HealthLevel::Error, _))) => {
                                (t!("status.lsp-broken"), Tone::Crimson)
                            }
                            _ if matches!(health, Some((rusty_lsp::HealthLevel::Warning, _))) => {
                                // "Partly loaded" is true and says nothing
                                // anybody can act on, and for the one warning
                                // rusty recognises it sits there for the
                                // whole session. Where the reason is the
                                // toolchain that cannot resolve dependencies,
                                // the status says *that* — three words the
                                // reader can do something about, with the
                                // whole explanation still in the tooltip.
                                let deps_lost = health.as_ref().is_some_and(|(_, why)| {
                                    why.as_deref()
                                        .is_some_and(|why| why.contains("--lockfile-path"))
                                });
                                let said = if deps_lost {
                                    t!("status.lsp-no-deps")
                                } else {
                                    t!("status.lsp-degraded")
                                };
                                (said, Tone::Amber)
                            }
                            // Busy comes before errors: while the index is
                            // being built, both the diagnostics and the
                            // completions are provisional, and "12 errors"
                            // over a half-loaded workspace is the wrong
                            // headline.
                            // The server's own words, which can run to a crate
                            // name per piece of work or a whole registry path —
                            // clipped in the bar (`Status::clip`), and read in
                            // full in the tooltip.
                            crate::state::LspStatus::Ready if state.lsp.progress.get().is_some() => {
                                let what = state.lsp.progress.get().unwrap_or_default();
                                (format!("rust-analyzer · {what}"), Tone::Amber)
                            }
                            crate::state::LspStatus::Ready if errors > 0 => {
                                (t!("status.lsp-errors", count = errors), Tone::Crimson)
                            }
                            crate::state::LspStatus::Ready => {
                                ("rust-analyzer".to_string(), Tone::Patina)
                            }
                            _ => (t!("status.lsp-missing"), Tone::Crimson),
                        };
                        // A clipped line is only readable whole on hover. The
                        // health reason stays the tooltip whenever there is
                        // one, since then the line says only what is wrong.
                        let title = if health.is_none() && lsp == crate::state::LspStatus::Ready {
                            state
                                .lsp
                                .progress
                                .get()
                                .map_or(title, |_| text.clone())
                        } else {
                            title
                        };
                        view! {
                            <Status
                                text=text
                                tone=tone
                                title=title
                                clip=true
                                on_click=Callback::new(move |_| {
                                    state.show_dock(crate::state::DockTab::Problems)
                                })
                            />
                        }
                    })
            }}

            // The mode, and the half-typed command beside it. A modal editor
            // whose mode is invisible is one where every other keystroke is a
            // guess — this is the first thing a Vim user's eye goes to.
            {move || {
                state
                    .editor.vim_on
                    .get()
                    .then(|| {
                        let (label, hint) = state
                            .editor.vim
                            .with(|vim| (vim.mode.label(), vim.hint()));
                        let tone = match label {
                            "INSERT" => Tone::Patina,
                            "NORMAL" => Tone::Neutral,
                            _ => Tone::Amber,
                        };
                        let text = if hint.is_empty() {
                            label.to_string()
                        } else {
                            format!("{label}  {hint}")
                        };
                        view! {
                            <Status
                                text=text
                                tone=tone
                                title=t!("status.vim-hint")
                            />
                        }
                    })
            }}

            {move || {
                let blocking = state.blocking_count();
                let total = state.problems().len();
                (total > 0)
                    .then(|| {
                        let text = if blocking > 0 {
                            t!("status.blocking", count = blocking)
                        } else {
                            t!("status.notes", count = total)
                        };
                        let tone = if blocking > 0 { Tone::Crimson } else { Tone::Amber };
                        view! {
                            <Status
                                text=text
                                tone=tone
                                title=t!("status.problems-hint")
                                on_click=Callback::new(move |_| {
                                    state.show_dock(crate::state::DockTab::Problems)
                                })
                            />
                        }
                    })
            }}

            {move || {
                state
                    .project.detected
                    .get()
                    .map(|project| {
                        let chip = project
                            .chip
                            .clone()
                            .unwrap_or_else(|| t!("status.no-chip"));
                        let target = project
                            .configured_target
                            .clone()
                            .unwrap_or_else(|| t!("status.no-target"));
                        let toolchain = project
                            .configured_toolchain
                            .clone()
                            .unwrap_or_else(|| t!("status.unpinned"));
                        // One chip, not three. The three values answer one
                        // question — what is this project built for — and the
                        // chip is the part of the answer anyone reads at a
                        // glance; the triple and the channel are what you look
                        // up when something is wrong, which is a click away.
                        //
                        // They were still labelled inline when they sat in the
                        // bar, because three bare values reading "esp32 ·
                        // xtensa-esp32-none-elf · esp" are a riddle. Inside the
                        // popover there is room to label them properly.
                        view! {
                            <BuiltFor chip=chip target=target toolchain=toolchain />
                        }
                    })
            }}

            <span class="flex-1" />

            // The right end is the chip's pins, opening upwards on click.
            // A dependency count and a board count sat here before — two
            // numbers that proved the IPC bridge alive and that nobody acted
            // on — while the pin map floated over the editor's corner as a
            // fixture. The status bar is where a fact about the project that
            // is one click from useful belongs.
            <pinmap::PinStatus />
        </footer>
    }
}
