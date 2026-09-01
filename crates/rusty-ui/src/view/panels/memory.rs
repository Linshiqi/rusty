//! Where the binary's bytes went.
//!
//! Exists for the failure that ends an afternoon: the linker says a region
//! overflowed by 3 kilobytes and names no cause. `cargo size` gives per-section
//! totals, which restates the situation. The question is always *what is
//! costing me 40 KB*, and only per-crate attribution answers it.
//!
//! So the crate table is the point of this screen and gets the room. Sections
//! are below it: worth having, rarely what anyone came for.

use leptos::prelude::*;

use rusty_embed::{CrateSize, Firmware, MemoryReport, SectionKindDto};

use crate::{
    controller, format,
    state::AppState,
    view::components::{Button, ButtonKind, CommandLine, Empty, Pill, Readout, SectionLabel, Tone},
};

#[component]
pub fn Memory() -> impl IntoView {
    let state = AppState::expect();

    // Analyse whatever build is current, whenever that changes. An explicit
    // button here would mean the panel's normal state is empty, and a panel that
    // shows nothing until it is prodded gets read as broken.
    Effect::new(move |previous: Option<Option<String>>| {
        let path = state.current_firmware().map(|f| f.path);
        // Effects re-run whenever any read signal changes; without comparing
        // against the last path, re-rendering the list would re-analyse the same
        // ELF on every keystroke elsewhere in the app.
        if let Some(path) = path.clone()
            && previous.flatten().as_deref() != Some(path.as_str())
        {
            controller::analyze_memory(state, path);
        }
        path
    });

    move || {
        if !state.has_project() {
            return view! {
                <Empty
                    title="No project open"
                    detail="Memory analysis reads the linked ELF a build produces, so there has to \
                            be a project to have built one."
                />
            }
            .into_any();
        }

        let builds = state.project.firmware.get();
        if builds.is_empty() {
            return view! {
                <Empty
                    title="Nothing built yet"
                    detail="rusty reads the ELF the linker produces rather than guessing at a \
                            path, so there is nothing to measure until a build has run."
                >
                    <div class="mt-1">
                        <CommandLine command="cargo build --release" />
                    </div>
                </Empty>
            }
            .into_any();
        }

        view! {
            <div class="flex-1 overflow-y-auto">
                <BuildPicker builds=builds />
                {move || {
                    state
                        .project.memory
                        .get()
                        .map(|report| view! { <Report report=report /> })
                }}
            </div>
        }
        .into_any()
    }
}

