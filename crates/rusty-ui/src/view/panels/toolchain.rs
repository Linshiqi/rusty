//! What this machine can build for, and what it is missing.
//!
//! Exists because of one specific hour-long failure: someone picks an ESP32 or
//! an S3, runs `cargo build`, and gets an unsupported-target error that never
//! mentions espup. Nothing in the message is searchable towards the fix.
//!
//! Deliberately useful with no project open — "is my machine set up?" is a fair
//! question to ask before there is anything to set it up *for*.

use leptos::prelude::*;

use crate::{
    controller,
    state::AppState,
    view::components::{Button, CommandLine, Dot, Pill, Readout, SectionLabel, Tone},
};

#[component]
pub fn Toolchain() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let Some(report) = state.toolchain.get() else {
            return view! {
                <div class="flex flex-1 items-center justify-center p-10">
                    <span class="text-body text-label-2">"Reading the toolchain…"</span>
                </div>
            }
            .into_any();
        };

        let status = &report.status;
        let installed = status.tools.iter().filter(|t| t.is_installed()).count();
        let total = status.tools.len();

        let required_target = report
            .required_target
            .clone()
            .unwrap_or_else(|| "—".to_string());
        let target_tone = match (&report.required_target, report.required_target_installed) {
            (Some(_), false) => Tone::Crimson,
            (Some(_), true) => Tone::Patina,
            (None, _) => Tone::Neutral,
        };
        let target_hint = match (&report.required_target, report.required_target_installed) {
            (Some(_), true) => "installed".to_string(),
            (Some(_), false) => "not installed".to_string(),
            (None, _) => "no project open".to_string(),
        };

        // Xtensa targets ship *inside* the espup toolchain rather than through
        // rustup, so their absence from `rustup target list` means nothing —
        // reporting it as missing would send people to run a command that
        // cannot help them.
        let esp_tone = if report.needs_esp_toolchain && !status.has_esp_toolchain {
            Tone::Crimson
        } else if status.has_esp_toolchain {
            Tone::Patina
        } else {
            Tone::Neutral
        };
        let esp_value = if status.has_esp_toolchain {
            "installed"
        } else {
            "absent"
        };
        let esp_hint = if report.needs_esp_toolchain {
            "this project needs it"
        } else {
            "not needed here"
        };

        view! {
            <div class="flex-1 overflow-y-auto">
                <div class="grid grid-cols-2 border-b border-line lg:grid-cols-3">
                    <Readout
                        label="Required target"
                        value=required_target
                        tone=target_tone
                        hint=target_hint
                    />
                    <Readout
                        label="Xtensa toolchain"
                        value=esp_value
                        tone=esp_tone
                        hint=esp_hint
                    />
                    <Readout
                        label="Tools"
                        value=format!("{installed}/{total}")
                        hint="on PATH"
                    />
                </div>

                <SectionLabel label="Tools" />
                <div>
                    {status
                        .tools
                        .clone()
                        .into_iter()
                        .map(|tool| {
                            let present = tool.is_installed();
                            view! {
                                <div class="flex items-start gap-2.5 border-b border-line px-4 py-2.5 last:border-b-0">
                                    <div class="mt-[6px]">
                                        <Dot tone=if present { Tone::Patina } else { Tone::Neutral } />
                                    </div>
                                    <div class="min-w-0 flex-1 select-text">
                                        <div class="flex items-center gap-2">
                                            <span class="font-mono text-body font-medium">
                                                {tool.name.clone()}
                                            </span>
                                            {present
                                                .then(|| {
                                                    view! {
                                                        <span class="truncate font-mono text-footnote text-label-3">
                                                            {tool.version.clone().unwrap_or_default()}
                                                        </span>
                                                    }
                                                })}
                                            {(!present && tool.required)
                                                .then(|| view! { <Pill label="required" tone=Tone::Crimson /> })}
                                        </div>
                                        <p class="mt-0.5 max-w-[70ch] text-callout text-label-2">
                                            {tool.purpose.clone()}
                                        </p>
                                        {(!present)
                                            .then(|| {
                                                let command = tool.install_command.clone();
                                                // rustup's "install command" is a URL, not
                                                // something to paste into a shell.
                                                if command.starts_with("http") {
                                                    view! {
                                                        <div class="mt-1.5 font-mono text-footnote text-slate select-text">
                                                            {command}
                                                        </div>
                                                    }
                                                        .into_any()
                                                } else {
                                                    view! {
                                                        <div class="mt-1.5">
                                                            <CommandLine command=command />
                                                        </div>
                                                    }
                                                        .into_any()
                                                }
                                            })}
                                    </div>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>

                <SectionLabel label="Toolchains" />
                <div class="px-4 pb-2">
                    {status
                        .toolchains
                        .clone()
                        .into_iter()
                        .map(|toolchain| {
                            view! {
                                <div class="flex items-center gap-2 border-b border-line py-1.5 last:border-b-0">
                                    <span class="font-mono text-callout select-text">
                                        {toolchain.name.clone()}
                                    </span>
                                    {toolchain.is_default.then(|| view! { <Pill label="default" /> })}
                                    {toolchain
                                        .is_esp
                                        .then(|| view! { <Pill label="Xtensa" tone=Tone::Rust /> })}
                                </div>
                            }
                        })
                        .collect_view()}
                </div>

                <SectionLabel label=format!("Targets ({})", status.installed_targets.len()) />
                <div class="flex flex-wrap gap-1.5 px-4 pb-6 select-text">
                    {status
                        .installed_targets
                        .clone()
                        .into_iter()
                        .map(|target| {
                            let is_required = report.required_target.as_deref() == Some(&target);
                            view! {
                                <span class=move || {
                                    let base = "rounded-[5px] px-2 py-0.5 font-mono text-footnote";
                                    if is_required {
                                        format!("{base} bg-patina-fill text-patina")
                                    } else {
                                        format!("{base} bg-sunken text-label-2")
                                    }
                                }>{target}</span>
                            }
                        })
                        .collect_view()}
                </div>

                <div class="border-t border-line px-4 py-3">
                    <Button
                        label="Re-scan"
                        on_click=Callback::new(move |_| controller::refresh_toolchain(state))
                        disabled=Signal::derive(move || state.is_busy())
                    />
                </div>
            </div>
        }
        .into_any()
    }
}
