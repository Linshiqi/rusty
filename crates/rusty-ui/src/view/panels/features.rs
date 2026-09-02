//! What a Cargo feature actually costs.
//!
//! Feature flags are discussed as if each one were independent. Under resolver
//! v2 they are not: cargo unifies features across the whole workspace, so
//! turning one off removes nothing if any other member still wants it. The
//! usual result is someone spending an afternoon disabling features and
//! watching the build stay exactly the same size.
//!
//! So every number here comes from a real resolution of the whole graph rather
//! than from reading the manifest. `marginal_crates` is the column that matters:
//! what flipping *this one switch* costs with every other switch held where it
//! is. That is the question, and it is not answerable from a dependency tree.

use leptos::prelude::*;

use rusty_core::{FeatureImpact, FeatureRow, FeatureSelection};

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{Empty, Pill, Readout, SectionLabel, Tone},
};

#[component]
pub fn Features() -> impl IntoView {
    let state = AppState::expect();

    // Resolve the first member as soon as there is a workspace. Making the user
    // pick a package before seeing anything would mean the panel's first screen
    // is a dropdown, which explains nothing about what the panel is for.
    Effect::new(move |_| {
        if state.project.feature_selection.with(Option::is_none)
            && let Some(first) = state
                .project
                .workspace
                .with(|w| w.as_ref().and_then(|w| w.members.first().cloned()))
        {
            controller::apply_features(
                state,
                FeatureSelection {
                    package: first.name,
                    features: Vec::new(),
                    default_features: true,
                },
            );
        }
    });

    move || {
        let Some(workspace) = state.project.workspace.get() else {
            return view! {
                <Empty
                    title=t!("features.no-analysis-title")
                    detail=t!("features.no-analysis-detail")
                />
            }
            .into_any();
        };

        let members = workspace.members.clone();

        view! {
            <div class="flex-1 overflow-y-auto">
                <PackagePicker members=members />
                {move || {
                    state
                        .project.feature_impact
                        .get()
                        .map(|impact| view! { <Impact impact=impact /> })
                }}
                <Matrix />
            </div>
        }
        .into_any()
    }
}

