//! The parts library: what can be dropped onto the board.
//!
//! Built-ins first, then whatever the project defines in `.rusty/parts/`.
//! Adding is all it does — a part arrives unwired and the sheet owns
//! everything after that, which is why this needs two props rather than the
//! editor's whole state.

use rusty_i18n::t;

use leptos::prelude::*;

use super::geometry::PartKind;

/// `on_add` carries the kind and a label stub — only the custom parts and the
/// RGB lens use the stub, so it is a string rather than another enum.
#[component]
pub(super) fn Library(
    user_parts: Vec<rusty_embed::PartDef>,
    on_add: Callback<(PartKind, String)>,
) -> impl IntoView {
    let add_part = move |kind: PartKind, label: String| on_add.run((kind, label));

    view! {
                <div class="flex w-[160px] flex-none flex-col gap-1 overflow-y-auto border-r border-line bg-sidebar p-2">
                    <span class="px-1 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        {t!("parts.heading")}
                    </span>
                    // One LED, not one per colour: the colour is a property,
                    // picked in the panel on the right once it is selected.
                    <button
                        type="button"
                        title=t!("parts.led-hint")
                        on:click=move |_| add_part(
                            PartKind::Led {
                                color: "red".to_string(),
                            },
                            String::new(),
                        )
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="size-3.5 rounded-full bg-[#ff5c5c]" />
                        <span>{t!("parts.led")}</span>
                    </button>
                    <button
                        type="button"
                        title=t!("parts.button-hint")
                        on:click=move |_| add_part(PartKind::Button, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="grid size-3.5 place-items-center rounded-[4px] bg-line-strong">
                            <span class="size-1.5 rounded-full bg-label-3" />
                        </span>
                        <span>{t!("parts.button")}</span>
                    </button>
                    <button
                        type="button"
                        title=t!("parts.analog-hint")
                        on:click=move |_| add_part(PartKind::Analog, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="grid h-3.5 w-3 shrink-0 place-items-center rounded-[2px] border border-line-strong">
                            <span class="h-px w-1.5 bg-label-3" />
                        </span>
                        <span>{t!("parts.analog")}</span>
                    </button>
                    <button
                        type="button"
                        title=t!("parts.motor-hint")
                        on:click=move |_| add_part(PartKind::Motor, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="relative grid size-3.5 shrink-0 place-items-center rounded-full border border-line-strong">
                            <span class="absolute top-0 left-1/2 h-1/2 w-px -translate-x-1/2 bg-label-3" />
                            <span class="size-1 rounded-full bg-label-3" />
                        </span>
                        <span>{t!("parts.motor")}</span>
                    </button>
                    <button
                        type="button"
                        title=t!("parts.rgb-hint")
                        on:click=move |_| add_part(PartKind::Rgb, "RGB".to_string())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="size-3.5 rounded-full bg-[conic-gradient(#ff5c5c,#3ddc84,#4aa8ff,#ff5c5c)]" />
                        <span>{t!("parts.rgb")}</span>
                    </button>
                    <button
                        type="button"
                        title=t!("parts.seven-hint")
                        on:click=move |_| add_part(PartKind::Seven, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="grid size-3.5 place-items-center rounded-[3px] bg-[#3a2323] font-mono text-[9px] leading-none text-[#ff5c5c]">
                            "8"
                        </span>
                        <span>{t!("parts.seven")}</span>
                    </button>
                    <button
                        type="button"
                        title=t!("parts.display-hint")
                        on:click=move |_| add_part(PartKind::Display, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="h-3 w-4 rounded-[2px] bg-[#0d1a12] ring-1 ring-[#1d4a2f]" />
                        <span>{t!("parts.display")}</span>
                    </button>
                    <button
                        type="button"
                        title=t!("parts.pot-hint")
                        on:click=move |_| add_part(PartKind::Pot, String::new())
                        class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                    >
                        <span class="grid size-3.5 place-items-center rounded-full bg-line-strong">
                            <span class="h-2 w-0.5 bg-[#c9a227]" />
                        </span>
                        <span>{t!("parts.pot")}</span>
                    </button>
                    {(!user_parts.is_empty())
                        .then(|| {
                            view! {
                                <span class="mt-2 px-1 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                    {t!("parts.custom")}
                                </span>
                            }
                        })}
                    {user_parts
                        .iter()
                        .map(|def| {
                            let name = def.name.clone();
                            let color = def.color.clone();
                            let add_color = color.clone();
                            let add_name = name.clone();
                            view! {
                                <button
                                    type="button"
                                    title=format!("{name} — from .rusty/parts/")
                                    on:click=move |_| {
                                        add_part(
                                            PartKind::Led {
                                                color: add_color.clone(),
                                            },
                                            add_name.clone(),
                                        )
                                    }
                                    class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                                >
                                    <span class=format!(
                                        "size-3.5 rounded-full {}",
                                        match color.as_str() {
                                            "blue" => "bg-[#4aa8ff]",
                                            "red" => "bg-[#ff5c5c]",
                                            "yellow" => "bg-[#ffd75c]",
                                            _ => "bg-[#3ddc84]",
                                        },
                                    ) />
                                    <span>{name.clone()}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                    <p class="mt-1 px-1 text-caption leading-snug text-label-4">
                        {t!("misc.own-parts")}
                    </p>
                </div>
    }
}
