//! The Environment page: can this machine build, flash and debug the
//! project, and what does it take to get there.
//!
//! Exists because of one specific hour-long failure: someone picks an ESP32 or
//! an S3, runs `cargo build`, and gets an unsupported-target error that never
//! mentions espup. Nothing in the message is searchable towards the fix.
//!
//! It was a grid of readouts over a list of binaries, and read like one: the
//! required target in red at headline size, "Xtensa toolchain: absent — not
//! needed here" at the *same* size beside it, a count of tools "on PATH". A
//! page somebody opens to ask "am I ready?" has to answer that first, in a
//! sentence, with the one button that fixes it — `flutter doctor` and the
//! ESP-IDF extension's doctor, drawn as the Settings page draws its groups.
//! Below the answer each tool sits in the group of work it serves, with its
//! state on the right where a Settings control sits, and its Install there
//! too — running in the row, not only in the dock.
//!
//! Deliberately useful with no project open — "is my machine set up?" is a
//! fair question to ask before there is anything to set it up *for*.

use leptos::prelude::*;

use rusty_embed::{ToolStatus, ToolchainReport};
use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{CommandLine, ProblemRow, Spinner},
    view::icon::{Icon, IconView},
    view::settings::{Group, Row},
};

/// Which group of work a tool serves. By name, because the report is a flat
/// list and the grouping is this page's opinion; an unknown tool goes with
/// the editing tools rather than nowhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Section {
    Build,
    Device,
    Editor,
}

fn section_of(tool: &str) -> Section {
    match tool {
        "msvc" | "ldproxy" | "riscv32-esp-elf-gcc" | "xtensa-esp-elf-gcc" => Section::Build,
        "espflash" | "probe-rs" | "codelldb" => Section::Device,
        _ => Section::Editor,
    }
}

/// The tools this project cannot do without that are not there, in the
/// order they install: the required tools, the Xtensa toolchain and the
/// chip's target — read off the report's own fields, the way the setup plan
/// reads them, so this page and the rows under it cannot disagree — and
/// then what the backend's blocking problems name: a project with a chip
/// and no flasher cannot reach its board. Targets are `target:<triple>`,
/// as the setup plan spells them.
fn needed(report: &ToolchainReport) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |name: String| {
        if !out.contains(&name) {
            out.push(name);
        }
    };
    for tool in &report.status.tools {
        if tool.required && !tool.is_installed() {
            add(tool.name.clone());
        }
    }
    if report.needs_esp_toolchain && !report.status.has_esp_toolchain {
        add("espup".to_string());
    }
    // An Xtensa target comes with the Xtensa toolchain, which is the row
    // that is missing then.
    if let Some(target) = &report.required_target
        && !report.required_target_installed
        && !target.starts_with("xtensa-")
    {
        add(format!("target:{target}"));
    }
    for problem in &report.problems {
        if problem.severity != rusty_embed::Severity::Blocking {
            continue;
        }
        match problem.kind.as_str() {
            "no-flasher" => add("espflash".to_string()),
            "ldproxy-missing" => add("ldproxy".to_string()),
            _ => {}
        }
    }
    out
}

/// The page's answer, before any list.
#[derive(Clone, PartialEq, Debug)]
enum Verdict {
    /// Rust itself is absent: nothing else can be installed until it is.
    NeedsRust,
    /// An `-msvc` host without the Visual Studio linker.
    NeedsLinker,
    /// Things this project needs, by name.
    Missing(Vec<String>),
    /// Ready; the count is optional tools not installed.
    Ready(usize),
}

fn verdict(report: &ToolchainReport) -> Verdict {
    let steps = rusty_embed::setup::plan(report);
    if steps
        .iter()
        .any(|s| s.tool == "rustup" && s.manual.is_some())
    {
        return Verdict::NeedsRust;
    }
    if steps.iter().any(|s| s.tool == "msvc" && s.manual.is_some()) {
        return Verdict::NeedsLinker;
    }
    let needed = needed(report);
    if !needed.is_empty() {
        return Verdict::Missing(needed);
    }
    Verdict::Ready(steps.len())
}

