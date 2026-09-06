//! The file finder — the title bar's search box, and Ctrl+P.
//!
//! VS Code's "Go to File": type part of a name, the list narrows, Enter
//! opens the pick in the group the user is in and Ctrl+Enter opens it
//! beside. The candidates are the file tree the Files panel already holds,
//! flattened — the tree is read whole with `target/` and dot-entries already
//! excluded, so there is no second walk to keep in step with the first.

use leptos::{ev, html, prelude::*};

use rusty_edit::Entry;

use rusty_i18n::t;

use crate::{controller, state::AppState};

/// How many rows the list shows. Past this the query is what narrows it.
const SHOWN: usize = 60;

/// Every file under the tree, project-relative, in tree order.
pub fn files(entries: &[Entry], out: &mut Vec<String>) {
    for entry in entries {
        if entry.is_dir {
            files(&entry.children, out);
        } else {
            out.push(entry.path.clone());
        }
    }
}

/// The indices of `paths` that match `query`, best first.
///
/// A match is every character of the query appearing in order, case-folded.
/// The query found whole in the file name beats it found whole anywhere in
/// the path, which beats it scattered through the name, which beats it
/// scattered through the path; a name that *starts* with it is better still;
/// and among equals the shorter path wins — so `main` finds `src/main.rs`
/// before `book/src/22-maintenance.md`. An empty query is the tree in its
/// own order.
pub fn rank(query: &str, paths: &[String]) -> Vec<usize> {
    let needle: Vec<char> = query.trim().to_lowercase().chars().collect();
    let mut scored: Vec<(i64, usize)> = paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| score(&needle, path).map(|score| (score, index)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

fn score(needle: &[char], path: &str) -> Option<i64> {
    let length = i64::try_from(path.len()).unwrap_or(i64::MAX);
    if needle.is_empty() {
        return Some(-length);
    }
    let lower = path.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let query: String = needle.iter().collect();
    let mut score = if name.contains(&query) {
        3000
    } else if lower.contains(&query) {
        2000
    } else if subsequence(needle, name) {
        1500
    } else if subsequence(needle, &lower) {
        1000
    } else {
        return None;
    };
    if name.starts_with(&query) {
        score += 500;
    }
    Some(score - length)
}

/// Whether every character of `needle` appears in `hay`, in order.
fn subsequence(needle: &[char], hay: &str) -> bool {
    let mut rest = hay.chars();
    needle.iter().all(|c| rest.any(|h| h == *c))
}

#[component]
pub fn QuickOpen() -> impl IntoView {
    let state = AppState::expect();
    let open = state.layout.quick_open;
    let query = RwSignal::new(String::new());
    let highlighted = RwSignal::new(0usize);
    let input: NodeRef<html::Input> = NodeRef::new();

    // Reset and focus each time it opens, as the palette does: a finder that
    // reopens showing the last search is one that needs clearing every time.
    Effect::new(move |_| {
        if open.get() {
            query.set(String::new());
            highlighted.set(0);
            if let Some(element) = input.get() {
                let _ = element.focus();
            }
        }
    });

    let all = Memo::new(move |_| {
        let mut out = Vec::new();
        state.editor.tree.with(|tree| files(tree, &mut out));
        out
    });
    let ranked = Signal::derive(move || {
        let paths = all.get();
        rank(&query.get(), &paths)
            .into_iter()
            .take(SHOWN)
            .map(|index| paths[index].clone())
            .collect::<Vec<_>>()
    });

    let pick = move |index: usize, beside: bool| {
        let Some(path) = ranked.get_untracked().get(index).cloned() else {
            return;
        };
        open.set(false);
        let target = state.focused();
        if beside {
            controller::open_beside(target, path);
        } else {
            controller::open_file(target, path);
        }
    };

    view! {
        <Show when=move || open.get()>
            // Dropped from the title bar's box rather than centred like the
            // palette: it is that box, opened.
            <div
                class="absolute inset-0 z-30 flex justify-center bg-black/25 pt-1"
                on:click=move |_| open.set(false)
            >
                <div
                    class="flex max-h-[60vh] w-[560px] flex-col overflow-hidden rounded-[12px] bg-raised shadow-2xl ring-1 ring-line-strong"
                    on:click=move |event| event.stop_propagation()
                >
                    <input
                        node_ref=input
                        class="h-12 flex-none border-b border-line bg-transparent px-4 text-strong text-label outline-none placeholder:text-label-3"
                        placeholder=t!("quick.placeholder")
                        on:input=move |event| {
                            query.set(event_target_value(&event));
                            highlighted.set(0);
                        }
                        on:keydown=move |event: ev::KeyboardEvent| {
                            let count = ranked.get_untracked().len();
                            match event.key().as_str() {
                                "ArrowDown" => {
                                    event.prevent_default();
                                    highlighted
                                        .update(|i| *i = if count == 0 { 0 } else { (*i + 1) % count });
                                }
                                "ArrowUp" => {
                                    event.prevent_default();
                                    highlighted.update(|i| {
                                        *i = if count == 0 { 0 } else { (*i + count - 1) % count }
                                    });
                                }
                                "Enter" => {
                                    event.prevent_default();
                                    pick(highlighted.get_untracked(), event.ctrl_key());
                                }
                                "Escape" => {
                                    event.prevent_default();
                                    open.set(false);
                                }
                                _ => {}
                            }
                        }
                    />
                    <div class="min-h-0 flex-1 overflow-y-auto py-1.5">
                        {move || {
                            let paths = ranked.get();
                            if paths.is_empty() {
                                return view! {
                                    <p class="px-4 py-3 text-callout text-label-2">{t!("quick.no-match")}</p>
                                }
                                    .into_any();
                            }
                            paths
                                .into_iter()
                                .enumerate()
                                .map(|(index, path)| {
                                    let (dir, name) = match path.rfind('/') {
                                        Some(at) => (path[..=at].to_string(), path[at + 1..].to_string()),
                                        None => (String::new(), path.clone()),
                                    };
                                    let selected = Signal::derive(move || highlighted.get() == index);
                                    view! {
                                        <button
                                            type="button"
                                            title=path.clone()
                                            on:mouseenter=move |_| highlighted.set(index)
                                            on:click=move |event: ev::MouseEvent| pick(index, event.ctrl_key())
                                            class=move || {
                                                let base = "flex w-full items-baseline gap-2 px-4 py-1.5 \
                                                            text-left font-mono text-footnote transition-colors";
                                                if selected.get() {
                                                    format!("{base} bg-selection text-rust")
                                                } else {
                                                    format!("{base} text-label-2")
                                                }
                                            }
                                        >
                                            <span class="shrink-0 text-label">{name}</span>
                                            <span class="min-w-0 flex-1 truncate text-label-3">{dir}</span>
                                        </button>
                                    }
                                })
                                .collect_view()
                                .into_any()
                        }}
                    </div>
                    <div class="border-t border-line px-4 py-1.5 text-caption text-label-3">
                        {t!("quick.hint")}
                    </div>
                </div>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_name_match_outranks_a_directory_match() {
        let p = paths(&["src/main/util.rs", "src/main.rs"]);
        assert_eq!(rank("main", &p), vec![1, 0]);
    }

    #[test]
    fn matching_is_case_folded_and_in_order() {
        let p = paths(&["core/src/math/Quaternion.rs", "book/README.md"]);
        assert_eq!(rank("quat", &p), vec![0]);
        assert_eq!(rank("QUAT", &p), vec![0]);
        assert!(
            rank("tauq", &p).is_empty(),
            "letters out of order match nothing"
        );
    }

    #[test]
    fn a_scattered_match_still_finds_the_file_but_ranks_below_a_whole_one() {
        let p = paths(&["src/config_loader.rs", "src/cl.rs"]);
        // "cl" is whole in the second name and scattered in the first.
        assert_eq!(rank("cl", &p), vec![1, 0]);
    }

    #[test]
    fn a_shorter_path_wins_a_tie() {
        let p = paths(&["a/very/long/path/lib.rs", "lib.rs"]);
        assert_eq!(rank("lib", &p), vec![1, 0]);
    }

    #[test]
    fn an_empty_query_lists_everything_in_tree_order() {
        let p = paths(&["b.rs", "a.rs"]);
        assert_eq!(rank("", &p), vec![0, 1]);
    }

    #[test]
    fn files_are_flattened_from_the_tree_in_its_order() {
        let tree = vec![
            Entry {
                name: "src".into(),
                path: "src".into(),
                is_dir: true,
                children: vec![Entry {
                    name: "main.rs".into(),
                    path: "src/main.rs".into(),
                    is_dir: false,
                    children: Vec::new(),
                }],
            },
            Entry {
                name: "Cargo.toml".into(),
                path: "Cargo.toml".into(),
                is_dir: false,
                children: Vec::new(),
            },
        ];
        let mut out = Vec::new();
        files(&tree, &mut out);
        assert_eq!(
            out,
            vec!["src/main.rs".to_string(), "Cargo.toml".to_string()]
        );
    }
}
