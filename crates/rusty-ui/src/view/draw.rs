//! The Draw tab: what a program drew with `rusty-draw` — the scene in a
//! space you can turn, every mark listed with its numbers, and the angles
//! between arrows that share a tail.
//!
//! The angles are the point. Perspective foreshortens every angle, so a
//! right angle cannot be judged by eye in a turned view; the list says it
//! in degrees, and the view draws the little square only where the numbers
//! the program printed make one (`rusty_embed::draw::angles`).

use leptos::{ev, html, prelude::*};

use rusty_embed::draw::{self, Angle, Flaw, Hand, Mark, Shape, Sketch};
use rusty_embed::spatial::sheet::steps::{deg, num, quat, vec3};
use rusty_embed::spatial::{Frame, Vec3};
use rusty_i18n::t;

use crate::controller;
use crate::scene::sketch::{MARGIN, compose, frame_points, ink};
use crate::scene::{self, Camera, Look, Projector};
use crate::state::{AppState, Framed};
use crate::view::icon::{Icon, IconView};
use crate::view::space::{colour, follow_size, svg_of};

/// The scene the tab shows, and what it says about itself.
#[component]
pub fn DrawTab() -> impl IntoView {
    let state = AppState::expect();
    let shown: Memo<Option<Sketch>> = Memo::new(move |_| {
        let title = state.draw.shown_title()?;
        state
            .draw
            .sketches
            .with(|all| all.iter().find(|s| s.title == title).cloned())
    });
    // Rebuilt when the tab goes from nothing drawn to something and back,
    // not per scene: a firmware redrawing every loop would otherwise tear
    // the view down and put it back fifty times a second.
    let empty = Memo::new(move |_| state.draw.sketches.with(Vec::is_empty));
    move || {
        if empty.get() {
            view! { <Empty /> }.into_any()
        } else {
            view! {
                <div class="flex min-h-0 flex-1">
                    <Legend shown=shown />
                    <div class="relative min-w-0 flex-1">
                        <Space shown=shown />
                    </div>
                </div>
            }
            .into_any()
        }
    }
}

/// Nothing drawn yet: what to write to draw something.
#[component]
fn Empty() -> impl IntoView {
    const SAMPLE: [&str; 6] = [
        "use rusty_draw::Scene;",
        "",
        "Scene::new(\"cross product\")",
        "    .vector(\"a\", a)",
        "    .vector(\"b\", b)",
        "    .vector(\"a × b\", a.cross(b));",
    ];
    const DEPENDENCY: [&str; 2] = [
        "[dev-dependencies]",
        "rusty-draw = { git = \"https://github.com/Linshiqi/rusty\" }",
    ];
    let code = "rounded-[6px] bg-sunken px-3 py-2 font-mono text-footnote text-label select-text";
    view! {
        <div class="flex min-h-0 flex-1 flex-col items-start gap-2 overflow-y-auto px-5 py-3 text-callout text-label-2">
            <p>{t!("draw.empty")}</p>
            <pre class=code>{SAMPLE.join("\n")}</pre>
            <p class="text-footnote text-label-3">{t!("draw.dependency")}</p>
            <pre class=code>{DEPENDENCY.join("\n")}</pre>
        </div>
    }
}

/// What a mark is called: its label, or its kind and its place.
fn name_of(index: usize, mark: &Mark) -> String {
    if !mark.label.is_empty() {
        return mark.label.clone();
    }
    let kind = match mark.shape {
        Shape::Vector { .. } => t!("draw.kind-vector"),
        Shape::Point { .. } => t!("draw.kind-point"),
        Shape::Line { .. } => t!("draw.kind-line"),
        Shape::Span { .. } => t!("draw.kind-span"),
        Shape::Frame { .. } => t!("draw.kind-frame"),
    };
    format!("{kind} {}", index + 1)
}

/// A mark's numbers, and the one fact worth reading beside them.
fn describe(mark: &Mark) -> (String, String) {
    let from = |at: Vec3| (at != Vec3::ZERO).then(|| t!("draw.at", point = vec3(at)));
    match mark.shape {
        Shape::Vector { at, v } => {
            let length = t!("draw.length", length = num(v.norm()));
            let detail = match from(at) {
                Some(from) => format!("{length} · {from}"),
                None => length,
            };
            (vec3(v), detail)
        }
        Shape::Point { at } => (vec3(at), String::new()),
        Shape::Line { a, b } => (
            format!("{} → {}", vec3(a), vec3(b)),
            t!("draw.length", length = num((b - a).norm())),
        ),
        Shape::Span { at, a, b } => {
            let area = t!("draw.area", area = num(a.cross(b).norm()));
            let detail = match from(at) {
                Some(from) => format!("{area} · {from}"),
                None => area,
            };
            (format!("{}, {}", vec3(a), vec3(b)), detail)
        }
        Shape::Frame { q } => (quat(q), String::new()),
    }
}

