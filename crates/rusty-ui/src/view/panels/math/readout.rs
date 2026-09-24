//! The selected row's value in full — an attitude as its quaternion, its
//! axis and angle, its three Euler angles, its matrix and the instrument a
//! pilot would read it on — and one click to put it in Rust.

use leptos::prelude::*;

use rusty_embed::spatial::sheet::steps::{deg, num};
use rusty_embed::spatial::sheet::{Row, Value};
use rusty_embed::spatial::{Euler, Frame, Mat3, Quat};
use rusty_i18n::t;

use super::rows::show;
use super::words;
use crate::scene::instrument;
use crate::state::AppState;
use crate::view::components::copy_to_clipboard;

/// The value as a Rust literal: arrays of `f32`, as a firmware keeps them —
/// `[w, x, y, z]` for a quaternion, which is the order this sheet writes
/// them in and the one the tooltip names.
fn rust(value: &Value) -> Option<String> {
    let f = |v: f64| {
        let text = format!("{v:.7}");
        let text = text.trim_end_matches('0').trim_end_matches('.').to_string();
        if text.contains('.') {
            text
        } else {
            format!("{text}.0")
        }
    };
    Some(match value {
        Value::Number(v) | Value::Angle(v) => f(*v),
        Value::Vec2(v) => format!("[{}, {}]", f(v.x), f(v.y)),
        Value::Vec3(v) => format!("[{}, {}, {}]", f(v.x), f(v.y), f(v.z)),
        Value::Quat(q) => format!("[{}, {}, {}, {}]", f(q.w), f(q.x), f(q.y), f(q.z)),
        Value::Euler(e) => format!("[{}, {}, {}]", f(e.roll), f(e.pitch), f(e.yaw)),
        Value::Mat3(m) => format!(
            "[{}]",
            m.m.iter()
                .map(|r| format!("[{}, {}, {}]", f(r[0]), f(r[1]), f(r[2])))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Verdict(_) => return None,
    })
}

#[component]
pub fn Readout(rows: Memo<Vec<Row>>) -> impl IntoView {
    let state = AppState::expect();
    let value = Memo::new(move |_| {
        state.math.selected.get().and_then(|i| {
            rows.with(|r| {
                r.get(i)
                    .and_then(|row| row.value.as_ref().and_then(|v| v.as_ref().ok()).cloned())
            })
        })
    });
    let copied = RwSignal::new(false);
    view! {
        <div class="flex min-h-0 flex-1 flex-col @min-[640px]:w-[340px] @min-[640px]:flex-none">
            <div class="flex h-[32px] flex-none items-center gap-2 border-b border-line px-3">
                <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {t!("math.value")}
                </span>
                <span class="flex-1" />
                {move || {
                    value
                        .get()
                        .and_then(|v| rust(&v))
                        .map(|literal| {
                            view! {
                                <button
                                    type="button"
                                    class="h-[22px] rounded-[6px] px-2 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                                    title=t!("math.copy-hint")
                                    on:click=move |_| {
                                        copy_to_clipboard(&literal);
                                        copied.set(true);
                                        set_timeout(
                                            move || { let _ = copied.try_set(false); },
                                            std::time::Duration::from_millis(1200),
                                        );
                                    }
                                >
                                    {move || if copied.get() { t!("math.copied") } else { t!("math.copy") }}
                                </button>
                            }
                        })
                }}
            </div>
            <div class="min-h-0 flex-1 overflow-y-auto px-3 py-2">
                {move || {
                    let frame = state.math.sheet.with(|s| s.frame);
                    match value.get() {
                        None => view! { <p class="text-footnote text-label-3">{t!("math.no-value")}</p> }.into_any(),
                        Some(Value::Quat(q)) => attitude(q, None, frame).into_any(),
                        Some(Value::Euler(e)) => attitude(Quat::from_euler(e), Some(e), frame).into_any(),
                        Some(Value::Mat3(m)) => matrix(m).into_any(),
                        Some(Value::Verdict(v)) => {
                            let error = if v.angle { deg(v.error) } else { num(v.error) };
                            view! {
                                <div class="text-body font-semibold text-label">{words::relation(v.relation)}</div>
                                <div class="mt-1 font-mono text-footnote text-label-2">"Δ = "{error}</div>
                            }
                            .into_any()
                        }
                        Some(other) => {
                            let extra = match &other {
                                Value::Vec3(v) => Some(format!("|v| = {}", num(v.norm()))),
                                Value::Vec2(v) => Some(format!(
                                    "|v| = {} · ∠X = {}",
                                    num(v.norm()),
                                    deg(v.y.atan2(v.x))
                                )),
                                _ => None,
                            };
                            view! {
                                <div class="font-mono text-callout text-label select-text">{show(&other)}</div>
                                {extra.map(|e| view! { <div class="mt-1 font-mono text-footnote text-label-2">{e}</div> })}
                            }
                            .into_any()
                        }
                    }
                }}
            </div>
        </div>
    }
}

fn line(label: String, value: String) -> impl IntoView {
    view! {
        <div class="flex items-baseline gap-2 py-[1px]">
            <span class="w-[74px] flex-none text-footnote text-label-3">{label}</span>
            <span class="min-w-0 font-mono text-footnote text-label select-text">{value}</span>
        </div>
    }
}

fn attitude(q: Quat, given: Option<Euler>, frame: Frame) -> impl IntoView {
    let unit = q.normalized().unwrap_or(Quat::IDENTITY);
    let e = given.unwrap_or_else(|| q.to_euler());
    let (axis, angle) = unit
        .axis_angle()
        .map(|(n, a)| {
            (
                format!("({}, {}, {})", num(n.x), num(n.y), num(n.z)),
                deg(a),
            )
        })
        .unwrap_or_else(|| ("—".into(), "0°".into()));
    let reading = instrument::read(unit, frame);
    let m = unit.to_mat3();
    view! {
        <div class="flex gap-3">
            <Horizon reading=reading />
            <div class="min-w-0 flex-1">
                {line("q".into(), format!("({}, {}, {}, {})", num(q.w), num(q.x), num(q.y), num(q.z)))}
                {line("|q|".into(), rusty_embed::spatial::sheet::steps::precise(q.norm()))}
                {line(t!("math.readout.axis"), axis)}
                {line(t!("math.readout.angle"), angle)}
                {line(t!("math.readout.roll"), deg(e.roll))}
                {line(t!("math.readout.pitch"), deg(e.pitch))}
                {line(t!("math.readout.yaw"), deg(e.yaw))}
            </div>
        </div>
        <div class="mt-2 text-footnote text-label-3">
            {t!(
                "math.readout.instrument",
                nose = deg(reading.nose_up),
                bank = deg(reading.bank_right),
                heading = deg(reading.heading)
            )}
        </div>
        <div class="mt-2">{matrix(m)}</div>
    }
}

fn matrix(m: Mat3) -> impl IntoView {
    view! {
        <div class="inline-grid grid-cols-3 gap-x-3 rounded-[6px] bg-sunken/60 px-2 py-1 font-mono text-footnote text-label-2 select-text">
            {m.m
                .iter()
                .flat_map(|row| row.iter().map(|v| view! { <span class="text-right tnum">{num(*v)}</span> }))
                .collect_view()}
        </div>
        <div class="mt-1 font-mono text-footnote text-label-3">
            {format!("det = {} · |RᵀR − I| = {}", num(m.det()), num(m.orthonormal_error()))}
        </div>
    }
}

/// An attitude indicator: sky over ground, the horizon turned against the
/// bank and moved by the nose, a ladder every ten degrees, and the aircraft
/// fixed in the middle. An instrument's colours, the same in every theme.
#[component]
fn Horizon(reading: instrument::Reading) -> impl IntoView {
    const SIZE: f64 = 108.0;
    const PER_DEGREE: f64 = 1.6;
    let c = SIZE / 2.0;
    let shift = reading.nose_up.to_degrees() * PER_DEGREE;
    let turn = -reading.bank_right.to_degrees();
    let ladder = [-30i32, -20, -10, 10, 20, 30]
        .into_iter()
        .map(|d| {
            let y = c + shift - f64::from(d) * PER_DEGREE;
            let half = if d % 20 == 0 { 16.0 } else { 10.0 };
            view! {
                <line
                    x1=c - half
                    x2=c + half
                    y1=y
                    y2=y
                    stroke="white"
                    stroke-opacity="0.7"
                    stroke-width="1"
                />
            }
        })
        .collect_view();
    view! {
        <svg width=SIZE height=SIZE class="flex-none" viewBox=format!("0 0 {SIZE} {SIZE}")>
            <defs>
                <clipPath id="math-horizon-clip">
                    <circle cx=c cy=c r=c - 1.0 />
                </clipPath>
            </defs>
            <g clip-path="url(#math-horizon-clip)">
                <g transform=format!("rotate({turn} {c} {c})")>
                    <rect x=-SIZE y=-SIZE * 2.0 + c + shift width=SIZE * 3.0 height=SIZE * 2.0 fill="#3b6ea5" />
                    <rect x=-SIZE y=c + shift width=SIZE * 3.0 height=SIZE * 2.0 fill="#7a5530" />
                    <line x1=-SIZE x2=SIZE * 2.0 y1=c + shift y2=c + shift stroke="white" stroke-width="1.5" />
                    {ladder}
                </g>
            </g>
            <circle cx=c cy=c r=c - 1.0 fill="none" stroke="var(--line-strong)" stroke-width="1.5" />
            <path
                d=format!("M{} {c}h16l6 7l6 -7h16", c - 22.0)
                fill="none"
                stroke="#f5a623"
                stroke-width="3"
                stroke-linecap="round"
                stroke-linejoin="round"
            />
            <circle cx=c cy=c r="2.5" fill="#f5a623" />
        </svg>
    }
}
