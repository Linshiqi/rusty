//! What a signal generator plays and what moves a sensor's readings, as the
//! inspector edits them: the sheet's own signals, saved with it.
//!
//! A text is checked as it is typed and written to the sheet only once it
//! reads. A sheet holding a signal that does not read is a run that plays
//! nothing and says why — the right answer to a file somebody edited by
//! hand, and the wrong one to a field that could have said so first.

use super::*;
use crate::lab::{preset_name, signal_error_text};
use crate::view::lab::sparkline;
use rusty_embed::generator::{self, GENERATOR_RATE, SIGNAL, SIGNAL_OF, SIGNAL_RATE};
use rusty_embed::signal::Signal;

/// Is this `input` event a keystroke an input method has not finished
/// with? The wizard's rule: text is judged once `compositionend` says the
/// letters are settled, never the pinyin a Chinese IME shows on the way.
fn composing(event: &ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    event
        .dyn_ref::<web_sys::InputEvent>()
        .is_some_and(web_sys::InputEvent::is_composing)
}

const FIELD: &str = "h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote \
                     text-label outline-none ring-1 ring-line focus:ring-rust";

/// One signal's text: checked as it is typed, written to the sheet under
/// `key` when it is left and reads. Empty is written as absent — silence on
/// a generator, a reading left to its slider on a sensor.
#[component]
fn SignalField(board: Board, index: usize, key: String, current: String) -> impl IntoView {
    let Board { parts, dirty, .. } = board;
    let draft = RwSignal::new(current.clone());
    let error = RwSignal::new(None::<String>);
    let judge = move |text: &str| {
        error.set(Signal::parse(text).err().map(|why| signal_error_text(&why)));
    };
    let write = move |text: String| {
        if text.trim() == current.trim() || Signal::parse(&text).is_err() {
            return;
        }
        board.checkpoint();
        parts.update(|list| edit::set_prop(list, index, &key, &text));
        dirty.set(true);
    };
    view! {
        <div class="flex min-w-0 flex-1 flex-col gap-1">
            <input
                type="text"
                spellcheck="false"
                placeholder="dc 1.2; sine f=50 a=0.1"
                prop:value=move || draft.get()
                on:input=move |event: ev::Event| {
                    let text = event_target_value(&event);
                    draft.set(text.clone());
                    if !composing(&event) {
                        judge(&text);
                    }
                }
                on:compositionend=move |event: ev::CompositionEvent| {
                    let text = event_target_value(&event);
                    draft.set(text.clone());
                    judge(&text);
                }
                on:change=move |event| write(event_target_value(&event))
                class=FIELD
            />
            {move || error.get().map(|why| view! {
                <p class="text-caption leading-snug text-crimson">{why}</p>
            })}
        </div>
    }
}

/// The presets, as a menu that writes the chosen one to `key`.
#[component]
fn Presets(board: Board, index: usize, key: String) -> impl IntoView {
    let Board { parts, dirty, .. } = board;
    view! {
        <select
            title=t!("lab.preset-hint")
            on:change=move |event| {
                let id = event_target_value(&event);
                if let Some(preset) = rusty_embed::signal::presets()
                    .into_iter()
                    .find(|preset| preset.id == id)
                {
                    board.checkpoint();
                    let text = preset.signal.to_string();
                    parts.update(|list| edit::set_prop(list, index, &key, &text));
                    dirty.set(true);
                }
            }
            class="h-[22px] max-w-[9rem] rounded-[5px] bg-sunken px-1 text-caption text-label-2 outline-none"
        >
            <option value="" selected=true>{t!("lab.presets")}</option>
            {rusty_embed::signal::presets()
                .into_iter()
                .map(|preset| view! { <option value=preset.id>{preset_name(preset.id)}</option> })
                .collect_view()}
        </select>
    }
}

/// A property with a number in it, written when it is left.
#[component]
fn NumberField(
    board: Board,
    index: usize,
    key: &'static str,
    label: String,
    hint: String,
    placeholder: &'static str,
    current: String,
) -> impl IntoView {
    let Board { parts, dirty, .. } = board;
    view! {
        <label class="flex items-center gap-2 text-footnote text-label-2" title=hint>
            <span class="shrink-0">{label}</span>
            <input
                type="text"
                placeholder=placeholder
                prop:value=current
                on:change=move |event| {
                    board.checkpoint();
                    let text = event_target_value(&event);
                    parts.update(|list| edit::set_prop(list, index, key, &text));
                    dirty.set(true);
                }
                class=FIELD
            />
        </label>
    }
}

