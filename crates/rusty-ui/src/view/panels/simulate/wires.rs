//! The wires, drawn over everything so no part can hide one: each with the
//! colour of what its net is doing, the handles that push a segment, the
//! joins' dots, and whatever wire is in hand.

use super::*;

/// Every wire on the sheet, and the one being pulled or drawn.
#[component]
pub(super) fn WireLayer(board: Board) -> impl IntoView {
    let Board {
        running,
        live,
        parts,
        wires,
        dirty,
        selected,
        selected_wire,
        hover_wire,
        menu,
        guides,
        drag,
        ghost,
        hover_pin,
        drawing,
        box_to,
        canvas,
        tones,
        placing,
        place_at,
        ..
    } = board;
    view! {
        <svg
            class="pointer-events-none absolute"
            style="left: -2000px; top: -2000px"
            width="6000"
            height="6000"
        >
            <g transform="translate(2000, 2000)">
                // ── wires: grab a segment, push it ──────────
                {move || {
                    let list = parts.get();
                    let picked = selected_wire.get();
                    let hovered = hover_wire.get();
                    let powered = running.get();
                    wires
                        .get()
                        .iter()
                        .enumerate()
                        .filter_map(|(wire_index, wire)| {
                            let ends = wire_ends(&list, wire)?;
                            let points = wire_path(&ends, &wire.bends);
                            let from = points[0];
                            let to = *points.last()?;
                            let is_picked = picked == Some(wire_index);
                            let is_hovered = hovered == Some(wire_index);
                            // A net's level colours its wires
                            // while the firmware runs: high is
                            // green, low is dim, unknown is the
                            // sheet's grey — and a net under
                            // PWM is green in pulses, because
                            // it is high for part of every
                            // period and has no one level.
                            let tone = powered
                                .then(|| tones.with(|all| all.get(wire_index).copied().flatten()))
                                .flatten();
                            let stroke = if is_picked {
                                "#e05d38"
                            } else if is_hovered {
                                "#b7c0cc"
                            } else {
                                match tone {
                                    Some(Tone::High | Tone::Switching) => "#5ecf7a",
                                    Some(Tone::Low) => "#5b6472",
                                    None => "#7d8694",
                                }
                            };
                            let dashes = if tone == Some(Tone::Switching) { "7 4" } else { "none" };
                            let width = if is_picked {
                                "2.4"
                            } else if is_hovered {
                                "2.0"
                            } else {
                                "1.6"
                            };
                            let path = points
                                .iter()
                                .map(|(x, y)| format!("{x},{y}"))
                                .collect::<Vec<_>>()
                                .join(" ");

                            // One grab handle per segment.
                            // Pushing a segment is how a
                            // schematic editor moves a corner
                            // — you never hunt for the vertex.
                            let grabs = points
                                .windows(2)
                                .enumerate()
                                .map(|(seg, pair)| {
                                    let (a, b) = (pair[0], pair[1]);
                                    let horizontal = (a.1 - b.1).abs() < 0.5;
                                    let cursor = if horizontal { "row-resize" } else { "col-resize" };
                                    let drawn = points.clone();
                                    view! {
                                        <line
                                            x1=a.0
                                            y1=a.1
                                            x2=b.0
                                            y2=b.1
                                            stroke="transparent"
                                            stroke-width="12"
                                            // Through to the parts while it runs:
                                            // a wire drawn across a switch's cap
                                            // took every press meant for it.
                                            style=move || {
                                                if live.get() {
                                                    "pointer-events: none".to_string()
                                                } else {
                                                    format!("pointer-events: stroke; cursor: {cursor}")
                                                }
                                            }
                                            on:pointerenter=move |_| hover_wire.set(Some(wire_index))
                                            on:pointerleave=move |_| hover_wire.set(None)
                                            on:contextmenu=move |event: ev::MouseEvent| {
                                                event.prevent_default();
                                                event.stop_propagation();
                                                if !live.get_untracked() {
                                                    selected.set(None);
                                                    selected_wire.set(Some(wire_index));
                                                }
                                                menu.set(Some((
                                                    f64::from(event.client_x()),
                                                    f64::from(event.client_y()),
                                                    MenuTarget::Wire(wire_index),
                                                )));
                                            }
                                            on:pointerdown=move |event: ev::PointerEvent| {
                                                if event.button() != 0 {
                                                    return;
                                                }
                                                // The sheet's, which pans, while it runs.
                                                if live.get_untracked() {
                                                    return;
                                                }
                                                event.prevent_default();
                                                event.stop_propagation();
                                                selected.set(None);
                                                selected_wire.set(Some(wire_index));
                                                if let Some(element) = canvas.get_untracked() {
                                                    let _ = element.focus();
                                                }
                                                board.checkpoint();
                                                // Freeze the drawn path into
                                                // real bends, so the segment
                                                // has movable points on both
                                                // sides — and the two anchored
                                                // ends stay put by growing an
                                                // elbow.
                                                let handles = wires
                                                    .try_update(|all| {
                                                        let w = all.get_mut(wire_index)?;
                                                        let mut inner: Vec<(f64, f64)> = drawn[1..drawn.len() - 1].to_vec();
                                                        let mut first = seg as isize - 1;
                                                        let mut second = seg as isize;
                                                        if first < 0 {
                                                            inner.insert(0, drawn[0]);
                                                            first = 0;
                                                            second = 1;
                                                        }
                                                        if second as usize >= inner.len() {
                                                            inner.push(to);
                                                            second = inner.len() as isize - 1;
                                                        }
                                                        w.bends = inner;
                                                        Some((first as usize, second as usize))
                                                    })
                                                    .flatten();
                                                if let Some((first, second)) = handles {
                                                    let world = board.to_world(
                                                        f64::from(event.client_x()),
                                                        f64::from(event.client_y()),
                                                    );
                                                    drag.set(Some(Drag::Segment {
                                                        wire: wire_index,
                                                        first,
                                                        second,
                                                        horizontal,
                                                        grab: if horizontal { world.1 } else { world.0 },
                                                        base: if horizontal { a.1 } else { a.0 },
                                                    }));
                                                }
                                            }
                                        />
                                    }
                                })
                                .collect_view();

                            // Corner pips, so the selected
                            // wire shows where its bends are.
                            let bends = is_picked.then(|| {
                                points[1..points.len() - 1]
                                    .iter()
                                    .map(|(bx, by)| {
                                        let (bx, by) = (*bx, *by);
                                        view! {
                                            <rect
                                                x=bx - 3.5
                                                y=by - 3.5
                                                width="7"
                                                height="7"
                                                fill="#e05d38"
                                                style="pointer-events: auto; cursor: pointer"
                                                on:dblclick=move |event: ev::MouseEvent| {
                                                    if live.get_untracked() {
                                                        return;
                                                    }
                                                    event.stop_propagation();
                                                    board.checkpoint();
                                                    wires.update(|all| {
                                                        if let Some(w) = all.get_mut(wire_index) {
                                                            w.bends.retain(|(wx, wy)| (wx - bx).abs() > 0.5 || (wy - by).abs() > 0.5);
                                                        }
                                                    });
                                                    dirty.set(true);
                                                }
                                            />
                                        }
                                    })
                                    .collect_view()
                            });

                            Some(view! {
                                <polyline
                                    points=path
                                    fill="none"
                                    stroke=stroke
                                    stroke-width=width
                                    stroke-dasharray=dashes
                                    style="pointer-events: none"
                                />
                                <circle cx=from.0 cy=from.1 r="2.2" fill="#c9a227" style="pointer-events: none" />
                                <circle cx=to.0 cy=to.1 r="2.2" fill="#c9a227" style="pointer-events: none" />
                                {bends}
                                {grabs}
                            })
                        })
                        .collect_view()
                }}

                // Junctions: where wires *join*, as opposed
                // to where they cross. Two ends at a pin is
                // one — the pin is a conductor too — and so
                // is an end landing on another wire's line,
                // which a branch makes and which the old
                // rule, reading pins alone, drew nothing for.
                {move || {
                    let list = parts.get();
                    let all = wires.get();
                    layout::junctions(&list, &all)
                        .into_iter()
                        .map(|(x, y)| {
                            view! {
                                <circle cx=x cy=y r="3.6" fill="#c9a227" style="pointer-events: none" />
                            }
                        })
                        .collect_view()
                }}

                // The armed part's ghost: the part itself,
                // where a click would plant it. A bare
                // rectangle said only "something goes here".
                {move || {
                    let symbol = placing.get()?;
                    let (x, y) = place_at.get()?;
                    let plan = art::layout(&symbol, "");
                    let (x0, y0, x1, y1) = plan.bounds;
                    let drawn = art::markup(&symbol, "");
                    Some(view! {
                        <g
                            transform=format!("translate({x} {y})")
                            opacity="0.7"
                            style="pointer-events: none"
                        >
                            <rect
                                x=x0 - 5.0
                                y=y0 - 5.0
                                width=x1 - x0 + 10.0
                                height=y1 - y0 + 10.0
                                rx="4"
                                fill="#e05d38"
                                fill-opacity="0.10"
                                stroke="#e05d38"
                                stroke-width="1.2"
                                stroke-dasharray="4 3"
                            />
                            <g inner_html=drawn></g>
                        </g>
                    })
                }}

                // ── alignment guides ────────────────────────
                {move || {
                    let (gx, gy) = guides.get();
                    view! {
                        {gx.map(|x| view! {
                            <line x1=x y1=-2000 x2=x y2=4000 stroke="#e0a838" stroke-width="0.8" stroke-dasharray="4 4" style="pointer-events: none" />
                        })}
                        {gy.map(|y| view! {
                            <line x1=-2000 y1=y x2=4000 y2=y stroke="#e0a838" stroke-width="0.8" stroke-dasharray="4 4" style="pointer-events: none" />
                        })}
                    }
                }}

                // ── the ghost while pulling a new wire ──────
                //
                // The route the wire will actually take
                // once it lands on a pin, computed by the
                // same `connection` that will make it — a
                // preview that showed a diagonal and then
                // drew an orthogonal route somewhere else
                // is a preview of nothing. With no pin in
                // reach it is the elbow out of the pin,
                // which is the shape a schematic wire has
                // whatever it ends on.
                // A wire being drawn click by click is drawn
                // the whole way: what the clicks have fixed,
                // solid, and the leg following the pointer,
                // dashed — both from the `Drawing` the next
                // click will use, so the click makes what
                // the screen showed.
                {move || {
                    let target = ghost.get()?;
                    let draft = drawing.get()?;
                    let landing = hover_pin.get();
                    let (fixed, live) = parts.with(|list| {
                        let part = list.get(draft.from.0)?;
                        let pin = part.pin(&draft.from.1)?;
                        let (start, out) = (pin_point(part, pin), pin_out(part, pin));
                        let mut fixed = vec![start];
                        fixed.extend(draft.placed.iter().copied());
                        let anchor = *fixed.last()?;
                        let mut live = vec![anchor];
                        match landing.as_ref() {
                            // Pin to pin with nothing laid
                            // between is the routed wire a
                            // drag makes, and so is its
                            // preview.
                            Some(to) if draft.placed.is_empty() => {
                                let routed = wires.with(|all| {
                                    edit::connection(
                                        list,
                                        all,
                                        (draft.from.0, &draft.from.1),
                                        (to.0, &to.1),
                                    )
                                });
                                match routed.and_then(|w| {
                                    wire_ends(list, &w).map(|e| wire_path(&e, &w.bends))
                                }) {
                                    Some(path) => live = path,
                                    None => live.extend(draft.leg_to(start, out, target)),
                                }
                            }
                            Some(to) => {
                                let end_part = list.get(to.0)?;
                                let end_pin = end_part.pin(&to.1)?;
                                live.extend(last_leg(
                                    anchor,
                                    pin_point(end_part, end_pin),
                                    pin_out(end_part, end_pin),
                                ));
                            }
                            None => live.extend(draft.leg_to(start, out, target)),
                        }
                        Some((fixed, simplify_route(live)))
                    })?;
                    let join = |points: &[(f64, f64)]| {
                        points
                            .iter()
                            .map(|(x, y)| format!("{x},{y}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    };
                    let solid = (fixed.len() >= 2).then(|| {
                        view! {
                            <polyline
                                points=join(&fixed)
                                fill="none"
                                stroke="#e0a838"
                                stroke-width="1.8"
                                style="pointer-events: none"
                            />
                        }
                    });
                    Some(view! {
                        <g>
                            {solid}
                            <polyline
                                points=join(&live)
                                fill="none"
                                stroke="#e0a838"
                                stroke-width="1.8"
                                stroke-dasharray="5 4"
                                style="pointer-events: none"
                            />
                        </g>
                    })
                }}
                {move || {
                    let target = ghost.get()?;
                    let Some(Drag::Wire { from, .. }) = drag.get() else {
                        return None;
                    };
                    let landing = hover_pin.get();
                    let points = parts.with(|list| {
                        let part = list.get(from.0)?;
                        let pin = part.pin(&from.1)?;
                        let start = pin_point(part, pin);
                        let out = pin_out(part, pin);
                        if let Some(to) = landing.as_ref() {
                            let wire = wires.with(|all| {
                                edit::connection(
                                    list,
                                    all,
                                    (from.0, &from.1),
                                    (to.0, &to.1),
                                )
                            });
                            if let Some(wire) = wire
                                && let Some(ends) = wire_ends(list, &wire)
                            {
                                return Some(wire_path(&ends, &wire.bends));
                            }
                        }
                        // Out along the pin, then one turn
                        // towards the pointer.
                        let step = ROW_PITCH;
                        let stub = (start.0 + out.0 * step, start.1 + out.1 * step);
                        let corner = if out.0.abs() > out.1.abs() {
                            (target.0, stub.1)
                        } else {
                            (stub.0, target.1)
                        };
                        Some(vec![start, stub, corner, target])
                    })?;
                    let path = points
                        .iter()
                        .map(|(x, y)| format!("{x},{y}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    Some(view! {
                        <polyline
                            points=path
                            fill="none"
                            stroke="#e0a838"
                            stroke-width="1.8"
                            stroke-dasharray="5 4"
                            style="pointer-events: none"
                        />
                    })
                }}

                // ── the rubber band ─────────────────────────
                {move || {
                    let Some(Drag::Box { start }) = drag.get() else {
                        return None;
                    };
                    let to = box_to.get()?;
                    Some(view! {
                        <rect
                            x=start.0.min(to.0)
                            y=start.1.min(to.1)
                            width=(start.0 - to.0).abs()
                            height=(start.1 - to.1).abs()
                            fill="#e0a838"
                            fill-opacity="0.08"
                            stroke="#e0a838"
                            stroke-width="1"
                            stroke-dasharray="4 3"
                            style="pointer-events: none"
                        />
                    })
                }}
            </g>
        </svg>
    }
}