#[component]
pub fn Toolchain() -> impl IntoView {
    let state = AppState::expect();

    let body = move || {
        let Some(report) = state.project.toolchain.get() else {
            return view! {
                <div class="flex flex-1 items-center justify-center gap-2 p-10 text-body text-label-2">
                    <Spinner size=14 />
                    {t!("toolchain.reading")}
                </div>
            }
            .into_any();
        };
        view! {
            <div class="min-h-0 flex-1 overflow-y-auto">
                <div class="mx-auto max-w-[46rem] px-6 py-5">
                    <Hero report=report.clone() />
                    <Notes report=report.clone() />
                    <BuildGroup report=report.clone() />
                    <ToolGroup report=report.clone() section=Section::Device />
                    <ToolGroup report=report.clone() section=Section::Editor />
                    <Downloads />
                    <Installed report=report />
                </div>
            </div>
        }
        .into_any()
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex items-center gap-2 border-b border-line px-5 py-2">
                <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {t!("panel.toolchain")}
                </span>
                <span class="flex-1" />
                <button
                    type="button"
                    title=t!("toolchain.refresh")
                    on:click=move |_| controller::refresh_toolchain(state)
                    class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label"
                >
                    <IconView icon=Icon::Refresh size=13 />
                </button>
            </div>
            {body}
        </div>
    }
}