/// One loop of what a signal plays, drawn small, and how long the loop is.
#[component]
fn Preview(samples: Vec<f64>, looped: generator::Loop, rate: u32) -> impl IntoView {
    const W: f64 = 200.0;
    const H: f64 = 36.0;
    let (path, range) = sparkline(&samples, W, H);
    let mut caption = t!(
        "simulate.signal-loop",
        rate = rate,
        seconds = looped.seconds(rate)
    );
    if !looped.seamless {
        caption.push_str(&t!("simulate.signal-seam"));
    }
    if looped.repeats_noise {
        caption.push_str(&t!("simulate.signal-repeats"));
    }
    let scale = range
        .map(|(low, high)| format!("{low:.3} … {high:.3}"))
        .unwrap_or_default();
    view! {
        <div class="flex flex-col gap-0.5">
            <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" class="h-9 w-full rounded-[4px] bg-sunken">
                <path d=path fill="none" stroke="#5fd0c8" stroke-width="1.2" vector-effect="non-scaling-stroke" />
            </svg>
            <p class="flex justify-between gap-2 font-mono text-caption text-label-4">
                <span>{caption}</span>
                <span>{scale}</span>
            </p>
        </div>
    }
}

/// A generator's signal, the rate it plays at, and the full scale of the
/// converter it feeds — which the sheet has to state, because no count can
/// be put on a pin without it.
#[component]
pub(super) fn GeneratorFields(
    board: Board,
    index: usize,
    props: std::collections::BTreeMap<String, String>,
) -> impl IntoView {
    let current = props.get(SIGNAL).cloned().unwrap_or_default();
    let rate = props.get("rate").cloned().unwrap_or_default();
    let fullscale = props.get("fullscale").cloned().unwrap_or_default();
    // One loop of this generator's own signal, as the backend renders it.
    let preview = board.parts.with_untracked(|list| {
        let part = &list.get(index)?.inst;
        let signal = generator::signal_at(part, SIGNAL)?.ok()?;
        let rate = generator::rate_at(part, "rate", GENERATOR_RATE).ok()?;
        let looped = generator::loop_of([&signal], rate);
        let samples = signal.render(
            f64::from(rate),
            looped.samples,
            generator::seed_of(part, SIGNAL),
        );
        Some((samples, looped, rate))
    });
    view! {
        <div class="flex flex-col gap-1.5">
            <div class="flex items-center gap-2">
                <span class="flex-1 text-footnote text-label-2" title=t!("simulate.signal-hint")>
                    {t!("simulate.signal")}
                </span>
                <Presets board=board index=index key=SIGNAL.to_string() />
            </div>
            <SignalField board=board index=index key=SIGNAL.to_string() current=current />
            {preview.map(|(samples, looped, rate)| view! {
                <Preview samples=samples looped=looped rate=rate />
            })}
            <NumberField
                board=board
                index=index
                key="rate"
                label=t!("simulate.signal-rate")
                hint=t!("simulate.signal-rate-hint")
                placeholder="20000"
                current=rate
            />
            <NumberField
                board=board
                index=index
                key="fullscale"
                label=t!("simulate.fullscale")
                hint=t!("simulate.fullscale-hint")
                placeholder="2.5"
                current=fullscale
            />
        </div>
    }
}

/// A signal for each reading of a sensor rusty answers for, and the rate
/// its register block plays at. A reading with no signal stays where its
/// slider puts it.
#[component]
pub(super) fn ReadingSignals(
    board: Board,
    index: usize,
    props: std::collections::BTreeMap<String, String>,
    readings: Vec<String>,
) -> impl IntoView {
    let rate = props.get(SIGNAL_RATE).cloned().unwrap_or_default();
    view! {
        <div class="flex flex-col gap-1.5">
            <span class="text-footnote text-label-2" title=t!("simulate.reading-signals-hint")>
                {t!("simulate.reading-signals")}
            </span>
            {readings
                .into_iter()
                .map(|reading| {
                    let key = format!("{SIGNAL_OF}{reading}");
                    let current = props.get(&key).cloned().unwrap_or_default();
                    view! {
                        <div class="flex items-start gap-2">
                            <span class="w-[3.5rem] shrink-0 pt-1 font-mono text-caption text-label-3">{reading}</span>
                            <SignalField board=board index=index key=key current=current />
                        </div>
                    }
                })
                .collect_view()}
            <NumberField
                board=board
                index=index
                key=SIGNAL_RATE
                label=t!("simulate.signal-rate")
                hint=t!("simulate.reading-rate-hint")
                placeholder=SENSOR_RATE_TEXT
                current=rate
            />
        </div>
    }
}

/// [`SENSOR_RATE`] as a placeholder reads it.
const SENSOR_RATE_TEXT: &str = "1000";

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::generator::SENSOR_RATE;

    #[test]
    fn the_placeholders_are_the_defaults() {
        assert_eq!(SENSOR_RATE_TEXT, SENSOR_RATE.to_string());
        assert_eq!("20000", GENERATOR_RATE.to_string());
    }
}
