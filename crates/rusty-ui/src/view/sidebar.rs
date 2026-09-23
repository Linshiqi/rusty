//! The activity bar down the left edge: one icon per panel, grouped, and
//! nothing else — it switches panels.

use super::*;

#[component]
pub(super) fn Sidebar() -> impl IntoView {
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
            aria-label=t!("misc.panels")
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
                                    let title = panel.title.clone();
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
                                                    t!("panel.needs-project", panel = title)
                                                } else if id == "files" && selected.get() {
                                                    t!("panel.files-toggle")
                                                } else {
                                                    title.clone()
                                                }
                                            }
                                            on:click=move |_| {
                                                let SettingsOpen(settings) =
                                                    expect_context::<SettingsOpen>();
                                                let already = !settings.get_untracked()
                                                    && state.layout.panel.get_untracked() == id;
                                                settings.set(false);
                                                state.layout.panel.set(id.to_string());
                                                // A second click on the switcher you are on
                                                // folds the file tree away, as VS Code's
                                                // activity bar folds its sidebar. Files is
                                                // the one panel with a list to fold.
                                                if already && id == "files" {
                                                    controller::toggle_tree(state);
                                                }
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
                            title=t!("toolbar.settings")
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
