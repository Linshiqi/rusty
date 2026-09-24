//! What the sheet understands, over the rows when asked for: every function
//! with the way it is written and what it does, grouped by the work it is
//! for.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::state::AppState;

/// The groups and their entries: how each is written, which is the same in
/// every language, and what it does, which is not.
fn groups() -> Vec<(String, Vec<(&'static str, String)>)> {
    vec![
        (
            t!("math.group.build"),
            vec![
                ("euler(roll, pitch, yaw)", t!("math.fn.euler")),
                ("quat(w, x, y, z)", t!("math.fn.quat")),
                ("axis_angle(axis, angle)", t!("math.fn.axis-angle")),
                ("from_rotvec(v)", t!("math.fn.from-rotvec")),
                ("from_to(a, b)", t!("math.fn.from-to")),
                ("from_dcm(R)", t!("math.fn.from-dcm")),
                ("identity", t!("math.fn.identity")),
            ],
        ),
        (
            t!("math.group.read"),
            vec![
                ("to_euler(q)", t!("math.fn.to-euler")),
                ("dcm(q)", t!("math.fn.dcm")),
                ("axis(q) · angle(q)", t!("math.fn.axis-angle-of")),
                ("to_rotvec(q)", t!("math.fn.to-rotvec")),
                ("norm(q) · normalize(q)", t!("math.fn.norm")),
                ("q.w  q.x  e.roll …", t!("math.fn.fields")),
            ],
        ),
        (
            t!("math.group.turn"),
            vec![
                (
                    "to_world(q, v) · rotate(q, v) · q * v",
                    t!("math.fn.to-world"),
                ),
                ("to_body(q, v)", t!("math.fn.to-body")),
                ("a * b", t!("math.fn.compose")),
                ("conj(q) · inv(q)", t!("math.fn.conj")),
                ("delta(a, b)", t!("math.fn.delta")),
                ("angle(a, b)", t!("math.fn.angle")),
                ("slerp(a, b, t)", t!("math.fn.slerp")),
                ("integrate(q, w, dt)", t!("math.fn.integrate")),
                ("integrate_linear(q, w, dt)", t!("math.fn.integrate-linear")),
            ],
        ),
        (
            t!("math.group.gravity"),
            vec![
                ("accel_at_rest(q)", t!("math.fn.accel-at-rest")),
                ("gravity_body(q)", t!("math.fn.gravity-body")),
                ("tilt(acc)", t!("math.fn.tilt")),
            ],
        ),
        (
            t!("math.group.vectors"),
            vec![
                ("(x, y, z) · (x, y)", t!("math.fn.tuple")),
                ("X  Y  Z", t!("math.fn.axes")),
                ("dot(a, b) · cross(a, b)", t!("math.fn.dot-cross")),
                (
                    "norm(v) · normalize(v) · project(a, b)",
                    t!("math.fn.vector"),
                ),
                ("rotate(angle, v)", t!("math.fn.rotate2")),
                (
                    "mat(r0, r1, r2) · transpose(R) · det(R) · R * v",
                    t!("math.fn.matrix"),
                ),
            ],
        ),
        (
            t!("math.group.check"),
            vec![
                ("check(mine, reference)", t!("math.fn.check")),
                ("tel(\"name\")", t!("math.fn.tel")),
                ("truth() · truth_rate()", t!("math.fn.truth")),
            ],
        ),
        (
            t!("math.group.numbers"),
            vec![
                ("30°  30 deg  0.5 rad", t!("math.fn.units")),
                ("deg(x) · rad(x)", t!("math.fn.deg-rad")),
                (
                    "sin cos tan asin acos atan atan2 sqrt abs pi",
                    t!("math.fn.scalars"),
                ),
                ("# …", t!("math.fn.remark")),
            ],
        ),
    ]
}

#[component]
pub fn Help() -> impl IntoView {
    let state = AppState::expect();
    move || {
        state.math.help.get().then(|| {
            view! {
                <div class="absolute inset-0 z-10 flex flex-col bg-sidebar">
                    <div class="flex h-[32px] flex-none items-center border-b border-line px-3">
                        <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                            {t!("math.help")}
                        </span>
                        <span class="flex-1" />
                        <button
                            type="button"
                            class="h-[22px] rounded-[6px] px-2 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                            on:click=move |_| state.math.help.set(false)
                        >
                            {t!("math.close")}
                        </button>
                    </div>
                    <div class="min-h-0 flex-1 overflow-y-auto px-3 py-2">
                        <p class="mb-2 text-footnote text-label-2">{t!("math.help-intro")}</p>
                        {groups()
                            .into_iter()
                            .map(|(title, entries)| {
                                view! {
                                    <div class="mt-3 mb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                        {title}
                                    </div>
                                    {entries
                                        .into_iter()
                                        .map(|(written, meaning)| {
                                            view! {
                                                <div class="py-1">
                                                    <div class="font-mono text-footnote text-label select-text">{written}</div>
                                                    <div class="text-footnote leading-relaxed text-label-3">{meaning}</div>
                                                </div>
                                            }
                                        })
                                        .collect_view()}
                                }
                            })
                            .collect_view()}
                    </div>
                </div>
            }
        })
    }
}
