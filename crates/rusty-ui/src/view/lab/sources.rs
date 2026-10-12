//! The signals on the sheet, and the knob each one has while a run goes
//! on. What each plays is `crate::lab`'s.

use leptos::prelude::*;

use rusty_embed::signal::Signal;
use rusty_i18n::t;

use crate::lab::{preset_name, signal_error_text, sources_of};
use crate::{
    controller,
    state::{AppState, Source},
};

/// The sheet's signals, and a field each to change what it plays while a
/// run goes on. Picking one is what the lab studies.
#[component]
pub(super) fn Sources() -> impl IntoView {
    let state = AppState::expect();
    let sources = Memo::new(move |_| {
        state.sim.plan.with(|plan| {
            plan.as_ref()
                .and_then(|plan| plan.board.as_ref())
                .map(sources_of)
                .unwrap_or_default()
        })
    });
    // The first one is studied until somebody picks another, and a pick
    // that is no longer on the sheet gives way to the first again.
    Effect::new(move |_| {
        let list = sources.get();
        let current = state.lab.source.get_untracked();
        let still_there = current
            .as_ref()
            .is_some_and(|picked| list.iter().any(|(source, _)| source == picked));
        if !still_there {
            state
                .lab
                .source
                .set(list.first().map(|(source, _)| source.clone()));
        }
    });

    move || {
        let list = sources.get();
        if list.is_empty() {
            return view! {
                <p class="px-3 py-2 text-caption leading-relaxed text-label-3">
                    {t!("lab.no-sources")}
                </p>
            }
            .into_any();
        }
        list.into_iter()
            .map(|(source, sheet_text)| view! { <SourceRow source=source sheet_text=sheet_text /> })
            .collect_view()
            .into_any()
    }
}

/// The sensors the firmware declared on its console, a field for each axis:
/// fed at the host's pace, and said to be.
#[component]
pub(super) fn ConsoleSources() -> impl IntoView {
    let state = AppState::expect();
    move || {
        let declared = state.sim.sensors.get();
        if declared.is_empty() {
            return None;
        }
        Some(view! {
            <span class="px-3 pb-1 pt-3 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                {t!("lab.console-heading")}
            </span>
            <p class="px-3 pb-1 text-caption leading-snug text-amber">{t!("lab.console-note")}</p>
            {declared
                .into_iter()
                .map(|def| {
                    let name = StoredValue::new(def.name.clone());
                    let unit = def.unit.clone().unwrap_or_default();
                    let axes = usize::from(def.components);
                    view! {
                        <div class="flex flex-col gap-1 border-b border-line px-3 py-2">
                            <div class="flex items-baseline gap-2">
                                <span class="min-w-0 flex-1 truncate font-mono text-footnote text-label">{def.name.clone()}</span>
                                <span class="text-caption text-label-3">{unit}</span>
                            </div>
                            {(0..axes)
                                .map(|axis| view! { <ConsoleAxis name=name axis=axis axes=axes /> })
                                .collect_view()}
                        </div>
                    }
                })
                .collect_view()}
        })
    }
}

/// One axis of a console sensor: a signal for it, played on Enter.
#[component]
fn ConsoleAxis(name: StoredValue<String>, axis: usize, axes: usize) -> impl IntoView {
    let state = AppState::expect();
    let draft = RwSignal::new(state.lab.console.with_untracked(|all| {
        name.with_value(|n| all.get(n).and_then(|texts| texts.get(axis).cloned()))
            .unwrap_or_default()
    }));
    let error = RwSignal::new(None::<String>);
    let running = move || state.app.session_running.get();
    let submit = move || {
        let text = draft.get_untracked();
        if let Err(why) = Signal::parse(&text) {
            error.set(Some(signal_error_text(&why)));
            return;
        }
        error.set(None);
        state.lab.console.update(|all| {
            let texts = all
                .entry(name.get_value())
                .or_insert_with(|| vec![String::new(); axes]);
            texts.resize(axes, String::new());
            texts[axis] = text;
        });
        controller::console_signals(state);
    };
    view! {
        <div class="flex items-center gap-2">
            <span class="w-4 shrink-0 font-mono text-caption text-label-4">{axis.to_string()}</span>
            <input
                type="text"
                spellcheck="false"
                placeholder="sine f=2 a=1"
                title=t!("lab.console-hint")
                disabled=move || !running()
                prop:value=move || draft.get()
                on:input=move |event| draft.set(event_target_value(&event))
                on:keydown=move |event: leptos::ev::KeyboardEvent| {
                    if event.key() == "Enter" && !event.is_composing() {
                        event.prevent_default();
                        submit();
                    }
                }
                class="h-[22px] min-w-0 flex-1 rounded-[5px] bg-sunken px-2 font-mono text-caption text-label outline-none ring-1 ring-line focus:ring-rust disabled:opacity-60"
            />
        </div>
        {move || error.get().map(|why| view! {
            <p class="text-caption leading-snug text-crimson">{why}</p>
        })}
    }
}

