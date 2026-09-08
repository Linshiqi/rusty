//! The parts library: every symbol the sheet may place, and the way to
//! bring in one it cannot.
//!
//! Grouped by library, in the order somebody looks: KiCad's `Device`
//! first, the simulator's own `rusty` parts, then everything imported from
//! LCSC and whatever the project keeps under `.rusty/symbols/`. Picking a
//! part arms it to the cursor; the sheet owns everything after that, which
//! is why this needs callbacks rather than the editor's whole state.

use leptos::{ev, prelude::*};
use rusty_embed::Symbol;
use rusty_embed::nets::{Behaviour, behaviour_of};
use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};

/// What a part looks like in the list: a small glyph by behaviour, so a
/// lamp and a resistor are told apart before the name is read.
fn glyph(symbol: &Symbol) -> &'static str {
    match behaviour_of(symbol) {
        Behaviour::Led => "size-3.5 rounded-full bg-[#ff5c5c]",
        Behaviour::Rgb => {
            "size-3.5 rounded-full bg-[conic-gradient(#ff5c5c,#3ddc84,#4aa8ff,#ff5c5c)]"
        }
        Behaviour::Resistor => "h-2 w-4 rounded-[2px] bg-[#d8a24b]",
        Behaviour::Capacitor => "h-3.5 w-2.5 border-x-2 border-[#d8a24b]",
        Behaviour::Switch => "size-3.5 rounded-[4px] bg-line-strong",
        Behaviour::Seven => "size-3.5 rounded-[3px] bg-[#3a2323]",
        Behaviour::Display => "h-3 w-4 rounded-[2px] bg-[#0d1a12] ring-1 ring-[#1d4a2f]",
        Behaviour::Pot => "size-3.5 rounded-full bg-line-strong ring-2 ring-[#c9a227]",
        Behaviour::Analog => "h-3.5 w-3 rounded-[2px] border border-line-strong",
        Behaviour::Motor => "size-3.5 rounded-full border border-line-strong",
        Behaviour::Power => "h-3.5 w-3.5 border-b-2 border-[#9aa2ae]",
        Behaviour::Label => "h-2.5 w-4 rounded-[2px] border border-[#5fd0c8]",
        Behaviour::Buzzer => "size-3.5 rounded-full bg-[#20242b] ring-1 ring-line-strong",
        Behaviour::Servo => "h-3 w-4 rounded-[2px] bg-[#20242b] ring-1 ring-line-strong",
        Behaviour::Other => "h-3 w-3.5 rounded-[2px] border border-[#d8a24b]",
    }
}

#[component]
pub(super) fn Library(
    symbols: Signal<Vec<Symbol>>,
    on_add: Callback<Symbol>,
    on_import: Callback<String>,
    importing: Signal<bool>,
) -> impl IntoView {
    let number = RwSignal::new(String::new());
    let submit = move || {
        let text = number.get_untracked();
        if text.trim().is_empty() {
            return;
        }
        on_import.run(text);
    };
    // The libraries in a fixed order, the rest alphabetical after them.
    let groups = Memo::new(move |_| {
        let mut names: Vec<String> =
            symbols.with(|list| list.iter().map(|s| s.library.clone()).collect());
        names.sort();
        names.dedup();
        let rank = |name: &str| match name {
            "Device" => 0,
            "rusty" => 1,
            "lcsc" => 2,
            _ => 3,
        };
        names.sort_by_key(|n| rank(n));
        names
    });

    view! {
        <div class="flex w-[172px] flex-none flex-col overflow-y-auto border-r border-line bg-sidebar">
            <div class="flex flex-col gap-1 p-2">
                <span class="px-1 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {t!("parts.heading")}
                </span>
                <For
                    each=move || groups.get()
                    key=|name| name.clone()
                    children=move |name: String| {
                        let heading = name.clone();
                        let members = Memo::new(move |_| {
                            symbols.with(|list| {
                                list.iter().filter(|s| s.library == name).cloned().collect::<Vec<_>>()
                            })
                        });
                        view! {
                            <span class="mt-1.5 px-1 text-caption text-label-4">{heading}</span>
                            <For
                                each=move || members.get()
                                key=|symbol| symbol.id()
                                children=move |symbol: Symbol| {
                                    let title = symbol
                                        .description
                                        .clone()
                                        .unwrap_or_else(|| symbol.id());
                                    let label = symbol.name.clone();
                                    let prefix = symbol.reference.trim_end_matches(['?', '_']).to_string();
                                    let glyph = glyph(&symbol);
                                    view! {
                                        <button
                                            type="button"
                                            title=title
                                            on:click=move |_| on_add.run(symbol.clone())
                                            class="flex items-center gap-2 rounded-[6px] px-2 py-1.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                                        >
                                            <span class=format!("shrink-0 {glyph}") />
                                            <span class="min-w-0 flex-1 truncate text-left">{label}</span>
                                            <span class="font-mono text-caption text-label-4">{prefix}</span>
                                        </button>
                                    }
                                }
                            />
                        }
                    }
                />
            </div>
            // A part LCSC sells, by its number: fetched from EasyEDA's
            // service, read into a symbol, kept in the data directory's
            // library, and armed to the cursor at once.
            <div class="mt-auto flex flex-col gap-1.5 border-t border-line p-2">
                <span class="px-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {t!("parts.import-heading")}
                </span>
                <div class="flex items-center gap-1">
                    <input
                        type="text"
                        placeholder="C2286"
                        title=t!("parts.import-hint")
                        prop:value=move || number.get()
                        on:input=move |event| number.set(event_target_value(&event))
                        on:keydown=move |event: ev::KeyboardEvent| {
                            if event.key() == "Enter" {
                                event.prevent_default();
                                submit();
                            }
                        }
                        class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                    />
                    <button
                        type="button"
                        title=t!("parts.import")
                        disabled=move || importing.get()
                        on:click=move |_| submit()
                        class="grid size-[26px] shrink-0 place-items-center rounded-[6px] bg-rust text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                    >
                        <IconView icon=Icon::Plus size=13 />
                    </button>
                </div>
                <p class="px-1 text-caption leading-snug text-label-4">
                    {move || {
                        if importing.get() {
                            t!("parts.importing")
                        } else {
                            t!("parts.import-note")
                        }
                    }}
                </p>
            </div>
        </div>
    }
}
