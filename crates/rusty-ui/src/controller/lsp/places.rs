//! Where things are: references, implementations, symbols, a macro's
//! expansion, a definition — and the one jump every answer goes through.

use super::*;

/// Which places a command asks the server for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceQuery {
    References,
    Implementations,
    TypeDefinition,
}

/// Where the thing at the caret of this group's editor is used, implemented
/// or typed.
///
/// One implementation or type definition is a jump, as a definition is; more
/// than one, or any references at all, is a list in the finder, titled with
/// the name asked about — even an empty one, which says there were none
/// rather than letting a key seem to do nothing.
pub fn find_places(state: AppState, query: PlaceQuery) {
    let Some(path) = state.active_path_now() else {
        return;
    };
    if !path.ends_with(".rs") {
        return;
    }
    let Some((line, col)) = caret_position(state) else {
        return;
    };
    let name = state
        .editor
        .draft
        .with_untracked(|text| word_around(text, line, col));
    let command = match query {
        PlaceQuery::References => cmd::lsp::REFERENCES,
        PlaceQuery::Implementations => cmd::lsp::IMPLEMENTATIONS,
        PlaceQuery::TypeDefinition => cmd::lsp::TYPE_DEFINITION,
    };
    let args = PathAt { path, line, col };
    spawn_local(async move {
        // Errors are the server warming up, as they are for a definition.
        let Ok(mut places) = ipc::call::<_, Vec<rusty_lsp::Place>>(command, &args).await else {
            return;
        };
        // By file, then down each file: the order a list is read in. The
        // server's own puts the declaration wherever its index found it.
        places.sort_by(|a, b| {
            let (a, b) = (&a.location, &b.location);
            (a.external, &a.path, a.line, a.col).cmp(&(b.external, &b.path, b.line, b.col))
        });
        if places.len() == 1 && query != PlaceQuery::References {
            go_to(state, places[0].location.clone());
            return;
        }
        let title = match query {
            PlaceQuery::References => t!("places.references", name = name.clone()),
            PlaceQuery::Implementations => t!("places.implementations", name = name.clone()),
            PlaceQuery::TypeDefinition => t!("places.type-definition", name = name.clone()),
        };
        state
            .layout
            .quick_places
            .set(Some(crate::state::PlaceList { title, places }));
        state.layout.quick_seed.set(String::new());
        state.layout.quick_open.set(true);
    });
}

/// The identifier a position is on or just after — what a list of its uses
/// is titled with.
pub(in crate::controller) fn word_around(text: &str, line: u32, col: u32) -> String {
    let chars: Vec<char> = text
        .split('\n')
        .nth(line as usize)
        .unwrap_or_default()
        .chars()
        .collect();
    let is_word = |c: &&char| c.is_alphanumeric() || **c == '_';
    let at = (col as usize).min(chars.len());
    let before = chars[..at].iter().rev().take_while(is_word).count();
    let after = chars[at..].iter().take_while(is_word).count();
    chars[at - before..at + after].iter().collect()
}

/// Expand the macro call under this group's caret all the way down, and open
/// the expansion beside the code, read-only — VS Code's "Expand macro
/// recursively", which opens it beside as well. Nothing to expand says so in
/// the dock, as a rename that found nothing does, rather than letting the
/// command seem to do nothing.
pub fn expand_macro(state: AppState) {
    let Some(path) = state.active_path_now().filter(|path| path.ends_with(".rs")) else {
        return;
    };
    if state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let Some((line, col)) = caret_position(state) else {
        return;
    };
    let args = PathAt { path, line, col };
    spawn_local(async move {
        match ipc::call::<_, Option<rusty_edit::Document>>(cmd::lsp::EXPAND_MACRO, &args).await {
            Ok(Some(document)) => {
                // Beside is the right group, from either side.
                let beside = state.group(crate::state::Group::Second);
                state.layout.split.set(true);
                state.layout.focus.set(beside.group);
                let panel = state.layout.panel.get_untracked();
                if panel != "files" && panel != "search" {
                    state.layout.panel.set("files".to_string());
                }
                show_document(beside, document, false);
            }
            Ok(None) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: t!("misc.no-macro"),
                level: Some(LogLevel::Warn),
            }),
            // The server warming up, as for a definition: not worth a banner.
            Err(_) => {}
        }
    });
}

/// Ask for the symbols the finder lists: the outline of this group's file
/// for `@`, the workspace's matching `words` for `#`. The answer is kept with
/// its ask, so a slow reply to `#gp` is not shown under `#gpio`.
pub fn ask_symbols(state: AppState, workspace: bool, words: String) {
    #[derive(serde::Serialize)]
    struct File {
        path: String,
    }
    #[derive(serde::Serialize)]
    struct Workspace {
        query: String,
    }

    if state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    spawn_local(async move {
        let (ask, answer) = if workspace {
            let ask = format!("#{words}");
            let answer = ipc::call::<_, Vec<rusty_lsp::Symbol>>(
                cmd::lsp::WORKSPACE_SYMBOLS,
                &Workspace { query: words },
            )
            .await;
            (ask, answer)
        } else {
            let Some(path) = state.active_path_now().filter(|path| path.ends_with(".rs")) else {
                return;
            };
            let ask = format!("@{path}");
            let answer =
                ipc::call::<_, Vec<rusty_lsp::Symbol>>(cmd::lsp::DOCUMENT_SYMBOLS, &File { path })
                    .await;
            (ask, answer)
        };
        if let Ok(symbols) = answer {
            state
                .layout
                .quick_symbols
                .set(Some(crate::state::SymbolAnswer { ask, symbols }));
        }
    });
}

/// Open where a location is, with the caret on it, remembering where the
/// caret was for Back. Outside the project, the file opens read-only.
pub fn go_to(state: AppState, location: rusty_lsp::Location) {
    // Files and Search keep an editor on screen; from anywhere else, the
    // jump lands in Files — a finder row picked over the Git panel.
    let panel = state.layout.panel.get_untracked();
    if panel != "files" && panel != "search" {
        state.layout.panel.set("files".to_string());
    }
    let current = state.active_path_now();
    if current.as_deref() != Some(location.path.as_str()) {
        if location.external {
            open_external(state, location.path.clone());
        } else {
            open_file(state, location.path.clone());
        }
    }
    remember_jump(state, &location);
    state.editor.reveal.set(Some(location));
}

/// Jump to wherever the thing at this position is defined.
///
/// The target lands in `state.editor.reveal`; if it is in another file, that file is
/// opened first and the editor applies the reveal once the document arrives.
pub fn goto_definition(state: AppState, path: String, line: u32, col: u32) {
    let args = PathAt { path, line, col };
    spawn_local(async move {
        // "No definition" is a normal answer over whitespace or a keyword, and
        // an error here is the server warming up. Neither is worth a banner.
        if let Ok(Some(location)) =
            ipc::call::<_, Option<rusty_lsp::Location>>(cmd::lsp::DEFINITION, &args).await
        {
            go_to(state, location);
        }
    });
}

/// Whether a place has a definition to go to — asked while Ctrl is held over
/// a name, so the name reads as a link before it is clicked, as VS Code's
/// does. `then` hears the answer; a server still warming up is a no.
pub fn has_definition(path: String, line: u32, col: u32, then: impl FnOnce(bool) + 'static) {
    let args = PathAt { path, line, col };
    spawn_local(async move {
        let found = matches!(
            ipc::call::<_, Option<rusty_lsp::Location>>(cmd::lsp::DEFINITION, &args).await,
            Ok(Some(_))
        );
        then(found);
    });
}