/// One signal: its name, what it plays, and — while a run goes on — a field
/// and the presets to change it. Changed here, it changes for this run; the
/// sheet keeps its own until the inspector changes it.
#[component]
fn SourceRow(source: Source, sheet_text: String) -> impl IntoView {
    let state = AppState::expect();
    let draft = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let playing = {
        let source = source.clone();
        Memo::new(move |_| {
            state
                .lab
                .playing
                .with(|playing| playing.get(&source).cloned())
                .unwrap_or_else(|| sheet_text.clone())
        })
    };
    // The field follows what plays, and a refusal is about a text that has
    // gone once a new one plays.
    Effect::new(move |_| {
        draft.set(playing.get());
        error.set(None);
    });
    let running = move || state.app.session_running.get();
    let picked = {
        let source = source.clone();
        move || state.lab.source.with(|s| s.as_ref() == Some(&source))
    };
    let submit = {
        let source = source.clone();
        move |text: String| match Signal::parse(&text) {
            Err(why) => error.set(Some(signal_error_text(&why))),
            Ok(_) => {
                error.set(None);
                controller::sim_signal_set(state, source.clone(), Some(text));
            }
        }
    };
    let pick = {
        let source = source.clone();
        move |_| state.lab.source.set(Some(source.clone()))
    };
    let submit_preset = submit.clone();
    let label = source.label();

    view! {
        <div
            on:click=pick
            class=move || {
                let base = "flex cursor-default flex-col gap-1 border-b border-line px-3 py-2";
                if picked() { format!("{base} bg-sunken") } else { base.to_string() }
            }
        >
            <div class="flex items-center gap-2">
                <span class="size-2 shrink-0 rounded-full bg-[#5fd0c8]" />
                <span class="min-w-0 flex-1 truncate font-mono text-footnote text-label">{label}</span>
                <select
                    title=t!("lab.preset-hint")
                    disabled=move || !running()
                    on:click=|event: leptos::ev::MouseEvent| event.stop_propagation()
                    on:change=move |event| {
                        let id = event_target_value(&event);
                        if let Some(preset) = rusty_embed::signal::presets()
                            .into_iter()
                            .find(|preset| preset.id == id)
                        {
                            submit_preset(preset.signal.to_string());
                        }
                    }
                    class="h-[22px] max-w-[8rem] rounded-[5px] bg-sunken px-1 text-caption text-label-2 outline-none disabled:opacity-40"
                >
                    <option value="" selected=true>{t!("lab.presets")}</option>
                    {rusty_embed::signal::presets()
                        .into_iter()
                        .map(|preset| view! { <option value=preset.id>{preset_name(preset.id)}</option> })
                        .collect_view()}
                </select>
            </div>
            <input
                type="text"
                spellcheck="false"
                title=move || if running() { t!("lab.live-hint") } else { t!("lab.idle-hint") }
                disabled=move || !running()
                prop:value=move || draft.get()
                on:input=move |event| draft.set(event_target_value(&event))
                on:keydown=move |event: leptos::ev::KeyboardEvent| {
                    if event.key() == "Enter" && !event.is_composing() {
                        event.prevent_default();
                        submit(draft.get_untracked());
                    }
                }
                class="h-[24px] min-w-0 rounded-[5px] bg-sunken px-2 font-mono text-caption text-label outline-none ring-1 ring-line focus:ring-rust disabled:opacity-60"
            />
            {move || error.get().map(|why| view! {
                <p class="text-caption leading-snug text-crimson">{why}</p>
            })}
        </div>
    }
}