fn flaw_text(flaw: Flaw) -> String {
    match flaw {
        Flaw::NotFinite => t!("draw.not-finite"),
        Flaw::Zero => t!("draw.zero"),
        Flaw::NotUnit(length) => t!("draw.not-unit", length = num(length)),
    }
}

/// The scene's name — a list of every scene when there are several — its
/// marks with their numbers, and the angles between its arrows.
#[component]
fn Legend(shown: Memo<Option<Sketch>>) -> impl IntoView {
    let state = AppState::expect();
    let angles: Memo<Vec<Angle>> = Memo::new(move |_| {
        shown.with(|s| {
            s.as_ref()
                .map(|s| draw::angles(&s.marks))
                .unwrap_or_default()
        })
    });
    let hand =
        Memo::new(move |_| shown.with(|s| s.as_ref().and_then(|s| draw::handedness(&s.marks))));

    let title_of = |title: &str| {
        if title.is_empty() {
            t!("draw.untitled")
        } else {
            title.to_string()
        }
    };
    let titles = move || {
        state
            .draw
            .sketches
            .with(|all| all.iter().map(|s| s.title.clone()).collect::<Vec<_>>())
    };
    let heading = move || {
        let titles = titles();
        match titles.as_slice() {
            [] => String::new(),
            [only] => title_of(only),
            many => t!("draw.scenes", count = many.len().to_string()),
        }
    };

    // Every scene a run drew, one row each, the one on the page lit: a
    // program that draws three scenes shows three rows. They were a
    // drop-down that showed the newest, and the two before it read as
    // never drawn. A click on the newest follows the newest again.
    let scenes = move || {
        let titles = titles();
        if titles.len() < 2 {
            return ().into_any();
        }
        let latest = state.draw.latest.get();
        let showing = state.draw.chosen.get().or_else(|| latest.clone());
        let rows = titles
            .into_iter()
            .map(|title| {
                let on = showing.as_deref() == Some(title.as_str());
                let class = if on {
                    "flex w-full items-center gap-2 rounded-[5px] bg-sunken px-2 py-[3px] text-left text-label"
                } else {
                    "flex w-full items-center gap-2 rounded-[5px] px-2 py-[3px] text-left text-label-2 hover:bg-sunken hover:text-label"
                };
                let newest = latest.as_deref() == Some(title.as_str());
                let shown_title = title_of(&title);
                view! {
                    <button
                        type="button"
                        class=class
                        title=t!("draw.pick-hint")
                        on:click=move |_| {
                            let title = (!newest).then(|| title.clone());
                            controller::choose_sketch(state, title);
                        }
                    >
                        <span class=if on { "text-rust" } else { "text-transparent" }>"▸"</span>
                        <span class="min-w-0 flex-1 truncate">{shown_title}</span>
                    </button>
                }
            })
            .collect_view();
        view! {
            <div class="mb-1 flex flex-col gap-px border-b border-line px-1 pb-1 font-sans">{rows}</div>
        }
        .into_any()
    };

    let notes = move || {
        shown.with(|s| {
            let s = s.as_ref()?;
            let mut notes = Vec::new();
            if !s.finished {
                notes.push(t!("draw.unfinished"));
            }
            if s.truncated {
                notes.push(t!("draw.truncated", count = draw::MAX_MARKS.to_string()));
            }
            (!notes.is_empty()).then(|| {
                view! {
                    <div class="px-3 pb-1 font-sans text-footnote text-amber">{notes.join(" · ")}</div>
                }
            })
        })
    };

    let rows = move || {
        shown.with(|s| {
            s.as_ref().map(|s| {
                s.marks
                    .iter()
                    .enumerate()
                    .map(|(i, mark)| mark_row(state, i, mark))
                    .collect_view()
            })
        })
    };

    let relations = move || {
        let names: Vec<String> = shown.with(|s| {
            s.as_ref()
                .map(|s| {
                    s.marks
                        .iter()
                        .enumerate()
                        .map(|(i, m)| name_of(i, m))
                        .collect()
                })
                .unwrap_or_default()
        });
        let found = angles.get();
        let hand = hand.get();
        if found.is_empty() && hand.is_none() {
            return ().into_any();
        }
        let name = |i: usize| names.get(i).cloned().unwrap_or_default();
        let hand_line = hand.map(|triple| {
            let [a, b, c] = triple.marks.map(name);
            let (text, tone) = match triple.hand {
                Hand::Right => (t!("draw.hand-right", a = a, b = b, c = c), "text-label-2"),
                Hand::Left => (t!("draw.hand-left", a = a, b = b, c = c), "text-amber"),
                Hand::Flat => (t!("draw.hand-flat", a = a, b = b, c = c), "text-label-2"),
            };
            view! { <div class=format!("px-3 pt-1 font-sans {tone}")>{text}</div> }
        });
        view! {
            <div class="mt-1 border-t border-line pt-1">
                <div class="px-3 py-0.5 font-sans text-label-3">{t!("draw.angles")}</div>
                {found
                    .iter()
                    .map(|angle| {
                        let square = angle.square.then(|| {
                            view! { <span class="text-patina" title=t!("draw.square")>"⟂"</span> }
                        });
                        view! {
                            <div class="flex items-baseline gap-2 px-3 py-[2px]">
                                <span class="min-w-0 flex-1 truncate text-label-2">
                                    {format!("{} ∠ {}", name(angle.a), name(angle.b))}
                                </span>
                                <span class="tnum text-label">{deg(angle.radians)}</span>
                                {square}
                            </div>
                        }
                    })
                    .collect_view()}
                {hand_line}
            </div>
        }
        .into_any()
    };

    view! {
        <div class="flex w-[300px] flex-none flex-col overflow-y-auto border-r border-line py-1 font-mono text-footnote select-text">
            <div class="flex items-center gap-1 px-3 pt-0.5 pb-1 font-sans text-footnote">
                <span class="min-w-0 flex-1 truncate font-medium text-label">{heading}</span>
                <button
                    type="button"
                    title=t!("draw.clear")
                    class="grid size-[22px] flex-none place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label"
                    on:click=move |_| controller::clear_drawings(state)
                >
                    <IconView icon=Icon::Close size=12 />
                </button>
            </div>
            {scenes}
            {notes}
            {rows}
            {relations}
        </div>
    }
}

