//! Starting a project, one decision at a time.
//!
//! A wizard, not a form. The earlier version put every chip, every runtime,
//! every option and every explanation on one scrolling page, which is a
//! settings screen wearing a wizard's name — and it made the reader do the
//! filtering that the tool exists to do for them.
//!
//! Explanations are attached to whatever is selected rather than printed under
//! every row. The consequence of choosing an ESP32 is worth a paragraph; the
//! consequences of the nine parts you did not choose are worth nothing, and
//! showing all ten at once buries the one that matters.

use leptos::prelude::*;

use rusty_embed::{Chip, Runtime, WizardChoice};

use crate::{
    controller,
    state::AppState,
    view::components::{Button, ButtonKind, CommandLine, Pill, Tone},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Chip,
    Runtime,
    Options,
    Review,
}

impl Step {
    const ALL: [Step; 4] = [Step::Chip, Step::Runtime, Step::Options, Step::Review];

    fn label(self) -> &'static str {
        match self {
            Step::Chip => "Chip",
            Step::Runtime => "Runtime",
            Step::Options => "Options",
            Step::Review => "Review",
        }
    }

    fn next(self) -> Option<Step> {
        match self {
            Step::Chip => Some(Step::Runtime),
            Step::Runtime => Some(Step::Options),
            Step::Options => Some(Step::Review),
            Step::Review => None,
        }
    }

    fn previous(self) -> Option<Step> {
        match self {
            Step::Chip => None,
            Step::Runtime => Some(Step::Chip),
            Step::Options => Some(Step::Runtime),
            Step::Review => Some(Step::Options),
        }
    }
}

#[component]
pub fn Wizard() -> impl IntoView {
    let state = AppState::expect();
    let step = RwSignal::new(Step::Chip);

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            if state.wizard_options.with(Vec::is_empty) {
                controller::load_wizard_options(state);
            }
            if state.wizard_choice.with(Option::is_none)
                && let Some(chip) = state.chips.with(|c| c.first().map(|c| c.id.clone()))
            {
                controller::choose(
                    state,
                    WizardChoice {
                        chip,
                        runtime: Runtime::BareMetal,
                        name: "firmware".to_string(),
                        options: Vec::new(),
                    },
                );
            }
        }
    });

    move || {
        let Some(choice) = state.wizard_choice.get() else {
            return view! {
                <p class="p-6 text-callout text-label-2">"Loading the chip catalogue…"</p>
            }
            .into_any();
        };

        view! {
            <div class="flex min-h-0 flex-1 flex-col">
                <Steps step=step />
                <div class="min-h-0 flex-1 overflow-y-auto">
                    {match step.get() {
                        Step::Chip => view! { <ChipStep choice=choice.clone() /> }.into_any(),
                        Step::Runtime => view! { <RuntimeStep choice=choice.clone() /> }.into_any(),
                        Step::Options => view! { <OptionsStep choice=choice.clone() /> }.into_any(),
                        Step::Review => view! { <ReviewStep choice=choice.clone() /> }.into_any(),
                    }}
                </div>
                <Footer step=step />
            </div>
        }
        .into_any()
    }
}

