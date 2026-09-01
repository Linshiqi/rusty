//! The chip's peripherals, read from the vendor's own SVD.

use leptos::prelude::*;

use crate::{
    controller,
    state::AppState,
    view::components::{Button, ButtonKind},
};

/// The chip's peripherals, as the target holds them right now.
///
/// The whole reason a debugger beats printf on embedded work: "did my GPIO
/// config actually take" is a question about a register, and the answer is
/// four bytes at a fixed address. The values come from one read of the
/// selected peripheral's whole block, refreshed on each stop.
#[component]
pub(super) fn RegistersTab() -> impl IntoView {
    let state = AppState::expect();

    // Read the map once per project; the file does not change under us.
    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.debug.registers.with(Option::is_none) && state.has_project() {
            controller::load_registers(state);
        }
    });

    // On every stop, re-read the selected peripheral: the values on screen
    // must be the ones the target holds *now*, not the ones from before the
    // last step.
    Effect::new(move |_| {
        let stopped = state
            .debug
            .session
            .with(|debug| debug.as_ref().is_some_and(|debug| !debug.running));
        let Some(name) = state.debug.peripheral.get() else {
            return;
        };
        if !stopped {
            return;
        }
        let span = state.debug.registers.with_untracked(|map| {
            map.as_ref()
                .and_then(Option::as_ref)
                .and_then(|map| map.peripherals.iter().find(|p| p.name == name))
                .map(|p| {
                    let end = p
                        .registers
                        .iter()
                        .filter(|r| r.readable)
                        .map(|r| r.offset + 4)
                        .max()
                        .unwrap_or(4);
                    (p.base, end.min(4096))
                })
        });
        if let Some((base, bytes)) = span {
            controller::read_peripheral(state, base, bytes);
        }
    });

    move || {
        let Some(loaded) = state.debug.registers.get() else {
            return view! {
                <p class="px-4 py-3 text-callout text-label-2">"Reading the chip's SVD…"</p>
            }
            .into_any();
        };
        let Some(map) = loaded else {
            // Refused rather than guessed: register addresses invented from
            // memory would be the worst possible answer here.
            return view! {
                <div class="flex flex-col items-start gap-3 px-4 py-3">
                    <p class="max-w-[70ch] text-callout leading-relaxed text-label-2">
                        "No SVD for this chip on this machine. rusty will not guess register \
                         addresses — fetch the vendor's description, or drop one in the \
                         project at .rusty/svd/<chip>.svd."
                    </p>
                    <Button
                        label="Fetch the SVD"
                        kind=ButtonKind::Primary
                        on_click=Callback::new(move |_| controller::fetch_svd(state))
                    />
                </div>
            }
            .into_any();
        };

        let names: Vec<String> = map.peripherals.iter().map(|p| p.name.clone()).collect();
        let selected = state
            .debug
            .peripheral
            .get()
            .or_else(|| names.first().cloned());
        let peripheral = selected
            .as_ref()
            .and_then(|name| map.peripherals.iter().find(|p| &p.name == name).cloned());
        let dropped = map.dropped;

        view! {
            <div class="flex min-h-0 flex-1">
                <div class="w-[180px] flex-none overflow-y-auto border-r border-line">
                    {names
                        .into_iter()
                        .map(|name| {
                            let is_selected = selected.as_deref() == Some(name.as_str());
                            let pick = name.clone();
                            view! {
                                <button
                                    type="button"
                                    on:click=move |_| state.debug.peripheral.set(Some(pick.clone()))
                                    class=if is_selected {
                                        "w-full bg-selection px-4 py-1 text-left font-mono text-footnote text-rust"
                                    } else {
                                        "w-full px-4 py-1 text-left font-mono text-footnote text-label-2 hover:bg-sunken"
                                    }
                                >
                                    {name}
                                </button>
                            }
                        })
                        .collect_view()}
                    {(dropped > 0)
                        .then(|| {
                            view! {
                                <p class="px-4 py-2 text-caption leading-snug text-label-4">
                                    {format!(
                                        "{dropped} more inherit from another peripheral, which rusty does not resolve yet.",
                                    )}
                                </p>
                            }
                        })}
                </div>
                <div class="min-w-0 flex-1 overflow-y-auto">
                    {peripheral
                        .map(|peripheral| {
                            let base = peripheral.base;
                            view! {
                                <div class="flex items-baseline gap-2 px-4 py-1.5">
                                    <span class="font-mono text-footnote text-label">
                                        {peripheral.name.clone()}
                                    </span>
                                    <span class="font-mono text-caption text-label-4">
                                        {format!("0x{base:08X}")}
                                    </span>
                                    <span class="min-w-0 truncate text-caption text-label-3">
                                        {peripheral.description.clone()}
                                    </span>
                                </div>
                                {peripheral
                                    .registers
                                    .into_iter()
                                    .map(|register| {
                                        view! { <RegisterRow base=base register=register /> }
                                    })
                                    .collect_view()}
                            }
                        })}
                </div>
            </div>
        }
        .into_any()
    }
}

