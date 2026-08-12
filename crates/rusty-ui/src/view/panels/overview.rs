//! The home screen.
//!
//! Opening an embedded project shows what it targets and everything wrong with
//! it — not a file tree. The four configuration files that decide a build
//! routinely disagree, and when they do the compiler blames none of them, so
//! naming the disagreement *is* the panel.

use leptos::prelude::*;

use rusty_embed::{Runtime, Severity};

use crate::{
    state::AppState,
    view::components::{Empty, ProblemRow, Readout, SectionLabel, Tone},
};

#[component]
pub fn Overview() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let Some(project) = state.project.get() else {
            return view! {
                <Empty
                    title="No project open"
                    detail="Choose a folder containing a Cargo.toml. rusty reads the resolved \
                            configuration rather than guessing from file names, so the numbers \
                            reflect what cargo would actually do."
                />
            }
            .into_any();
        };

        let chip = project.chip.clone();
        let chip_label = chip.clone().unwrap_or_else(|| "unknown".into());
        let runtime = project.runtime;
        let target = project
            .configured_target
            .clone()
            .unwrap_or_else(|| "unset".into());
        let toolchain_pin = project
            .configured_toolchain
            .clone()
            .unwrap_or_else(|| "unpinned".into());

        // Problems from both sources, blocking first. The user wants the thing
        // stopping the build, not the file it happened to be found in.
        let mut problems = project.problems.clone();
        if let Some(report) = state.toolchain.get() {
            problems.extend(report.problems.clone());
        }
        problems.sort_by_key(|p| match p.severity {
            Severity::Blocking => 0,
            Severity::Warning => 1,
            Severity::Info => 2,
        });
        let blocking = problems
            .iter()
            .filter(|p| p.severity == Severity::Blocking)
            .count();

        let chip_tone = if chip.is_none() { Tone::Crimson } else { Tone::Neutral };
        let target_tone = if project.configured_target.is_none() {
            Tone::Crimson
        } else {
            Tone::Neutral
        };
        let problems_tone = if blocking > 0 { Tone::Crimson } else { Tone::Patina };

        // Computed here rather than inline: `view!` cannot parse a bare `match`
        // or `if` as an attribute value, and precomputing reads better than
        // wrapping each one in braces.
        let runtime_value = match runtime {
            Some(Runtime::BareMetal) => "no_std",
            Some(Runtime::EspIdf) => "std",
            None => "unknown",
        };
        let runtime_hint = runtime
            .map(|r| r.label().to_string())
            .unwrap_or_else(|| "no HAL found".into());
        let problems_hint = if blocking > 0 {
            format!("{blocking} blocking")
        } else {
            "nothing blocking".to_string()
        };
        let chip_hint = project
            .chip_source
            .clone()
            .unwrap_or_else(|| "not detected".into());

        view! {
            <div class="flex-1 overflow-y-auto">
                <div class="grid grid-cols-2 border-b border-line lg:grid-cols-4">
                    <Readout label="Chip" value=chip_label tone=chip_tone hint=chip_hint />
                    <Readout label="Runtime" value=runtime_value hint=runtime_hint />
                    <Readout
                        label="Target"
                        value=target
                        tone=target_tone
                        hint=".cargo/config.toml"
                    />
                    <Readout
                        label="Problems"
                        value=problems.len().to_string()
                        tone=problems_tone
                        hint=problems_hint
                    />
                </div>

                <SectionLabel label="Problems" />
                {if problems.is_empty() {
                    view! {
                        <p class="px-4 pb-4 text-body text-label-2">
                            "Nothing wrong that rusty can see. The four configuration files agree \
                             and the toolchain can build for this part."
                        </p>
                    }
                        .into_any()
                } else {
                    problems
                        .into_iter()
                        .map(|problem| view! { <ProblemRow problem=problem /> })
                        .collect_view()
                        .into_any()
                }}

                <SectionLabel label="Configuration" />
                <dl class="grid grid-cols-[max-content_1fr] gap-x-5 gap-y-1.5 px-4 pb-4 text-callout select-text">
                    <dt class="text-label-3">"Toolchain"</dt>
                    <dd class="m-0 font-mono">{toolchain_pin}</dd>
                    <dt class="text-label-3">"Root"</dt>
                    <dd class="m-0 font-mono break-all">{project.root.clone()}</dd>
                    <dt class="text-label-3">"Read"</dt>
                    <dd class="m-0 font-mono text-label-2">{project.evidence.join("  ·  ")}</dd>
                </dl>

                {(!project.frameworks.is_empty())
                    .then(|| {
                        view! {
                            <SectionLabel label="Frameworks" />
                            <ul class="px-4 pb-6 text-callout select-text">
                                {project
                                    .frameworks
                                    .clone()
                                    .into_iter()
                                    .map(|framework| {
                                        // `name — purpose`, split so the crate name can be
                                        // mono and the explanation plain.
                                        let (name, purpose) = framework
                                            .split_once(" — ")
                                            .map(|(a, b)| (a.to_string(), b.to_string()))
                                            .unwrap_or((framework.clone(), String::new()));
                                        view! {
                                            <li class="flex gap-2 border-b border-line py-1.5 last:border-b-0">
                                                <span class="w-40 shrink-0 font-mono">{name}</span>
                                                <span class="text-label-2">{purpose}</span>
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ul>
                        }
                    })}
            </div>
        }
        .into_any()
    }
}