/// Where you are and how far there is to go.
#[component]
fn Steps(step: RwSignal<Step>) -> impl IntoView {
    view! {
        <div class="flex flex-none items-center gap-1 border-b border-line px-3 py-2">
            {Step::ALL
                .into_iter()
                .enumerate()
                .map(|(index, this)| {
                    // Bound out of the macro: a bare `>` in an attribute value
                    // is read as the tag's closing bracket, so the comparison
                    // would silently become the attribute and the error lands
                    // on a type mismatch nowhere near the cause.
                    let unreached = move || this > step.get();
                    view! {
                        {(index > 0)
                            .then(|| {
                                view! { <span class="mx-0.5 h-px w-4 bg-line" /> }
                            })}
                        // Earlier steps stay clickable. A wizard that will only
                        // go forwards makes people restart it to change their
                        // mind about the first question.
                        <button
                            type="button"
                            on:click=move |_| step.set(this)
                            disabled=unreached
                            class=move || {
                                let base = "flex items-center gap-1.5 rounded-[6px] px-2 py-0.5 \
                                            text-callout transition-colors \
                                            disabled:pointer-events-none disabled:text-label-3";
                                if step.get() == this {
                                    format!("{base} bg-selection font-medium text-rust")
                                } else {
                                    format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                }
                            }
                        >
                            <span class="tnum font-mono text-footnote">{format!("{}", index + 1)}</span>
                            {this.label()}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
fn Footer(step: RwSignal<Step>) -> impl IntoView {
    view! {
        <div class="flex flex-none items-center gap-2 border-t border-line px-4 py-2.5">
            {move || {
                step.get()
                    .previous()
                    .map(|previous| {
                        view! {
                            <Button
                                label="Back"
                                on_click=Callback::new(move |_| step.set(previous))
                            />
                        }
                    })
            }}
            <span class="flex-1" />
            {move || {
                step.get()
                    .next()
                    .map(|next| {
                        view! {
                            <Button
                                label="Next"
                                kind=ButtonKind::Primary
                                on_click=Callback::new(move |_| step.set(next))
                            />
                        }
                    })
            }}
        </div>
    }
}

/// A step's working area: a list on the left, the detail for whatever is
/// selected on the right.
///
/// The split is the whole point. Ten rows of prose is a wall; ten rows plus one
/// paragraph is a choice with its consequence attached.
/// Change one field of the choice in flight and ask what it now means.
///
/// Every control on every step does exactly this, so it is worth a name: the
/// alternative is four copies of the same clone-mutate-resubmit, and the day
/// one of them forgets to re-explain is the day the wizard lies.
fn amend(state: AppState, edit: impl FnOnce(&mut WizardChoice)) {
    if let Some(mut next) = state.wizard_choice.get_untracked() {
        edit(&mut next);
        controller::choose(state, next);
    }
}

/// Turn an option on, and everything it cannot work without.
///
/// Silently, and shown as such: the requirements appear ticked immediately, so
/// the state on screen is the state that will be generated. Offering `wifi`
/// alone and failing at the end — which is what happened — makes the user
/// decode `Invalid options provided` after they have already chosen a folder.
fn turn_on(choice: &mut WizardChoice, id: &str) {
    for option in std::iter::once(id.to_string()).chain(wizard_requirements(id)) {
        if !choice.options.contains(&option) {
            choice.options.push(option);
        }
    }
}

/// Turn an option off, and anything that needed it.
fn turn_off(choice: &mut WizardChoice, id: &str) {
    choice.options.retain(|option| {
        option != id && !wizard_requirements(option).iter().any(|r| r == id)
    });
}

/// What an option needs, from the catalogue the backend sent.
///
/// A lookup, not a graph walk: `requires` arrives already closed over, so there
/// is no second traversal here to disagree with the one the generator's plan is
/// checked against.
fn wizard_requirements(id: &str) -> Vec<String> {
    AppState::expect().wizard_options.with(|options| {
        options
            .iter()
            .find(|option| option.id == id)
            .map(|option| option.requires.clone())
            .unwrap_or_default()
    })
}

#[component]
fn Split(list: AnyView, detail: AnyView) -> impl IntoView {
    view! {
        <div class="flex h-full min-h-0">
            <div class="min-w-0 flex-1 overflow-y-auto p-2">{list}</div>
            <div class="w-[300px] flex-none overflow-y-auto border-l border-line bg-sidebar px-4 py-3">
                {detail}
            </div>
        </div>
    }
}

#[component]
fn DetailHeading(#[prop(into)] title: String) -> impl IntoView {
    view! {
        <div class="mb-1.5 text-body font-semibold tracking-tight">{title}</div>
    }
}

#[component]
fn ChipStep(choice: WizardChoice) -> impl IntoView {
    let state = AppState::expect();
    // Search, because the catalogue is ten parts today and the STM32 range
    // alone is hundreds. A picker that only works at ten is one that has to be
    // rewritten the first time somebody adds a vendor.
    let query = RwSignal::new(String::new());
    let selected = choice.chip.clone();

    let chosen = Signal::derive(move || {
        let id = state.wizard_choice.with(|c| c.as_ref().map(|c| c.chip.clone()));
        state
            .chips
            .with(|chips| chips.iter().find(|c| Some(&c.id) == id.as_ref()).cloned())
    });

    let list = view! {
        <input
            class="mb-2 h-[28px] w-full rounded-[6px] bg-sunken px-2.5 text-callout outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
            placeholder="Filter by name, architecture or radio…"
            on:input=move |event| query.set(event_target_value(&event))
        />
            {move || {
                let needle = query.get().to_lowercase();
                let current = selected.clone();
                state
                    .chips
                    .get()
                    .into_iter()
                    .filter(|chip| {
                        needle.is_empty()
                            || chip.name.to_lowercase().contains(&needle)
                            || chip.arch.label().to_lowercase().contains(&needle)
                            || chip.radios.iter().any(|r| r.to_lowercase().contains(&needle))
                    })
                    .map(|chip| {
                        let is_selected = chip.id == current;
                        let pick = chip.id.clone();
                        view! {
                            <button
                                type="button"
                                on:click=move |_| {
                                    amend(state, |next| next.chip = pick.clone())
                                }
                                class=move || {
                                    let base = "flex w-full items-center gap-2 rounded-[6px] \
                                                px-2 py-1.5 text-left transition-colors";
                                    if is_selected {
                                        format!("{base} bg-selection text-rust")
                                    } else {
                                        format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                    }
                                }
                            >
                                <span class="flex-1 truncate text-body font-medium">
                                    {chip.name.clone()}
                                </span>
                                <span class="shrink-0 text-footnote text-label-3">
                                    {chip.arch.label()}
                                </span>
                            </button>
                        }
                    })
                    .collect_view()
            }}
    }
    .into_any();

    let detail = view! {
        {move || {
            chosen
                .get()
                .map(|chip| view! { <ChipDetail chip=chip /> })
        }}
    }
    .into_any();

    view! { <Split list=list detail=detail /> }
}

#[component]
fn ChipDetail(chip: Chip) -> impl IntoView {
    let forked = chip.needs_esp_toolchain();
    let radios = chip.radios.join(", ");
    let sram = crate::format::bytes(chip.sram_bytes as u64);
    let flash = chip.flash_bytes.map(|b| crate::format::bytes(b as u64));

    view! {
        <DetailHeading title=chip.name.clone() />
        <div class="mb-3 flex flex-wrap gap-1.5">
            <Pill label=chip.arch.label() />
            <Pill label=format!("{} core", chip.cores) />
            {forked
                .then(|| view! { <Pill label="needs espup" tone=Tone::Amber /> })}
        </div>

        // The single fact that most changes what the next hour looks like. It
        // belongs here, on the part that has it, not as a badge on ten rows.
        <p class="mb-3 text-callout leading-relaxed text-label-2">
            {if forked {
                "Upstream Rust cannot emit code for this core. Building it needs Espressif's \
                 forked LLVM, which espup installs as a separate toolchain — a download measured \
                 in gigabytes before the first build."
            } else {
                "Stock Rust targets this core. One `rustup target add` and it builds — no forked \
                 compiler, no extra toolchain to keep current."
            }}
        </p>

        <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-footnote">
            <dt class="text-label-3">"SRAM"</dt>
            <dd class="m-0 font-mono text-label-2">{sram}</dd>
            {flash
                .map(|flash| {
                    view! {
                        <dt class="text-label-3">"Flash"</dt>
                        <dd class="m-0 font-mono text-label-2">{flash}</dd>
                    }
                })}
            <dt class="text-label-3">"Target"</dt>
            <dd class="m-0 font-mono text-label-2 select-text">{chip.bare_metal_target.clone()}</dd>
            {(!radios.is_empty())
                .then(|| {
                    view! {
                        <dt class="text-label-3">"Radios"</dt>
                        <dd class="m-0 text-label-2">{radios}</dd>
                    }
                })}
        </dl>
    }
}

#[component]
fn RuntimeStep(choice: WizardChoice) -> impl IntoView {
    let state = AppState::expect();
    let selected = choice.runtime;

    // `std` is not offered for a part that has no `std` target. Listing it and
    // failing at the end is the "plausible answer, wrong path" failure this
    // whole workbench exists to avoid.
    let available = Signal::derive(move || {
        let chip = state.wizard_choice.with(|c| c.as_ref().map(|c| c.chip.clone()));
        state.chips.with(|chips| {
            chips
                .iter()
                .find(|c| Some(&c.id) == chip.as_ref())
                .map(|c| c.std_target.is_some())
                .unwrap_or(false)
        })
    });

    let list = view! {
        {[Runtime::BareMetal, Runtime::EspIdf]
                .into_iter()
                .map(|runtime| {
                    let disabled = Signal::derive(move || {
                        runtime == Runtime::EspIdf && !available.get()
                    });
                    view! {
                        <button
                            type="button"
                            disabled=move || disabled.get()
                            on:click=move |_| amend(state, |next| next.runtime = runtime)
                            class=move || {
                                let base = "flex w-full items-center gap-2 rounded-[6px] px-2 \
                                            py-1.5 text-left transition-colors \
                                            disabled:pointer-events-none disabled:opacity-35";
                                if runtime == selected {
                                    format!("{base} bg-selection text-rust")
                                } else {
                                    format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                }
                            }
                        >
                            <span class="flex-1 text-body font-medium">{runtime.label()}</span>
                            {move || {
                                (runtime == Runtime::EspIdf && !available.get())
                                    .then(|| {
                                        view! {
                                            <span class="text-footnote text-label-3">
                                                "not available for this chip"
                                            </span>
                                        }
                                    })
                            }}
                        </button>
                    }
                })
                .collect_view()}
    }
    .into_any();

    let detail = view! {
        <DetailHeading title=selected.label().to_string() />
        <p class="text-callout leading-relaxed text-label-2">
            {match selected {
                Runtime::BareMetal => {
                    "No operating system and no C toolchain. Builds take seconds, the binary \
                     is small enough to reason about, and everything the chip does you call \
                     directly. No threads, no sockets, no filesystem."
                }
                Runtime::EspIdf => {
                    "Links Espressif's C framework, which brings `std`: threads, sockets, a \
                     filesystem. The cost is the whole ESP-IDF build — minutes rather than \
                     seconds — and a much larger image."
                }
            }}
        </p>
    }
    .into_any();

    view! { <Split list=list detail=detail /> }
}

#[component]
fn OptionsStep(choice: WizardChoice) -> impl IntoView {
    let state = AppState::expect();
    // What the pane on the right is describing. Follows the pointer, falls back
    // to whatever was last touched, so the pane is never blank.
    let focused = RwSignal::new(None::<String>);
    let chosen = choice.options.clone();

    let list = view! {
        {move || {
                let chosen = chosen.clone();
                state
                    .wizard_options
                    .get()
                    .into_iter()
                    .map(|option| {
                        let on = chosen.contains(&option.id);
                        let id = option.id.clone();
                        let hover_id = option.id.clone();
                        view! {
                            <button
                                type="button"
                                on:mouseenter=move |_| focused.set(Some(hover_id.clone()))
                                on:click=move |_| {
                                    amend(
                                        state,
                                        |next| {
                                            if on {
                                                turn_off(next, &id);
                                            } else {
                                                turn_on(next, &id);
                                            }
                                        },
                                    )
                                }
                                class=move || {
                                    let base = "flex w-full items-center gap-2.5 rounded-[6px] \
                                                px-2 py-1.5 text-left transition-colors";
                                    if on {
                                        format!("{base} bg-selection text-rust")
                                    } else {
                                        format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                    }
                                }
                            >
                                <span class=move || {
                                    let base = "grid size-[14px] shrink-0 place-items-center \
                                                rounded-[3px] ring-1";
                                    if on {
                                        format!("{base} bg-rust text-white ring-rust")
                                    } else {
                                        format!("{base} ring-line-strong")
                                    }
                                }>
                                    {on
                                        .then(|| {
                                            view! {
                                                <svg width="9" height="9" viewBox="0 0 10 10" aria-hidden="true">
                                                    <path
                                                        d="M1.5 5.2l2.4 2.4L8.5 3"
                                                        fill="none"
                                                        stroke="currentColor"
                                                        stroke-width="1.6"
                                                    />
                                                </svg>
                                            }
                                        })}
                                </span>
                                <span class="flex-1 truncate text-body">{option.label.clone()}</span>
                                <span class="shrink-0 font-mono text-footnote text-label-3">
                                    {option.id.clone()}
                                </span>
                            </button>
                        }
                    })
                    .collect_view()
            }}
    }
    .into_any();

    let detail = view! {
        {move || {
                let id = focused.get();
                let option = state.wizard_options.with(|options| {
                    id.as_ref()
                        .and_then(|id| options.iter().find(|o| &o.id == id))
                        .cloned()
                });
                match option {
                    Some(option) => {
                        let requires = option.requires.clone();
                        view! {
                            <DetailHeading title=option.label.clone() />
                            <p class="text-callout leading-relaxed text-label-2">{option.detail}</p>
                            // Said before it happens, not discovered after. The
                            // extra ticks would otherwise look like a bug.
                            {(!requires.is_empty())
                                .then(|| {
                                    view! {
                                        <p class="mt-2 text-footnote leading-relaxed text-label-3">
                                            "Turns on "
                                            <span class="font-mono">{requires.join(", ")}</span>
                                            " as well — it does not work without them."
                                        </p>
                                    }
                                })}
                        }
                            .into_any()
                    }
                    None => {
                        view! {
                            <p class="text-callout leading-relaxed text-label-3">
                                "Point at an option to see what it commits the project to."
                            </p>
                        }
                            .into_any()
                    }
                }
            }}
    }
    .into_any();

    view! { <Split list=list detail=detail /> }
}

/// Name it, read the consequences, copy the command.
#[component]
fn ReviewStep(choice: WizardChoice) -> impl IntoView {
    let state = AppState::expect();
    let name = choice.name.clone();
    let summary_chip = choice.chip.clone();
    let summary_runtime = choice.runtime.label();
    let summary_options = choice.options.clone();

    view! {
        <div class="mx-auto max-w-[70ch] px-4 py-4">
            <label class="mb-4 block">
                <span class="mb-1.5 block text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    "Project name"
                </span>
                <input
                    class="h-[30px] w-[280px] rounded-[6px] bg-sunken px-2.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust"
                    prop:value=name
                    on:input=move |event| {
                        let value = event_target_value(&event);
                        amend(state, |next| next.name = value);
                    }
                />
            </label>

            <div class="mb-4 flex flex-wrap items-center gap-1.5">
                <Pill label=summary_chip tone=Tone::Rust />
                <Pill label=summary_runtime />
                {summary_options
                    .into_iter()
                    .map(|option| view! { <Pill label=option /> })
                    .collect_view()}
            </div>

            {move || {
                let explanations = state.wizard_explanations.get();
                (!explanations.is_empty())
                    .then(|| {
                        view! {
                            <div class="mb-4 rounded-[8px] bg-sunken/60 px-3 py-2.5">
                                {explanations
                                    .into_iter()
                                    .map(|explanation| {
                                        view! {
                                            <div class="border-b border-line py-1.5 first:pt-0 last:border-b-0 last:pb-0">
                                                <div class="text-callout font-medium">
                                                    {explanation.topic}
                                                </div>
                                                <p class="mt-0.5 text-footnote leading-relaxed text-label-2">
                                                    {explanation.detail}
                                                </p>
                                                {explanation
                                                    .consequence
                                                    .map(|c| {
                                                        view! {
                                                            <div class="mt-1 font-mono text-footnote text-label-3 select-text">
                                                                "→ "{c}
                                                            </div>
                                                        }
                                                    })}
                                            </div>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })
            }}

            {move || {
                let Some(plan) = state.wizard_plan.get() else {
                    return view! {
                        <p class="text-callout text-label-2">
                            "This combination has no generator command — the terminal below says \
                             why."
                        </p>
                    }
                        .into_any();
                };
                let busy = state.session_running.get();
                let named = state
                    .wizard_choice
                    .with(|c| c.as_ref().is_some_and(|c| !c.name.trim().is_empty()));

                // Check the generator is there *before* offering the button.
                // The toolchain report already probed every tool rusty drives,
                // so this costs nothing — and finding out after choosing a
                // folder is finding out one step too late.
                let generator = plan.program.clone();
                let missing = state.toolchain.with(|report| {
                    report.as_ref().and_then(|report| {
                        report
                            .status
                            .tools
                            .iter()
                            .find(|tool| tool.name == generator)
                            .filter(|tool| !tool.is_installed())
                            .map(|tool| tool.install_command.clone())
                    })
                });

                if let Some(install) = missing {
                    let run = install.clone();
                    return view! {
                        <div class="rounded-[8px] bg-amber-fill px-3 py-2.5">
                            <div class="text-body font-medium">
                                {format!("`{}` is not installed", plan.program)}
                            </div>
                            <p class="mt-0.5 text-callout leading-relaxed text-label-2">
                                "It is what generates the project. Installing it is a one-off."
                            </p>
                            <div class="mt-2 flex items-center gap-2">
                                <Button
                                    label="Install it"
                                    kind=ButtonKind::Primary
                                    disabled=Signal::derive(move || busy)
                                    on_click=Callback::new(move |_| {
                                        controller::install_tool(state, run.clone())
                                    })
                                />
                                <span class="text-footnote text-label-3">
                                    "runs in the terminal below"
                                </span>
                            </div>
                            <div class="mt-2">
                                <CommandLine command=install />
                            </div>
                        </div>
                    }
                        .into_any();
                }

                view! {
                    <div class="flex items-center gap-2">
                        <Button
                            label="Choose a folder and create"
                            kind=ButtonKind::Primary
                            disabled=Signal::derive(move || busy || !named)
                            on_click=Callback::new(move |_| {
                                if let Some(choice) = state.wizard_choice.get_untracked() {
                                    controller::create_project(state, choice);
                                }
                            })
                        />
                        <span class="text-footnote text-label-3">
                            "rusty runs it and opens the result."
                        </span>
                    </div>

                    // The command stays visible underneath. Showing it is what
                    // makes the tool checkable and pasteable into a bug report;
                    // showing it *instead* of acting was the mistake.
                    <div class="mt-3">
                        <CommandLine command=plan.display />
                    </div>
                }
                    .into_any()
            }}
        </div>
    }
}
