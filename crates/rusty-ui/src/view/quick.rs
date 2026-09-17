//! The file finder — the title bar's search icon, and Ctrl+P.
//!
//! VS Code's "Go to File": type part of a name, the list narrows, Enter
//! opens the pick in the group the user is in and Ctrl+Enter opens it
//! beside. The candidates are the file tree the Files panel already holds,
//! flattened — the tree is read whole with `target/` and dot-entries already
//! excluded, so there is no second walk to keep in step with the first.
//!
//! And VS Code's prefixes: `@` lists the symbols of the file in front and
//! `#` the workspace's, both asked of rust-analyzer, which is what Ctrl+Shift+O
//! and Ctrl+T open it with. A command that finds several places — a symbol's
//! references, its implementations — lists them here too, under a heading
//! naming what they are: one list to learn, with one set of keys.

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

/// What the finder is listing: files; places a command asked the language
/// server for; the symbols of the file in front, after `@`; the workspace's,
/// after `#`; or a line of the file in front, after `:`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listing {
    Files,
    Places,
    FileSymbols,
    WorkspaceSymbols,
    Line,
}

/// The line, and the column after a second colon, that `42` or `42:7`
/// names — both counted from 1, as people number lines. Anything else names
/// nothing.
fn line_target(words: &str) -> Option<(u32, u32)> {
    let mut parts = words.trim().splitn(2, ':');
    let line = parts.next()?.trim().parse::<u32>().ok()?;
    let col = match parts.next().map(str::trim) {
        None | Some("") => 1,
        Some(col) => col.parse::<u32>().ok()?,
    };
    Some((line, col))
}

/// One row, whatever the listing.
#[derive(Clone)]
struct Row {
    /// The strong text: a file's name, a symbol, a place's file and line.
    name: String,
    /// The quiet text after it: the folder, what a symbol sits in, the line.
    detail: String,
    /// What kind of symbol, in a column of its own before the name.
    kind: Option<String>,
    /// How deep in a file's outline, drawn as indentation.
    depth: u32,
    title: String,
    target: Target,
}

#[derive(Clone)]
enum Target {
    File(String),
    Place(rusty_lsp::Location),
}

fn file_row(path: &str) -> Row {
    let (dir, name) = match path.rfind('/') {
        Some(at) => (path[..=at].to_string(), path[at + 1..].to_string()),
        None => (String::new(), path.to_string()),
    };
    Row {
        name,
        detail: dir,
        kind: None,
        depth: 0,
        title: path.to_string(),
        target: Target::File(path.to_string()),
    }
}

/// A place as a row: the file and line in front, the code on the line after
/// it — the code is what tells a hundred rows in one file apart.
fn place_row(place: &rusty_lsp::Place) -> Row {
    let location = &place.location;
    let file = location.path.rsplit('/').next().unwrap_or(&location.path);
    Row {
        name: format!("{file}:{}", location.line + 1),
        detail: place.text.trim().to_string(),
        kind: None,
        depth: 0,
        title: format!("{}:{}", location.path, location.line + 1),
        target: Target::Place(location.clone()),
    }
}

fn symbol_row(symbol: &rusty_lsp::Symbol, workspace: bool) -> Row {
    let location = &symbol.location;
    let detail = match (&symbol.container, workspace) {
        (Some(container), true) => format!("{container} · {}", location.path),
        (None, true) => location.path.clone(),
        (container, false) => container.clone().unwrap_or_default(),
    };
    Row {
        name: symbol.name.clone(),
        detail,
        kind: Some(symbol.kind.clone()),
        depth: if workspace { 0 } else { symbol.depth },
        title: format!("{}:{}", location.path, location.line + 1),
        target: Target::Place(location.clone()),
    }
}

/// `rows` in `rank`'s order for `words`, or as they came when nothing is
/// typed: an outline in document order and references grouped by file mean
/// more than the same rows sorted by how long they are.
fn narrowed(words: &str, rows: Vec<Row>, key: impl Fn(&Row) -> String) -> Vec<Row> {
    if words.trim().is_empty() {
        return rows.into_iter().take(SHOWN_OUTLINE).collect();
    }
    let keys: Vec<String> = rows.iter().map(key).collect();
    rank(words, &keys)
        .into_iter()
        .take(SHOWN)
        .map(|index| rows[index].clone())
        .collect()
}

/// How many rows an outline or a list of places shows unfiltered. More than
/// a file finder's: scrolling a file's outline is the use.
const SHOWN_OUTLINE: usize = 500;

