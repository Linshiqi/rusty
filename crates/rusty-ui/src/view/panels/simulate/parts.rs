//! The parts: one group per part, keyed by index, every field a view reads
//! coming through its own memo — so a drag frame touches one part's
//! transform and nothing else.

use super::*;

/// Every part on the sheet as the component it is, with its pins, its
/// labels and whatever the firmware is doing to it.
#[component]
pub(super) fn PartsLayer(board: Board) -> impl IntoView {
    let Board {
        state,
        running,
        live,
        hover_part,
        library_open,
        chip_label,
        kit_look,
        rows,
        parts,
        wires,
        selected,
        selected_wire,
        menu,
        drag,
        hover_pin,
        marked,
        group_start,
        canvas,
        pressed,
        no_connect,
        period,
        weights,
        key_down,
        ..
    } = board;
    view! {
        <svg
            class="absolute"
            style="left: -2000px; top: -2000px; pointer-events: none"
            width="6000"
            height="6000"
        >
            <g transform="translate(2000, 2000)">
                <For
                    each=move || 0..parts.with(Vec::len)
                    key=|index| *index
                    children=move |index: usize| {
                        let this = Memo::new(move |_| {
                            parts.with(|list| list.get(index).cloned())
                        });
                        let reference = Memo::new(move |_| {
                            this.with(|p| p.as_ref().map(|p| p.inst.reference.clone()).unwrap_or_default())
                        });
                        let symbol = Memo::new(move |_| {
                            this.with(|p| p.as_ref().and_then(|p| p.symbol.clone()))
                        });
                        let place = Memo::new(move |_| {
                            this.with(|p| {
                                p.as_ref()
                                    .map(|p| (p.inst.x, p.inst.y, p.inst.rot, p.inst.mirror))
                                    .unwrap_or_default()
                            })
                        });
                        let value = Memo::new(move |_| {
                            this.with(|p| p.as_ref().map(|p| p.inst.value.clone()).unwrap_or_default())
                        });
                        let is_kit = Memo::new(move |_| {
                            this.with(|p| p.as_ref().is_some_and(|p| p.is_kit()))
                        });
                        let behaviour = Memo::new(move |_| {
                            symbol.with(|s| s.as_ref().map(behaviour_of))
                        });
                        let markup = Memo::new(move |_| {
                            let value = value.get();
                            symbol.with(|s| {
                                s.as_ref().map(|s| art::markup(s, &value)).unwrap_or_default()
                            })
                        });
                        // Where the drawing puts the leads,
                        // the light and the screen.
                        let plan = Memo::new(move |_| {
                            let value = value.get();
                            symbol.with(|s| s.as_ref().map(|s| art::layout(s, &value)))
                        });
                        let bbox = Memo::new(move |_| {
                            this.with(|p| p.as_ref().map(part_box).unwrap_or((0.0, 0.0, 0.0, 0.0)))
                        });
                        let is_selected =
                            Signal::derive(move || selected.get() == Some(index));
                        let is_marked =
                            Signal::derive(move || marked.with(|m| m.contains(&index)));
                        // How brightly its light is drawn,
                        // 0 to 1: from the current the solver
                        // says it carries over the period
                        // when it can say, and otherwise from
                        // how much of it the rules call the
                        // part lit — as though lit meant
                        // fully, which is how every lamp was
                        // drawn before there were numbers.
                        let glow = Memo::new(move |_| {
                            let reference = reference.get();
                            period.with(|p| {
                                weights.with(|w| match p.reading(w, &reference) {
                                    Ok(Some(read)) => glow::of_current(read.through),
                                    _ => glow::of_share(p.lit(w, &reference)),
                                })
                            })
                        });
                        let is_pressed = Memo::new(move |_| {
                            let reference = reference.get();
                            pressed.with(|p| p.contains(&reference))
                        });
                        // `pins` in the drawing's own frame —
                        // offsets from the part's anchor — so a
                        // drag frame moves the group and not
                        // every dot in it.
                        let pin_dots = Memo::new(move |_| {
                            this.with(|p| {
                                let Some(p) = p.as_ref() else {
                                    return Vec::new();
                                };
                                let Some(plan) = part_layout(p) else {
                                    return Vec::new();
                                };
                                plan.spots
                                    .iter()
                                    .map(|spot| {
                                        let ((x, y), _) = spot_on_sheet(p, spot);
                                        // The pin's place in its symbol travels
                                        // with the dot: a no-connect is about a
                                        // pin, and the menu is `Copy`.
                                        let at = p
                                            .symbol
                                            .as_ref()
                                            .and_then(|s| {
                                                s.pins
                                                    .iter()
                                                    .position(|pin| pin.number == spot.number)
                                            })
                                            .unwrap_or(0);
                                        (spot.number.clone(), x - p.inst.x, y - p.inst.y, at)
                                    })
                                    .collect::<Vec<_>>()
                            })
                        });
                        let wired_pins = Memo::new(move |_| {
                            let list = parts.get();
                            wires.with(|all| {
                                pin_dots
                                    .get()
                                    .iter()
                                    .filter(|(number, _, _, _)| {
                                        !edit::wires_at(&list, all, index, number).is_empty()
                                    })
                                    .map(|(number, _, _, _)| number.clone())
                                    .collect::<Vec<_>>()
                            })
                        });
                        let labels = Memo::new(move |_| {
                            this.with(|p| {
                                let Some(p) = p.as_ref() else {
                                    return Vec::new();
                                };
                                pin_labels(p)
                                    .into_iter()
                                    .map(|l| Label { x: l.x - p.inst.x, y: l.y - p.inst.y, ..l })
                                    .collect::<Vec<_>>()
                            })
                        });

                        let start_wire = move |event: ev::PointerEvent, number: String| {
                            if event.button() != 0 {
                                return;
                            }
                            // Left to the part and the sheet
                            // under it while the board runs.
                            if live.get_untracked() {
                                return;
                            }
                            event.prevent_default();
                            event.stop_propagation();
                            selected.set(Some(index));
                            selected_wire.set(None);
                            if let Some(element) = canvas.get_untracked() {
                                let _ = element.focus();
                            }
                            let press = board.to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            drag.set(Some(Drag::Wire { from: (index, number), press }));
                        };

                        let on_down = move |event: ev::PointerEvent| {
                            if event.button() != 0 {
                                return;
                            }
                            // While it runs a switch is pressed
                            // and anything else is the sheet
                            // under the pointer, which pans.
                            if live.get_untracked() {
                                if behaviour.get_untracked() == Some(Behaviour::Switch) {
                                    event.prevent_default();
                                    event.stop_propagation();
                                    board.press(index, true);
                                }
                                return;
                            }
                            event.prevent_default();
                            event.stop_propagation();
                            if let Some(element) = canvas.get_untracked() {
                                let _ = element.focus();
                            }
                            library_open.set(false);
                            selected_wire.set(None);
                            // Shift adds this part to the selection
                            // or takes it out — KiCad's modifier. A
                            // plain press selects it alone, unless it
                            // is already one of several, in which
                            // case the whole group is what moves.
                            if event.shift_key() {
                                marked.update(|m| {
                                    match m.iter().position(|i| *i == index) {
                                        Some(at) => {
                                            m.remove(at);
                                        }
                                        None => m.push(index),
                                    }
                                });
                                selected.set(Some(index));
                                return;
                            }
                            selected.set(Some(index));
                            if !marked.with_untracked(|m| m.contains(&index)) {
                                marked.set(vec![index]);
                            }
                            // A switch is pressed, not dragged, while
                            // the firmware runs — the thing the sheet
                            // is for at that moment.
                            if behaviour.get_untracked() == Some(Behaviour::Switch)
                                && running.get_untracked()
                            {
                                board.press(index, true);
                                return;
                            }
                            board.checkpoint();
                            let world = board.to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            let (x, y, _, _) = place.get_untracked();
                            let moving: Vec<usize> = marked.with_untracked(|m| {
                                let mut all = m.clone();
                                if !all.contains(&index) {
                                    all.push(index);
                                }
                                all
                            });
                            // Judge each wire's legs now, once:
                            // judged live they would flip as the
                            // part crosses its own bend.
                            let legs = parts.with_untracked(|list| {
                                wires.with_untracked(|all| wire_legs(list, all, &moving))
                            });
                            group_start.set(parts.with_untracked(|list| {
                                moving
                                    .iter()
                                    .filter(|i| **i != index)
                                    .filter_map(|i| list.get(*i).map(|p| (*i, (p.inst.x, p.inst.y))))
                                    .collect()
                            }));
                            drag.set(Some(Drag::Part {
                                index,
                                dx: world.0 - x,
                                dy: world.1 - y,
                                from: (x, y),
                                legs,
                            }));
                        };
                        let on_up = move |_| {
                            if is_pressed.get_untracked() {
                                board.press(index, false);
                            }
                        };
                        let on_menu = move |event: ev::MouseEvent| {
                            event.prevent_default();
                            event.stop_propagation();
                            if !live.get_untracked() {
                                selected.set(Some(index));
                                selected_wire.set(None);
                                if !marked.with_untracked(|m| m.contains(&index)) {
                                    marked.set(vec![index]);
                                }
                            }
                            menu.set(Some((
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                                MenuTarget::Part(index),
                            )));
                        };

                        // The body: the devkit's art, or the
                        // symbol's graphics scaled from
                        // millimetres and flipped upright.
                        let body = move || {
                            if is_kit.get() {
                                let drawn = rows.get();
                                let kit_h = kit_height(drawn.len());
                                let per_side = drawn.len().div_ceil(2).max(1);
                                let art = chip_label.with_value(|label| kit_art(kit_look, kit_h, label));
                                let (_, _, rot, mirror) = place.get();
                                let flip = if mirror { -1 } else { 1 };
                                let transform = format!("rotate({rot}) scale({flip} 1)");
                                let labels = drawn
                                    .iter()
                                    .enumerate()
                                    .map(|(row, spec)| {
                                        // The row's name sits eight pixels in from
                                        // its pin, inside the board edge, which is
                                        // where a devkit's silkscreen puts it. The
                                        // position is turned with the board and the
                                        // text is not, the way `pin_labels` places
                                        // every other part's: a name printed
                                        // sideways is one nobody reads.
                                        let (px, py) = row_offset(drawn.len(), row);
                                        let inward = if row < per_side { 1.0 } else { -1.0 };
                                        let (x, y) =
                                            orient((px + inward * 8.0, py), rot, mirror);
                                        let (ox, oy) = orient((inward, 0.0), rot, mirror);
                                        // Which way "into the board" now points
                                        // decides how the name hangs off its pin.
                                        let (anchor, dy) = if ox.abs() > oy.abs() {
                                            (if ox > 0.0 { "start" } else { "end" }, 3.0)
                                        } else if oy > 0.0 {
                                            ("middle", 8.0)
                                        } else {
                                            ("middle", -3.0)
                                        };
                                        let label = spec.label.clone();
                                        view! {
                                            <text
                                                x=x
                                                y=y + dy
                                                text-anchor=anchor
                                                font-family="ui-monospace"
                                                font-size="7.5"
                                                fill="#98a1ae"
                                                style="pointer-events: none"
                                            >
                                                {label}
                                            </text>
                                        }
                                    })
                                    .collect_view();
                                view! {
                                    <g transform=transform inner_html=art></g>
                                    {labels}
                                }
                                    .into_any()
                            } else {
                                let (_, _, rot, mirror) = place.get();
                                let flip = if mirror { -1 } else { 1 };
                                let transform = format!("rotate({rot}) scale({flip} 1)");
                                view! {
                                    <g transform=transform inner_html=move || markup.get()></g>
                                }
                                    .into_any()
                            }
                        };

                        // What the rules say about the part,
                        // drawn over its body: a glow on a lit
                        // lamp, the mixed colour of an RGB
                        // lens, the lit segments of a digit,
                        // the sunk cap of a pressed switch.
                        let face = move || {
                            let Some(plan) = plan.get() else {
                                return ().into_any();
                            };
                            let (_, _, rot, mirror) = place.get();
                            // The drawing turns with the part;
                            // what is written on it does not.
                            let turned = move |point: (f64, f64)| orient(point, rot, mirror);
                            match behaviour.get() {
                                Some(Behaviour::Led | Behaviour::Rgb) => {
                                    let Some((lx, ly, r)) = plan.lens else {
                                        return ().into_any();
                                    };
                                    let (cx, cy) = turned((lx, ly));
                                    let rgb = behaviour.get() == Some(Behaviour::Rgb);
                                    // A dark lamp is its own
                                    // colour dimmed, not grey:
                                    // a red LED is red on the
                                    // desk with the power off.
                                    let (on, dark) = lamp_colors(&value.get());
                                    let dark = if rgb { rgb_color(false, false, false) } else { dark };
                                    // And the light over it, as
                                    // a colour and how much of
                                    // it: the two attributes a
                                    // duty moves, so a breathing
                                    // lamp redraws those and not
                                    // the lamp.
                                    let light = Memo::new(move |_| {
                                        if rgb {
                                            let reference = reference.get();
                                            let share = |name: &str| board.lit_share(&reference, name);
                                            glow::lens(share("R"), share("G"), share("B"))
                                        } else {
                                            (on.to_string(), glow.get())
                                        }
                                    });
                                    view! {
                                        <circle
                                            cx=cx
                                            cy=cy
                                            r=r
                                            fill=dark
                                            fill-opacity="0.5"
                                            stroke="#0b0e12"
                                            stroke-opacity="0.55"
                                            stroke-width="0.9"
                                            style="pointer-events: none"
                                        />
                                        // The glow is cast by the
                                        // light itself, so it fades
                                        // with it: a drop shadow
                                        // takes the element's own
                                        // opacity.
                                        <circle
                                            cx=cx
                                            cy=cy
                                            r=r
                                            fill=move || light.with(|(colour, _)| colour.clone())
                                            fill-opacity=move || light.with(|(_, level)| format!("{:.3}", 0.95 * level))
                                            style=move || {
                                                light.with(|(colour, level)| {
                                                    if *level > 0.0 {
                                                        format!(
                                                            "filter: drop-shadow(0 0 5px {colour}) drop-shadow(0 0 13px {colour}); pointer-events: none",
                                                        )
                                                    } else {
                                                        "pointer-events: none".to_string()
                                                    }
                                                })
                                            }
                                        />
                                        <ellipse
                                            cx=cx - r * 0.3
                                            cy=cy - r * 0.35
                                            rx=r * 0.28
                                            ry=r * 0.42
                                            fill="#ffffff"
                                            fill-opacity=move || light.with(|(_, level)| format!("{:.3}", 0.16 + 0.34 * level))
                                            style="pointer-events: none"
                                        />
                                    }
                                        .into_any()
                                }
                                Some(Behaviour::Seven) => {
                                    let Some((fx, fy, fw, fh)) = plan.face else {
                                        return ().into_any();
                                    };
                                    let (cx, cy) =
                                        turned((fx + fw / 2.0, fy + fh / 2.0));
                                    // Each segment lit for its
                                    // own share of the period,
                                    // so a digit dimmed by PWM
                                    // is a dim digit.
                                    let seg = move |name: &'static str| {
                                        move || {
                                            let share = board.lit_share(&reference.get(), name);
                                            glow::mix("#3a2323", "#ff5c5c", glow::of_share(share))
                                        }
                                    };
                                    let transform =
                                        format!("translate({} {})", cx - 13.0, cy - 21.0);
                                    view! {
                                        <g transform=transform style="pointer-events: none">
                                            <rect x="6" y="2" width="14" height="4" rx="2" fill=seg("a") />
                                            <rect x="19" y="5" width="4" height="13" rx="2" fill=seg("b") />
                                            <rect x="19" y="23" width="4" height="13" rx="2" fill=seg("c") />
                                            <rect x="6" y="36" width="14" height="4" rx="2" fill=seg("d") />
                                            <rect x="3" y="23" width="4" height="13" rx="2" fill=seg("e") />
                                            <rect x="3" y="5" width="4" height="13" rx="2" fill=seg("f") />
                                            <rect x="6" y="19" width="14" height="4" rx="2" fill=seg("g") />
                                        </g>
                                    }
                                        .into_any()
                                }
                                // A matrix keypad: sixteen
                                // caps, each its own target,
                                // because a key is what is
                                // pressed and the part is
                                // only what carries them.
                                Some(Behaviour::Keypad) => {
                                    let keys = art::keypad_keys(&plan);
                                    let running_now = running.get();
                                    keys.into_iter()
                                        .enumerate()
                                        .map(|(at, (label, kx, ky, half))| {
                                            let (row, column) = (at / 4, at % 4);
                                            let (cx, cy) = turned((kx, ky));
                                            let held = key_down.get()
                                                == Some((index, row, column));
                                            // Pressed rather than
                                            // dragged while the
                                            // firmware runs, as a
                                            // switch is; with
                                            // nothing running the
                                            // press falls through
                                            // to the part, which
                                            // is what moves it.
                                            let down = move |event: ev::PointerEvent| {
                                                if event.button() != 0 || !running_now {
                                                    return;
                                                }
                                                event.prevent_default();
                                                event.stop_propagation();
                                                board.press_key(index, row, column, true);
                                            };
                                            let up = move |_: ev::PointerEvent| {
                                                board.press_key(index, row, column, false);
                                            };
                                            let fill = if held { "#4b5361" } else { "transparent" };
                                            let style = if running_now {
                                                "cursor: pointer"
                                            } else {
                                                "pointer-events: none"
                                            };
                                            view! {
                                                <rect
                                                    x=cx - half
                                                    y=cy - half
                                                    width=half * 2.0
                                                    height=half * 2.0
                                                    rx="2"
                                                    fill=fill
                                                    style=style
                                                    on:pointerdown=down
                                                    on:pointerup=up
                                                    on:pointerleave=up
                                                >
                                                    <title>{label.to_string()}</title>
                                                </rect>
                                            }
                                        })
                                        .collect_view()
                                        .into_any()
                                }
                                // A chain of addressable LEDs,
                                // lit by the bytes the wire
                                // carried rather than by a
                                // level on a pin: one
                                // transmission sets all of
                                // them, and the order on the
                                // wire is the chain's own.
                                Some(Behaviour::Strip) => {
                                    let reference = reference.get();
                                    let colours = board.gpio_for(&reference, "DIN")
                                        .map(|gpio| {
                                            state.sim.rmt.with(|rmt| {
                                                rmt.get(&gpio)
                                                    .map(|bytes| {
                                                        rusty_embed::strip_colours(bytes)
                                                    })
                                                    .unwrap_or_default()
                                            })
                                        })
                                        .unwrap_or_default();
                                    let lenses = art::strip_lenses(&plan);
                                    lenses
                                        .into_iter()
                                        .enumerate()
                                        .map(|(index, (lx, ly, r))| {
                                            // Dark is dark: an LED told
                                            // to be black and one the
                                            // firmware has not reached
                                            // are the same on the desk.
                                            let (red, green, blue) = colours
                                                .get(index)
                                                .copied()
                                                .unwrap_or((0, 0, 0));
                                            let lit = red as u16 + green as u16 + blue as u16 > 0;
                                            let (cx, cy) = turned((lx, ly));
                                            let fill = format!("rgb({red} {green} {blue})");
                                            let style = if lit {
                                                format!(
                                                    "pointer-events: none; filter: drop-shadow(0 0 {:.1}px rgb({red} {green} {blue}))",
                                                    r * 0.9,
                                                )
                                            } else {
                                                "pointer-events: none".to_string()
                                            };
                                            view! {
                                                <rect
                                                    x=cx - r * 0.72
                                                    y=cy - r * 0.72
                                                    width=r * 1.44
                                                    height=r * 1.44
                                                    rx="1"
                                                    fill=fill
                                                    style=style
                                                />
                                            }
                                        })
                                        .collect_view()
                                        .into_any()
                                }
                                // What the firmware prints, on
                                // the screen it prints it to —
                                // upright, whichever way the
                                // module is turned. Or, where the
                                // sheet says which controller is
                                // behind the glass, what the
                                // firmware's own display driver
                                // drew: an SSD1306 has no state a
                                // driver reads back, so the bus
                                // carries the whole picture.
                                Some(Behaviour::Display) => {
                                    let Some((fx, fy, fw, fh)) = plan.face else {
                                        return ().into_any();
                                    };
                                    let (cx, cy) =
                                        turned((fx + fw / 2.0, fy + fh / 2.0));
                                    let drawn = Memo::new(move |_| {
                                        let address = this
                                            .with(|p| p.as_ref().and_then(display_address))?;
                                        state.sim.screens.with(|screens| {
                                            let screen = screens.get(&address)?;
                                            Some((
                                                pixel_path(screen),
                                                (fw / screen.width() as f64)
                                                    .min(fh / screen.height() as f64),
                                                (screen.width() as f64, screen.height() as f64),
                                                screen.is_on(),
                                            ))
                                        })
                                    });
                                    view! {
                                        {move || {
                                            let Some((path, scale, (w, h), on)) = drawn.get() else {
                                                return view! {
                                                    <text
                                                        x=cx
                                                        y=cy + 3.0
                                                        text-anchor="middle"
                                                        font-family="ui-monospace"
                                                        font-size="8.5"
                                                        fill="#3ddc84"
                                                        style="pointer-events: none"
                                                    >
                                                        {move || {
                                                            let text = state.sim.display.get();
                                                            if text.is_empty() {
                                                                "········".to_string()
                                                            } else {
                                                                text
                                                            }
                                                        }}
                                                    </text>
                                                }
                                                    .into_any();
                                            };
                                            // In the screen's own pixels, scaled onto
                                            // the glass: the path is the decoder's
                                            // answer and nothing here does arithmetic
                                            // on it. A panel the driver has not
                                            // switched on is dimmed rather than
                                            // blanked — dark glass and a firmware that
                                            // drew nothing look the same on the desk
                                            // and are different faults.
                                            let transform = format!(
                                                "translate({} {}) scale({scale})",
                                                cx - w * scale / 2.0,
                                                cy - h * scale / 2.0,
                                            );
                                            view! {
                                                <g
                                                    transform=transform
                                                    opacity=if on { "1" } else { "0.15" }
                                                    style="pointer-events: none"
                                                >
                                                    <path
                                                        d=path
                                                        stroke="#3ddc84"
                                                        stroke-width="1"
                                                        fill="none"
                                                        shape-rendering="crispEdges"
                                                    />
                                                </g>
                                            }
                                                .into_any()
                                        }}
                                    }
                                        .into_any()
                                }
                                // A sounder says it is
                                // sounding: rings rather than
                                // a noise nobody asked their
                                // machine to make.
                                Some(Behaviour::Buzzer) => {
                                    let Some((lx, ly, r)) = plan.lens else {
                                        return ().into_any();
                                    };
                                    let (cx, cy) = turned((lx, ly));
                                    let on = glow.get() > 0.0;
                                    view! {
                                        <circle
                                            cx=cx
                                            cy=cy
                                            r=r
                                            fill="none"
                                            stroke=if on { "#ffd75c" } else { "#4a515c" }
                                            stroke-width=if on { "2.4" } else { "1" }
                                            stroke-dasharray="3 3"
                                            style="pointer-events: none"
                                        >
                                            {on
                                                .then(|| {
                                                    view! {
                                                        <animate
                                                            attributeName="r"
                                                            values=format!("{};{}", r * 0.6, r)
                                                            dur="0.5s"
                                                            repeatCount="indefinite"
                                                        />
                                                    }
                                                })}
                                        </circle>
                                    }
                                        .into_any()
                                }
                                // The horn follows the duty on
                                // the signal pin, which is what
                                // a servo is told and all a
                                // sheet can honestly show.
                                Some(Behaviour::Servo) => {
                                    let Some((fx, fy, fw, fh)) = plan.face else {
                                        return ().into_any();
                                    };
                                    let (cx, cy) =
                                        turned((fx + fw / 2.0 + 18.0, fy + fh / 2.0));
                                    let reference = reference.get();
                                    let drive = board.gpio_for(&reference, "SIG").and_then(|gpio| {
                                        state.sim.pwm.with(|pwm| pwm.get(&gpio).copied())
                                    });
                                    // Where the horn stands, from the
                                    // width of the pulse when the
                                    // emulator said how often — a servo
                                    // answers to that and not to how
                                    // hard. The ends are the part's,
                                    // because 500..2500 and 1000..2000
                                    // are forty degrees apart at each
                                    // end and both are ordinary.
                                    let (min_us, max_us) = this.with(|p| {
                                        let props = p.as_ref().map(|p| &p.inst);
                                        (
                                            props.and_then(|i| i.prop("min")).unwrap_or(SERVO_MIN_US),
                                            props.and_then(|i| i.prop("max")).unwrap_or(SERVO_MAX_US),
                                        )
                                    });
                                    let angle = drive
                                        .map(|d| f64::from(d.servo_angle(min_us, max_us)) - 90.0);
                                    let transform = format!(
                                        "translate({cx} {cy}) rotate({:.1})",
                                        angle.unwrap_or(0.0)
                                    );
                                    view! {
                                        <g transform=transform style="pointer-events: none">
                                            <rect
                                                x="-2"
                                                y="-16"
                                                width="4"
                                                height="18"
                                                rx="2"
                                                fill=if angle.is_some() { "#e3e7ec" } else { "#5a626e" }
                                            />
                                            <circle cx="0" cy="0" r="3" fill="#9aa2ae" />
                                        </g>
                                    }
                                        .into_any()
                                }
                                // The cap sinks as well as
                                // colouring: a tactile switch
                                // moves, and the eye reads the
                                // movement before the colour.
                                Some(Behaviour::Switch) => {
                                    let Some((lx, ly, r)) = plan.lens else {
                                        return ().into_any();
                                    };
                                    let (cx, cy) = turned((lx, ly));
                                    let down = is_pressed.get();
                                    view! {
                                        <circle
                                            cx=cx
                                            cy=cy
                                            r=if down { r - 1.2 } else { r }
                                            fill=if down { "#e05d38" } else { "#39404a" }
                                            stroke="#5a626e"
                                            stroke-width="1"
                                            style="pointer-events: none"
                                        />
                                    }
                                        .into_any()
                                }
                                _ => ().into_any(),
                            }
                        };

                        // The gold dots wires pull out of; a
                        // dot that nothing reaches pulses so
                        // an unwired part reads as unfinished
                        // rather than as broken.
                        let dots = move || {
                            pin_dots
                                .get()
                                .into_iter()
                                .map(|(number, dx, dy, at)| {
                                    // A `Copy` handle to the number, so the
                                    // closures below can each be used more
                                    // than once.
                                    let number = StoredValue::new(number);
                                    let wired = move || {
                                        wired_pins.with(|w| number.with_value(|n| w.contains(n)))
                                    };
                                    let target = move || {
                                        hover_pin.with(|h| {
                                            h.as_ref().is_some_and(|(p, n)| {
                                                *p == index && number.with_value(|mine| n == mine)
                                            })
                                        })
                                    };
                                    // Answered for: KiCad's cross, and no more
                                    // pulsing — the pulse means "unfinished" and
                                    // this one is finished.
                                    let answered = move || {
                                        parts.with(|list| {
                                            let Some(part) = list.get(index) else {
                                                return false;
                                            };
                                            let Some(symbol) = part.symbol.as_ref() else {
                                                return false;
                                            };
                                            let Some(found) = number.with_value(|n| {
                                                symbol.pins.iter().find(|p| p.number == *n).cloned()
                                            }) else {
                                                return false;
                                            };
                                            let named = PinRef::new(
                                                &part.inst.reference,
                                                symbol.wire_key(&found),
                                            );
                                            no_connect.with(|marks| marks.contains(&named))
                                        })
                                    };
                                    view! {
                                        <circle
                                            cx=dx
                                            cy=dy
                                            r=move || if target() { 5.5 } else { 3.4 }
                                            fill=move || if target() { "#ffd75c" } else if wired() { "#c9a227" } else { "#e0a838" }
                                            class=move || if wired() || target() || answered() { "" } else { "animate-pulse" }
                                            style=move || {
                                                if live.get() {
                                                    "pointer-events: all; cursor: inherit"
                                                } else {
                                                    "pointer-events: all; cursor: crosshair"
                                                }
                                            }
                                            on:pointerdown=move |event: ev::PointerEvent| start_wire(event, number.get_value())
                                            on:dblclick=move |event: ev::MouseEvent| {
                                                if live.get_untracked() {
                                                    return;
                                                }
                                                event.stop_propagation();
                                                board.disconnect_pin(index, number.get_value());
                                            }
                                            on:contextmenu=move |event: ev::MouseEvent| {
                                                event.prevent_default();
                                                event.stop_propagation();
                                                menu.set(Some((
                                                    f64::from(event.client_x()),
                                                    f64::from(event.client_y()),
                                                    MenuTarget::Pin(index, at),
                                                )));
                                            }
                                        >
                                            <title>{t!("simulate.pin-hint")}</title>
                                        </circle>
                                        <Show when=answered>
                                            <g
                                                stroke="#d05a5a"
                                                stroke-width="1.6"
                                                stroke-linecap="round"
                                                style="pointer-events: none"
                                            >
                                                <line x1=dx - 4.0 y1=dy - 4.0 x2=dx + 4.0 y2=dy + 4.0 />
                                                <line x1=dx - 4.0 y1=dy + 4.0 x2=dx + 4.0 y2=dy - 4.0 />
                                            </g>
                                        </Show>
                                    }
                                })
                                .collect_view()
                        };

                        // Pin names and numbers, upright
                        // whatever the body does.
                        let texts = move || {
                            if is_kit.get() {
                                return ().into_any();
                            }
                            labels
                                .get()
                                .into_iter()
                                .map(|label| {
                                    view! {
                                        <text
                                            x=label.x
                                            y=label.y
                                            text-anchor=label.anchor
                                            font-family="ui-monospace"
                                            font-size=label.size
                                            fill="#98a1ae"
                                            style="pointer-events: none"
                                        >
                                            {label.text}
                                        </text>
                                    }
                                })
                                .collect_view()
                                .into_any()
                        };

                        // The reference above the body and the
                        // value below, KiCad's placement; an
                        // unknown symbol says so where its body
                        // would be.
                        let captions = move || {
                            if is_kit.get() {
                                return ().into_any();
                            }
                            let (x0, y0, x1, y1) = bbox.get();
                            let (px, py, _, _) = place.get();
                            let cx = (x0 + x1) / 2.0 - px;
                            let top = y0 - py - 5.0;
                            let bottom = y1 - py + 11.0;
                            let unknown = symbol.with(Option::is_none);
                            let missing = unknown.then(|| {
                                let id = this.with(|p| p.as_ref().map(|p| p.inst.symbol.clone()).unwrap_or_default());
                                let (w, h) = UNKNOWN_BOX;
                                view! {
                                    <rect x=-w / 2.0 y=-h / 2.0 width=w height=h rx="4" fill="#2c313a" stroke="#e05d38" stroke-dasharray="4 3" style="pointer-events: none" />
                                    <text x="0" y="3" text-anchor="middle" font-family="ui-monospace" font-size="7" fill="#e05d38" style="pointer-events: none">{id}</text>
                                }
                            });
                            // A rail and a label are their
                            // own value, printed inside the
                            // drawing; a second copy under it
                            // reads as a mistake. Their
                            // reference is `#PWR`, which
                            // nobody needs to see either.
                            let own = symbol
                                .with(|s| s.as_ref().is_some_and(art::draws_own_value));
                            view! {
                                {missing}
                                {(!own)
                                    .then(|| {
                                        view! {
                                            <text x=cx y=top text-anchor="middle" font-family="ui-monospace" font-size="9" fill="#5fd0c8" style="pointer-events: none">
                                                {move || reference.get()}
                                            </text>
                                            <text x=cx y=bottom text-anchor="middle" font-family="ui-monospace" font-size="8" fill="#d7dce3" style="pointer-events: none">
                                                {move || value.get()}
                                            </text>
                                        }
                                    })}
                            }
                                .into_any()
                        };

                        let ring = move || {
                            let (x0, y0, x1, y1) = bbox.get();
                            let (px, py, _, _) = place.get();
                            let stroke = if is_selected.get() {
                                "#e05d38"
                            } else if is_marked.get() {
                                "#e05d3899"
                            } else {
                                "transparent"
                            };
                            view! {
                                <rect
                                    x=x0 - px - 4.0
                                    y=y0 - py - 4.0
                                    width=x1 - x0 + 8.0
                                    height=y1 - y0 + 8.0
                                    rx="4"
                                    fill="transparent"
                                    stroke=stroke
                                    stroke-width="1.2"
                                    stroke-dasharray="4 3"
                                    style="pointer-events: all"
                                />
                            }
                        };

                        let cursor = move || {
                            let grabbing = matches!(drag.get(), Some(Drag::Part { .. }))
                                && (is_selected.get() || is_marked.get());
                            if grabbing {
                                "cursor: grabbing"
                            } else if behaviour.get() == Some(Behaviour::Switch) && running.get() {
                                "cursor: pointer"
                            } else if live.get() {
                                "cursor: default"
                            } else {
                                "cursor: grab"
                            }
                        };

                        view! {
                            <g
                                transform=move || {
                                    let (x, y, _, _) = place.get();
                                    format!("translate({x} {y})")
                                }
                                style=cursor
                                on:pointerdown=on_down
                                on:pointerup=on_up
                                on:pointerenter=move |_| hover_part.set(Some(index))
                                on:pointerleave=move |event| {
                                    on_up(event);
                                    hover_part.set(None);
                                }
                                on:contextmenu=on_menu
                            >
                                {ring}
                                {body}
                                {face}
                                {texts}
                                {captions}
                                {dots}
                            </g>
                        }
                    }
                />
            </g>
        </svg>
    }
}