/// One register: what it holds now, and what its bits mean.
#[component]
fn RegisterRow(base: u64, register: rusty_embed::Register) -> impl IntoView {
    let state = AppState::expect();
    let open = RwSignal::new(false);
    let address = base + u64::from(register.offset);
    let readable = register.readable;
    let fields = register.fields.clone();
    let offset = register.offset;
    let name = register.name.clone();
    let description = register.description.clone();

    // The value, assembled little-endian out of whichever span covers it.
    let value = Signal::derive(move || {
        if !readable {
            return None;
        }
        state.debug.session.with(|debug| {
            let debug = debug.as_ref()?;
            let read = debug.memory.iter().find(|read| {
                address >= read.begin && address + 4 <= read.begin + read.data.len() as u64
            })?;
            let at = (address - read.begin) as usize;
            let bytes = read.data.get(at..at + 4)?;
            Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        })
    });

    view! {
        <div class="border-b border-line last:border-b-0">
            <button
                type="button"
                on:click=move |_| open.update(|o| *o = !*o)
                class="flex w-full items-baseline gap-3 px-4 py-1 text-left hover:bg-sunken"
            >
                <span class="w-[7ch] shrink-0 font-mono text-caption text-label-4">
                    {format!("+0x{offset:03X}")}
                </span>
                <span class="w-[22ch] shrink-0 truncate font-mono text-footnote text-label">
                    {name}
                </span>
                <span class="w-[12ch] shrink-0 font-mono text-footnote text-rust select-text">
                    {move || match value.get() {
                        Some(value) => format!("0x{value:08X}"),
                        // Not a zero: a register never read and a register
                        // reading zero are different facts, and showing zero
                        // for both is how a debugger lies.
                        None if readable => "—".to_string(),
                        None => "write-only".to_string(),
                    }}
                </span>
                <span class="min-w-0 truncate text-caption text-label-3">{description}</span>
            </button>
            <Show when=move || open.get()>
                <div class="px-4 pb-2">
                    {fields
                        .iter()
                        .map(|field| {
                            let (bit, width) = (field.offset, field.width);
                            let field_name = field.name.clone();
                            let field_help = field.description.clone();
                            let bits = if width == 1 {
                                format!("[{bit}]")
                            } else {
                                format!("[{}:{bit}]", bit + width - 1)
                            };
                            view! {
                                <div class="flex items-baseline gap-3 py-0.5 pl-8">
                                    <span class="w-[8ch] shrink-0 font-mono text-caption text-label-4">
                                        {bits}
                                    </span>
                                    <span class="w-[20ch] shrink-0 truncate font-mono text-caption text-label-2">
                                        {field_name}
                                    </span>
                                    <span class="w-[8ch] shrink-0 font-mono text-caption text-rust">
                                        {move || {
                                            value
                                                .get()
                                                .map(|value| {
                                                    let mask = if width >= 32 {
                                                        u32::MAX
                                                    } else {
                                                        (1u32 << width) - 1
                                                    };
                                                    ((value >> bit) & mask).to_string()
                                                })
                                                .unwrap_or_default()
                                        }}
                                    </span>
                                    <span class="min-w-0 truncate text-caption text-label-4">
                                        {field_help}
                                    </span>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>
            </Show>
        </div>
    }
}
