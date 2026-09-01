//! The shell, and the registry it renders.
//!
//! Structure: a VSCode-style activity bar of icons down the left edge, a
//! unified toolbar, and a status bar carrying the facts you check without
//! looking away from what you are doing. The labels went when the bar did —
//! a tooltip names the icon, and the width the labels cost bought nothing.
//!
//! The shell knows no panel by name. It renders whatever [`panels::all`]
//! returns — the commitment in `docs/extensibility.md` that a contributed panel
//! can slot in later without the shell being rewritten to accept it.

pub mod components;
pub mod dock;
pub mod icon;
pub mod loclink;
pub mod markdown;
pub mod menu;
pub mod palette;
pub mod panels;
pub mod pinmap;
pub mod plot;
pub mod settings;
pub mod split;
pub mod terminal;
pub mod transport;
pub mod waves;

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
                        title="Close this window and reopen the file in the main one"
                        on:click=move |_| controller::reattach(state, home.clone())
                        class="flex h-[26px] items-center gap-1.5 rounded-[6px] px-2.5 text-footnote text-label-2 transition-colors hover:bg-sunken hover:text-label"
                    >
                        "↩ Back to the main window"
                    </button>
                </div>
                <div class="flex min-h-0 flex-1 flex-col">
                    {panels::files_view()}
                </div>
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
            // The context toolbar: whatever the active workspace registered.
            // Absent registration collapses the row entirely — no dead strip
            // over panels that brought no tools.
            {move || {
                let content = state.layout.toolbar.get()?;
                Some(view! {
                    <div class="flex h-10 flex-none items-center gap-1.5 border-b border-line bg-content px-3">
                        {content.run(())}
                    </div>
                })
            }}
            <div class="relative flex min-h-0 flex-1">

                <palette::Palette open=palette_open chrome=chrome />
                <Sidebar />
                <main class="flex min-w-0 flex-1 flex-col overflow-hidden">
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
                        // Over the editor's corner, and only there: the pin
                        // map answers a question you have while reading code.
                        // `pointer-events-none` on the layer so the corner it
                        // does not fill still belongs to the editor.
                        {move || {
                            (!settings_open.get() && state.layout.panel.get() == "files")
                                .then(|| {
                                    view! {
                                        <div class="pointer-events-none absolute inset-0">
                                            <pinmap::PinMap />
                                        </div>
                                    }
                                })
                        }}
                    </div>
                    <dock::Dock />
                </main>
                // The assistant, VSCode-chat style: a right-hand drawer the
                // title-bar icon toggles, beside whatever panel is active.
                <Show when=move || state.ai.open.get()>
                    <aside class="flex w-[400px] flex-none flex-col border-l border-line bg-sidebar">
                        <div class="flex flex-none items-center gap-2 border-b border-line px-3 py-1.5">
                            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                "Assistant"
                            </span>
                            <span class="flex-1" />
                            <button
                                type="button"
                                title="Close"
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
                        title="No project open"
                        detail="Choose a folder containing a Cargo.toml. rusty reads the \
                                project's four configuration files and cross-checks them, so \
                                it can open — and diagnose — a project that does not build."
                    >
                        <OpenProjectButton kind=ButtonKind::Primary />
                        // The way back to yesterday's work, one click deep.
                        {move || {
                            let recents = state.app.recents.get();
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

    // Group in declaration order so the bar reads top to bottom the way the
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
            class="flex w-[46px] flex-none flex-col overflow-y-auto border-r border-line bg-sidebar pt-1.5 pb-2"
            aria-label="Panels"
        >
            {sections
                .into_iter()
                .enumerate()
                .map(|(index, (_, group))| {
                    view! {
                        // A rule between groups, as VSCode draws them; the
                        // first group starts at the top edge.
                        {(index > 0)
                            .then(|| view! { <div class="mx-3 my-2 h-px bg-line" /> })}
                        <div class="flex flex-col items-center gap-0.5">
                            {group
                                .into_iter()
                                .map(|panel| {
                                    let id = panel.id;
                                    let title = panel.title;
                                    let icon = panel.icon;
                                    let needs_project = panel.needs_project;
                                    let selected = Signal::derive(move || {
                                        state.layout.panel.get() == id
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
                                            on:click=move |_| {
                                                let SettingsOpen(settings) =
                                                    expect_context::<SettingsOpen>();
                                                settings.set(false);
                                                state.layout.panel.set(id.to_string());
                                            }
                                            class=move || {
                                                let base = "grid size-8 place-items-center rounded-[6px] \
                                                    transition-colors disabled:pointer-events-none \
                                                    disabled:opacity-35";
                                                if selected.get() {
                                                    format!("{base} bg-selection text-rust")
                                                } else {
                                                    format!(
                                                        "{base} text-label-2 hover:bg-sunken hover:text-label",
                                                    )
                                                }
                                            }
                                        >
                                            <IconView icon=icon />
                                        </button>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                })
                .collect_view()}
            <div class="mt-auto flex flex-col items-center pt-2">
                {
                    let SettingsOpen(settings) = expect_context::<SettingsOpen>();
                    view! {
                        <button
                            type="button"
                            title="Settings (Ctrl+,)"
                            on:click=move |_| settings.update(|open| *open = !*open)
                            class=move || {
                                let base =
                                    "grid size-8 place-items-center rounded-[6px] transition-colors";
                                if settings.get() {
                                    format!("{base} bg-selection text-rust")
                                } else {
                                    format!(
                                        "{base} text-label-2 hover:bg-sunken hover:text-label",
                                    )
                                }
                            }
                        >
                            <IconView icon=Icon::Settings />
                        </button>
                    }
                }
            </div>
        </nav>
    }
}

/// One segment of the status bar.
#[component]
fn Status(
    #[prop(into)] text: String,
    /// A dim prefix naming what the value is.
    #[prop(optional, into)]
    label: Option<String>,
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

/// What this project is built for: the chip in the bar, the rest on click.
///
/// The popover opens upwards because the bar is the last row on screen — a
/// menu that renders below it is a menu nobody sees.
#[component]
fn BuiltFor(chip: String, target: String, toolchain: String) -> impl IntoView {
    let open = RwSignal::new(false);
    // The proposed switch, once one has been planned. Held here rather than
    // applied on click: what a chip switch touches is exactly what somebody
    // needs to read before it happens.
    let proposal = RwSignal::new(None::<rusty_embed::Migration>);
    let picking = RwSignal::new(false);
    let current = chip.clone();
    let row = |label: &'static str, value: String, note: &'static str| {
        view! {
            <div class="flex flex-col gap-0.5 px-3 py-1.5">
                <div class="flex items-baseline gap-2">
                    <span class="w-[4.5rem] shrink-0 text-label-3">{label}</span>
                    <span class="min-w-0 break-all text-label select-text">{value}</span>
                </div>
                <span class="pl-[calc(4.5rem+0.5rem)] text-caption text-label-4">{note}</span>
            </div>
        }
    };

    view! {
        <div class="relative h-full">
            <button
                type="button"
                title="What this project builds for — click for the target and toolchain"
                on:click=move |_| open.update(|it| *it = !*it)
                class="flex h-full items-center gap-1.5 border-r border-line px-3 transition-colors hover:bg-sunken hover:text-label"
            >
                <span class="text-label-3">"chip"</span>
                {chip}
                <span class="text-label-4">"▴"</span>
            </button>
            {move || {
                open.get()
                    .then(|| {
                        let dismiss = move |_| {
                            open.set(false);
                            picking.set(false);
                            proposal.set(None);
                        };
                        let current = current.clone();
                        view! {
                            // Full-screen catcher, so clicking anywhere else
                            // closes it — the behaviour every menu in here has.
                            <div class="fixed inset-0 z-40" on:click=dismiss />
                            // Sized to what is in it, capped so a migration's
                            // notes wrap instead of running off. A fixed width
                            // wide enough for the plan left two short rows
                            // sitting in an otherwise empty box.
                            <div class="absolute bottom-full left-0 z-50 mb-px max-h-[70vh] w-max max-w-[34rem] min-w-[14rem] overflow-y-auto rounded-t-[8px] border border-line bg-raised py-1.5 shadow-lg">
                                {row(
                                    "target",
                                    target.clone(),
                                    "target triple, from .cargo/config.toml",
                                )}
                                {row(
                                    "toolchain",
                                    toolchain.clone(),
                                    "channel, from rust-toolchain.toml",
                                )}
                                <div class="my-1 h-px bg-line" />
                                <SwitchChip current=current picking=picking proposal=proposal />
                            </div>
                        }
                    })
            }}
        </div>
    }
}

/// Moving the project to another chip, in three steps that are all reversible
/// until the last one: pick, read what it would do, apply.
///
/// The offer lives beside the chip because that is the fact it changes. The
/// answer to "must I recreate the project" is no for the configuration and
/// yes for anything naming a pin, and the plan says which is which rather
/// than implying it did everything.
#[component]
fn SwitchChip(
    current: String,
    picking: RwSignal<bool>,
    proposal: RwSignal<Option<rusty_embed::Migration>>,
) -> impl IntoView {
    let state = AppState::expect();

    view! {
        {move || {
            if let Some(plan) = proposal.get() {
                let blocker = plan.blocker.clone();
                let heading = format!("{} → {}", plan.from, plan.to);
                let files = plan.files.clone();
                let notes = plan.notes.clone();
                let runnable = plan.clone();
                return view! {
                    <div class="px-3 py-1.5">
                        <div class="mb-1.5 font-mono text-footnote text-label">{heading}</div>
                        {blocker
                            .clone()
                            .map(|why| {
                                view! {
                                    <p class="rounded-[6px] bg-amber-fill px-2.5 py-2 text-caption leading-relaxed text-amber select-text">
                                        {why}
                                    </p>
                                }
                            })}
                        {(blocker.is_none())
                            .then(|| {
                                view! {
                                    <div class="mb-1.5 flex flex-col gap-0.5">
                                        {files
                                            .into_iter()
                                            .map(|file| {
                                                let count = file.edits.len();
                                                view! {
                                                    <div class="flex items-baseline justify-between gap-2 font-mono text-caption">
                                                        <span class="text-label-2">{file.path}</span>
                                                        <span class="shrink-0 text-label-4">
                                                            {format!(
                                                                "{count} change{}",
                                                                if count == 1 { "" } else { "s" },
                                                            )}
                                                        </span>
                                                    </div>
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                    <ul class="mb-2 flex flex-col gap-1">
                                        {notes
                                            .into_iter()
                                            .map(|note| {
                                                view! {
                                                    <li class="text-caption leading-relaxed text-label-3 select-text">
                                                        "— "{note}
                                                    </li>
                                                }
                                            })
                                            .collect_view()}
                                    </ul>
                                    <div class="flex gap-2">
                                        <button
                                            type="button"
                                            on:click=move |_| {
                                                controller::apply_migration(
                                                    state,
                                                    runnable.clone(),
                                                    proposal,
                                                )
                                            }
                                            class="rounded-[6px] bg-rust px-2.5 py-1 text-caption text-window transition-opacity hover:opacity-90"
                                        >
                                            "Switch"
                                        </button>
                                        <button
                                            type="button"
                                            on:click=move |_| proposal.set(None)
                                            class="rounded-[6px] px-2.5 py-1 text-caption text-label-2 transition-colors hover:bg-sunken hover:text-label"
                                        >
                                            "Cancel"
                                        </button>
                                    </div>
                                }
                            })}
                    </div>
                }
                    .into_any();
            }
            if !picking.get() {
                return view! {
                    <button
                        type="button"
                        on:click=move |_| picking.set(true)
                        class="flex w-full items-center px-3 py-1.5 text-left text-footnote text-label-2 transition-colors hover:bg-sunken hover:text-label"
                    >
                        "Switch this project to another chip…"
                    </button>
                }
                    .into_any();
            }
            let current = current.clone();
            // Which HAL this project's part sits behind. A switch is only
            // mechanical within one, so the list says which rows are a switch
            // and which are a new project — before the click, not after it.
            let ours = state
                .project.chips
                .get()
                .into_iter()
                .find(|chip| chip.id == current)
                .and_then(|chip| chip.hal);
            view! {
                <div class="max-h-56 overflow-y-auto py-0.5">
                    {state
                        .project.chips
                        .get()
                        .into_iter()
                        .filter(|chip| chip.id != current)
                        .map(|chip| {
                            let id = chip.id.clone();
                            let same_hal = chip.hal.is_some() && chip.hal == ours;
                            let detail = if same_hal {
                                format!("{} · {}", chip.arch.label(), chip.bare_metal_target)
                            } else {
                                format!(
                                    "{} · different HAL — a new project, not a switch",
                                    chip.arch.label(),
                                )
                            };
                            let tone = if same_hal { "text-label-4" } else { "text-amber" };
                            view! {
                                <button
                                    type="button"
                                    disabled=!same_hal
                                    title=if same_hal {
                                        String::new()
                                    } else {
                                        "Every call your firmware makes to the HAL differs. \
                                         The wizard creates a project for this part."
                                            .to_string()
                                    }
                                    on:click=move |_| {
                                        controller::plan_migration(state, id.clone(), proposal)
                                    }
                                    class="flex w-full flex-col items-start px-3 py-1 text-left transition-colors hover:bg-sunken disabled:pointer-events-none disabled:opacity-55"
                                >
                                    <span class="font-mono text-footnote text-label">{chip.name}</span>
                                    <span class=format!("font-mono text-caption {tone}")>{detail}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            }
                .into_any()
        }}
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
                let lsp = state.lsp.status.get();
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

            // The mode, and the half-typed command beside it. A modal editor
            // whose mode is invisible is one where every other keystroke is a
            // guess — this is the first thing a Vim user's eye goes to.
            {move || {
                state
                    .editor.vim_on
                    .get()
                    .then(|| {
                        let (label, hint) = state
                            .editor.vim
                            .with(|vim| (vim.mode.label(), vim.hint()));
                        let tone = match label {
                            "INSERT" => Tone::Patina,
                            "NORMAL" => Tone::Neutral,
                            _ => Tone::Amber,
                        };
                        let text = if hint.is_empty() {
                            label.to_string()
                        } else {
                            format!("{label}  {hint}")
                        };
                        view! {
                            <Status
                                text=text
                                tone=tone
                                title="Modal editing. Escape returns to normal mode."
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
                    .project.detected
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
                        // One chip, not three. The three values answer one
                        // question — what is this project built for — and the
                        // chip is the part of the answer anyone reads at a
                        // glance; the triple and the channel are what you look
                        // up when something is wrong, which is a click away.
                        //
                        // They were still labelled inline when they sat in the
                        // bar, because three bare values reading "esp32 ·
                        // xtensa-esp32-none-elf · esp" are a riddle. Inside the
                        // popover there is room to label them properly.
                        view! {
                            <BuiltFor chip=chip target=target toolchain=toolchain />
                        }
                    })
            }}

            <span class="flex-1" />

            {move || {
                state
                    .project.workspace
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
                let boards = state.project.boards.with(Vec::len);
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
