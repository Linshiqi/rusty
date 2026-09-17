//! The open editors, one tab each.

use leptos::{ev, html, prelude::*};

use rusty_edit::Document;

use rusty_i18n::t;

use super::*;
use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, MenuItem, MenuSeparator},
    view::icon::{Icon, IconView},
};

/// The open editors, one tab each. Clicking fronts a tab with its draft and
/// caret exactly as left; the cross closes it, asking first when unsaved
/// work would go with it.
#[component]
pub(super) fn TabStrip() -> impl IntoView {
    let state = AppState::expect();
    let menu = RwSignal::new(None::<(f64, f64, String)>);

    if state.app.detached.with_untracked(Option::is_some) {
        return ().into_any();
    }
    view! {
        <div class="flex flex-none items-stretch border-b border-line bg-sidebar">
            <div class="flex min-w-0 flex-1 items-stretch overflow-x-auto">
            {move || {
                let active = state.active_path()
                    .unwrap_or_default();
                let open = state.editor.tabs.get();
                // As much of the path as it takes to tell two tabs apart,
                // and no more — a workspace's three `Cargo.toml`s were three
                // identical labels. See [`tab_hints`].
                let hints = tab_hints(&open);
                open
                    .into_iter()
                    .zip(hints)
                    .map(|(path, hint)| {
                        let name = path
                            .rsplit(['/', '\\'])
                            .next()
                            .unwrap_or(path.as_str())
                            .to_string();
                        let is_active = path == active;
                        // Dirty is per-tab: the active one compares live
                        // draft to document, a parked one compares its
                        // stashed pair.
                        // One derivation, in `AppState`: the replace has to
                        // ask the same question of the same files, and two
                        // copies of "is this dirty" is how a replace writes
                        // over a draft the dot said was there.
                        let dirty = {
                            let path = path.clone();
                            Signal::derive(move || state.is_dirty(&path))
                        };
                        // The disk moved under an unsaved draft. Distinct
                        // from dirty, and shown as well as it rather than
                        // instead: the tab has two problems at once and
                        // saving it would overwrite somebody else's change.
                        let stale = {
                            let path = path.clone();
                            Signal::derive(move || {
                                state.editor.stale.with(|list| list.contains(&path))
                            })
                        };
                        let activate = {
                            let path = path.clone();
                            move |_| controller::activate_tab(state, path.clone())
                        };
                        let close = {
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                event.stop_propagation();
                                controller::close_tab(state, path.clone());
                            }
                        };
                        let middle_close = {
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                if event.button() == 1 {
                                    event.prevent_default();
                                    controller::close_tab(state, path.clone());
                                }
                            }
                        };
                        // Dimmed when no `mod` declares it, the way the
                        // tree dims it and the way VS Code dims a file the
                        // project does not build. The tab is where the eye
                        // is once the file is open.
                        let unlinked = {
                            let path = path.clone();
                            Signal::derive(move || state.is_unlinked(&path))
                        };
                        let tab_class = if is_active {
                            "group flex cursor-pointer items-center gap-1.5 border-r border-line \
                             bg-canvas px-2.5 py-1.5 font-mono text-footnote text-label"
                        } else {
                            "group flex cursor-pointer items-center gap-1.5 border-r border-line \
                             px-2.5 py-1.5 font-mono text-footnote text-label-3 hover:bg-sunken \
                             hover:text-label-2"
                        };
                        let open_menu = {
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                event.prevent_default();
                                event.stop_propagation();
                                menu.set(Some((
                                    f64::from(event.client_x()),
                                    f64::from(event.client_y()),
                                    path.clone(),
                                )));
                            }
                        };
                        view! {
                            <div
                                title=path.clone()
                                on:click=activate
                                on:auxclick=middle_close
                                on:contextmenu=open_menu
                                class=move || {
                                    if unlinked.get() {
                                        format!("{tab_class} opacity-60")
                                    } else {
                                        tab_class.to_string()
                                    }
                                }
                            >
                                <span class="max-w-[18ch] truncate">{name}</span>
                                // Quieter than the name: it is there to
                                // separate, not to be read.
                                {(!hint.is_empty())
                                    .then(|| {
                                        view! {
                                            <span class="max-w-[14ch] shrink truncate text-label-3">
                                                {hint}
                                            </span>
                                        }
                                    })}
                                {move || {
                                    dirty
                                        .get()
                                        .then(|| {
                                            view! {
                                                <span
                                                    class="size-1.5 shrink-0 rounded-full bg-rust"
                                                    title=t!("files.unsaved")
                                                />
                                            }
                                        })
                                }}
                                {move || {
                                    stale
                                        .get()
                                        .then(|| {
                                            view! {
                                                <span
                                                    class="shrink-0 leading-none text-amber"
                                                    title=t!("files.stale")
                                                >
                                                    "⚠"
                                                </span>
                                            }
                                        })
                                }}
                                <button
                                    type="button"
                                    title=t!("files.close")
                                    on:click=close
                                    class="rounded-[4px] px-0.5 leading-none text-label-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-selection hover:text-label"
                                >
                                    "×"
                                </button>
                            </div>
                        }
                    })
                    .collect_view()
            }}
            </div>
            // Split, at the strip's end where VS Code keeps it — on the left
            // group only, since there is nothing further right of the right
            // group. Disabled with one tab: moving a group's only file across
            // leaves an empty pane.
            {(state.group == crate::state::Group::First).then(|| {
                view! {
                    {move || {
                        let enough = state.editor.tabs.with(|tabs| tabs.len() >= 2);
                        view! {
                            <button
                                type="button"
                                title=if enough { t!("files.split") } else { t!("files.split-needs-two") }
                                disabled=!enough
                                on:click=move |_| controller::split_active(state)
                                class="grid w-8 shrink-0 place-items-center border-l border-line text-label-3 hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-35"
                            >
                                <IconView icon=Icon::Columns size=14 />
                            </button>
                        }
                    }}
                }
            })}

            {move || {
                let (x, y, path) = menu.get()?;
                let close = Callback::new(move |_| menu.set(None));
                let (this, others, copy, float, beside) = (
                    path.clone(),
                    path.clone(),
                    path.clone(),
                    path.clone(),
                    path.clone(),
                );
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            <MenuItem
                                label=t!("context.tab-close")
                                shortcut="Ctrl+W"
                                on_select=Callback::new(move |_| {
                                    controller::close_tab(state, this.clone());
                                    menu.set(None);
                                })
                            />
                            // Left group only: "beside" is the right group, and
                            // a tab already there has nowhere further to go.
                            {(state.group == crate::state::Group::First).then(|| {
                                let beside = beside.clone();
                                view! {
                                    <MenuItem
                                        label=t!("context.tab-open-beside")
                                        on_select=Callback::new(move |_| {
                                            controller::open_beside(state, beside.clone());
                                            menu.set(None);
                                        })
                                    />
                                }
                            })}
                            <MenuItem
                                label=t!("context.tab-new-window")
                                on_select=Callback::new(move |_| {
                                    controller::detach_file(state, float.clone());
                                    // The dirty guard inside close_tab still
                                    // applies: unsaved work keeps its tab here.
                                    controller::close_tab(state, float.clone());
                                    menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.tab-close-others")
                                on_select=Callback::new(move |_| {
                                    for open in state.editor.tabs.get_untracked() {
                                        if open != others {
                                            controller::close_tab(state, open);
                                        }
                                    }
                                    menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label=t!("context.tab-copy-path")
                                on_select=Callback::new(move |_| {
                                    copy_to_clipboard(&copy);
                                    menu.set(None);
                                })
                            />
                        </ContextMenu>
                    },
                )
            }}
        </div>
    }
    .into_any()
}

