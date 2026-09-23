//! The shell, and the registry it renders.
//!
//! Structure: a VSCode-style activity bar of icons down the left edge that
//! only switches panels, the project's verbs in the title bar beside the
//! project's name (`run.rs`), the debugger's transport floating over the
//! working area while a session is live (`transport.rs`), and a status bar
//! carrying the facts you check without looking away from what you are
//! doing. The labels went when the bar did — a tooltip names the icon, and
//! the width the labels cost bought nothing.
//!
//! The shell knows no panel by name. It renders whatever [`panels::all`]
//! returns — the commitment in `docs/extensibility.md` that a contributed panel
//! can slot in later without the shell being rewritten to accept it.

mod activity;
mod clone;
pub mod components;
mod device;
pub mod dock;
pub mod icon;
pub mod loclink;
pub mod markdown;
pub mod menu;
pub mod palette;
pub mod panels;
pub mod pinmap;
pub mod plot;
mod quick;
mod run;
pub mod settings;
mod setup;
mod sidebar;
pub mod split;
mod status;
mod switcher;
pub mod terminal;
pub mod transport;
mod update;
pub mod waves;

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, Divider},
    view::{
        components::{Button, ButtonKind, ErrorBanner, Tone},
        icon::{Icon, IconView},
    },
};
use sidebar::*;
use status::*;

/// Whether the settings overlay is up.
///
/// In context rather than in `AppState` because it is chrome, not workbench
/// state — no panel has any business reading it, and the assistant should never
/// be able to see it.
#[derive(Clone, Copy)]
pub struct SettingsOpen(pub RwSignal<bool>);

/// A panel as the shell sees it.
pub struct Panel {
    pub id: &'static str,
    pub title: String,
    /// Sidebar group. Empty means "belongs to no category" and is drawn as a
    /// rule rather than a heading — for panels like the wizard and the
    /// assistant, which are not about the open project or the attached device
    /// and would be miscategorised by either heading.
    pub section: &'static str,
    pub icon: Icon,
    /// Disabled until a project is open. The wizard and the assistant are not.
    pub needs_project: bool,
    pub render: fn() -> AnyView,
    /// Reachable but not listed: the wizard lives in File > New project,
    /// the assistant in the title-bar toggle. A sidebar of destinations
    /// stays a sidebar of destinations.
    pub hidden: bool,
}

