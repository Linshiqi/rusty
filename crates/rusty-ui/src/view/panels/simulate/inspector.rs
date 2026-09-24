//! The properties panel: what the selection is, what it is joined to and
//! what it reads, and the fields that change it.

use super::*;

/// Beside the sheet, or floating over it beside the editor: the selected wire,
/// the marked group, or the selected part with its fields.
#[component]
pub(super) fn Inspector(board: Board) -> impl IntoView {
    let Board {
        state,
        compact,
        live,
        sensors,
        parts,
        wires,
        dirty,
        selected,
        selected_wire,
        marked,
        period,
        weights,
        findings,
        ..
    } = board;
    view! {
        <div class=move || {
            if !compact {
                "flex w-[200px] flex-none flex-col overflow-y-auto border-l border-line bg-sidebar"
            } else if board.inspecting() {
                if board.inspector_left() {
                    "absolute top-12 left-2 z-30 flex max-h-[calc(100%-3.5rem)] w-[220px] flex-col overflow-y-auto rounded-[8px] bg-sidebar shadow-2xl ring-1 ring-line-strong"
                } else {
                    "absolute top-12 right-2 z-30 flex max-h-[calc(100%-3.5rem)] w-[220px] flex-col overflow-y-auto rounded-[8px] bg-sidebar shadow-2xl ring-1 ring-line-strong"
                }
            } else {
                "hidden"
            }
        }>
            {move || {
                // A selected wire outranks a selected part.
                if let Some(index) = selected_wire.get() {
                    let Some(wire) = wires.with(|all| all.get(index).cloned()) else {
                        return ().into_any();
                    };
                    let bends = wire.bends.len();
                    return view! {
                        <div class="flex flex-col gap-2 p-3">
                            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                {t!("simulate.wire")}
                            </span>
                            <p class="font-mono text-footnote text-label-2">
                                {format!("{} → {}", wire.from, wire.to)}
                            </p>
                            <p class="text-footnote text-label-4">
                                {t!("simulate.wire-bends", bends = bends.to_string())}
                            </p>
                            // The probe. A wire on a sheet is a
                            // question — what is this actually
                            // joined to, and what is it sitting at
                            // — and the answer was only ever the
                            // colour of the line while it ran.
                            {move || {
                                let Some(members) = period.with(|p| {
                                    let nets = p.nets();
                                    nets.net_of(&wire.from).map(|net| {
                                        nets.members(net)
                                            .iter()
                                            .map(PinRef::to_string)
                                            .collect::<Vec<String>>()
                                    })
                                }) else {
                                    return ().into_any();
                                };
                                let level = board.level_of(&wire.from);
                                let tone = match level {
                                    Some(period::Level::High | period::Level::Switching(_)) => "text-[#5ecf7a]",
                                    Some(period::Level::Low) => "text-label-2",
                                    None => "text-label-4",
                                };
                                let word = level_word(level);
                                // And what it is *at*. High and low
                                // are the rules' reading; this is
                                // the solver's, and on a divider
                                // they are the same net saying two
                                // different useful things — "not
                                // being driven" and "1.65 V".
                                //
                                // Where there is no number, the
                                // reason goes in its place. This is
                                // the moment somebody asked what a
                                // net is at, so it is the moment to
                                // say which property would answer
                                // them — a probe that silently
                                // showed nothing would read as a
                                // feature that does not work.
                                let at = period.with(|p| {
                                    weights.with(|w| match p.volts_at(w, &wire.from) {
                                        Ok(volts) => Ok(volts.map(readout::volts)),
                                        Err(why) => Err(unsolved_text(why)),
                                    })
                                });
                                view! {
                                    <div class="flex flex-col gap-1 border-t border-line pt-2">
                                        <span class="text-caption text-label-4">
                                            {t!("simulate.net")}
                                        </span>
                                        <div class="flex items-baseline gap-2">
                                            <p class=format!("font-mono text-footnote {tone}")>{word}</p>
                                            {at
                                                .as_ref()
                                                .ok()
                                                .and_then(|volts| volts.clone())
                                                .map(|volts| {
                                                    view! {
                                                        <p class="font-mono text-footnote text-label">{volts}</p>
                                                    }
                                                })}
                                        </div>
                                        {at
                                            .err()
                                            .map(|why| {
                                                view! {
                                                    <p class="text-caption leading-snug text-label-4">{why}</p>
                                                }
                                            })}
                                        <p class="font-mono text-caption leading-snug text-label-3 select-text">
                                            {members.join("  ")}
                                        </p>
                                    </div>
                                }
                                    .into_any()
                            }}
                            <button
                                type="button"
                                on:click=move |_| board.straighten_wire(index)
                                class="rounded-[6px] px-2 py-1 text-footnote text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label"
                            >
                                {t!("simulate.straighten")}
                            </button>
                            <button
                                type="button"
                                on:click=move |_| board.delete_selection()
                                class="rounded-[6px] px-2 py-1 text-footnote text-crimson ring-1 ring-line hover:bg-sunken"
                            >
                                {t!("simulate.disconnect-del")}
                            </button>
                        </div>
                    }
                        .into_any();
                }
                // Several parts under the band: how many, and what
                // the keys do to them.
                let count = marked.with(Vec::len);
                if count > 1 {
                    return view! {
                        <div class="flex flex-col gap-2 p-3">
                            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                {t!("simulate.selected-count", count = count)}
                            </span>
                            <p class="text-caption leading-snug text-label-4">
                                {t!("simulate.group-hint")}
                            </p>
                            <button
                                type="button"
                                on:click=move |_| board.remove_marked()
                                class="rounded-[6px] px-2 py-1 text-footnote text-crimson ring-1 ring-line hover:bg-sunken"
                            >
                                {t!("simulate.remove-selected")}
                            </button>
                        </div>
                    }
                        .into_any();
                }
                let Some(index) = selected.get() else {
                    return view! {
                        <p class="p-3 text-footnote text-label-4">
                            {if live.get() {
                                t!("simulate.live-hint")
                            } else {
                                t!("simulate.nothing-selected")
                            }}
                        </p>
                    }
                        .into_any();
                };
                let Some(part) = parts.with(|list| list.get(index).cloned()) else {
                    return ().into_any();
                };
                if part.is_kit() {
                    return view! {
                        <div class="flex flex-col gap-2 p-3">
                            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                {part.inst.reference.clone()}
                            </span>
                            <p class="font-mono text-footnote text-label-2">{part.inst.value.clone()}</p>
                            <p class="text-caption leading-snug text-label-4">{t!("simulate.kit-hint")}</p>
                        </div>
                    }
                        .into_any();
                }

                let behaviour = part.symbol.as_ref().map(behaviour_of);
                let symbol_id = part.inst.symbol.clone();
                let symbol_name = part
                    .symbol
                    .as_ref()
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| symbol_id.clone());
                let description = part.symbol.as_ref().and_then(|s| s.description.clone());
                let reference = part.inst.reference.clone();
                // Every pin, with what it is wired to — the panel and
                // the drawing cannot call one pin two things because
                // both read the symbol.
                let list = parts.get();
                let all = wires.get();
                let pin_rows: Vec<(String, String)> = part
                    .pins()
                    .iter()
                    .map(|pin| {
                        let mut ends: Vec<String> = edit::wires_at(&list, &all, index, &pin.number)
                            .into_iter()
                            .filter_map(|w| {
                                let wire = all.get(w)?;
                                let other = if edit::end_is(&list, &wire.from, index, &pin.number) {
                                    &wire.to
                                } else {
                                    &wire.from
                                };
                                Some(other.to_string())
                            })
                            .collect();
                        ends.sort();
                        let label = if pin.name == "~" || pin.name == pin.number {
                            pin.number.clone()
                        } else {
                            format!("{} ({})", pin.name, pin.number)
                        };
                        (label, ends.join(", "))
                    })
                    .collect();
                let own_warnings: Vec<String> = findings.with(|found| {
                    found
                        .iter()
                        .filter(|w| match w {
                            Warning::LedWithoutResistor { part } | Warning::SwitchDrivesNothing { part } => {
                                *part == reference
                            }
                            _ => false,
                        })
                        .map(warning_text)
                        .collect()
                });
                let analog_max = part.inst.props.get("max").cloned().unwrap_or_default();
                // The bus fields are offered to a part that has the
                // pins for it and to no other: a lamp with an I2C
                // address is a claim about a part that cannot carry
                // one, and the emulator would honour it.
                let on_a_bus = part
                    .pins()
                    .iter()
                    .any(|pin| pin.name.eq_ignore_ascii_case("SDA"));
                let bus_addr = part.inst.props.get("addr").cloned().unwrap_or_default();
                let bus_regs = part.inst.props.get("regs").cloned().unwrap_or_default();
                // What this address has actually done on the bus, most
                // recent last. The point of showing it beside the
                // fields is that "the slider does nothing" and "the
                // firmware never asked" look identical without it.
                let bus_address = rusty_embed::nets::hex_address(&bus_addr);
                // The wire's half of the same idea, offered to a part
                // that has a clock pin.
                let on_a_wire = part
                    .pins()
                    .iter()
                    .any(|pin| pin.name.eq_ignore_ascii_case("SCK"));
                let wire_select = part.inst.props.get("cs").cloned().unwrap_or_default();
                let wire_miso = part.inst.props.get("miso").cloned().unwrap_or_default();
                let wire_line = wire_select.trim().parse::<u8>().ok();
                let wire_traffic = move || {
                    let Some(select) = wire_line else {
                        return Vec::new();
                    };
                    state.sim.spi.with(|wire| {
                        wire.iter()
                            .filter(|report| report.select == select)
                            .rev()
                            .take(6)
                            .map(|report| {
                                let bytes: String = report
                                    .bytes
                                    .iter()
                                    .map(|b| format!("{b:02x}"))
                                    .collect();
                                format!("{} {bytes}", report.verb)
                            })
                            .collect::<Vec<_>>()
                    })
                };
                let bus_traffic = move || {
                    let Some(address) = bus_address else {
                        return Vec::new();
                    };
                    state.sim.i2c.with(|bus| {
                        bus.iter()
                            .filter(|report| report.address == address)
                            .rev()
                            .take(6)
                            .map(|report| {
                                let bytes: String = report
                                    .bytes
                                    .iter()
                                    .map(|b| format!("{b:02x}"))
                                    .collect();
                                format!("{} {bytes}", report.verb)
                            })
                            .collect::<Vec<_>>()
                    })
                };

                view! {
                    <div class="flex flex-col gap-2 p-3">
                        <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                            {symbol_name}
                        </span>
                        <p class="font-mono text-caption text-label-4 select-text" title=description.clone().unwrap_or_default()>
                            {symbol_id.clone()}
                        </p>
                        {part.symbol.is_none().then(|| view! {
                            <p class="text-caption leading-snug text-crimson">{t!("simulate.unknown-symbol")}</p>
                        })}
                        // The reference, KiCad's word for the name on
                        // the sheet: renaming it renames the wires'
                        // ends, and a taken name is refused.
                        <input
                            type="text"
                            title=t!("simulate.reference-hint")
                            prop:value=part.inst.reference.clone()
                            on:change=move |event| {
                                board.checkpoint();
                                let text = event_target_value(&event);
                                let ok = parts
                                    .try_update(|list| {
                                        wires.try_update(|all| edit::rename(list, all, index, &text)).unwrap_or(false)
                                    })
                                    .unwrap_or(false);
                                if ok {
                                    dirty.set(true);
                                }
                            }
                            class="h-[26px] rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                        />
                        <input
                            type="text"
                            title=t!("simulate.value-hint")
                            placeholder=t!("simulate.value")
                            prop:value=part.inst.value.clone()
                            on:change=move |event| {
                                board.checkpoint();
                                let text = event_target_value(&event);
                                parts.update(|list| edit::set_value(list, index, &text));
                                dirty.set(true);
                            }
                            class="h-[26px] rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                        />
                        {(behaviour == Some(Behaviour::Led)).then(|| {
                            view! {
                                <div class="flex items-center gap-1.5">
                                    {[
                                        ("green", "bg-[#3ddc84]", t!("parts.color-green")),
                                        ("blue", "bg-[#4aa8ff]", t!("parts.color-blue")),
                                        ("red", "bg-[#ff5c5c]", t!("parts.color-red")),
                                        ("yellow", "bg-[#ffd75c]", t!("parts.color-yellow")),
                                    ]
                                        .into_iter()
                                        .map(|(name, swatch, label)| {
                                            view! {
                                                <button
                                                    type="button"
                                                    title=label
                                                    on:click=move |_| {
                                                        board.checkpoint();
                                                        parts.update(|list| edit::set_value(list, index, name));
                                                        dirty.set(true);
                                                    }
                                                    class=format!("size-5 rounded-full ring-1 ring-line hover:ring-2 {swatch}")
                                                />
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                        })}
                        {(behaviour == Some(Behaviour::Analog)).then(|| {
                            view! {
                                <label class="flex items-center gap-2 text-footnote text-label-2">
                                    <span class="shrink-0">{t!("simulate.analog-max")}</span>
                                    <input
                                        type="text"
                                        title=t!("simulate.max-hint")
                                        placeholder="4095"
                                        prop:value=analog_max.clone()
                                        on:change=move |event| {
                                            board.checkpoint();
                                            let text = event_target_value(&event);
                                            parts.update(|list| edit::set_prop(list, index, "max", &text));
                                            dirty.set(true);
                                        }
                                        class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    />
                                </label>
                            }
                        })}
                        // What a generator plays, how fast, and the
                        // converter's full scale that turns it into
                        // counts.
                        {(behaviour == Some(Behaviour::Generator)).then(|| {
                            view! {
                                <signal_fields::GeneratorFields
                                    board=board
                                    index=index
                                    props=part.inst.props.clone()
                                />
                            }
                        })}
                        // The two pulse widths this servo answers
                        // to. Stated rather than assumed: 500..2500
                        // and 1000..2000 are both ordinary, and
                        // reading one as the other puts the horn
                        // forty degrees from where the firmware asked
                        // at each end. Empty is the first pair.
                        {(behaviour == Some(Behaviour::Servo)).then(|| {
                            let low = part.inst.props.get("min").cloned().unwrap_or_default();
                            let high = part.inst.props.get("max").cloned().unwrap_or_default();
                            let set = move |key: &'static str| {
                                move |event: web_sys::Event| {
                                    board.checkpoint();
                                    let text = event_target_value(&event);
                                    parts.update(|list| edit::set_prop(list, index, key, &text));
                                    dirty.set(true);
                                }
                            };
                            view! {
                                <label
                                    class="flex items-center gap-2 text-footnote text-label-2"
                                    title=t!("simulate.servo-pulse-hint")
                                >
                                    <span class="shrink-0">{t!("simulate.servo-pulse")}</span>
                                    <input
                                        type="text"
                                        placeholder="500"
                                        prop:value=low
                                        on:change=set("min")
                                        class="h-[26px] w-0 min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    />
                                    <input
                                        type="text"
                                        placeholder="2500"
                                        prop:value=high
                                        on:change=set("max")
                                        class="h-[26px] w-0 min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    />
                                </label>
                            }
                        })}
                        // Which part a sensor module is: one rusty
                        // answers for register by register, or the
                        // channel the firmware declared.
                        {(behaviour == Some(Behaviour::Sensor)).then(|| {
                            let current = part.inst.props.get("model").cloned().unwrap_or_default();
                            let known = sensors.with_value(|all| {
                                rusty_embed::sensor::Spec::find(all, &current).map(|spec| spec.id.clone())
                            });
                            let has_address = part
                                .inst
                                .props
                                .get("addr")
                                .is_some_and(|address| !address.trim().is_empty());
                            view! {
                                <label
                                    class="flex items-center gap-2 text-footnote text-label-2"
                                    title=t!("simulate.sensor-model-hint")
                                >
                                    <span class="shrink-0">{t!("simulate.sensor-model")}</span>
                                    <select
                                        on:change=move |event| {
                                            board.checkpoint();
                                            let id = event_target_value(&event);
                                            let first = sensors.with_value(|all| {
                                                rusty_embed::sensor::Spec::find(all, &id)
                                                    .and_then(|spec| spec.addresses.first().copied())
                                            });
                                            parts.update(|list| {
                                                edit::set_prop(list, index, "model", &id);
                                                // The address the breakout ships with,
                                                // unless the sheet already said one.
                                                if let Some(address) = first
                                                    && !has_address
                                                {
                                                    let address = format!("{address:02x}");
                                                    edit::set_prop(list, index, "addr", &address);
                                                }
                                            });
                                            dirty.set(true);
                                        }
                                        class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-1.5 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    >
                                        <option value="" selected=known.is_none()>
                                            {t!("simulate.sensor-model-declared")}
                                        </option>
                                        {sensors
                                            .get_value()
                                            .into_iter()
                                            .map(|spec| {
                                                let chosen = known.as_deref() == Some(spec.id.as_str());
                                                view! {
                                                    <option value=spec.id selected=chosen>
                                                        {spec.name}
                                                    </option>
                                                }
                                            })
                                            .collect_view()}
                                    </select>
                                </label>
                                // A part rusty answers for can have its
                                // readings moved by a signal, played in its
                                // own registers against the firmware's clock.
                                {known.as_ref().and_then(|id| {
                                    let readings: Vec<String> = sensors.with_value(|all| {
                                        rusty_embed::sensor::Spec::find(all, id).map(|spec| {
                                            spec.channels.iter().map(|c| c.key.clone()).collect()
                                        })
                                    })?;
                                    Some(view! {
                                        <signal_fields::ReadingSignals
                                            board=board
                                            index=index
                                            props=part.inst.props.clone()
                                            readings=readings
                                        />
                                    })
                                })}
                            }
                        })}
                        // Which controller is behind the glass, so
                        // the bytes on the bus can be read as a
                        // picture. Named by the sheet rather than
                        // taken from the traffic: the SH1106's window
                        // sits two columns into its RAM, and a
                        // decoder that chose for itself would draw
                        // something nobody could check. Until it is
                        // named the screen shows what the firmware
                        // prints to `[rusty:disp]`, as it always has.
                        {(behaviour == Some(Behaviour::Display)).then(|| {
                            let current = part.inst.props.get("panel").cloned().unwrap_or_default();
                            let current_panel = rusty_embed::screen::Panel::from_id(&current);
                            let has_address = part
                                .inst
                                .props
                                .get("addr")
                                .is_some_and(|address| !address.trim().is_empty());
                            view! {
                                <label
                                    class="flex items-center gap-2 text-footnote text-label-2"
                                    title=t!("simulate.screen-panel-hint")
                                >
                                    <span class="shrink-0">{t!("simulate.screen-panel")}</span>
                                    <select
                                        on:change=move |event| {
                                            board.checkpoint();
                                            let id = event_target_value(&event);
                                            let named = rusty_embed::screen::Panel::from_id(&id).is_some();
                                            parts.update(|list| {
                                                edit::set_prop(list, index, "panel", &id);
                                                // The address these modules ship
                                                // with, unless the sheet said one.
                                                if named && !has_address {
                                                    edit::set_prop(list, index, "addr", "3c");
                                                }
                                            });
                                            dirty.set(true);
                                        }
                                        class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-1.5 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    >
                                        <option value="" selected=current_panel.is_none()>
                                            {t!("simulate.screen-panel-text")}
                                        </option>
                                        {rusty_embed::screen::Panel::ALL
                                            .into_iter()
                                            .map(|panel| {
                                                let chosen = current_panel == Some(panel);
                                                view! {
                                                    <option value=panel.id() selected=chosen>
                                                        {panel.name()}
                                                    </option>
                                                }
                                            })
                                            .collect_view()}
                                    </select>
                                </label>
                            }
                        })}
                        // A panel nobody has switched on is drawn
                        // faintly, and here is where it says why: a
                        // dark screen and a driver that never sent
                        // `0xAF` look identical on the desk.
                        {move || {
                            let address = parts
                                .with(|list| list.get(index).and_then(display_address))?;
                            let dark = state.sim.screens.with(|screens| {
                                screens.get(&address).is_some_and(|screen| {
                                    screen.written() && !screen.is_on()
                                })
                            });
                            dark.then(|| {
                                view! {
                                    <p class="text-footnote text-label-3">
                                        {t!("simulate.screen-off")}
                                    </p>
                                }
                            })
                        }}
                        {on_a_bus.then(|| {
                            view! {
                                <label class="flex items-center gap-2 text-footnote text-label-2">
                                    <span class="shrink-0">{t!("simulate.bus-address")}</span>
                                    <input
                                        type="text"
                                        title=t!("simulate.bus-address-hint")
                                        placeholder="68"
                                        prop:value=bus_addr.clone()
                                        on:change=move |event| {
                                            board.checkpoint();
                                            let text = event_target_value(&event);
                                            parts.update(|list| edit::set_prop(list, index, "addr", &text));
                                            dirty.set(true);
                                        }
                                        class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    />
                                </label>
                                <label class="flex items-center gap-2 text-footnote text-label-2">
                                    <span class="shrink-0">{t!("simulate.bus-registers")}</span>
                                    <input
                                        type="text"
                                        title=t!("simulate.bus-registers-hint")
                                        placeholder="75=68,3b=010203040506"
                                        prop:value=bus_regs.clone()
                                        on:change=move |event| {
                                            board.checkpoint();
                                            let text = event_target_value(&event);
                                            parts.update(|list| edit::set_prop(list, index, "regs", &text));
                                            dirty.set(true);
                                        }
                                        class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    />
                                </label>
                                {move || {
                                    let traffic = bus_traffic();
                                    (!traffic.is_empty()).then(|| {
                                        view! {
                                            <div class="flex flex-col gap-0.5">
                                                <span class="text-caption text-label-4">
                                                    {t!("simulate.bus-traffic")}
                                                </span>
                                                {traffic
                                                    .into_iter()
                                                    .map(|line| view! {
                                                        <p class="font-mono text-caption text-label-3">{line}</p>
                                                    })
                                                    .collect_view()}
                                            </div>
                                        }
                                    })
                                }}
                            }
                        })}
                        {on_a_wire.then(|| {
                            view! {
                                <label class="flex items-center gap-2 text-footnote text-label-2">
                                    <span class="shrink-0">{t!("simulate.wire-select")}</span>
                                    <input
                                        type="text"
                                        title=t!("simulate.wire-select-hint")
                                        placeholder="0"
                                        prop:value=wire_select.clone()
                                        on:change=move |event| {
                                            board.checkpoint();
                                            let text = event_target_value(&event);
                                            parts.update(|list| edit::set_prop(list, index, "cs", &text));
                                            dirty.set(true);
                                        }
                                        class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    />
                                </label>
                                <label class="flex items-center gap-2 text-footnote text-label-2">
                                    <span class="shrink-0">{t!("simulate.wire-answer")}</span>
                                    <input
                                        type="text"
                                        title=t!("simulate.wire-answer-hint")
                                        placeholder="1a68"
                                        prop:value=wire_miso.clone()
                                        on:change=move |event| {
                                            board.checkpoint();
                                            let text = event_target_value(&event);
                                            parts.update(|list| edit::set_prop(list, index, "miso", &text));
                                            dirty.set(true);
                                        }
                                        class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                    />
                                </label>
                                {move || {
                                    let traffic = wire_traffic();
                                    (!traffic.is_empty()).then(|| {
                                        view! {
                                            <div class="flex flex-col gap-0.5">
                                                <span class="text-caption text-label-4">
                                                    {t!("simulate.wire-traffic")}
                                                </span>
                                                {traffic
                                                    .into_iter()
                                                    .map(|line| view! {
                                                        <p class="font-mono text-caption text-label-3">{line}</p>
                                                    })
                                                    .collect_view()}
                                            </div>
                                        }
                                    })
                                }}
                            }
                        })}
                        // What the solver makes of this part: what
                        // is across it and what is going through
                        // it, under the value that decides both, so
                        // changing 330 to 1k and watching the
                        // current move is one glance.
                        //
                        // Silent for a part with no electrical
                        // model — a label, a display, a rail — and
                        // for a refusal about some *other* part,
                        // which belongs beside that one.
                        {
                            let reference = reference.clone();
                            move || {
                                period
                                    .with(|p| {
                                        weights.with(|w| match p.reading(w, &reference) {
                                            Ok(found) => found.map(|read| {
                                                Ok((
                                                    readout::volts(read.across),
                                                    readout::amps(read.through),
                                                    readout::watts(read.watts),
                                                    read.steady,
                                                ))
                                            }),
                                            Err(why) => (why.part() == Some(reference.as_str()))
                                                .then(|| Err(unsolved_text(why))),
                                        })
                                    })
                                    .map(|reading| match reading {
                                        Ok((across, through, watts, steady)) => {
                                            view! {
                                                <div class="flex flex-col gap-0.5">
                                                    <span class="text-caption text-label-4">
                                                        {if steady {
                                                            t!("simulate.measured")
                                                        } else {
                                                            t!("simulate.measured-average")
                                                        }}
                                                    </span>
                                                    // One line at this column's width:
                                                    // three significant figures caps
                                                    // each figure, so the widest a
                                                    // reading gets is three negative
                                                    // ones — 154px of the 176 there
                                                    // are, measured in the panel.
                                                    <p class="flex items-baseline gap-2 font-mono text-footnote text-label">
                                                        <span>{across}</span>
                                                        <span class="text-label-3">{through}</span>
                                                        <span class="text-caption text-label-4">{watts}</span>
                                                    </p>
                                                </div>
                                            }
                                                .into_any()
                                        }
                                        Err(why) => {
                                            view! {
                                                <p class="rounded-[6px] bg-sunken px-2 py-1 text-caption leading-snug text-label-3">
                                                    {why}
                                                </p>
                                            }
                                                .into_any()
                                        }
                                    })
                            }
                        }
                        <div class="flex flex-col gap-1">
                            <span class="text-caption text-label-4">{t!("simulate.pins")}</span>
                            {pin_rows
                                .into_iter()
                                .map(|(label, ends)| {
                                    let unwired = ends.is_empty();
                                    view! {
                                        <p class="flex items-baseline gap-2 font-mono text-footnote text-label-2">
                                            <span class="w-[7ch] shrink-0 truncate text-label-3">{label}</span>
                                            <span class=if unwired { "text-label-4" } else { "" }>
                                                {if unwired { t!("simulate.unwired") } else { ends }}
                                            </span>
                                        </p>
                                    }
                                })
                                .collect_view()}
                            <p class="text-caption leading-snug text-label-4">
                                {t!("simulate.wire-hint")}
                            </p>
                        </div>
                        {own_warnings
                            .into_iter()
                            .map(|text| view! {
                                <p class="rounded-[6px] bg-amber-fill px-2 py-1 text-caption leading-snug text-label-2">{text}</p>
                            })
                            .collect_view()}
                        <button
                            type="button"
                            on:click=move |_| board.remove_part(index)
                            class="rounded-[6px] px-2 py-1 text-footnote text-crimson ring-1 ring-line hover:bg-sunken"
                        >
                            {t!("simulate.remove-del")}
                        </button>
                    </div>
                }
                    .into_any()
            }}
        </div>
    }
}