/// One mark in the list: its colour, its name and what it measures, and
/// its numbers under them. The pointer over it draws it at full strength
/// and the rest faintly.
fn mark_row(state: AppState, index: usize, mark: &Mark) -> impl IntoView + use<> {
    let name = name_of(index, mark);
    let (numbers, detail) = describe(mark);
    let whole = numbers.clone();
    let flaw = draw::flaw(mark).map(flaw_text);
    view! {
        <div
            class="px-3 py-[3px] hover:bg-sunken"
            on:mouseenter=move |_| state.draw.hovered.set(Some(index))
            on:mouseleave=move |_| state.draw.hovered.set(None)
        >
            <div class="flex items-center gap-2">
                <span
                    class="size-2 flex-none rounded-full"
                    style=format!("background: {}", colour(ink(index, mark)))
                />
                <span class="min-w-0 flex-1 truncate text-label">{name}</span>
                <span class="flex-none font-sans text-label-3">{detail}</span>
            </div>
            <div class="truncate pl-4 text-label-2" title=whole>
                {numbers}
            </div>
            {flaw.map(|text| view! { <div class="pl-4 font-sans text-crimson">{text}</div> })}
        </div>
    }
}

/// The sketch in space: turned by dragging, zoomed by the wheel, looked at
/// from the named views, fitted when a new scene arrives.
#[component]
fn Space(shown: Memo<Option<Sketch>>) -> impl IntoView {
    let state = AppState::expect();
    let host = NodeRef::<html::Div>::new();
    let size = follow_size(host);
    let drag: RwSignal<Option<(f64, f64)>> = RwSignal::new(None);

    // Framed for a new scene, for this one grown or shrunk past a factor of
    // two, or for a view that changed shape by a third — the first
    // measurement of it among them. Anything less keeps the zoom somebody
    // chose.
    Effect::new(move |_| {
        let (w, h) = size.get();
        let Some((title, reach, points)) = shown.with(|s| {
            s.as_ref()
                .map(|s| (s.title.clone(), s.reach(), frame_points(s)))
        }) else {
            return;
        };
        let stale = state.draw.fitted.with_value(|fitted| {
            fitted
                .as_ref()
                .is_none_or(|framed| framed.stale(&title, reach, (w, h)))
        });
        if stale {
            state.draw.fitted.set_value(Some(Framed {
                title,
                reach,
                size: (w, h),
            }));
            state
                .draw
                .camera
                .update(|c| *c = c.framing(&points, Frame::ZUp, w, h, MARGIN));
        }
    });

    let angles: Memo<Vec<Angle>> = Memo::new(move |_| {
        shown.with(|s| {
            s.as_ref()
                .map(|s| draw::angles(&s.marks))
                .unwrap_or_default()
        })
    });
    let drawn = Memo::new(move |_| {
        let (w, h) = size.get();
        let camera = state.draw.camera.get();
        let hovered = state.draw.hovered.get();
        let items = shown.with(|s| {
            s.as_ref()
                .map(|s| angles.with(|angles| compose(s, angles, hovered)))
                .unwrap_or_default()
        });
        scene::render(&items, &Projector::new(&camera, Frame::ZUp, w, h))
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
        drag.set(Some((x, y)));
        state
            .draw
            .camera
            .update(|c| *c = c.orbit(x - x0, y - y0, Frame::ZUp));
    };
    let on_up = move |_: ev::PointerEvent| drag.set(None);
    let on_wheel = move |event: ev::WheelEvent| {
        event.prevent_default();
        let factor = (event.delta_y() * 0.0015).exp();
        state.draw.camera.update(|c| *c = c.zoomed(factor));
    };
    // A named view is framed afresh: from the top a scene can be twice as
    // wide on the page as it was from the side.
    let look = move |look: Look| {
        let (w, h) = size.get_untracked();
        let points = shown.with_untracked(|s| s.as_ref().map(frame_points).unwrap_or_default());
        state
            .draw
            .camera
            .set(Camera::look(look).framing(&points, Frame::ZUp, w, h, MARGIN));
    };
    let fit = move || {
        let (w, h) = size.get_untracked();
        let points = shown.with_untracked(|s| s.as_ref().map(frame_points).unwrap_or_default());
        state
            .draw
            .camera
            .update(|c| *c = c.framing(&points, Frame::ZUp, w, h, MARGIN));
    };

    let tool =
        "h-[22px] rounded-[5px] px-2 text-footnote text-label-2 hover:bg-sunken hover:text-label";
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
            <div class="pointer-events-none absolute top-2 left-3 max-w-[50%] truncate font-sans text-footnote font-medium text-label-2">
                {move || shown.with(|s| s.as_ref().map(|s| s.title.clone()).unwrap_or_default())}
            </div>
            <div class="absolute top-1.5 right-2 flex items-center gap-0.5 rounded-[7px] bg-raised/90 p-0.5 ring-1 ring-line">
                <button type="button" class=tool title=t!("draw.look-iso-hint") on:click=move |_| look(Look::Iso)>
                    {t!("draw.look-iso")}
                </button>
                <button type="button" class=tool title=t!("draw.look-top-hint") on:click=move |_| look(Look::Top)>
                    {t!("draw.look-top")}
                </button>
                <button type="button" class=tool title=t!("draw.look-front-hint") on:click=move |_| look(Look::Front)>
                    {t!("draw.look-front")}
                </button>
                <button type="button" class=tool title=t!("draw.look-side-hint") on:click=move |_| look(Look::Side)>
                    {t!("draw.look-side")}
                </button>
                <button type="button" class=tool title=t!("draw.fit-hint") on:click=move |_| fit()>
                    {t!("draw.fit")}
                </button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::draw::parse_draw;

    fn sketch(lines: &[&str]) -> Sketch {
        let mut book = draw::Sketchbook::default();
        lines
            .iter()
            .filter_map(|line| book.read(parse_draw(line).expect(line)))
            .last()
            .expect("a scene")
    }

    #[test]
    fn an_unnamed_mark_is_called_by_its_kind_and_place() {
        let s = sketch(&["[rusty:draw] point 1 2 3", "[rusty:draw] end"]);
        assert!(name_of(0, &s.marks[0]).ends_with(" 1"));
        let (numbers, _) = describe(&s.marks[0]);
        assert_eq!(numbers, "(1, 2, 3)");
    }
}