#[component]
pub fn App() -> impl IntoView {
    let state = AppState::new();
    state.provide();

    // Applied before the first paint, so the window never flashes the wrong
    // theme on the way to the right one.
    crate::theme::init();
    split::install(state);

    // Reattach to whatever the backend still holds; a frontend reload during
    // development should not lose the open project.
    controller::restore(state);
    // And correct the language if the stored setting differs from the
    // system's. Usually it does not, and this does nothing.
    controller::restore_locale();

    // The browser's own chrome never belongs in the app: no native context
    // menu anywhere (surfaces that want one draw their own), and none of the
    // navigation shortcuts — F5 would tear the workbench down mid-session,
    // Ctrl+P prints a web page nobody asked for, Alt+arrows walk a history
    // that does not exist. F12 stays: our own debugging lives there.
    let menus = window_event_listener(leptos::ev::contextmenu, |event| {
        event.prevent_default();
    });
    let keys = window_event_listener(leptos::ev::keydown, |event: leptos::ev::KeyboardEvent| {
        let key = event.key();
        let blocked = key == "F5"
            || (event.ctrl_key() && matches!(key.as_str(), "p" | "P" | "u" | "U" | "j" | "J"))
            || (event.alt_key() && matches!(key.as_str(), "ArrowLeft" | "ArrowRight"));
        if blocked {
            event.prevent_default();
        }
    });
    // Leaked deliberately: they live as long as the window, and dropping the
    // handles would silently detach them.
    std::mem::forget(menus);
    std::mem::forget(keys);

    let settings_open = RwSignal::new(false);
    provide_context(SettingsOpen(settings_open));

    // A `?detach=<path>` boot means this window is one file's editor, not
    // the whole shell: no sidebar, no dock, no menu bar — the OS gives it a
    // frame, and closing it is closing it.
    if let Some(path) = state.app.detached.get_untracked() {
        let opened = RwSignal::new(false);
        let home = path.clone();
        Effect::new(move |_| {
            if state.has_project() && !opened.get_untracked() {
                opened.set(true);
                controller::open_file(state, path.clone());
            }
        });
        return view! {
            <div class="flex h-full flex-col bg-content text-label">
                // The way back. VSCode drags an editor into the main window;
                // that needs native drop targets between OS windows, so this
                // is the same destination by a button — and a torn-off file
                // that can only be closed is a file with no way home.
                //
                // Labelled rather than an icon: it is the only control in a
                // window with no other chrome, and "how do I get back" is
                // exactly the question it has to answer at a glance.
                <div class="flex h-8 flex-none items-center justify-end border-b border-line px-2">
                    <button
                        type="button"
                        title=t!("chrome.back-to-main-hint")
                        on:click=move |_| controller::reattach(state, home.clone())
                        class="flex h-[26px] items-center gap-1.5 rounded-[6px] px-2.5 text-footnote text-label-2 transition-colors hover:bg-sunken hover:text-label"
                    >
                        {t!("chrome.back-to-main")}
                    </button>
                </div>
                <div class="flex min-h-0 flex-1 flex-col">
                    {panels::files_view()}
                </div>
            </div>
        }
        .into_any();
    }

    // A `?gitdiff=<target>` boot is one commit torn off into its own window,
    // as Fork does: the commit's pane and nothing else. It reattaches to the
    // backend's project like the editor window does, then opens the commit.
    if let Some(target) = state.git.window_target.get_untracked() {
        let opened = RwSignal::new(false);
        Effect::new(move |_| {
            if state.has_project() && !opened.get_untracked() {
                opened.set(true);
                controller::select_commit(state, target.clone());
            }
        });
        return view! {
            <div class="flex h-full flex-col bg-content text-label">
                {panels::git::commit_window()}
            </div>
        }
        .into_any();
    }

    let palette_open = RwSignal::new(false);
    let chrome = crate::command::Chrome {
        settings_open,
        palette_open,
    };
    palette::install(state, chrome);
    switcher::install(state, chrome);
    // Only the shell listens for a file coming home; a detached window has no
    // business reopening one.
    controller::watch_reattach(state);

    view! {
        <div class="flex h-full flex-col bg-content text-label">
            <menu::MenuBar chrome=chrome />
            // Overlays are anchored to the working area, not the window, so
            // they cannot cover the title bar. Settings used to: its own Done
            // button ended up underneath the menu bar and the page became a
            // room with no door.
            // The context actions were a strip across the top of the window,
            // then a column in the rail under the panel switchers. Neither
            // survived: the strip cost forty pixels on every panel, and the
            // column mixed four kinds of button — switchers, project verbs, a
            // debug session's transport and the panel's own actions — at one
            // weight, with Run in a different place on every panel. Each
            // kind has its own home now: verbs in the title bar (`run.rs`),
            // the transport floating below, panel actions in the panel's
            // header row, and the rail switches panels and nothing else.
            <div class="relative flex min-h-0 flex-1">

                <palette::Palette open=palette_open chrome=chrome />
                <quick::QuickOpen />
                <switcher::EditorSwitcher />
                // The environment check. Anchored to the working area like
                // every other overlay, so it cannot cover the title bar and
                // leave a window with no way out.
                <setup::SetupSheet />
                <clone::CloneSheet />
                <update::UpdateSheet />
                <Sidebar />
                <main class="relative flex min-w-0 flex-1 flex-col overflow-hidden">
                    // Over the working area, not in its flow. As a row above
                    // the panel, the banner pushed everything under it down
                    // forty pixels on arrival and back up on dismissal, and a
                    // click already in flight landed on whatever had moved
                    // under the pointer — a tree row, a dock tab — instead of
                    // the thing aimed at.
                    <div class="pointer-events-none absolute inset-x-0 top-0 z-30 flex justify-end">
                        <div class="pointer-events-auto w-full max-w-[640px]">
                            {move || {
                                state
                                    .app.error
                                    .get()
                                    .map(|error| {
                                        view! {
                                            <ErrorBanner
                                                error=error
                                                on_dismiss=Callback::new(move |_| {
                                                    controller::dismiss_error(state)
                                                })
                                            />
                                        }
                                    })
                            }}
                        </div>
                    </div>
                    // The debugger's transport, floating over the working area
                    // while a session is live — VS Code's debug toolbar. An
                    // overlay, so its arrival moves nothing; centred, so it is
                    // the same reach from the editor and from the board, the
                    // two places a stopped program sends you.
                    <div class="pointer-events-none absolute inset-x-0 top-2 z-30 flex justify-center">
                        <div class="pointer-events-auto">
                            <transport::DebugTransport />
                        </div>
                    </div>
                    // The dock sits under the panel rather than under the whole
                    // window, as Xcode's debug area does: the sidebar is
                    // navigation and stays whole, the output belongs to the
                    // thing being worked on.
                    <div class="relative flex min-h-0 flex-1 flex-col">
                        {move || {
                            if settings_open.get() {
                                view! { <settings::Settings /> }.into_any()
                            } else {
                                view! { <Stage /> }.into_any()
                            }
                        }}
                    </div>
                    <dock::Dock />
                </main>
                // The assistant, VSCode-chat style: a right-hand drawer the
                // title-bar icon toggles, beside whatever panel is active.
                <Show when=move || state.ai.open.get()>
                    // The drawer's left edge is a divider like the tree's
                    // right edge; the handle draws the hairline, so the aside
                    // carries no border of its own.
                    <split::Handle divider=Divider::Assistant />
                    <aside
                        class="flex flex-none flex-col bg-sidebar"
                        style=move || format!("width: {}px", state.layout.assistant_width.get())
                    >
                        <div class="flex flex-none items-center gap-2 border-b border-line px-3 py-1.5">
                            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                {t!("chrome.assistant")}
                            </span>
                            <span class="flex-1" />
                            <button
                                type="button"
                                title=t!("chrome.close")
                                on:click=move |_| state.ai.open.set(false)
                                class="rounded-[5px] px-1.5 text-footnote text-label-3 hover:text-label"
                            >
                                "×"
                            </button>
                        </div>
                        {panels::assistant_view()}
                    </aside>
                </Show>
            </div>
            <StatusBar />
        </div>
    }
    .into_any()
}

