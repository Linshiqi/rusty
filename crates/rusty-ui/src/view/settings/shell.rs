//! The overlay itself, and the three shapes every category is built from.
//!
//! The shapes are macOS System Settings': a page is a title over a column of
//! **groups**, a group is a rounded box of **rows** divided by hairlines, and
//! a row is a label on the left with its control on the right. Explanation,
//! where a row needs any, is one short line under the group — never a
//! paragraph beside the control. The first version had a paragraph under
//! every field, and the user's verdict was exact: cluttered, and written to
//! be admired rather than read.

use leptos::prelude::*;

use rusty_i18n::t;

use super::*;

#[component]
pub fn Settings() -> impl IntoView {
    let selected = RwSignal::new(Category::Appearance);

    view! {
        <div class="flex min-h-0 flex-1 flex-col bg-content">
            // No Done and no Save: every control applies the moment it is
            // touched, and leaving is any click in the sidebar. A button that
            // only closes teaches people to wonder what it commits.
            <header class="flex h-10 flex-none items-center border-b border-line px-4">
                <span class="text-strong font-semibold tracking-tight">{t!("palette.settings")}</span>
            </header>

            <div class="flex min-h-0 flex-1">
                <nav class="w-[168px] flex-none overflow-y-auto border-r border-line bg-sidebar p-2">
                    {Category::ALL
                        .into_iter()
                        .map(|category| {
                            let is_selected = Signal::derive(move || selected.get() == category);
                            view! {
                                <button
                                    type="button"
                                    on:click=move |_| selected.set(category)
                                    class=move || {
                                        let base = "mb-0.5 flex h-[28px] w-full items-center \
                                                    rounded-[6px] px-2.5 text-left text-body \
                                                    transition-colors";
                                        if is_selected.get() {
                                            format!("{base} bg-selection font-medium text-rust")
                                        } else {
                                            format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                        }
                                    }
                                >
                                    {category.label()}
                                </button>
                            }
                        })
                        .collect_view()}
                </nav>

                <div class="min-w-0 flex-1 overflow-y-auto px-8 py-6">
                    <div class="max-w-[640px]">
                        <h1 class="mb-5 text-heading font-semibold tracking-tight text-label">
                            {move || selected.get().label()}
                        </h1>
                        {move || match selected.get() {
                            Category::Appearance => view! { <Appearance /> }.into_any(),
                            Category::Editor => view! { <EditorSettings /> }.into_any(),
                            Category::Keyboard => view! { <Keyboard /> }.into_any(),
                            Category::Terminal => view! { <TerminalShell /> }.into_any(),
                            Category::Language => view! { <Language /> }.into_any(),
                            Category::Assistant => view! { <Assistant /> }.into_any(),
                            Category::Catalogue => view! { <CatalogueSettings /> }.into_any(),
                            Category::Storage => view! { <StorageSettings /> }.into_any(),
                            Category::Network => view! { <NetworkSettings /> }.into_any(),
                            Category::Updates => view! { <UpdateSettings /> }.into_any(),
                        }}
                    </div>
                </div>
            </div>
        </div>
    }
}

/// A rounded box of rows, with an optional small title above and one line
/// of explanation below. The footer is the only place prose goes.
#[component]
pub(super) fn Group(
    #[prop(optional, into)] title: Option<String>,
    #[prop(optional, into)] footer: Option<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <section class="mb-6 last:mb-0">
            {title.map(|title| {
                view! {
                    <h2 class="mb-1.5 px-1 text-footnote font-semibold tracking-[0.04em] text-label-3 uppercase">
                        {title}
                    </h2>
                }
            })}
            <div class="divide-y divide-line overflow-hidden rounded-[10px] bg-raised ring-1 ring-line">
                {children()}
            </div>
            {footer.map(|text| {
                view! { <p class="mt-1.5 px-1 text-footnote leading-relaxed text-label-3">{text}</p> }
            })}
        </section>
    }
}

