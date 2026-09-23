//! The interactive faces: a knob, a source, a screen, a rotor — HTML placed
//! under a symbol's body, each taking the pointer only where it must, so a
//! press beside a slider is still a press on the sheet.

use super::*;

/// Every part's face, keyed by index like the parts themselves.
#[component]
pub(super) fn Faces(board: Board) -> impl IntoView {
    let Board {
        state,
        sensors,
        parts,
        dirty,
        ..
    } = board;
    view! {
        <For
            each=move || 0..parts.with(Vec::len)
            key=|index| *index
            children=move |index: usize| {
                let this = Memo::new(move |_| parts.with(|list| list.get(index).cloned()));
                let behaviour = Memo::new(move |_| {
                    this.with(|p| p.as_ref().and_then(|p| p.symbol.as_ref()).map(behaviour_of))
                });
                let reference = Memo::new(move |_| {
                    this.with(|p| p.as_ref().map(|p| p.inst.reference.clone()).unwrap_or_default())
                });
                let anchor = Memo::new(move |_| {
                    this.with(|p| {
                        p.as_ref().map(|p| {
                            let (x0, _, x1, y1) = part_box(p);
                            ((x0 + x1) / 2.0, y1 + 14.0)
                        }).unwrap_or_default()
                    })
                });
                let gpio_at = move |pin: &str| board.gpio_for(&reference.get_untracked(), pin);
                move || {
                    let (cx, cy) = anchor.get();
                    let style = format!("left: {cx}px; top: {cy}px; transform: translateX(-50%)");
                    match behaviour.get() {
                        // The module the firmware asked to be
                        // fed. Its value names the channel;
                        // a channel the firmware never
                        // declared gets no slider, for the
                        // reason the tunables get none — a
                        // range rusty invented is how
                        // somebody injects 2000 deg/s into a
                        // loop written for 250.
                        // A sensor rusty answers for register by
                        // register: a slider per reading, in the
                        // units a person reads. Between runs a
                        // slider says where the next run starts,
                        // which is the sheet's; during one it
                        // moves the reading the firmware gets.
                        Some(Behaviour::Sensor)
                            if this.with(|p| {
                                p.as_ref()
                                    .and_then(|p| p.inst.props.get("model"))
                                    .is_some_and(|id| {
                                        sensors.with_value(|all| {
                                            rusty_embed::sensor::Spec::find(all, id).is_some()
                                        })
                                    })
                            }) =>
                        {
                            let (model, props) = this.with(|p| {
                                let inst = &p.as_ref()?.inst;
                                let model = sensors.with_value(|all| {
                                    rusty_embed::sensor::Spec::find(all, inst.props.get("model")?).cloned()
                                })?;
                                Some((model, inst.props.clone()))
                            })?;
                            let part_ref = reference.get_untracked();
                            Some(view! {
                                <div class="pointer-events-none absolute flex flex-col gap-0.5" style=style>
                                    <span class="font-mono text-caption text-label-3">{model.name.clone()}</span>
                                    {model
                                        .channels
                                        .iter()
                                        .map(|channel| {
                                            let key = channel.key.clone();
                                            let (min, max, unit) = (channel.min, channel.max, channel.unit.clone());
                                            let start = props
                                                .get(&key)
                                                .and_then(|text| text.trim().parse::<f64>().ok())
                                                .unwrap_or(channel.rest);
                                            let shown = {
                                                let part_ref = part_ref.clone();
                                                let key = key.clone();
                                                Memo::new(move |_| {
                                                    state.sim.readings.with(|held| {
                                                        held.get(&(part_ref.clone(), key.clone())).copied()
                                                    })
                                                    .unwrap_or(start)
                                                })
                                            };
                                            let moved = part_ref.clone();
                                            let typed = key.clone();
                                            let written = key.clone();
                                            view! {
                                                <span class="flex items-center gap-1.5">
                                                    <span class="w-[11ch] truncate text-caption text-label-3">
                                                        {reading_label(&key)}
                                                    </span>
                                                    // Any value, not a grid: a stepped slider shows
                                                    // 0.5 g as 0.52 and moves the reading the moment
                                                    // it is touched.
                                                    <input
                                                        type="range"
                                                        min=min
                                                        max=max
                                                        step="any"
                                                        prop:value=move || shown.get().to_string()
                                                        on:pointerdown=move |event: ev::PointerEvent| {
                                                            event.stop_propagation()
                                                        }
                                                        on:input=move |event: ev::Event| {
                                                            if let Ok(value) = event_target_value(&event).parse::<f64>() {
                                                                controller::sim_reading(state, moved.clone(), typed.clone(), value);
                                                            }
                                                        }
                                                        on:change=move |event: ev::Event| {
                                                            if state.app.session_running.get_untracked() {
                                                                return;
                                                            }
                                                            if let Ok(value) = event_target_value(&event).parse::<f64>() {
                                                                board.checkpoint();
                                                                parts.update(|list| {
                                                                    edit::set_prop(list, index, &written, &reading_text(value))
                                                                });
                                                                dirty.set(true);
                                                            }
                                                        }
                                                        class="pointer-events-auto w-[70px] accent-[#5fd0c8]"
                                                    />
                                                    <span class="w-[10ch] text-right font-mono text-caption text-label-2">
                                                        {move || format!("{} {unit}", reading_text(shown.get()))}
                                                    </span>
                                                </span>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                                .into_any())
                        }
                        Some(Behaviour::Sensor) => {
                            let wanted = this
                                .with(|p| p.as_ref().map(|p| p.inst.value.trim().to_string()))
                                .unwrap_or_default();
                            let declared = state
                                .sim
                                .sensors
                                .with(|all| all.iter().find(|s| s.name == wanted).cloned());
                            let Some(def) = declared else {
                                return Some(view! {
                                    <div class="pointer-events-none absolute max-w-[190px]" style=style>
                                        <span class="rounded-[4px] bg-raised px-1.5 py-1 text-caption leading-snug text-label-4 ring-1 ring-line">
                                            {if wanted.is_empty() {
                                                t!("simulate.sensor-unnamed")
                                            } else {
                                                t!("simulate.sensor-undeclared", name = wanted.clone())
                                            }}
                                        </span>
                                    </div>
                                }
                                    .into_any());
                            };
                            let count = def.components.max(1) as usize;
                            let min = def.min.unwrap_or(-1.0);
                            let max = def.max.unwrap_or(1.0);
                            let unit = def.unit.clone().unwrap_or_default();
                            let name = def.name.clone();
                            let held = {
                                let name = name.clone();
                                move || {
                                    state.sim.sensor_values.with(|all| {
                                        all.get(&name).cloned().unwrap_or_else(|| vec![0.0; count])
                                    })
                                }
                            };
                            Some(view! {
                                <div class="pointer-events-none absolute flex flex-col gap-0.5" style=style>
                                    <span class="font-mono text-caption text-label-3">
                                        {format!("{name} {unit}")}
                                    </span>
                                    {(0..count)
                                        .map(|axis| {
                                            let name = name.clone();
                                            let held = held.clone();
                                            // A `Copy` handle, so the slider and the number beside it
                                            // can both read the sample.
                                            let sample = StoredValue::new(held.clone());
                                            let shown = move || {
                                                sample.with_value(|held| held().get(axis).copied().unwrap_or(0.0))
                                            };
                                            view! {
                                                <span class="flex items-center gap-1.5">
                                                    <input
                                                        type="range"
                                                        min=min
                                                        max=max
                                                        step=(max - min) / 200.0
                                                        prop:value=move || shown().to_string()
                                                        on:pointerdown=move |event: ev::PointerEvent| {
                                                            event.stop_propagation()
                                                        }
                                                        on:input=move |event: ev::Event| {
                                                            let Ok(value) = event_target_value(&event).parse::<f32>()
                                                            else {
                                                                return;
                                                            };
                                                            let mut sample = held();
                                                            sample.resize(count, 0.0);
                                                            sample[axis] = value;
                                                            controller::sim_sensor(state, name.clone(), sample);
                                                        }
                                                        class="pointer-events-auto w-[70px] accent-[#5fd0c8]"
                                                    />
                                                    <span class="w-[5ch] text-right font-mono text-caption text-label-2">
                                                        {move || format!("{:.2}", shown())}
                                                    </span>
                                                </span>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                                .into_any())
                        }
                        Some(Behaviour::Pot) => {
                            // Where the sheet says this knob starts, read the way
                            // the analog source reads its own — the backend sends
                            // the matching counts as soon as a run connects, so
                            // the panel and the converter agree before the first
                            // drag rather than after it.
                            let start_turn = this.with(|p| {
                                p.as_ref().map_or(rusty_embed::nets::POT_REST, |p| {
                                    rusty_embed::nets::pot_start(&p.inst)
                                })
                            });
                            let turned = RwSignal::new(start_turn);
                            let span = board.pot_span_for(&reference.get_untracked());
                            // The converter's full scale, named the same way the
                            // analog source names it, so one part does not read
                            // on a different scale from its neighbour.
                            let max = this.with(|p| {
                                p.as_ref().map_or(rusty_embed::nets::ADC_MAX, |p| {
                                    rusty_embed::nets::adc_max(&p.inst)
                                })
                            });
                            let angle = move || -135.0 + f64::from(turned.get()) / 255.0 * 270.0;
                            Some(view! {
                                <div class="pointer-events-none absolute flex items-center gap-1.5" style=style>
                                    <span class="relative grid size-5 shrink-0 place-items-center rounded-full bg-[#3a404a] ring-1 ring-[#5a626e]">
                                        <span
                                            class="absolute top-[3px] left-1/2 h-[7px] w-[2px] rounded-full bg-[#c9a227]"
                                            style=move || format!("transform-origin: 1px 7px; transform: translateX(-1px) rotate({:.0}deg)", angle())
                                        />
                                    </span>
                                    <input
                                        type="range"
                                        min="0"
                                        max="255"
                                        value=start_turn
                                        title=t!("simulate.pot-hint")
                                        on:pointerdown=move |event: ev::PointerEvent| event.stop_propagation()
                                        on:input=move |event: ev::Event| {
                                            if let Ok(value) = event_target_value(&event).parse::<u8>() {
                                                turned.set(value);
                                                // The text line, for firmware that
                                                // reads rusty's protocol.
                                                if let Some(gpio) = gpio_at("W") {
                                                    controller::sim_pot(state, gpio, value);
                                                }
                                                // And the counts, for firmware that
                                                // just calls read_oneshot() — but
                                                // only where the sheet said what
                                                // the track's ends are on.
                                                if let Some(span) = span {
                                                    controller::sim_analog(
                                                        state,
                                                        span.gpio,
                                                        span.counts(value, max),
                                                    );
                                                }
                                            }
                                        }
                                        class="pointer-events-auto w-[56px] accent-[#c9a227]"
                                    />
                                </div>
                            }.into_any())
                        }
                        Some(Behaviour::Analog) => {
                            // Counts, and the count is what is
                            // shown. rusty does not know the
                            // divider on this board, so it does
                            // not print a voltage it cannot
                            // stand behind.
                            let max = this.with(|p| {
                                p.as_ref().map_or(rusty_embed::nets::ADC_MAX, |p| {
                                    rusty_embed::nets::adc_max(&p.inst)
                                })
                            });
                            // Where the sheet says this source
                            // starts, until somebody moves it.
                            // The backend sends the same value
                            // down the pin channel as soon as
                            // a run connects, so the slider and
                            // the converter agree before the
                            // first drag rather than after it.
                            let start = this.with(|p| {
                                p.as_ref().map_or(0, |p| rusty_embed::nets::analog_start(&p.inst))
                            });
                            let held = move || {
                                gpio_at("OUT")
                                    .map(|gpio| state.sim.analog.with(|a| a.get(&gpio).copied().unwrap_or(start)))
                                    .unwrap_or(start)
                            };
                            Some(view! {
                                <div class="pointer-events-none absolute flex items-center gap-2" style=style>
                                    <input
                                        type="range"
                                        min="0"
                                        max=max
                                        prop:value=move || held().to_string()
                                        on:pointerdown=move |event: ev::PointerEvent| event.stop_propagation()
                                        on:input=move |event: ev::Event| {
                                            if let Ok(value) = event_target_value(&event).parse::<u16>()
                                                && let Some(gpio) = gpio_at("OUT")
                                            {
                                                controller::sim_analog(state, gpio, value);
                                            }
                                        }
                                        class="pointer-events-auto w-[68px] accent-[#4aa8ff]"
                                    />
                                    // What the firmware's own
                                    // converter last took off
                                    // this pin, which is the
                                    // difference between a
                                    // slider that does nothing
                                    // and firmware that is not
                                    // reading. Only rusty's
                                    // emulator can say, so no
                                    // claim when it has not.
                                    <span
                                        class="w-[4ch] text-right font-mono text-caption text-label-2"
                                        title=move || {
                                            match gpio_at("OUT")
                                                .and_then(|gpio| state.sim.adc.with(|a| a.get(&gpio).copied()))
                                            {
                                                Some(counts) => t!("simulate.adc-read", counts = counts),
                                                None => t!("simulate.adc-unread"),
                                            }
                                        }
                                    >
                                        {held}
                                    </span>
                                </div>
                            }.into_any())
                        }
                        Some(Behaviour::Motor) => {
                            let duty = move || {
                                gpio_at("PWM").and_then(|gpio| state.sim.pwm.with(|pwm| pwm.get(&gpio).copied()))
                            };
                            let drive = move || {
                                let reference = reference.get();
                                // A direction pin under PWM has
                                // no one level, and reads as
                                // none rather than as either.
                                let level = |pin: &str| match board.level_of(&PinRef::new(reference.as_str(), pin)) {
                                    Some(period::Level::High) => Some(true),
                                    Some(period::Level::Low) => Some(false),
                                    _ => None,
                                };
                                // A fan has no direction pins,
                                // and reading two unwired inputs
                                // as two lows would call it
                                // COAST and stop a motor that has
                                // nowhere to say otherwise.
                                match (level("IN1"), level("IN2")) {
                                    (None, None) => rusty_embed::Drive::Forward,
                                    (a, b) => rusty_embed::Drive::from_inputs(a.unwrap_or(false), b.unwrap_or(false)),
                                }
                            };
                            let spin = move || match duty().map(|d| d.duty) {
                                Some(d) if d > 0.01 && drive().turns() => {
                                    let seconds = (0.25 / d).clamp(0.25, 4.0);
                                    let way = match drive() {
                                        rusty_embed::Drive::Reverse => "reverse",
                                        _ => "normal",
                                    };
                                    format!("animation-duration: {seconds:.2}s; animation-direction: {way}")
                                }
                                _ => "animation: none".to_string(),
                            };
                            let readout = move || match duty() {
                                None => t!("simulate.no-duty"),
                                Some(d) => {
                                    format!("{:.0}% {}", d.duty * 100.0, drive().label())
                                }
                            };
                            let tone = move || match duty() {
                                None => "text-label-3",
                                Some(_) if !drive().turns() => "text-label-2",
                                Some(_) => "text-label",
                            };
                            Some(view! {
                                <div class="pointer-events-none absolute flex items-center gap-2" style=style>
                                    <span class="relative grid size-5 shrink-0 place-items-center rounded-full border border-line-strong bg-sunken">
                                        <span class="absolute inset-[3px] animate-spin" style=spin>
                                            <span class="absolute top-0 left-1/2 h-1/2 w-px -translate-x-1/2 bg-label-2" />
                                            <span class="absolute bottom-0 left-1/2 h-1/2 w-px -translate-x-1/2 bg-line-strong" />
                                        </span>
                                        <span class="size-1 rounded-full bg-label-3" />
                                    </span>
                                    <span class=move || format!("font-mono text-caption {}", tone())>{readout}</span>
                                </div>
                            }.into_any())
                        }
                        _ => None,
                    }
                }
            }
        />
    }
}
