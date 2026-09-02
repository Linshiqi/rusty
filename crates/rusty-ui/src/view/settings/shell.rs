//! The overlay itself: the category list, and the field row every setting uses.

use leptos::prelude::*;

use rusty_i18n::t;

use super::*;

#[component]
pub fn Settings() -> impl IntoView {
    let selected = RwSignal::new(Category::Appearance);

    view! {
        <div class="flex min-h-0 flex-1 flex-col bg-content">
                // No Done and no Save: every control here applies the
                // moment it is touched, and leaving is any click in the
                // sidebar. A button that only closes teaches people to
                // wonder what it commits.
                <header class="flex h-10 flex-none items-center gap-3 border-b border-line px-4">
                    <span class="text-strong font-semibold tracking-tight">{t!("palette.settings")}</span>
                </header>

                <div class="flex min-h-0 flex-1">
                    <nav class="w-[168px] flex-none overflow-y-auto border-r border-line bg-sidebar p-2">
                        {Category::ALL
                            .into_iter()
                            .map(|category| {
                                let is_selected = Signal::derive(move || {
                                    selected.get() == category
                                });
                                view! {
                                    <button
                                        type="button"
                                        on:click=move |_| selected.set(category)
                                        class=move || {
                                            let base = "w-full rounded-[6px] px-2 py-1.5 text-left \
                                                        transition-colors";
                                            if is_selected.get() {
                                                format!("{base} bg-selection text-rust")
                                            } else {
                                                format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                            }
                                        }
                                    >
                                        <div class="text-body font-medium">{category.label()}</div>
                                        <div class="text-footnote text-label-3">
                                            {category.summary()}
                                        </div>
                                    </button>
                                }
                            })
                            .collect_view()}
                    </nav>

                    <div class="min-w-0 flex-1 overflow-y-auto px-6 py-5">
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
    }
}

/// A titled block within a category.
#[component]
pub(super) fn Field(
    #[prop(into)] label: String,
    #[prop(optional, into)] help: Option<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="mb-6 max-w-[62ch] last:mb-0">
            <div class="mb-2 text-body font-medium">{label}</div>
            {children()}
            {help.map(|text| {
                view! { <p class="mt-2 text-callout leading-relaxed text-label-2">{text}</p> }
            })}
        </div>
    }
}

/// A labelled single-line field.
#[component]
pub(super) fn TextRow(
    #[prop(into)] label: String,
    value: Signal<String>,
    on_input: Callback<String>,
) -> impl IntoView {
    view! {
        <label class="flex items-center gap-3">
            <span class="w-[72px] shrink-0 text-callout text-label-2">{label}</span>
            <input
                class="h-[28px] flex-1 rounded-[6px] bg-sunken px-2.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust"
                prop:value=move || value.get()
                on:input=move |event| on_input.run(event_target_value(&event))
            />
        </label>
    }
}