/// One setting: its name, an optional second line under it, and the control
/// on the right. A control too wide for the right — a URL, a path — goes on a
/// `stacked` row, under the label at full width.
#[component]
pub(super) fn Row(
    #[prop(into)] label: String,
    /// The second line. A `MaybeProp`, so a caller may hand it a signal —
    /// the assistant's "current" row follows the saved provider — or a
    /// plain string; a `String` prop here read the signal once and the row
    /// said "not configured" over a provider that had just been saved.
    #[prop(optional, into)]
    detail: MaybeProp<String>,
    #[prop(optional)] stacked: bool,
    children: Children,
) -> impl IntoView {
    let detail = move || {
        detail
            .get()
            .map(|d| view! { <div class="text-footnote text-label-3">{d}</div> })
    };
    if stacked {
        return view! {
            <div class="flex flex-col gap-2 px-3.5 py-2.5">
                <div>
                    <div class="text-body text-label">{label}</div>
                    {detail}
                </div>
                <div class="flex min-w-0 flex-wrap items-center gap-2">{children()}</div>
            </div>
        }
        .into_any();
    }
    view! {
        <div class="flex min-h-[40px] items-center gap-4 px-3.5 py-2">
            <div class="min-w-0 flex-1">
                <div class="text-body text-label">{label}</div>
                {detail}
            </div>
            <div class="flex shrink-0 items-center gap-2">{children()}</div>
        </div>
    }
    .into_any()
}

/// A row that is only a note — a status line, a warning — in the group's
/// own voice rather than a control's.
#[component]
pub(super) fn NoteRow(
    #[prop(into)] text: String,
    #[prop(default = false)] warn: bool,
) -> impl IntoView {
    let tone = if warn { "text-amber" } else { "text-label-2" };
    view! {
        <div class=format!("px-3.5 py-2 text-callout leading-relaxed select-text {tone}")>{text}</div>
    }
}

/// The exclusive-choice control macOS uses for a handful of options.
#[component]
pub(super) fn Segmented(children: Children) -> impl IntoView {
    view! { <div class="inline-flex rounded-[7px] bg-sunken p-0.5">{children()}</div> }
}

#[component]
pub(super) fn Segment(
    #[prop(into)] label: String,
    #[prop(into)] selected: Signal<bool>,
    on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            on:click=move |_| on_click.run(())
            class=move || {
                let base = "h-[24px] rounded-[5px] px-3 text-callout whitespace-nowrap transition-colors";
                if selected.get() {
                    format!("{base} bg-content font-medium text-label shadow-sm")
                } else {
                    format!("{base} text-label-2 hover:text-label")
                }
            }
        >
            {label}
        </button>
    }
}

/// An on/off switch, for a setting that is exactly that. The pill is the
/// accent when on, the hairline colour when off, and the knob slides.
#[component]
pub(super) fn Switch(#[prop(into)] on: Signal<bool>, on_toggle: Callback<bool>) -> impl IntoView {
    view! {
        <button
            type="button"
            role="switch"
            aria-checked=move || on.get().to_string()
            on:click=move |_| on_toggle.run(!on.get_untracked())
            class=move || {
                let base = "relative h-[20px] w-[34px] shrink-0 rounded-full transition-colors";
                if on.get() {
                    format!("{base} bg-rust")
                } else {
                    format!("{base} bg-line-strong")
                }
            }
        >
            <span class=move || {
                // `left-0` is load-bearing: without it the knob's static
                // position is the button's centred content box, and the
                // translate for "on" carried it past the track's right edge.
                let base = "absolute top-[2px] left-0 size-4 rounded-full bg-white shadow-sm transition-transform";
                if on.get() {
                    format!("{base} translate-x-[16px]")
                } else {
                    format!("{base} translate-x-[2px]")
                }
            } />
        </button>
    }
}

/// A single-line text field in the group's style. Monospace, because
/// everything typed into one here is a URL, a path or a name a machine reads.
#[component]
pub(super) fn TextField(
    #[prop(into)] value: Signal<String>,
    on_input: Callback<String>,
    #[prop(optional, into)] placeholder: Option<String>,
    #[prop(default = "w-[300px]")] width: &'static str,
    #[prop(default = "text")] kind: &'static str,
) -> impl IntoView {
    view! {
        <input
            type=kind
            placeholder=placeholder
            autocomplete="off"
            spellcheck="false"
            class=format!(
                "h-[26px] rounded-[6px] bg-sunken px-2.5 font-mono text-footnote text-label \
                 outline-none ring-1 ring-line placeholder:text-label-4 focus:ring-rust {width}"
            )
            prop:value=move || value.get()
            on:input=move |event| on_input.run(event_target_value(&event))
        />
    }
}
