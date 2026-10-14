//! The view: space with the aircraft in it, or the plane — the rows' vectors
//! in their colours, the selected row's attitude as the aircraft, and the
//! working of the step being read drawn where it happens.

use leptos::{ev, html, prelude::*};

pub(super) use rusty_embed::scene::math::turns;
use rusty_embed::scene::math::{compose_plane, compose_space, plane_suits};
use rusty_embed::spatial::Frame;
use rusty_embed::spatial::sheet::{Row, Value};
use rusty_i18n::t;

use crate::scene::{self, Camera, Ink, Item, Preset, Projector, model};
use crate::state::AppState;
use crate::view::space::{follow_size, nice_step, svg_of};

#[component]
pub fn SceneView(rows: Memo<Vec<Row>>) -> impl IntoView {
    let state = AppState::expect();
    let host = NodeRef::<html::Div>::new();
    let size = follow_size(host);

    // The plane's own pan and zoom: its centre, and a factor on the scale
    // that fits the vectors.
    let plane = RwSignal::new((0.0_f64, 0.0_f64, 1.0_f64));
    let drag: RwSignal<Option<(f64, f64)>> = RwSignal::new(None);

    let flat = Memo::new(move |_| {
        let chosen = state.math.flat.get();
        let selected = state.math.selected.get();
        chosen.unwrap_or_else(|| rows.with(|r| plane_suits(r, selected)))
    });

    // The plane's scale: the vectors fitted, times the user's zoom.
    let plane_scale = move || {
        let (w, h) = size.get();
        let extent = rows.with(|rows| {
            rows.iter()
                .filter_map(|r| match &r.value {
                    Some(Ok(Value::Vec2(v))) => Some(v.norm()),
                    _ => None,
                })
                .fold(1.0_f64, f64::max)
        });
        (w.min(h) * 0.42 / extent) * plane.get().2
    };

    let drawn = Memo::new(move |_| {
        let (w, h) = size.get();
        let frame = state.math.sheet.with(|s| s.frame);
        let hidden = state.math.hidden.get();
        let selected = state.math.selected.get();
        let step = state.math.step.get();
        let progress = state.math.progress.get();
        rows.with(|rows| {
            if flat.get() {
                let scale = plane_scale();
                let (cx, cy, _) = plane.get();
                let half = (w.max(h) / scale) * 0.6;
                let grid_step = nice_step(80.0 / scale);
                let items = compose_plane(rows, &hidden, selected, step, half, grid_step, (cx, cy));
                scene::render(&items, &Projector::plane(scale, (cx, cy), w, h))
            } else {
                let camera = state.math.camera.get();
                let items = compose_space(rows, &hidden, selected, step, progress, frame);
                scene::render(&items, &Projector::new(&camera, frame, w, h))
            }
        })
    });

    let on_down = move |event: ev::PointerEvent| {
        if event.button() != 0 {
            return;
        }
        drag.set(Some((
            f64::from(event.client_x()),
            f64::from(event.client_y()),
        )));
        if let Some(target) = event.current_target() {
            use wasm_bindgen::JsCast;
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                let _ = el.set_pointer_capture(event.pointer_id());
            }
        }
    };
    let on_move = move |event: ev::PointerEvent| {
        let Some((x0, y0)) = drag.get_untracked() else {
            return;
        };
        let (x, y) = (f64::from(event.client_x()), f64::from(event.client_y()));
        let (dx, dy) = (x - x0, y - y0);
        drag.set(Some((x, y)));
        if flat.get_untracked() {
            let scale = plane_scale();
            plane.update(|(cx, cy, _)| {
                *cx -= dx / scale;
                *cy += dy / scale;
            });
        } else {
            let frame = state.math.sheet.with_untracked(|s| s.frame);
            state.math.camera.update(|c| *c = c.orbit(dx, dy, frame));
        }
    };
    let on_up = move |_: ev::PointerEvent| drag.set(None);
    let on_wheel = move |event: ev::WheelEvent| {
        event.prevent_default();
        let factor = (event.delta_y() * 0.0015).exp();
        if flat.get_untracked() {
            plane.update(|(_, _, zoom)| *zoom = (*zoom / factor).clamp(0.05, 50.0));
        } else {
            state.math.camera.update(|c| *c = c.zoomed(factor));
        }
    };

    let preset = move |p: Preset| {
        let frame = state.math.sheet.with_untracked(|s| s.frame);
        let distance = state.math.camera.with_untracked(|c| c.distance);
        state.math.camera.set(Camera {
            distance,
            ..Camera::preset(p, frame)
        });
    };
    let fit = move || {
        if flat.get_untracked() {
            plane.set((0.0, 0.0, 1.0));
            return;
        }
        let frame = state.math.sheet.with_untracked(|s| s.frame);
        let hidden = state.math.hidden.get_untracked();
        let selected = state.math.selected.get_untracked();
        let step = state.math.step.get_untracked();
        let items =
            rows.with_untracked(|rows| compose_space(rows, &hidden, selected, step, 1.0, frame));
        let items: Vec<Item> = items
            .into_iter()
            .filter(|(item, _)| !matches!(item, Item::Line { ink: Ink::Grid, .. }))
            .map(|(item, _)| item)
            .collect();
        let extent = model::extent(&items).max(1.3);
        state.math.camera.update(|c| *c = c.fitting(extent));
    };

    let tool =
        "h-[24px] rounded-[6px] px-2 text-footnote text-label-2 hover:bg-sunken hover:text-label";
    view! {
        <div node_ref=host class="absolute inset-0 overflow-hidden bg-content">
            <svg
                class="block h-full w-full touch-none select-none"
                style=move || if drag.get().is_some() { "cursor: grabbing" } else { "cursor: grab" }
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_up
                on:wheel=on_wheel
            >
                {move || drawn.get().into_iter().map(svg_of).collect_view()}
            </svg>
            <div class="absolute top-2 right-2 flex items-center gap-1 rounded-[8px] bg-raised/90 p-0.5 ring-1 ring-line">
                <button
                    type="button"
                    class=tool
                    on:click=move |_| state.math.flat.set(Some(!flat.get_untracked()))
                    title=t!("math.view-toggle-hint")
                >
                    {move || if flat.get() { t!("math.view-3d") } else { t!("math.view-2d") }}
                </button>
                {move || {
                    (!flat.get())
                        .then(|| {
                            view! {
                                <button type="button" class=tool on:click=move |_| preset(Preset::Chase)>
                                    {t!("math.view-chase")}
                                </button>
                                <button type="button" class=tool on:click=move |_| preset(Preset::Top)>
                                    {t!("math.view-top")}
                                </button>
                                <button type="button" class=tool on:click=move |_| preset(Preset::Front)>
                                    {t!("math.view-front")}
                                </button>
                                <button type="button" class=tool on:click=move |_| preset(Preset::Side)>
                                    {t!("math.view-side")}
                                </button>
                            }
                        })
                }}
                <button type="button" class=tool on:click=move |_| fit() title=t!("math.view-fit-hint")>
                    {t!("math.view-fit")}
                </button>
            </div>
            <div class="pointer-events-none absolute bottom-2 left-3 flex items-center gap-2 font-mono text-footnote text-label-3">
                <span style="color: #e5484d">"X"</span>
                <span style="color: #30a46c">"Y"</span>
                <span style="color: #3e63dd">"Z"</span>
                <span>
                    {move || {
                        match state.math.sheet.with(|s| s.frame) {
                            Frame::ZUp => t!("math.frame-z-up"),
                            Frame::ZDown => t!("math.frame-z-down"),
                        }
                    }}
                </span>
            </div>
        </div>
    }
}
