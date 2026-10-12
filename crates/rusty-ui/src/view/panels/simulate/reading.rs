//! The line along the sheet's foot: what the rules found, where the pin
//! levels come from, and — while it runs — what the pointer is over,
//! measured.

use super::*;

/// The findings, the pins' source and the probe's reading, at the foot of the
/// sheet.
#[component]
pub(super) fn ReadingLine(board: Board) -> impl IntoView {
    let Board {
        state,
        running,
        live,
        hover_part,
        parts,
        wires,
        hover_wire,
        findings,
        ..
    } = board;
    view! {
        <div class="pointer-events-none absolute bottom-2 left-3 flex max-w-[calc(100%-1.5rem)] flex-col gap-1">
            {move || {
                let findings = findings.get();
                (!findings.is_empty()).then(|| {
                    view! {
                        <div class="pointer-events-auto flex max-w-[60ch] flex-col gap-0.5 rounded-[6px] bg-amber-fill/90 px-2 py-1.5 ring-1 ring-line">
                            {findings
                                .iter()
                                .map(|warning| {
                                    view! {
                                        <span class="text-caption leading-snug text-label-2">{warning_text(warning)}</span>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                })
            }}
            // While it runs, what the pointer is over, measured:
            // the probe and the inspector's numbers, said on the
            // board instead of in a panel over it.
            {move || {
                if !live.get() {
                    return None;
                }
                let text = if let Some(index) = hover_part.get() {
                    let reference = parts
                        .with(|list| list.get(index).map(|p| p.inst.reference.clone()))?;
                    let reading = board.measured(&reference)?;
                    let (volts, amps) =
                        (readout::volts(reading.across), readout::amps(reading.through));
                    // An average no instant has — a lamp under
                    // PWM never sits at its average voltage —
                    // says that it is one.
                    if reading.steady {
                        format!("{reference} · {volts} · {amps}")
                    } else {
                        format!(
                            "{reference} · {}",
                            t!("simulate.reading-average", volts = volts, amps = amps)
                        )
                    }
                } else {
                    let index = hover_wire.get()?;
                    let from = wires.with(|all| all.get(index).map(|w| w.from.clone()))?;
                    let word = level_word(board.level_of(&from));
                    match board.volts_of(&from).map(readout::volts) {
                        Some(volts) => format!("{word} · {volts}"),
                        None => word,
                    }
                };
                Some(view! {
                    <span class="self-start rounded-[6px] bg-raised/90 px-2 py-1 font-mono text-caption text-label ring-1 ring-line">
                        {text}
                    </span>
                })
            }}
            {move || {
                let (label, detail) = match state.sim.pin_source.get() {
                    rusty_embed::PinSource::Emulator => (
                        t!("simulate.pins-emulator"),
                        t!("simulate.pins-emulator-detail"),
                    ),
                    rusty_embed::PinSource::Firmware => (
                        t!("simulate.pins-firmware"),
                        t!("simulate.pins-firmware-detail"),
                    ),
                };
                running.get().then(|| {
                    view! {
                        <span
                            class="pointer-events-auto cursor-help self-start text-footnote text-label-3 underline decoration-dotted underline-offset-2"
                            title=detail
                        >
                            {label}
                        </span>
                    }
                })
            }}
        </div>
    }
}