/// Which build is being measured.
///
/// Always shown, even with one build: the alternative is a screen full of
/// numbers with nothing saying which binary produced them, and `debug` and
/// `release` figures differ by enough to make that dangerous.
#[component]
fn BuildPicker(builds: Vec<Firmware>) -> impl IntoView {
    let state = AppState::expect();
    let current = Signal::derive(move || state.current_firmware().map(|f| f.path));

    view! {
        <div class="flex flex-wrap items-center gap-1.5 border-b border-line px-4 py-2.5">
            {builds
                .into_iter()
                .map(|build| {
                    let path = build.path.clone();
                    let selected = Signal::derive({
                        let path = path.clone();
                        move || current.get().as_deref() == Some(path.as_str())
                    });
                    let age = build.modified.map(format::since);
                    // A build for a different triple than the project is
                    // configured for is the one to look at hardest — it flashes
                    // cleanly and then behaves like a hardware fault.
                    let stale = !build.matches_configured_target;

                    view! {
                        <button
                            type="button"
                            title=build.path.clone()
                            on:click=move |_| state.project.selected_firmware.set(Some(path.clone()))
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
                            <span class="font-medium">{build.name}</span>
                            <span class="font-mono text-footnote text-label-3">{build.profile}</span>
                            <span class="tnum font-mono text-footnote text-label-3">
                                {format::bytes(build.bytes)}
                            </span>
                            {age.map(|a| {
                                view! { <span class="text-footnote text-label-3">{a}</span> }
                            })}
                            {stale
                                .then(|| {
                                    view! { <Pill label=build.target tone=Tone::Amber /> }
                                })}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
fn Report(report: MemoryReport) -> impl IntoView {
    let (flash_value, flash_unit) = format::bytes_parts(report.totals.flash_bytes);
    let (ram_value, ram_unit) = format::bytes_parts(report.totals.ram_bytes);

    let ram_fraction = report.totals.ram_fraction();
    // Nominal SRAM, not what the linker will grant: the ROM bootloader and the
    // cache configuration both take a share. Anything approaching this number is
    // already in trouble, so the amber threshold is well below full.
    let ram_tone = match ram_fraction {
        Some(f) if f >= 0.9 => Tone::Crimson,
        Some(f) if f >= 0.7 => Tone::Amber,
        Some(_) => Tone::Patina,
        None => Tone::Neutral,
    };
    let ram_hint = match (ram_fraction, report.totals.ram_capacity) {
        (Some(f), Some(capacity)) => {
            format!(
                "{} of {} nominal",
                format::percent(f),
                format::bytes(capacity as u64)
            )
        }
        _ => "chip capacity unknown".to_string(),
    };

    let attributed: u64 = report.crates.iter().map(|c| c.total).sum();
    let largest = report.crates.first().map(|c| c.total).unwrap_or(0);

    view! {
        <div class="grid grid-cols-2 border-b border-line lg:grid-cols-3">
            <Readout
                label="Flash image"
                value=flash_value
                unit=flash_unit
                hint="code and constants written to the device"
            />
            <Readout
                label="Static RAM"
                value=ram_value
                unit=ram_unit
                tone=ram_tone
                hint=ram_hint
            />
            <Readout
                label="Unattributed"
                value=format::bytes(report.unattributed_bytes)
                hint="assembly, C and ROM stubs — no crate to blame"
            />
        </div>

        <SectionLabel label="By crate" />
        <CrateTable crates=report.crates largest=largest total=attributed />

        <SectionLabel label="By section" />
        <table class="w-full border-collapse text-callout">
            <thead>
                <tr class="border-b border-line text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    <th class="px-4 py-1.5 text-left font-semibold">"Section"</th>
                    <th class="px-4 py-1.5 text-left font-semibold">"Costs"</th>
                    <th class="px-4 py-1.5 text-right font-semibold">"Address"</th>
                    <th class="px-4 py-1.5 text-right font-semibold">"Size"</th>
                </tr>
            </thead>
            <tbody>
                {report
                    .sections
                    .into_iter()
                    .map(|section| {
                        let (in_flash, in_ram) = section.kind.budget();
                        let costs = match (in_flash, in_ram) {
                            (true, true) => "flash + RAM",
                            (true, false) => "flash",
                            (false, true) => "RAM",
                            (false, false) => "—",
                        };
                        view! {
                            <tr class="border-b border-line last:border-b-0">
                                <td class="px-4 py-1.5 font-mono text-footnote select-text">
                                    {section.name}
                                </td>
                                <td class="px-4 py-1.5">
                                    <span class="flex items-center gap-2">
                                        <Pill
                                            label=section.kind.label()
                                            tone=kind_tone(section.kind)
                                        />
                                        <span class="text-footnote text-label-3">{costs}</span>
                                    </span>
                                </td>
                                <td class="tnum px-4 py-1.5 text-right font-mono text-footnote text-label-3">
                                    {format!("{:#010x}", section.address)}
                                </td>
                                <td class="tnum px-4 py-1.5 text-right font-mono">
                                    {format::bytes(section.size)}
                                </td>
                            </tr>
                        }
                    })
                    .collect_view()}
            </tbody>
        </table>

        <div class="flex items-center gap-3 px-4 py-3">
            <span class="font-mono text-footnote text-label-3 select-text">{report.elf_path}</span>
        </div>
    }
}

/// The crate table, longest bar first.
///
/// Bars rather than a chart: the comparison that matters is "which of these is
/// the big one", and a row already carries the name and the number. A pie would
/// add a legend and take the numbers away.
#[component]
fn CrateTable(crates: Vec<CrateSize>, largest: u64, total: u64) -> impl IntoView {
    let state = AppState::expect();

    if crates.is_empty() {
        return view! {
            <p class="px-4 pb-4 text-callout text-label-2">
                "No symbols could be attributed to a crate. That usually means the binary was \
                 stripped, in which case a debug build will have the symbol table."
            </p>
        }
        .into_any();
    }

    // Everything is on one screen or none of it is. Fifteen rows covers a real
    // firmware's interesting crates; the rest are rounding error, and their
    // total is stated rather than silently dropped.
    const SHOWN: usize = 15;
    let hidden = crates.len().saturating_sub(SHOWN);
    let hidden_bytes: u64 = crates.iter().skip(SHOWN).map(|c| c.total).sum();
    let expanded = RwSignal::new(false);

    view! {
        <table class="w-full border-collapse text-callout">
            <thead>
                <tr class="border-b border-line text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    <th class="px-4 py-1.5 text-left font-semibold">"Crate"</th>
                    <th class="px-4 py-1.5 text-right font-semibold">"Code"</th>
                    <th class="px-4 py-1.5 text-right font-semibold">"Read-only"</th>
                    <th class="px-4 py-1.5 text-right font-semibold">"RAM"</th>
                    <th class="px-4 py-1.5 text-right font-semibold">"Total"</th>
                </tr>
            </thead>
            <tbody>
                {move || {
                    let limit = if expanded.get() { crates.len() } else { SHOWN };
                    crates
                        .iter()
                        .take(limit)
                        .map(|item| {
                            // Bars are scaled against the largest crate rather
                            // than the total: against the total every row in a
                            // healthy binary is a sliver, which conveys nothing.
                            let width = if largest == 0 {
                                0.0
                            } else {
                                item.total as f64 / largest as f64 * 100.0
                            };
                            let ram = item.data + item.bss;
                            let name = item.name.clone();
                            view! {
                                <tr class="group border-b border-line last:border-b-0">
                                    <td class="relative px-4 py-1.5">
                                        <span
                                            class="absolute inset-y-0.5 left-0 rounded-r-[2px] bg-rust-fill"
                                            style=format!("width: {width:.2}%")
                                        />
                                        <span class="relative font-mono text-footnote select-text">
                                            {name}
                                        </span>
                                    </td>
                                    <td class="tnum px-4 py-1.5 text-right font-mono text-label-2">
                                        {format::bytes(item.code)}
                                    </td>
                                    <td class="tnum px-4 py-1.5 text-right font-mono text-label-2">
                                        {format::bytes(item.read_only_data)}
                                    </td>
                                    <td class="tnum px-4 py-1.5 text-right font-mono text-label-2">
                                        {format::bytes(ram)}
                                    </td>
                                    <td class="tnum px-4 py-1.5 text-right font-mono font-medium">
                                        {format::bytes(item.total)}
                                    </td>
                                </tr>
                            }
                        })
                        .collect_view()
                }}
            </tbody>
        </table>

        <div class="flex items-center gap-3 px-4 py-2">
            // `Button` takes a plain label, so the whole button is rebuilt on
            // toggle rather than giving the design system a reactive prop that
            // only this one caller needs.
            {(hidden > 0)
                .then_some({
                    move || {
                        let label = if expanded.get() {
                            "Show fewer".to_string()
                        } else {
                            format!("{hidden} more ({})", format::bytes(hidden_bytes))
                        };
                        view! {
                            <Button
                                label=label
                                kind=ButtonKind::Quiet
                                on_click=Callback::new(move |_| expanded.update(|e| *e = !*e))
                            />
                        }
                    }
                })}
            <span class="tnum text-footnote text-label-3">
                {format::bytes(total)}" attributed"
            </span>
            <span class="flex-1" />
            <Button
                label="Re-read"
                kind=ButtonKind::Quiet
                title="Analyse the binary again, after a rebuild"
                on_click=Callback::new(move |_| controller::refresh_firmware(state))
            />
        </div>
    }
        .into_any()
}

fn kind_tone(kind: SectionKindDto) -> Tone {
    match kind {
        SectionKindDto::Code => Tone::Rust,
        SectionKindDto::ReadOnlyData => Tone::Slate,
        SectionKindDto::InitialisedData => Tone::Amber,
        SectionKindDto::ZeroedData => Tone::Patina,
    }
}
