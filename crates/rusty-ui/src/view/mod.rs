//! The shell, and the registry it renders.
//!
//! Structure borrowed from macOS: a real sidebar with labels rather than an
//! icon rail, a unified toolbar, and a status bar carrying the facts you check
//! without looking away from what you are doing.
//!
//! The shell knows no panel by name. It renders whatever [`panels::all`]
//! returns — the commitment in `docs/extensibility.md` that a contributed panel
//! can slot in later without the shell being rewritten to accept it.

pub mod components;
pub mod dock;
pub mod icon;
pub mod menu;
pub mod palette;
pub mod panels;
pub mod settings;
pub mod split;
pub mod terminal;

use leptos::prelude::*;

use crate::{
    controller,
    state::AppState,
    view::{
        components::{Button, ButtonKind, Dot, ErrorBanner, Tone},
        icon::{Icon, IconView},
    },
};

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
    pub title: &'static str,
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

    let settings_open = RwSignal::new(false);
    provide_context(SettingsOpen(settings_open));
    let palette_open = RwSignal::new(false);
    let chrome = crate::command::Chrome {
        settings_open,
        palette_open,
    };
    palette::install(state, chrome);

    view! {
        <div class="flex h-full flex-col bg-content text-label">
            <menu::MenuBar chrome=chrome />
            // Overlays are anchored to the working area, not the window, so
            // they cannot cover the title bar. Settings used to: its own Done
            // button ended up underneath the menu bar and the page became a
            // room with no door.
            <div class="relative flex min-h-0 flex-1">
                <settings::Settings open=settings_open />
                <palette::Palette open=palette_open chrome=chrome />
                <Sidebar />
                <split::Handle divider=crate::state::Divider::Sidebar />
                <main class="flex min-w-0 flex-1 flex-col overflow-hidden">
                    {move || {
                        state
                            .error
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
                    // The dock sits under the panel rather than under the whole
                    // window, as Xcode's debug area does: the sidebar is
                    // navigation and stays whole, the output belongs to the
                    // thing being worked on.
                    <div class="flex min-h-0 flex-1 flex-col">
                        <Stage />
                    </div>
                    <dock::Dock />
                </main>
                // The assistant, VSCode-chat style: a right-hand drawer the
                // title-bar icon toggles, beside whatever panel is active.
                <Show when=move || state.assistant_open.get()>
                    <aside class="flex w-[400px] flex-none flex-col border-l border-line bg-sidebar">
                        <div class="flex flex-none items-center gap-2 border-b border-line px-3 py-1.5">
                            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                "Assistant"
                            </span>
                            <span class="flex-1" />
                            <button
                                type="button"
                                title="Close"
                                on:click=move |_| state.assistant_open.set(false)
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
}

/// Renders the active panel, or explains why it cannot.
#[component]
fn Stage() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let active = state.active_panel.get();
        let panel = panels::all()
            .into_iter()
            .find(|p| p.id == active)
            .or_else(|| panels::all().into_iter().next());

        match panel {
            Some(panel) if panel.needs_project && !state.has_project() => {
                view! {
                    <components::Empty
                        title="No project open"
                        detail="Choose a folder containing a Cargo.toml. rusty reads the \
                                project's four configuration files and cross-checks them, so \
                                it can open — and diagnose — a project that does not build."
                    >
                        <OpenProjectButton kind=ButtonKind::Primary />
                        // The way back to yesterday's work, one click deep.
                        {move || {
                            let recents = state.recents.get();
                            (!recents.is_empty())
                                .then(|| {
                                    view! {
                                        <div class="mt-4 flex w-full max-w-[52ch] flex-col gap-0.5 text-left">
                                            <div class="mb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                                "Recent"
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

#[component]
fn OpenProjectButton(#[prop(default = ButtonKind::Normal)] kind: ButtonKind) -> impl IntoView {
    let state = AppState::expect();
    let open = Callback::new(move |_| controller::choose_project(state));

    view! { <Button label="Open project…" kind=kind on_click=open /> }
}

#[component]
fn Sidebar() -> impl IntoView {
    let state = AppState::expect();
    let all: Vec<Panel> = panels::all().into_iter().filter(|p| !p.hidden).collect();

    // Group in declaration order so the sidebar reads top to bottom the way the
    // work does: understand the project, then talk to the device.
    let mut sections: Vec<(&'static str, Vec<&Panel>)> = Vec::new();
    for panel in &all {
        match sections.last_mut() {
            Some((name, group)) if *name == panel.section => group.push(panel),
            _ => sections.push((panel.section, vec![panel])),
        }
    }

    view! {
        <nav
            class="flex flex-none flex-col overflow-y-auto bg-sidebar pb-2"
            style=move || format!("width: {}px", state.sidebar_width.get())
            aria-label="Panels"
        >
            {sections
                .into_iter()
                .map(|(section, group)| {
                    view! {
                        // A group with no name still has to look like a group.
                        // Rendering nothing at all left "New project" and
                        // "Assistant" sitting directly under the DEVICE heading,
                        // which reads as a claim that they are device panels —
                        // and neither of them touches a device.
                        {if section.is_empty() {
                            view! { <div class="mx-4 mt-3 mb-2 h-px bg-line" /> }.into_any()
                        } else {
                            view! { <components::SectionLabel label=section /> }.into_any()
                        }}
                        <div class="px-2">
                            {group
                                .into_iter()
                                .map(|panel| {
                                    let id = panel.id;
                                    let title = panel.title;
                                    let icon = panel.icon;
                                    let needs_project = panel.needs_project;
                                    let selected = Signal::derive(move || {
                                        state.active_panel.get() == id
                                    });
                                    let disabled = Signal::derive(move || {
                                        needs_project && !state.has_project()
                                    });
                                    view! {
                                        <button
                                            type="button"
                                            role="tab"
                                            aria-selected=move || selected.get().to_string()
                                            disabled=move || disabled.get()
                                            title=move || {
                                                if disabled.get() {
                                                    format!("{title} — open a project first")
                                                } else {
                                                    title.to_string()
                                                }
                                            }
                                            on:click=move |_| state.active_panel.set(id.to_string())
                                            class=move || {
                                                let base = "flex w-full items-center gap-2.5 rounded-[6px] \
                                                    px-2 py-[5px] text-body transition-colors \
                                                    disabled:pointer-events-none disabled:opacity-35";
                                                if selected.get() {
                                                    // macOS marks the selected row with a filled
                                                    // rounded rect, not a rule down the edge.
                                                    format!("{base} bg-selection font-medium text-rust")
                                                } else {
                                                    format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                                }
                                            }
                                        >
                                            <IconView icon=icon />
                                            <span class="truncate">{title}</span>
                                        </button>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                })
                .collect_view()}
        </nav>
    }
}

/// One segment of the status bar.
#[component]
fn Status(
    #[prop(into)] text: String,
    /// A dim prefix naming what the value is.
    #[prop(optional, into)] label: Option<String>,
    #[prop(optional)] tone: Option<Tone>,
    #[prop(optional, into)] title: Option<String>,
    #[prop(optional)] on_click: Option<Callback<()>>,
) -> impl IntoView {
    let colour = match tone {
        Some(Tone::Crimson) => "text-crimson",
        Some(Tone::Amber) => "text-amber",
        Some(Tone::Patina) => "text-patina",
        Some(Tone::Rust) => "text-rust",
        _ => "",
    };
    let interactive = if on_click.is_some() {
        "cursor-default hover:bg-sunken hover:text-label"
    } else {
        ""
    };
    view! {
        <button
            type="button"
            disabled=on_click.is_none()
            title=title
            on:click=move |_| {
                if let Some(cb) = on_click {
                    cb.run(());
                }
            }
            class=format!(
                "flex h-full items-center gap-1.5 border-r border-line px-3 transition-colors \
                 disabled:pointer-events-none {colour} {interactive}",
            )
        >
            {label.map(|label| view! { <span class="text-label-3">{label}</span> })}
            {text}
        </button>
    }
}

/// The facts you check without looking away from what you are doing.
///
/// A status bar that only says "idle" is a decoration. This one carries the
/// answers to the questions asked most often while working — what am I
/// targeting, what is stopping the build, what is plugged in — and the problem
/// count opens the dock rather than merely reporting a number.
#[component]
fn StatusBar() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <footer class="flex h-[26px] flex-none items-center border-t border-line bg-window font-mono text-footnote text-label-2">
            {move || {
                let busy = state.is_busy();
                view! {
                    <span class="flex h-full items-center gap-1.5 border-r border-line px-3">
                        <Dot tone=if busy { Tone::Amber } else { Tone::Patina } />
                        {if busy { "working" } else { "ready" }}
                    </span>
                }
            }}

            {move || {
                let (errors, _) = state.diag_counts();
                // Absence explains itself: no chip means the status has nothing
                // to say, but a missing language server looks like "the editor
                // is broken" unless something names it.
                let lsp = state.lsp_status.get();
                (state.has_project() && lsp != crate::state::LspStatus::Off)
                    .then(|| {
                        let (text, tone) = match lsp {
                            crate::state::LspStatus::Starting => {
                                ("rust-analyzer starting".to_string(), Tone::Neutral)
                            }
                            crate::state::LspStatus::Ready if errors > 0 => {
                                (format!("{errors} errors"), Tone::Crimson)
                            }
                            crate::state::LspStatus::Ready => {
                                ("rust-analyzer".to_string(), Tone::Patina)
                            }
                            _ => ("rust-analyzer missing".to_string(), Tone::Crimson),
                        };
                        view! {
                            <Status
                                text=text
                                tone=tone
                                title="The language server behind the editor"
                                on_click=Callback::new(move |_| {
                                    state.show_dock(crate::state::DockTab::Problems)
                                })
                            />
                        }
                    })
            }}

            {move || {
                let blocking = state.blocking_count();
                let total = state.problems().len();
                (total > 0)
                    .then(|| {
                        let text = if blocking > 0 {
                            format!("{blocking} blocking")
                        } else {
                            format!("{total} notes")
                        };
                        let tone = if blocking > 0 { Tone::Crimson } else { Tone::Amber };
                        view! {
                            <Status
                                text=text
                                tone=tone
                                title="Show them in the panel below"
                                on_click=Callback::new(move |_| {
                                    state.show_dock(crate::state::DockTab::Problems)
                                })
                            />
                        }
                    })
            }}

            {move || {
                state
                    .project
                    .get()
                    .map(|project| {
                        let chip = project.chip.clone().unwrap_or_else(|| "no chip".into());
                        let target = project
                            .configured_target
                            .clone()
                            .unwrap_or_else(|| "no target".into());
                        let toolchain = project
                            .configured_toolchain
                            .clone()
                            .unwrap_or_else(|| "unpinned".into());
                        // Labelled inline, not only by tooltip: three bare
                        // values reading "esp32 · xtensa-esp32-none-elf · esp"
                        // are a riddle, and nobody hovers a status bar to solve
                        // one.
                        view! {
                            <Status label="chip" text=chip title="Detected chip" />
                            <Status
                                label="target"
                                text=target
                                title="Target triple from .cargo/config.toml"
                            />
                            <Status
                                label="toolchain"
                                text=toolchain
                                title="Toolchain from rust-toolchain.toml"
                            />
                        }
                    })
            }}

            <span class="flex-1" />

            {move || {
                state
                    .workspace
                    .get()
                    .map(|report| {
                        view! {
                            <span class="flex h-full items-center border-l border-line px-3">
                                {format!("{} deps", report.vitals.resolved_deps)}
                            </span>
                        }
                    })
            }}

            // Doubles as proof the IPC bridge is alive: these numbers can only
            // be non-zero if a command round-tripped.
            {move || {
                let boards = state.boards.with(Vec::len);
                view! {
                    <span
                        class="flex h-full items-center border-l border-line px-3"
                        title="Boards known, after your own and the project's files are layered in"
                    >
                        {format!("{boards} boards")}
                    </span>
                }
            }}
        </footer>
    }
}