/// The answer, in a sentence, with the one button that fixes it.
#[component]
fn Hero(report: ToolchainReport) -> impl IntoView {
    let state = AppState::expect();
    let verdict = verdict(&report);
    // What the answer is about: the chip and target when a project says
    // them, the machine when nothing is open.
    let context = state.project.detected.with_untracked(|project| {
        project.as_ref().map(|project| {
            let chip = project.chip.clone().unwrap_or_else(|| t!("status.no-chip"));
            let target = report
                .required_target
                .clone()
                .or_else(|| project.configured_target.clone())
                .unwrap_or_default();
            let toolchain = project
                .configured_toolchain
                .clone()
                .unwrap_or_else(|| t!("status.unpinned"));
            [chip, target, toolchain]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ")
        })
    });

    move || {
        // An install under way is the headline, whichever button began it.
        if let Some(tool) = state.setup.busy.get() {
            return view! {
                <div class="mb-5 flex items-center gap-3 rounded-[10px] bg-raised px-4 py-3.5 ring-1 ring-line">
                    <span class="text-amber">
                        <Spinner size=20 />
                    </span>
                    <div class="min-w-0 flex-1">
                        <div class="text-body font-medium text-label">
                            {t!("environment.installing", tool = display_name(&tool))}
                        </div>
                        <div class="text-footnote text-label-3">{t!("environment.installing-detail")}</div>
                    </div>
                    <button
                        type="button"
                        on:click=move |_| state.show_dock(crate::state::DockTab::Output)
                        class="shrink-0 rounded-[6px] px-2.5 py-1 text-footnote text-rust hover:bg-sunken"
                    >
                        {t!("environment.show-output")}
                    </button>
                </div>
            }
            .into_any();
        }

        let has_project = context.is_some();
        let (icon, colour, title, detail) = match &verdict {
            Verdict::NeedsRust => (
                Icon::Warn,
                "text-crimson",
                t!("environment.needs-rust"),
                t!("environment.needs-rust-detail"),
            ),
            Verdict::NeedsLinker => (
                Icon::Warn,
                "text-crimson",
                t!("environment.needs-linker"),
                t!("environment.needs-linker-detail"),
            ),
            Verdict::Missing(tools) => (
                Icon::Warn,
                "text-crimson",
                if has_project {
                    t!("environment.missing", count = tools.len().to_string())
                } else {
                    t!(
                        "environment.missing-machine",
                        count = tools.len().to_string()
                    )
                },
                tools
                    .iter()
                    .map(|tool| display_name(tool))
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
            Verdict::Ready(optional) => (
                Icon::Check,
                "text-patina",
                if has_project {
                    t!("environment.ready")
                } else {
                    t!("environment.ready-machine")
                },
                if *optional > 0 {
                    t!("environment.optional-missing", count = optional.to_string())
                } else {
                    context
                        .clone()
                        .unwrap_or_else(|| t!("environment.open-project-hint"))
                },
            ),
        };
        let action = match &verdict {
            Verdict::NeedsRust => Some(
                view! {
                    <button
                        type="button"
                        on:click=move |_| controller::open_url(state, "https://rustup.rs".to_string())
                        class="shrink-0 rounded-[6px] bg-rust px-3 py-1.5 text-footnote font-medium text-white hover:opacity-90"
                    >
                        {t!("setup.open-rustup")}
                    </button>
                }
                .into_any(),
            ),
            Verdict::NeedsLinker => {
                let url = report
                    .status
                    .tools
                    .iter()
                    .find(|t| t.name == "msvc")
                    .map(|t| t.install_command.clone())
                    .unwrap_or_default();
                Some(
                    view! {
                        <button
                            type="button"
                            on:click=move |_| controller::open_url(state, url.clone())
                            class="shrink-0 rounded-[6px] bg-rust px-3 py-1.5 text-footnote font-medium text-white hover:opacity-90"
                        >
                            {t!("environment.open-download")}
                        </button>
                    }
                    .into_any(),
                )
            }
            Verdict::Missing(tools) => {
                let tools = tools.clone();
                Some(
                    view! {
                        <button
                            type="button"
                            disabled=move || state.app.session_running.get()
                            on:click=move |_| controller::install_needed(state, tools.clone())
                            class="shrink-0 rounded-[6px] bg-rust px-3 py-1.5 text-footnote font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                        >
                            {t!("environment.install-missing")}
                        </button>
                    }
                    .into_any(),
                )
            }
            Verdict::Ready(_) => None,
        };
        // The project's line under a ready verdict that had something else
        // to say, so what "ready" is about is never lost.
        let context_line = matches!(&verdict, Verdict::Ready(n) if *n > 0)
            .then(|| context.clone())
            .flatten();
        let detail_title = detail.clone();

        view! {
            <div class="mb-5 flex items-center gap-3 rounded-[10px] bg-raised px-4 py-3.5 ring-1 ring-line">
                <span class=colour>
                    <IconView icon=icon size=22 />
                </span>
                <div class="min-w-0 flex-1">
                    <div class="text-body font-medium text-label">{title}</div>
                    <div class="truncate text-footnote text-label-3" title=detail_title>
                        {detail}
                    </div>
                    {context_line
                        .map(|line| {
                            view! { <div class="truncate text-footnote text-label-4">{line}</div> }
                        })}
                </div>
                {action}
            </div>
        }
        .into_any()
    }
}

/// What the report says that is not about a tool being absent — the
/// toolchain that loses rust-analyzer its dependencies, say.
#[component]
fn Notes(report: ToolchainReport) -> impl IntoView {
    let warnings: Vec<_> = report
        .problems
        .into_iter()
        .filter(|p| p.severity != rusty_embed::Severity::Blocking)
        .collect();
    (!warnings.is_empty()).then(|| {
        view! {
            <div class="mb-6 overflow-hidden rounded-[10px] bg-raised ring-1 ring-line">
                {warnings
                    .into_iter()
                    .map(|problem| view! { <ProblemRow problem=problem /> })
                    .collect_view()}
            </div>
        }
    })
}

/// A tool's name the way a person reads it, with the binary kept for the
/// rows that show it in mono beneath.
fn display_name(tool: &str) -> String {
    if let Some(target) = tool.strip_prefix("target:") {
        return t!("environment.target-named", target = target.to_string());
    }
    match tool {
        "espup" => t!("environment.xtensa"),
        "msvc" => t!("environment.msvc"),
        other => other.to_string(),
    }
}

/// Building: Rust, the chip's target, the Xtensa toolchain when the chip
/// needs it, and the build's own helpers.
#[component]
fn BuildGroup(report: ToolchainReport) -> impl IntoView {
    let state = AppState::expect();
    let has_project = state.has_project_now();
    let needed = needed(&report);
    let status = report.status.clone();

    let rustup = status.tools.iter().find(|t| t.name == "rustup").cloned();
    let default_toolchain = status
        .toolchains
        .iter()
        .find(|t| t.is_default)
        .map(|t| t.name.clone());

    let rust_row = view! {
        <Row
            label=t!("environment.rust")
            detail=crate::i18n::tool_purpose("rustup", &t!("tool.rustup"))
        >
            {match rustup.as_ref().filter(|t| t.is_installed()) {
                Some(tool) => {
                    view! { <Present version=default_toolchain.clone().or(tool.version.clone()) path=tool.path.clone() /> }
                        .into_any()
                }
                None => view! { <Absent needed=true /> }.into_any(),
            }}
        </Row>
    };

    // The chip's target, when a project names one. An Xtensa target is not
    // rustup's to add — espup brings it — so it has no button of its own.
    let target_row = report.required_target.clone().map(|target| {
        let installed = report.required_target_installed;
        let xtensa = target.starts_with("xtensa-");
        let step = format!("target:{target}");
        view! {
            <Row label=t!("environment.target") detail=target.clone()>
                {if installed {
                    view! { <Present version=None path=None /> }.into_any()
                } else if xtensa {
                    view! {
                        <span class="text-footnote text-label-3">{t!("environment.target-by-espup")}</span>
                    }
                        .into_any()
                } else {
                    view! { <InstallCell tool=step needed=true /> }.into_any()
                }}
            </Row>
        }
    });

    // Only when the chip needs it. "Absent — not needed here" in the same
    // type as a real problem was the old page's most confusing line.
    let xtensa_row = report.needs_esp_toolchain.then(|| {
        let present = status.has_esp_toolchain;
        view! {
            <Row label=t!("environment.xtensa") detail=t!("environment.xtensa-detail")>
                {if present {
                    view! { <Present version=Some("esp".to_string()) path=None /> }.into_any()
                } else {
                    view! { <InstallCell tool="espup".to_string() needed=true /> }.into_any()
                }}
            </Row>
        }
    });

    let helper_rows = status
        .tools
        .iter()
        .filter(|tool| section_of(&tool.name) == Section::Build)
        .cloned()
        .map(|tool| {
            let needed = needed.contains(&tool.name);
            view! { <ToolRow tool=tool needed=needed /> }
        })
        .collect_view();

    let title = if has_project {
        t!("environment.group-build")
    } else {
        t!("environment.group-build-machine")
    };
    view! {
        <Group title=title>
            {rust_row}
            {target_row}
            {xtensa_row}
            {helper_rows}
        </Group>
    }
}

/// The tools of one group of work, each in its row.
#[component]
fn ToolGroup(report: ToolchainReport, section: Section) -> impl IntoView {
    let needed = needed(&report);
    let tools: Vec<ToolStatus> = report
        .status
        .tools
        .into_iter()
        // rustup has the Rust row, espup the Xtensa row.
        .filter(|tool| !matches!(tool.name.as_str(), "rustup" | "espup"))
        .filter(|tool| section_of(&tool.name) == section)
        .collect();
    if tools.is_empty() {
        return None;
    }
    let title = match section {
        Section::Device => t!("environment.group-device"),
        Section::Editor => t!("environment.group-editor"),
        Section::Build => t!("environment.group-build"),
    };
    Some(view! {
        <Group title=title>
            {tools
                .into_iter()
                .map(|tool| {
                    let needed = needed.contains(&tool.name);
                    view! { <ToolRow tool=tool needed=needed /> }
                })
                .collect_view()}
        </Group>
    })
}

/// One tool: what it is for on the left, where it stands on the right.
#[component]
fn ToolRow(tool: ToolStatus, needed: bool) -> impl IntoView {
    let purpose = crate::i18n::tool_purpose(&tool.name, &tool.purpose);
    let label = display_name(&tool.name);
    let right = if tool.is_installed() {
        view! { <Present version=tool.version.clone() path=tool.path.clone() /> }.into_any()
    } else if tool.installable && !tool.install_command.starts_with("http") {
        view! { <InstallCell tool=tool.name.clone() needed=needed /> }.into_any()
    } else {
        // Nothing rusty can install: the page it is installed from, or the
        // command for a terminal.
        view! { <Manual command=tool.install_command.clone() needed=needed /> }.into_any()
    };
    view! {
        <Row label=label detail=purpose>
            {right}
        </Row>
    }
}

/// Installed: the version, and where it is on hover — which copy is being
/// used, and on which disk it sits.
#[component]
fn Present(version: Option<String>, path: Option<String>) -> impl IntoView {
    let shown = version.unwrap_or_else(|| t!("environment.installed"));
    view! {
        <span
            class="flex max-w-[16rem] items-center gap-1.5 text-footnote text-label-3"
            title=path.unwrap_or_default()
        >
            <span class="min-w-0 truncate font-mono">{shown}</span>
            <span class="text-patina">
                <IconView icon=Icon::Check size=13 />
            </span>
        </span>
    }
}

/// Not installed, and nothing to press.
#[component]
fn Absent(needed: bool) -> impl IntoView {
    let (text, colour) = if needed {
        (t!("environment.missing-one"), "text-crimson")
    } else {
        (t!("environment.not-installed"), "text-label-3")
    };
    view! { <span class=format!("text-footnote {colour}")>{text}</span> }
}

/// Not installed, and rusty can install it: the button, the spinner while
/// it runs, and the command to run by hand when it fails.
#[component]
fn InstallCell(tool: String, needed: bool) -> impl IntoView {
    let state = AppState::expect();
    let name = tool.clone();
    let busy = {
        let name = name.clone();
        Signal::derive(move || {
            state
                .setup
                .busy
                .with(|b| b.as_deref() == Some(name.as_str()))
        })
    };
    let failed = {
        let name = name.clone();
        Signal::derive(move || state.setup.failed.with(|f| f.contains(&name)))
    };
    let command = state.project.toolchain.with_untracked(|report| {
        report.as_ref().and_then(|report| {
            rusty_embed::setup::plan(report)
                .into_iter()
                .find(|step| step.tool == name)
                .map(|step| step.command)
        })
    });
    // A tool the plan will not run — installed meanwhile, or offered only
    // where a chip asks for it — is a state, not a button.
    let Some(command) = command else {
        return view! { <Absent needed=needed /> }.into_any();
    };

    view! {
        <div class="flex flex-col items-end gap-1.5">
            {move || {
                if busy.get() {
                    return view! {
                        <span class="flex items-center gap-1.5 text-footnote text-amber">
                            <Spinner size=12 />
                            {t!("environment.installing-short")}
                        </span>
                    }
                    .into_any();
                }
                let (label, look) = if failed.get() {
                    (t!("environment.retry"), "bg-crimson text-white")
                } else if needed {
                    (t!("toolchain.install"), "bg-rust text-white")
                } else {
                    (t!("toolchain.install"), "text-rust ring-1 ring-line hover:bg-sunken")
                };
                let tool = tool.clone();
                view! {
                    <span class="flex items-center gap-2">
                        {(!failed.get())
                            .then(|| view! { <Absent needed=needed /> })}
                        <button
                            type="button"
                            disabled=move || {
                                state.app.session_running.get()
                                    || state.setup.busy.with(Option::is_some)
                            }
                            on:click=move |_| controller::install_step(state, tool.clone())
                            class=format!(
                                "rounded-[6px] px-2.5 py-0.5 text-footnote font-medium hover:opacity-90 \
                                 disabled:pointer-events-none disabled:opacity-40 {look}",
                            )
                        >
                            {label}
                        </button>
                    </span>
                }
                .into_any()
            }}
            // The command earns its place back only when the button failed.
            {move || {
                failed
                    .get()
                    .then(|| {
                        let command = command.clone();
                        view! {
                            <div class="flex max-w-[22rem] flex-col items-end gap-1">
                                <span class="text-caption text-crimson">{t!("toolchain.install-failed")}</span>
                                <CommandLine command=command />
                            </div>
                        }
                    })
            }}
        </div>
    }
    .into_any()
}

/// Not something rusty installs: a page to open, or a command to copy.
#[component]
fn Manual(command: String, needed: bool) -> impl IntoView {
    let state = AppState::expect();
    if command.starts_with("http") {
        let url = command.clone();
        return view! {
            <span class="flex items-center gap-2">
                <Absent needed=needed />
                <button
                    type="button"
                    on:click=move |_| controller::open_url(state, url.clone())
                    class="rounded-[6px] px-2.5 py-0.5 text-footnote font-medium text-rust ring-1 ring-line hover:bg-sunken"
                >
                    {t!("environment.open-page")}
                </button>
            </span>
        }
        .into_any();
    }
    view! {
        <div class="flex max-w-[22rem] flex-col items-end gap-1">
            <Absent needed=needed />
            <CommandLine command=command />
        </div>
    }
    .into_any()
}

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
        let size = bytes.get().map(crate::format::bytes).unwrap_or_default();
        Some(view! {
            <Group title=t!("environment.group-downloads") footer=t!("toolchain.move-note")>
                <Row label=t!("toolchain.downloads-here") detail=location.path stacked=false>
                    <span class="text-footnote text-label-3">{size}</span>
                    <button
                        type="button"
                        on:click=move |_| {
                            let crate::view::SettingsOpen(open) = expect_context();
                            open.set(true);
                        }
                        class="rounded-[6px] px-2.5 py-0.5 text-footnote text-rust ring-1 ring-line hover:bg-sunken"
                    >
                        {t!("toolchain.move")}
                    </button>
                </Row>
            </Group>
        })
    }
}