/// What to write beside each tab's name so two tabs are never the same word.
///
/// A workspace has three `Cargo.toml`s, two `main.rs`es and two `lib.rs`es,
/// and the strip showed each of them as its bare file name — eleven tabs with
/// six distinct labels between them, reported as "I cannot find the file the
/// tab is showing". VS Code's rule: the name alone while it is unique, and
/// otherwise as much of the path above it as it takes to tell them apart, no
/// more.
///
/// Parallel to `paths`, empty where the name stands on its own. The suffix
/// grows one directory at a time and stops at the first depth that separates
/// the whole group, so `core/Cargo.toml` against `firmware/Cargo.toml` is
/// `core` and `firmware` rather than either full path — and two `lib.rs`es
/// both under a `src` go to `core/src` and `firmware/src`, because one
/// segment does not part them.
///
/// A file at the project root keeps a bare name even when it shares one. It
/// is the only member of its group with nothing above it, so "no suffix" is
/// itself the distinguishing mark, and inventing a word for it (`./`, the
/// project's name) would be a label the path does not contain.
pub(super) fn tab_hints(paths: &[String]) -> Vec<String> {
    let name_of =
        |path: &str| -> String { path.rsplit(['/', '\\']).next().unwrap_or(path).to_string() };
    // The directories above the file, nearest first: `core/src/math/q.rs`
    // gives `["math", "src", "core"]`.
    let parents = |path: &str| -> Vec<String> {
        let mut parts: Vec<String> = path.split(['/', '\\']).map(str::to_string).collect();
        parts.pop();
        parts.reverse();
        parts
    };

    let mut hints = vec![String::new(); paths.len()];
    for (index, path) in paths.iter().enumerate() {
        let name = name_of(path);
        // Everyone else wearing this name. One tab per path, so a path equal
        // to this one is this one.
        let rivals: Vec<usize> = paths
            .iter()
            .enumerate()
            .filter(|(other, p)| *other != index && name_of(p) == name)
            .map(|(other, _)| other)
            .collect();
        if rivals.is_empty() {
            continue;
        }
        let mine = parents(path);
        let deepest = rivals
            .iter()
            .map(|&other| parents(&paths[other]).len())
            .chain(std::iter::once(mine.len()))
            .max()
            .unwrap_or(0);
        for depth in 1..=deepest {
            let suffix = |parts: &[String]| parts.iter().take(depth).cloned().collect::<Vec<_>>();
            let ours = suffix(&mine);
            if rivals
                .iter()
                .all(|&other| suffix(&parents(&paths[other])) != ours)
            {
                // Written the way the path reads, outermost first.
                let mut shown = ours;
                shown.reverse();
                hints[index] = shown.join("/");
                break;
            }
        }
    }
    hints
}