/// Which workspace member is being resolved for.
#[component]
fn PackagePicker(members: Vec<rusty_core::MemberInfo>) -> impl IntoView {
    let state = AppState::expect();
    let current = Signal::derive(move || {
        state
            .project
            .feature_selection
            .with(|s| s.as_ref().map(|s| s.package.clone()))
    });

    view! {
        <div class="flex flex-wrap items-center gap-1.5 border-b border-line px-4 py-2.5">
            {members
                .into_iter()
                .map(|member| {
                    let name = member.name.clone();
                    let pick = name.clone();
                    let selected = Signal::derive({
                        let name = name.clone();
                        move || current.get().as_deref() == Some(name.as_str())
                    });
                    // Switching package resets the selection to that package's
                    // defaults rather than carrying feature names across — a
                    // feature called `std` in one crate has nothing to do with
                    // `std` in another.
                    let count = member.features.len();

                    view! {
                        <button
                            type="button"
                            on:click=move |_| {
                                controller::apply_features(
                                    state,
                                    FeatureSelection {
                                        package: pick.clone(),
                                        features: Vec::new(),
                                        default_features: true,
                                    },
                                );
                            }
                            class=move || {
                                let base = "flex items-center gap-2 rounded-[6px] px-2.5 py-1 \
                                            text-callout transition-colors";
                                if selected.get() {
                                    format!("{base} bg-selection text-rust")
                                } else {
                                    format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                }
                            }
                        >
                            <span class="font-medium">{name}</span>
                            <span class="tnum font-mono text-footnote text-label-3">
                                {format!("{count}")}
                            </span>
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
fn Impact(impact: FeatureImpact) -> impl IntoView {
    let delta = impact.delta_crates;
    // Signed and explicit. A bare "412" next to "409" makes the reader do the
    // subtraction, and the whole point of this screen is the difference.
    let delta_label = if delta > 0 {
        format!("+{delta}")
    } else {
        delta.to_string()
    };
    let delta_tone = match delta {
        d if d > 0 => Tone::Amber,
        d if d < 0 => Tone::Patina,
        _ => Tone::Neutral,
    };

    let units = impact.delta_build_units;
    let units_label = if units > 0 {
        format!("+{units}")
    } else {
        units.to_string()
    };

    view! {
        <div class="grid grid-cols-2 border-b border-line lg:grid-cols-3">
            <Readout
                label=t!("features.resolved")
                value=impact.resolved_crates.to_string()
                hint=format!("{} under this package's defaults", impact.baseline_crates)
            />
            <Readout
                label=t!("features.against-defaults")
                value=delta_label
                tone=delta_tone
                hint="after workspace-wide unification"
            />
            <Readout
                label=t!("features.build-units")
                value=units_label
                hint="proc-macros and build scripts — they serialize the build"
            />
        </div>

        {(!impact.added.is_empty() || !impact.removed.is_empty())
            .then(|| {
                view! {
                    <div class="grid gap-x-6 border-b border-line px-4 py-3 md:grid-cols-2">
                        <CrateList
                            label=t!("features.pulled-in")
                            names=impact.added.clone()
                            tone=Tone::Amber
                        />
                        <CrateList
                            label=t!("features.dropped")
                            names=impact.removed.clone()
                            tone=Tone::Patina
                        />
                    </div>
                }
            })}
    }
}

#[component]
fn CrateList(#[prop(into)] label: String, names: Vec<String>, tone: Tone) -> impl IntoView {
    if names.is_empty() {
        return ().into_any();
    }
    let count = names.len();

    view! {
        <div class="min-w-0">
            <div class="mb-1.5 flex items-center gap-2">
                <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {label}
                </span>
                <Pill label=format!("{count}") tone=tone />
            </div>
            <div class="flex flex-wrap gap-x-3 gap-y-0.5 font-mono text-footnote text-label-2 select-text">
                {names.into_iter().map(|n| view! { <span>{n}</span> }).collect_view()}
            </div>
        </div>
    }
    .into_any()
}

/// The switches, with what each one costs on its own.
#[component]
fn Matrix() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let rows = state.project.feature_rows.get();
        let Some(selection) = state.project.feature_selection.get() else {
            return ().into_any();
        };

        if rows.is_empty() {
            return view! {
                <p class="px-4 py-4 text-callout text-label-2">
                    {format!("`{}` declares no features.", selection.package)}
                </p>
            }
            .into_any();
        }

        view! {
            <SectionLabel label=t!("features.features") />
            <DefaultsRow selection=selection.clone() />

            <div>
                {rows
                    .into_iter()
                    .map(|row| view! { <FeatureSwitch row=row selection=selection.clone() /> })
                    .collect_view()}
            </div>
        }
        .into_any()
    }
}

/// `--no-default-features`, which is a switch like any other and is where most
/// of the saving usually is.
#[component]
fn DefaultsRow(selection: FeatureSelection) -> impl IntoView {
    let state = AppState::expect();
    let on = selection.default_features;

    view! {
        <label class="flex cursor-pointer items-center gap-3 border-y border-line bg-sunken/40 px-4 py-2">
            <Switch
                on=on
                toggle=Callback::new(move |_| {
                    let mut next = selection.clone();
                    next.default_features = !next.default_features;
                    controller::apply_features(state, next);
                })
            />
            <span class="font-mono text-footnote">"default"</span>
            <span class="text-callout text-label-2">
                {t!("misc.default-features")}
            </span>
        </label>
    }
}

#[component]
fn FeatureSwitch(row: FeatureRow, selection: FeatureSelection) -> impl IntoView {
    let state = AppState::expect();
    let name = row.name.clone();
    let toggled = row.name.clone();
    let enabled = row.enabled;

    let marginal = row.marginal_crates;
    // Zero is the interesting case and needs saying, not leaving blank: it is
    // the unification result people do not believe until they see it.
    let (cost_label, cost_tone) = match marginal {
        0 => ("no change".to_string(), Tone::Neutral),
        d if d > 0 => (format!("+{d} crates"), Tone::Amber),
        d => (format!("{d} crates"), Tone::Patina),
    };

    let enables = row.enables.clone();

    view! {
        <label class="flex cursor-pointer items-start gap-3 border-b border-line px-4 py-2 last:border-b-0 hover:bg-sunken/40">
            <div class="mt-0.5">
                <Switch
                    on=enabled
                    toggle=Callback::new(move |_| {
                        let mut next = selection.clone();
                        if enabled {
                            next.features.retain(|f| f != &toggled);
                        } else if !next.features.contains(&toggled) {
                            next.features.push(toggled.clone());
                        }
                        controller::apply_features(state, next);
                    })
                />
            </div>

            <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-center gap-2">
                    <span class="font-mono text-footnote select-text">{name}</span>
                    {row.in_default.then(|| view! { <Pill label=t!("features.default") /> })}
                    // The definition of "marginal" is what makes this number
                    // mean anything, and it is the same sentence on every row —
                    // so it hangs off the number rather than being printed once
                    // at the top where nobody reads it twice.
                    <span title=t!("features.marginal")>
                        <Pill label=cost_label tone=cost_tone />
                    </span>
                </div>
                {(!enables.is_empty())
                    .then(|| {
                        view! {
                            <div class="mt-0.5 font-mono text-footnote text-label-3 select-text">
                                "→ "{enables.join(", ")}
                            </div>
                        }
                    })}
            </div>
        </label>
    }
}

/// A macOS-style switch.
///
/// Not a checkbox: these are settings that take effect immediately, and the
/// platform reserves checkboxes for choices that need confirming.
#[component]
fn Switch(on: bool, toggle: Callback<()>) -> impl IntoView {
    let track = if on { "bg-rust" } else { "bg-line-strong" };
    let knob = if on {
        "translate-x-[14px]"
    } else {
        "translate-x-0"
    };

    view! {
        <button
            type="button"
            role="switch"
            aria-checked=on.to_string()
            on:click=move |_| toggle.run(())
            class=format!(
                "relative h-[16px] w-[30px] shrink-0 rounded-full transition-colors {track}",
            )
        >
            <span class=format!(
                "absolute top-[2px] left-[2px] size-[12px] rounded-full bg-white shadow-sm \
                 transition-transform {knob}",
            ) />
        </button>
    }
}