/// Everything installed, folded away: the answer to "which toolchains do I
/// have" is worth a click, not a third of the page.
#[component]
fn Installed(report: ToolchainReport) -> impl IntoView {
    let status = report.status;
    let required = report.required_target;
    view! {
        <details class="group mt-2 mb-6">
            <summary class="cursor-pointer px-1 py-1 text-footnote font-semibold tracking-[0.04em] text-label-3 uppercase select-none hover:text-label-2">
                {t!(
                    "environment.installed-summary",
                    toolchains = status.toolchains.len().to_string(),
                    targets = status.installed_targets.len().to_string()
                )}
            </summary>
            <div class="mt-2 rounded-[10px] bg-raised px-3.5 py-3 ring-1 ring-line">
                <div class="mb-1.5 text-caption text-label-3">{t!("toolchain.toolchains")}</div>
                <div class="mb-3 flex flex-wrap gap-1.5 select-text">
                    {status
                        .toolchains
                        .into_iter()
                        .map(|toolchain| {
                            let look = if toolchain.is_default {
                                "bg-rust-fill text-rust"
                            } else {
                                "bg-sunken text-label-2"
                            };
                            view! {
                                <span class=format!(
                                    "rounded-[5px] px-2 py-0.5 font-mono text-footnote {look}",
                                )>
                                    {toolchain.name}
                                    {toolchain.is_default.then(|| format!(" · {}", t!("toolchain.default")))}
                                </span>
                            }
                        })
                        .collect_view()}
                </div>
                <div class="mb-1.5 text-caption text-label-3">
                    {t!("toolchain.targets", count = status.installed_targets.len())}
                </div>
                <div class="flex flex-wrap gap-1.5 select-text">
                    {status
                        .installed_targets
                        .into_iter()
                        .map(|target| {
                            let look = if required.as_deref() == Some(target.as_str()) {
                                "bg-patina-fill text-patina"
                            } else {
                                "bg-sunken text-label-2"
                            };
                            view! {
                                <span class=format!(
                                    "rounded-[5px] px-2 py-0.5 font-mono text-footnote {look}",
                                )>{target}</span>
                            }
                        })
                        .collect_view()}
                </div>
            </div>
        </details>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::{Problem, Severity, Toolchain, ToolchainStatus};

    fn tool(name: &str, installed: bool, required: bool) -> ToolStatus {
        ToolStatus {
            name: name.to_string(),
            purpose: String::new(),
            version: installed.then(|| "1.0".to_string()),
            path: installed.then(|| format!("/bin/{name}")),
            install_command: format!("cargo install {name}"),
            installable: true,
            required,
        }
    }

    fn report(tools: Vec<ToolStatus>, problems: Vec<Problem>) -> ToolchainReport {
        ToolchainReport {
            status: ToolchainStatus {
                toolchains: vec![Toolchain {
                    name: "stable".to_string(),
                    is_default: true,
                    is_esp: false,
                }],
                installed_targets: vec![],
                tools,
                has_esp_toolchain: false,
            },
            required_target: Some("riscv32imc-unknown-none-elf".to_string()),
            required_target_installed: false,
            needs_esp_toolchain: false,
            problems,
        }
    }

    /// Needed is what stops *this project*: the target it cannot build for,
    /// read off the report itself — the first version counted it only when
    /// a problem named it, and a page said "one thing missing" above two
    /// rows marked missing — and the flasher the backend's blocking problem
    /// names, since a project with a chip cannot reach its board without
    /// one. An optional tool that is absent is not needed.
    #[test]
    fn needed_is_what_the_project_cannot_do_without() {
        let report = report(
            vec![
                tool("rustup", true, true),
                tool("espflash", false, false),
                tool("probe-rs", false, false),
            ],
            vec![Problem::new(Severity::Blocking, "no-flasher", "t", "d")],
        );
        assert_eq!(
            needed(&report),
            ["target:riscv32imc-unknown-none-elf", "espflash"]
        );
        assert_eq!(
            verdict(&report),
            Verdict::Missing(vec![
                "target:riscv32imc-unknown-none-elf".to_string(),
                "espflash".to_string(),
            ])
        );
    }

    /// An Xtensa chip's target comes with the Xtensa toolchain, so the
    /// toolchain is what is missing, once.
    #[test]
    fn an_xtensa_target_is_counted_as_its_toolchain() {
        let mut xtensa = report(vec![tool("rustup", true, true)], vec![]);
        xtensa.required_target = Some("xtensa-esp32-none-elf".to_string());
        xtensa.needs_esp_toolchain = true;
        assert_eq!(needed(&xtensa), ["espup"]);
    }

    /// Rust absent is its own answer: everything else rides on it, so the
    /// page does not offer six buttons that cannot work.
    #[test]
    fn without_rust_the_answer_is_rust() {
        let report = report(vec![tool("rustup", false, true)], vec![]);
        assert_eq!(verdict(&report), Verdict::NeedsRust);
    }

    /// Ready, with the optional tools counted rather than listed as
    /// problems — a red badge for probe-rs on a USB-serial board is crying
    /// wolf.
    #[test]
    fn ready_counts_the_optional_tools_it_did_not_need() {
        let mut ready = report(
            vec![
                tool("rustup", true, true),
                tool("espflash", true, false),
                tool("probe-rs", false, false),
            ],
            vec![],
        );
        ready.required_target_installed = true;
        assert_eq!(verdict(&ready), Verdict::Ready(1));
    }

    #[test]
    fn tools_sit_in_the_group_of_work_they_serve() {
        assert_eq!(section_of("espflash"), Section::Device);
        assert_eq!(section_of("probe-rs"), Section::Device);
        assert_eq!(section_of("msvc"), Section::Build);
        assert_eq!(section_of("riscv32-esp-elf-gcc"), Section::Build);
        assert_eq!(section_of("rust-analyzer"), Section::Editor);
        assert_eq!(section_of("something-new"), Section::Editor);
    }
}