#[component]
pub(super) fn Header(
    document: Document,
    /// The editing surface's textarea, when the document is open in one:
    /// Save formats through it and lands the caret where the eye is. The
    /// Markdown page view has no textarea, and saves the draft as it is.
    #[prop(optional)]
    area: Option<NodeRef<html::Textarea>>,
) -> impl IntoView {
    let state = AppState::expect();
    let saved = document.text.clone();
    let path = document.path.clone();
    let read_only = document.read_only;
    let dirty = Signal::derive(move || state.editor.draft.with(|draft| draft != &saved));
    // In no crate's module tree, so rust-analyzer answers nothing here. Said
    // by *dimming* the name, as VS Code says a file the project does not
    // build, with the reason in the tooltip — and by nothing else. It was a
    // full-width amber banner with a sentence of explanation and a button,
    // which is a paragraph where a shade of grey is the whole message: the
    // user's verdict was "a pile of warning text". The fix stays one click
    // away where every other fix is, on the hover card over the squiggle.
    let unlinked = {
        let path = path.clone();
        Signal::derive(move || state.is_unlinked(&path))
    };

    view! {
        <div class="flex flex-none items-center gap-2 border-b border-line px-3 py-1.5">
            <span
                class=move || {
                    let base = "truncate font-mono text-footnote";
                    if unlinked.get() {
                        format!("{base} opacity-60")
                    } else {
                        base.to_string()
                    }
                }
                title=move || {
                    if unlinked.get() { t!("misc.unlinked-file") } else { String::new() }
                }
            >
                {document.path}
            </span>
            {move || {
                dirty
                    .get()
                    .then(|| {
                        view! {
                            <span class="size-1.5 shrink-0 rounded-full bg-rust" title=t!("misc.unsaved") />
                        }
                    })
            }}
            <span class="flex-1" />
            // Save, at the right of the file it saves. It was in the rail,
            // between Build and Flash, as if it acted on the project.
            <button
                type="button"
                title=t!("toolbar.save")
                disabled=read_only
                on:click=move |_| match area {
                    Some(area) => format_and_save(state, area),
                    None => controller::save_file(state),
                }
                class=move || {
                    let base = "grid size-6 shrink-0 place-items-center rounded-[5px] \
                                hover:bg-sunken disabled:pointer-events-none disabled:opacity-40";
                    if dirty.get() {
                        format!("{base} text-rust")
                    } else {
                        format!("{base} text-label-3 hover:text-label")
                    }
                }
            >
                <IconView icon=Icon::Save size=14 />
            </button>
            // Markdown and SVG only — the two files that are both a text and
            // a thing the text draws. Every other file has one way to read
            // it, and a toggle that does nothing on 95% of tabs is chrome.
            {(super::editor::is_markdown(&path) || super::editor::is_svg(&path))
                .then(|| {
                    let path = path.clone();
                    let page = super::editor::is_markdown(&path);
                    let showing_source = {
                        let path = path.clone();
                        Signal::derive(move || {
                            state.editor.source_view.with(|v| v.contains(&path))
                        })
                    };
                    view! {
                        <button
                            type="button"
                            title=move || {
                                match (showing_source.get(), page) {
                                    (true, true) => t!("markdown.show-preview"),
                                    (true, false) => t!("image.show-picture"),
                                    (false, true) => t!("markdown.show-source"),
                                    (false, false) => t!("image.show-source"),
                                }
                            }
                            on:click=move |_| {
                                let path = path.clone();
                                state
                                    .editor
                                    .source_view
                                    .update(|open| match open.iter().position(|p| *p == path) {
                                        Some(at) => {
                                            open.remove(at);
                                        }
                                        None => open.push(path),
                                    })
                            }
                            class="shrink-0 rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                        >
                            {move || {
                                match (showing_source.get(), page) {
                                    (true, true) => t!("markdown.preview"),
                                    (true, false) => t!("image.picture"),
                                    (false, true) => t!("markdown.source"),
                                    (false, false) => t!("image.source"),
                                }
                            }}
                        </button>
                    }
                })}
            {document
                .read_only
                .then(|| {
                    view! {
                        <span
                            class="rounded-full bg-sunken px-2 text-footnote text-label-2"
                            title=t!("tabs.read-only-hint")
                        >
                            {t!("tabs.read-only")}
                        </span>
                    }
                })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(paths: &[&str]) -> Vec<String> {
        tab_hints(&paths.iter().map(|p| (*p).to_string()).collect::<Vec<_>>())
    }

    /// A name nobody else wears is written alone.
    #[test]
    fn a_unique_name_carries_no_suffix() {
        assert_eq!(hints(&["core/src/lib.rs", "firmware/build.rs"]), ["", ""]);
    }

    /// The workspace that produced the report: three `Cargo.toml`s. One
    /// directory apiece parts them, and the one at the root stays bare —
    /// having nothing above it is what tells it apart.
    #[test]
    fn three_manifests_are_told_apart_by_one_directory_each() {
        assert_eq!(
            hints(&["Cargo.toml", "core/Cargo.toml", "firmware/Cargo.toml"]),
            ["", "core", "firmware"]
        );
    }

    /// One segment does not always do it: two `lib.rs`es both under a `src`
    /// need the crate above it, and only that.
    #[test]
    fn the_suffix_grows_until_it_separates_them() {
        assert_eq!(
            hints(&["core/src/lib.rs", "firmware/src/lib.rs"]),
            ["core/src", "firmware/src"]
        );
    }

    /// And it stops as soon as it has: `bin` against `src` is enough, so
    /// neither grows to the crate.
    #[test]
    fn the_suffix_stops_at_the_first_depth_that_works() {
        assert_eq!(
            hints(&["firmware/src/main.rs", "firmware/src/bin/main.rs"]),
            ["src", "bin"]
        );
    }

    /// Three at once, two of which need more than the others.
    #[test]
    fn each_tab_grows_only_as_far_as_its_own_group_needs() {
        assert_eq!(
            hints(&[
                "Cargo.toml",
                "core/Cargo.toml",
                "core/src/lib.rs",
                "firmware/src/lib.rs",
            ]),
            ["", "core", "core/src", "firmware/src"]
        );
    }

    /// Windows separators arrive from the same places project-relative paths
    /// do, and must not read as one long file name.
    #[test]
    fn a_backslash_is_a_separator_too() {
        assert_eq!(
            hints(&["core\\Cargo.toml", "firmware\\Cargo.toml"]),
            ["core", "firmware"]
        );
    }
}
