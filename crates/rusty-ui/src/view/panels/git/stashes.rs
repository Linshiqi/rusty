//! What has been put aside, and the three things one can do to each. A
//! stash clicked opens below like a commit — it is one, and `git show` on
//! `stash@{n}` against its first parent is exactly the working tree it holds.

use leptos::{ev, prelude::*};

use rusty_i18n::t;

use crate::{controller, format, state::AppState, view::components::Button};

#[component]
pub(super) fn Stashes() -> impl IntoView {
    let state = AppState::expect();
    let dirty = Signal::derive(move || {
        state
            .git
            .status
            .with(|s| s.as_ref().is_some_and(|s| !s.entries.is_empty()))
    });
    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex items-center gap-2 border-b border-line px-4 py-2">
                <input
                    type="text"
                    placeholder=t!("git.stash-placeholder")
                    class="h-[28px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2.5 text-footnote outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                    prop:value=move || state.git.stash_note.get()
                    on:input=move |event| state.git.stash_note.set(event_target_value(&event))
                />
                <Button
                    label=t!("git.stash-save")
                    disabled=Signal::derive(move || !dirty.get())
                    on_click=Callback::new(move |_| controller::stash_save(state))
                />
            </div>
            <div class="min-h-0 flex-1 overflow-y-auto py-1">
                {move || {
                    let stashes = state.git.stashes.get();
                    if stashes.is_empty() {
                        return view! {
                            <p class="px-4 py-3 text-callout text-label-3">{t!("git.no-stashes")}</p>
                        }
                        .into_any();
                    }
                    stashes
                        .into_iter()
                        .map(|stash| {
                            let when = format::commit_when(stash.time);
                            let exact = format::full_time(stash.time);
                            let name = format!("stash@{{{}}}", stash.index);
                            let lit = name.clone();
                            let class = move || {
                                if state.git.selected.with(|s| s.as_deref() == Some(lit.as_str())) {
                                    "flex cursor-pointer items-center gap-3 bg-selection px-4 py-1.5"
                                } else {
                                    "flex cursor-pointer items-center gap-3 px-4 py-1.5 hover:bg-sunken"
                                }
                            };
                            let (apply, pop, drop) = (stash.index, stash.index, stash.index);
                            view! {
                                <div
                                    class=class
                                    on:click=move |_| controller::select_commit(state, name.clone())
                                >
                                    <span class="shrink-0 font-mono text-footnote text-label-3">{stash.label}</span>
                                    <span class="min-w-0 flex-1 truncate text-body">{stash.message}</span>
                                    <span class="shrink-0 text-footnote text-label-4" title=exact>{when}</span>
                                    // The buttons act on the stash without also opening it.
                                    <div
                                        class="flex items-center gap-2"
                                        on:click=move |event: ev::MouseEvent| event.stop_propagation()
                                    >
                                        <Button
                                            label=t!("git.apply")
                                            on_click=Callback::new(move |_| controller::stash_apply(state, apply))
                                        />
                                        <Button
                                            label=t!("git.pop")
                                            on_click=Callback::new(move |_| controller::stash_pop(state, pop))
                                        />
                                        <Button
                                            label=t!("git.drop")
                                            on_click=Callback::new(move |_| controller::stash_drop(state, drop))
                                        />
                                    </div>
                                </div>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </div>
    }
}