/// Renders the active panel, or explains why it cannot.
#[component]
fn Stage() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let active = state.layout.panel.get();
        let panel = panels::all()
            .into_iter()
            .find(|p| p.id == active)
            .or_else(|| panels::all().into_iter().next());

        match panel {
            Some(panel) if panel.needs_project && !state.has_project() => {
                view! {
                    <components::Empty
                        title=t!("chrome.no-project-title")
                        detail=t!("chrome.no-project-detail")
                    >
                        <OpenProjectButton kind=ButtonKind::Primary />
                        <Playgrounds />
                        // The way back to yesterday's work, one click deep.
                        {move || {
                            let recents = state.app.recents.get();
                            (!recents.is_empty())
                                .then(|| {
                                    view! {
                                        <div class="mt-4 flex w-full max-w-[52ch] flex-col gap-0.5 text-left">
                                            <div class="mb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                                {t!("chrome.recent")}
                                            </div>
                                            {recents
                                                .into_iter()
                                                .take(6)
                                                .map(|path| {
                                                    let open = path.clone();
                                                    let name = crate::command::recent_label(&path);
                                                    view! {
                                                        <button
                                                            type="button"
                                                            title=path
                                                            on:click=move |_| {
                                                                controller::open_recent(
                                                                    state,
                                                                    open.clone(),
                                                                    true,
                                                                )
                                                            }
                                                            class="truncate rounded-[6px] px-2 py-1 text-left text-callout text-label-2 transition-colors hover:bg-sunken hover:text-label"
                                                        >
                                                            {name}
                                                        </button>
                                                    }
                                                })
                                                .collect_view()}
                                        </div>
                                    }
                                })
                        }}
                    </components::Empty>
                }
                .into_any()
            }
            Some(panel) => (panel.render)(),
            None => ().into_any(),
        }
    }
}

/// The other way to have something to work on: a playground per chip, its
/// code beside a board that runs it, with no project to make first.
#[component]
fn Playgrounds() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let cards = rusty_embed::PLAYGROUND_CHIPS
            .into_iter()
            .map(|chip| {
                let name = crate::command::chip_name(state, chip);
                // What it takes, from the catalogue: the architecture, and
                // whether stable Rust builds for it or espup's has to.
                let needs = state.project.chips.with(|chips| {
                    chips.iter().find(|c| c.id == chip).map(|c| {
                        let toolchain = match c.toolchain {
                            rusty_embed::ToolchainRequirement::Stock => {
                                t!("chrome.playground-stable")
                            }
                            rusty_embed::ToolchainRequirement::EspXtensa => {
                                t!("chrome.playground-espup")
                            }
                        };
                        format!("{} · {toolchain}", c.arch.label())
                    })
                });
                view! {
                    <button
                        type="button"
                        on:click=move |_| controller::open_playground(state, chip)
                        class="flex min-w-0 flex-col items-start gap-0.5 rounded-[8px] px-3 py-2 text-left ring-1 ring-line transition-colors hover:bg-sunken hover:ring-line-strong"
                    >
                        <span class="flex items-center gap-1.5 text-callout font-medium">
                            <span class="text-rust">
                                <IconView icon=Icon::Simulate size=14 />
                            </span>
                            {name}
                        </span>
                        <span class="truncate text-footnote text-label-3">
                            {needs.unwrap_or_default()}
                        </span>
                    </button>
                }
            })
            .collect_view();
        view! {
            <div class="mt-5 flex w-full max-w-[52ch] flex-col gap-1.5 text-left">
                <div class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {t!("chrome.playground")}
                </div>
                <p class="text-footnote text-label-3">{t!("chrome.playground-detail")}</p>
                <div class="mt-1 grid grid-cols-2 gap-2">{cards}</div>
            </div>
        }
    }
}

#[component]
fn OpenProjectButton(#[prop(default = ButtonKind::Normal)] kind: ButtonKind) -> impl IntoView {
    let state = AppState::expect();
    let open = Callback::new(move |_| controller::choose_project(state));

    view! { <Button label=t!("menu.file.open-project") kind=kind on_click=open /> }
}