#[component]
pub fn QuickOpen() -> impl IntoView {
    let state = AppState::expect();
    let open = state.layout.quick_open;
    let query = RwSignal::new(String::new());
    let highlighted = RwSignal::new(0usize);
    let input: NodeRef<html::Input> = NodeRef::new();

    // Reset and focus each time it opens, as the palette does: a finder that
    // reopens showing the last search is one that needs clearing every time.
    // It opens with whatever the command that opened it typed — `@` for the
    // file's symbols, `#` for the workspace's. Closing forgets a list of
    // places, which belonged to the command that asked for it.
    Effect::new(move |_| {
        if open.get() {
            query.set(state.layout.quick_seed.get_untracked());
            highlighted.set(0);
            if let Some(element) = input.get() {
                let _ = element.focus();
            }
        } else {
            state.layout.quick_places.set(None);
        }
    });

    let listing = Memo::new(move |_| {
        if state.layout.quick_places.with(Option::is_some) {
            return Listing::Places;
        }
        query.with(|typed| match typed.chars().next() {
            Some('@') => Listing::FileSymbols,
            Some('#') => Listing::WorkspaceSymbols,
            Some(':') => Listing::Line,
            _ => Listing::Files,
        })
    });
    // What was typed after the prefix.
    let words = move || query.with(|typed| typed.get(1..).unwrap_or_default().trim().to_string());
    // The symbols ask `@` is answered under, for the file in front.
    let file_ask = move || {
        state
            .focused()
            .active_path_now()
            .filter(|path| path.ends_with(".rs"))
            .map(|path| format!("@{path}"))
    };

    // Ask for symbols: once per file for `@`, and a beat after typing stops
    // for `#`, whose answer depends on the words.
    let symbol_wait = StoredValue::new(0u64);
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        match listing.get() {
            Listing::FileSymbols => {
                let wanted = file_ask();
                let have = state
                    .layout
                    .quick_symbols
                    .with_untracked(|answer| answer.as_ref().map(|answer| answer.ask.clone()));
                if wanted.is_some() && wanted != have {
                    controller::ask_symbols(state.focused(), false, String::new());
                }
            }
            Listing::WorkspaceSymbols => {
                query.track();
                let typed = words();
                if typed.is_empty() {
                    return;
                }
                let turn = symbol_wait.get_value() + 1;
                symbol_wait.set_value(turn);
                set_timeout(
                    move || {
                        if symbol_wait.try_get_value() == Some(turn) {
                            controller::ask_symbols(state, true, typed);
                        }
                    },
                    std::time::Duration::from_millis(150),
                );
            }
            Listing::Files | Listing::Places | Listing::Line => {}
        }
    });

    let all = Memo::new(move |_| {
        let mut out = Vec::new();
        state.editor.tree.with(|tree| files(tree, &mut out));
        out
    });
    let rows = Signal::derive(move || -> Vec<Row> {
        match listing.get() {
            Listing::Files => {
                let paths = all.get();
                rank(&query.get(), &paths)
                    .into_iter()
                    .take(SHOWN)
                    .map(|index| file_row(&paths[index]))
                    .collect()
            }
            Listing::Places => {
                let rows = state.layout.quick_places.with(|list| {
                    list.as_ref()
                        .map(|list| list.places.iter().map(place_row).collect())
                        .unwrap_or_default()
                });
                narrowed(&query.get(), rows, |row| {
                    format!("{} {}", row.title, row.detail)
                })
            }
            Listing::FileSymbols => {
                let wanted = file_ask();
                let rows = state.layout.quick_symbols.with(|answer| {
                    answer
                        .as_ref()
                        .filter(|answer| Some(&answer.ask) == wanted.as_ref())
                        .map(|answer| {
                            answer
                                .symbols
                                .iter()
                                .map(|symbol| symbol_row(symbol, false))
                                .collect()
                        })
                        .unwrap_or_default()
                });
                narrowed(&words(), rows, |row| row.name.clone())
            }
            Listing::WorkspaceSymbols => {
                let typed = words();
                // The latest answer while it is still about what is being
                // typed — asked for `gp`, shown and narrowed under `gpio`
                // until `gpio`'s own answer arrives.
                let rows = state.layout.quick_symbols.with(|answer| {
                    answer
                        .as_ref()
                        .filter(|answer| {
                            answer
                                .ask
                                .strip_prefix('#')
                                .is_some_and(|asked| !asked.is_empty() && typed.starts_with(asked))
                        })
                        .map(|answer| {
                            answer
                                .symbols
                                .iter()
                                .map(|symbol| symbol_row(symbol, true))
                                .collect()
                        })
                        .unwrap_or_default()
                });
                if typed.is_empty() {
                    Vec::new()
                } else {
                    narrowed(&typed, rows, |row| row.name.clone())
                }
            }
            Listing::Line => {
                let target = state.focused();
                let (Some(path), Some((line, col))) =
                    (target.active_path_now(), line_target(&words()))
                else {
                    return Vec::new();
                };
                // Past the end is the last line, as every editor's Go to Line
                // clamps: a number one too large is not worth a refusal.
                let (line, text) = target.editor.draft.with(|draft| {
                    let count = draft.split('\n').count() as u32;
                    let line = line.clamp(1, count.max(1));
                    let text = draft
                        .split('\n')
                        .nth((line - 1) as usize)
                        .unwrap_or_default();
                    (line, text.trim().to_string())
                });
                vec![Row {
                    name: t!("quick.line", line = line),
                    detail: text,
                    kind: None,
                    depth: 0,
                    title: format!("{path}:{line}"),
                    // The file in front, so nothing is opened and whether it
                    // is a library's does not come into it.
                    target: Target::Place(rusty_lsp::Location {
                        path,
                        line: line - 1,
                        col: col - 1,
                        external: false,
                    }),
                }]
            }
        }
    });
    // What an empty list means, which depends on what it is a list of.
    let empty = move || match listing.get() {
        Listing::Line if state.focused().active_path_now().is_none() => t!("quick.no-document"),
        Listing::Line => t!("quick.line-hint"),
        Listing::Files => t!("quick.no-match"),
        Listing::Places => t!("quick.no-place"),
        Listing::FileSymbols => match file_ask() {
            None => t!("quick.no-file"),
            Some(ask) => {
                let answered = state
                    .layout
                    .quick_symbols
                    .with(|answer| answer.as_ref().is_some_and(|answer| answer.ask == ask));
                if answered {
                    t!("quick.no-symbol")
                } else {
                    t!("quick.asking")
                }
            }
        },
        Listing::WorkspaceSymbols => {
            let typed = words();
            if typed.is_empty() {
                t!("quick.type-to-search")
            } else if state.layout.quick_symbols.with(|answer| {
                answer
                    .as_ref()
                    .is_some_and(|answer| answer.ask == format!("#{typed}"))
            }) {
                t!("quick.no-symbol")
            } else {
                t!("quick.asking")
            }
        }
    };

    let pick = move |index: usize, beside: bool| {
        let Some(row) = rows.get_untracked().get(index).cloned() else {
            return;
        };
        open.set(false);
        let target = state.focused();
        match row.target {
            Target::File(path) if beside => controller::open_beside(target, path),
            Target::File(path) => controller::open_file(target, path),
            Target::Place(location) => controller::go_to(target, location),
        }
    };

    view! {
        <Show when=move || open.get()>
            // Dropped from the top, as VS Code's quick open is, rather than
            // centred like the palette.
            <div
                class="absolute inset-0 z-30 flex justify-center bg-black/25 pt-2"
                on:click=move |_| open.set(false)
            >
                <div
                    class="flex max-h-[60vh] w-[640px] flex-col overflow-hidden rounded-[12px] bg-raised shadow-2xl ring-1 ring-line-strong"
                    on:click=move |event| event.stop_propagation()
                >
                    <input
                        node_ref=input
                        class="h-12 flex-none border-b border-line bg-transparent px-4 text-strong text-label outline-none placeholder:text-label-3"
                        placeholder=t!("quick.placeholder")
                        prop:value=move || query.get()
                        on:input=move |event| {
                            query.set(event_target_value(&event));
                            highlighted.set(0);
                        }
                        on:keydown=move |event: ev::KeyboardEvent| {
                            let count = rows.get_untracked().len();
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
                    // A list of places says what it lists, and how many.
                    {move || {
                        state
                            .layout
                            .quick_places
                            .with(|list| {
                                list.as_ref()
                                    .map(|list| {
                                        let heading = format!("{} · {}", list.title, list.places.len());
                                        view! {
                                            <div class="flex-none border-b border-line px-4 py-1.5 text-footnote text-label-2">
                                                {heading}
                                            </div>
                                        }
                                    })
                            })
                    }}
                    <div class="min-h-0 flex-1 overflow-y-auto py-1.5">
                        {move || {
                            let rows = rows.get();
                            if rows.is_empty() {
                                return view! {
                                    <p class="px-4 py-3 text-callout text-label-2">{empty()}</p>
                                }
                                    .into_any();
                            }
                            rows
                                .into_iter()
                                .enumerate()
                                .map(|(index, row)| {
                                    let selected = Signal::derive(move || highlighted.get() == index);
                                    let indent = format!("padding-left: {}ch", row.depth * 2);
                                    view! {
                                        <button
                                            type="button"
                                            title=row.title
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
                                            {row
                                                .kind
                                                .map(|kind| {
                                                    view! {
                                                        <span class="w-[9ch] shrink-0 truncate text-label-3">{kind}</span>
                                                    }
                                                })}
                                            <span class="shrink-0 text-label" style=indent>
                                                {row.name}
                                            </span>
                                            <span class="min-w-0 flex-1 truncate text-label-3">{row.detail}</span>
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
    fn a_line_is_named_by_its_number_and_a_column_after_a_colon() {
        assert_eq!(line_target("42"), Some((42, 1)));
        assert_eq!(line_target(" 42:7 "), Some((42, 7)));
        assert_eq!(line_target("42:"), Some((42, 1)));
        assert_eq!(line_target(""), None);
        assert_eq!(line_target("forty"), None);
        assert_eq!(line_target("42:x"), None);
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
