//! What this machine can build for, and what it is missing.
//!
//! Exists because of one specific hour-long failure: someone picks an ESP32 or
//! an S3, runs `cargo build`, and gets an unsupported-target error that never
//! mentions espup. Nothing in the message is searchable towards the fix.
//!
//! Deliberately useful with no project open — "is my machine set up?" is a fair
//! question to ask before there is anything to set it up *for*.

use leptos::prelude::*;

use crate::view::icon::{Icon, IconView};
use crate::{
    controller,
    state::AppState,
    view::components::{CommandLine, Dot, Pill, Readout, SectionLabel, Tone},
};

/// What rusty downloaded itself, how much of the disk it is, and the way to
/// put it somewhere else.
///
/// QEMU and the two esp-gdb builds are most of a data directory and none of
/// that is visible from a folder nobody opens; the tools installed by cargo
/// sit in `~/.cargo/bin` and are not rusty's to relocate, which is worth
/// saying rather than implying.
#[component]
fn Downloads() -> impl IntoView {
    // Owned here rather than in AppState: nothing else reads it, and a panel
    // that only shows a fact does not need the fact to outlive it.
    let location = RwSignal::new(None::<rusty_embed::StorageLocation>);
    let bytes = RwSignal::new(None::<u64>);
    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            controller::load_storage_location(location);
            controller::load_storage_footprint(bytes);
        }
    });

    move || {
        let location = location.get()?;
        let size = bytes
            .get()
            .map(|bytes| format!(" · {}", crate::format::bytes(bytes)))
            .unwrap_or_default();
        Some(view! {
            <div class="mx-4 mb-2 rounded-[8px] border border-line px-3 py-2.5">
                <div class="flex items-baseline justify-between gap-3">
                    <span class="text-callout text-label-2">
                        "QEMU and the debuggers rusty downloads live here"{size}
                    </span>
                    <button
                        type="button"
                        on:click=move |_| {
                            let crate::view::SettingsOpen(open) = expect_context();
                            open.set(true);
                        }
                        class="shrink-0 rounded-[6px] px-2 py-0.5 text-footnote text-rust transition-colors hover:bg-sunken"
                    >
                        "Move…"
                    </button>
                </div>
                <p class="mt-0.5 font-mono text-caption text-label-4 select-text">{location.path}</p>
                <p class="mt-1 max-w-[70ch] text-caption leading-relaxed text-label-3">
                    "Moving it copies everything to the new folder and leaves the originals \
                     until you delete them. Tools installed by cargo are in ~/.cargo/bin and \
                     stay there — that path is cargo's, not rusty's."
                </p>
            </div>
        })
    }
}

#[component]
pub fn Toolchain() -> impl IntoView {
    let state = AppState::expect();

    let toolbar = Callback::new(move |_| {
        view! {
            <button
                type="button"
                title="Probe the machine again"
                on:click=move |_| controller::refresh_toolchain(state)
                class="grid size-7 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label"
            >
                <IconView icon=Icon::Refresh size=15 />
            </button>
        }
        .into_any()
    });
    Effect::new(move |_| {
        state.toolbar.set(Some(toolbar));
    });
    on_cleanup(move || state.toolbar.set(None));

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
                                        // Where it actually is. Answers the two
                                        // questions every one of these raises —
                                        // which copy is being used, and which
                                        // disk it is filling.
                                        {tool
                                            .path
                                            .clone()
                                            .map(|path| {
                                                view! {
                                                    <p class="mt-0.5 truncate font-mono text-caption text-label-4 select-text">
                                                        {path}
                                                    </p>
                                                }
                                            })}
                                        {(!present)
                                            .then(|| {
                                                let command = tool.install_command.clone();
                                                // rustup's "install command" is a URL — it
                                                // is the installer everything else rides
                                                // on, so it stays a pointer, not a button.
                                                if command.starts_with("http") {
                                                    return view! {
                                                        <div class="mt-1.5 font-mono text-footnote text-slate select-text">
                                                            {command}
                                                        </div>
                                                    }
                                                        .into_any();
                                                }
                                                let name = tool.name.clone();
                                                let failed = {
                                                    let name = name.clone();
                                                    Signal::derive(move || {
                                                        state
                                                            .sim_install_failed
                                                            .with(|f| f.contains(&name))
                                                    })
                                                };
                                                view! {
                                                    <div class="mt-1.5 flex items-center gap-2.5">
                                                        <button
                                                            type="button"
                                                            disabled=move || {
                                                                state.session_running.get()
                                                            }
                                                            on:click={
                                                                let name = name.clone();
                                                                move |_| {
                                                                    controller::install_sim_tool(
                                                                        state,
                                                                        name.clone(),
                                                                    )
                                                                }
                                                            }
                                                            class="rounded-[6px] bg-rust px-2.5 py-0.5 text-footnote font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                                                        >
                                                            "Install"
                                                        </button>
                                                        {move || {
                                                            state
                                                                .session_running
                                                                .get()
                                                                .then(|| {
                                                                    view! {
                                                                        <span class="text-footnote text-label-3">
                                                                            "output in the dock"
                                                                        </span>
                                                                    }
                                                                })
                                                        }}
                                                    </div>
                                                    // The command earns its place back only
                                                    // when the button has failed.
                                                    {move || {
                                                        let command = command.clone();
                                                        failed
                                                            .get()
                                                            .then(|| {
                                                                view! {
                                                                    <div class="mt-1.5">
                                                                        <p class="mb-1 text-footnote text-label-2">
                                                                            "Automatic install failed — by hand:"
                                                                        </p>
                                                                        <CommandLine command=command />
                                                                    </div>
                                                                }
                                                            })
                                                    }}
                                                }
                                                    .into_any()
                                            })}
                                    </div>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>

                // Where the big ones live. The tools above come from two
                // different places and only one of them is rusty's to move —
                // saying which, with the number that decides it, beats a
                // Settings page nobody visits until the disk is full.
                <Downloads />

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
            </div>
        }
        .into_any()
    }
}
