//! Answers that change text: completions and the import an accepted one
//! brings, quick fixes and refactorings, renames — and the writing of edits
//! that land in files other than the one being edited, which the frontend
//! holds no buffer for.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Mutex, atomic::Ordering},
    time::Duration,
};

use serde_json::{Value, json};

use super::LspClient;
use crate::{
    convert,
    error::{Error, Result},
    model::{ActionEdit, CodeActionFix, CodeActions, CompletionList},
    uri::{uri_to_absolute, uri_to_relative},
};

/// How many lazily-resolved code actions get a `codeAction/resolve` round
/// trip per request, and how long each may take. Twenty-four sequential
/// round trips at the full request budget is how one slow server turned
/// Ctrl+. into a six-minute wait.
const MAX_RESOLVES: usize = 8;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// How many answers of each kind are kept — completions for resolving an
/// accepted item, code actions for applying one. The popup asks again on
/// every keystroke, so the answer an item was picked from is usually the
/// newest or one behind it; four is margin.
const KEPT_REPLIES: usize = 4;

/// One answer as the server sent it — a completion list's items, or a
/// code action's WorkspaceEdits. Both are looked up the same way, by the
/// number the answer went out under and the index within it.
struct Reply {
    path: String,
    number: u64,
    items: Vec<Value>,
}

/// The latest answers of one kind, raw, each with the path it answered for
/// and its number. Newest last, at most [`KEPT_REPLIES`].
#[derive(Default)]
pub(super) struct Kept(Mutex<VecDeque<Reply>>);

impl Kept {
    /// Keep an answer's items under the number it went out with, and forget
    /// the oldest past the limit.
    fn keep(&self, path: &str, number: u64, items: Vec<Value>) {
        let mut kept = self.0.lock().expect("lsp kept answers");
        kept.push_back(Reply {
            path: path.to_string(),
            number,
            items,
        });
        while kept.len() > KEPT_REPLIES {
            kept.pop_front();
        }
    }

    /// The `index`th item of answer `reply` for `path` — `None` once that
    /// answer is no longer kept, when it was for another file, or when it
    /// never had that many items.
    fn item(&self, path: &str, reply: u64, index: u32) -> Option<Value> {
        let kept = self.0.lock().expect("lsp kept answers");
        kept.iter()
            .find(|answer| answer.number == reply && answer.path == path)
            .and_then(|answer| answer.items.get(index as usize).cloned())
    }
}

impl LspClient {
    /// What could complete at this position. Columns are scalars, as
    /// everywhere on the frontend side.
    pub fn completion(&self, path: &str, line: u32, col: u32) -> Result<CompletionList> {
        let result = self.shared.request(
            "textDocument/completion",
            self.position_params(path, line, col),
        )?;
        let text = self.shared.open_text(path).unwrap_or_default();
        let raw: Vec<Value> = result
            .get("items")
            .and_then(Value::as_array)
            .or_else(|| result.as_array())
            .cloned()
            .unwrap_or_default();
        let number = self.shared.replies.fetch_add(1, Ordering::Relaxed);
        self.shared.completions.keep(path, number, raw);
        let mut list = convert::completion_items(&result, &text, self.shared.encoding());
        list.reply = number;
        Ok(list)
    }

    /// The edits an accepted completion makes besides its insertion — the
    /// `use` line for an item that was not in scope — fetched from the
    /// server for the `index`th item of answer `reply` for `path`.
    ///
    /// Empty when that answer is no longer kept, or was for another file:
    /// the insertion has already happened by then, and an import added for
    /// the wrong item would be worse than none. Items that carried their
    /// edits eagerly are answered without a round trip.
    pub fn resolve_completion(
        &self,
        path: &str,
        reply: u64,
        index: u32,
    ) -> Result<Vec<ActionEdit>> {
        let Some(item) = self.shared.completions.item(path, reply, index) else {
            return Ok(Vec::new());
        };
        let text = self.shared.open_text(path).unwrap_or_default();
        let encoding = self.shared.encoding();
        if item.get("additionalTextEdits").is_some() {
            return Ok(convert::completion_additional_edits(&item, &text, encoding));
        }
        let resolved =
            self.shared
                .request_within("completionItem/resolve", item, RESOLVE_TIMEOUT)?;
        Ok(convert::completion_additional_edits(
            &resolved, &text, encoding,
        ))
    }

    /// The quick fixes and refactorings available at a position, with their
    /// edits for `path` resolved and converted — ready to splice — and the
    /// other files each one changes named, their edits kept here for
    /// [`LspClient::apply_action_elsewhere`].
    ///
    /// Lazily-resolved actions get a `codeAction/resolve` round trip each, up
    /// to a budget. An action that creates, renames or deletes a file, edits
    /// a file outside the project, or only carries a server-side command is
    /// dropped. A resolve that fails is swallowed only while there is
    /// something else to offer — an empty menu with a reason in hand is an
    /// error the caller should hear.
    pub fn code_actions(&self, path: &str, line: u32, col: u32) -> Result<CodeActions> {
        let position = self.protocol_position(path, line, col);
        let result = self.shared.request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": self.shared.uri(path) },
                "range": { "start": position, "end": position },
                // Empty is fine: rust-analyzer matches its own diagnostics by
                // range rather than trusting the client's copy.
                "context": { "diagnostics": [] },
            }),
        )?;

        let ours = self.shared.uri(path);
        let text = self.shared.open_text(path).unwrap_or_default();
        let encoding = self.shared.encoding();
        let mut fixes = Vec::new();
        let mut kept = Vec::new();
        let mut resolves = 0usize;
        let mut failed: Option<Error> = None;
        for offer in result.as_array().into_iter().flatten() {
            let Some(title) = offer["title"].as_str() else {
                continue;
            };
            let kind = offer["kind"].as_str().map(str::to_string);

            let resolved;
            let action = if offer.get("edit").is_some() {
                offer
            } else {
                if resolves >= MAX_RESOLVES {
                    continue;
                }
                resolves += 1;
                match self.shared.request_within(
                    "codeAction/resolve",
                    offer.clone(),
                    RESOLVE_TIMEOUT,
                ) {
                    Ok(full) => {
                        resolved = full;
                        &resolved
                    }
                    Err(error) => {
                        failed.get_or_insert(error);
                        continue;
                    }
                }
            };

            let Some((mine, theirs)) = convert::split_edits(&action["edit"], &ours) else {
                continue;
            };
            let Some(edits) = convert::action_edits(&mine, &text, encoding) else {
                continue;
            };
            // The other files, by the names the tree gives them. One outside
            // the project refuses the action: nothing here writes there.
            let mut elsewhere = Vec::with_capacity(theirs.len());
            for (uri, _) in &theirs {
                match uri_to_relative(uri, &self.shared.root) {
                    Some(relative) => elsewhere.push(relative),
                    None => {
                        elsewhere.clear();
                        break;
                    }
                }
            }
            if elsewhere.len() != theirs.len() || (edits.is_empty() && elsewhere.is_empty()) {
                continue;
            }
            fixes.push(CodeActionFix {
                title: title.to_string(),
                kind,
                edits,
                elsewhere,
            });
            kept.push(action["edit"].clone());
        }
        let number = self.shared.replies.fetch_add(1, Ordering::Relaxed);
        self.shared.actions.keep(path, number, kept);
        match (fixes.is_empty(), failed) {
            (true, Some(error)) => Err(error),
            _ => Ok(CodeActions {
                fixes,
                reply: number,
            }),
        }
    }

    /// The part of an accepted fix that lands in other files, written there:
    /// the `index`th fix of answer `reply` for `path`, less its edits to
    /// `path` itself, which the frontend has already spliced into its own
    /// buffer. Written the way a rename is, and answering with the files
    /// that changed.
    ///
    /// Empty when that answer is no longer kept or was for another file.
    /// The number is what makes that safe: the caret and a hover both ask,
    /// and applying by index against whichever answer happened to be last
    /// would write one position's edits from another position's click.
    pub fn apply_action_elsewhere(
        &self,
        path: &str,
        reply: u64,
        index: u32,
    ) -> Result<Vec<String>> {
        let Some(edit) = self.shared.actions.item(path, reply, index) else {
            return Ok(Vec::new());
        };
        let ours = self.shared.uri(path);
        let Some((_, theirs)) = convert::split_edits(&edit, &ours) else {
            return Ok(Vec::new());
        };
        self.write_edits("textDocument/codeAction", theirs)
    }

    /// Rename the symbol at this position, everywhere, and write the files.
    ///
    /// Applied here rather than returned, because converting the server's
    /// columns needs each file's own text and only a file this client can
    /// read has any. A code action refuses when other files are involved;
    /// a rename must not — a `pub fn` renamed in one file and not its callers
    /// is a broken build, and that is the *normal* case.
    ///
    /// Every file is read and converted before any is written. A file the
    /// server names that cannot be read, or that has changed since the
    /// server read it, refuses the whole rename — not the half of it that
    /// came after. The caller is expected to have saved first: these edits
    /// land on disk, and an unsaved buffer would be overwritten by its own
    /// stale bytes on the next save. Returns the paths that changed, newest
    /// knowledge for whoever has them open.
    pub fn rename(&self, path: &str, line: u32, col: u32, new_name: &str) -> Result<Vec<String>> {
        let mut params = self.position_params(path, line, col);
        params["newName"] = json!(new_name);
        let result = self.shared.request("textDocument/rename", params)?;

        let Some(by_file) = convert::edits_by_file(&result) else {
            return Err(Error::Server {
                method: "textDocument/rename".into(),
                message: "this rename also moves a file, which rusty cannot apply yet — \
                          rename the module in the file tree instead"
                    .into(),
            });
        };
        self.write_edits("textDocument/rename", by_file)
    }

    /// Write a server's edits to the files they name — a rename, or the part
    /// of a quick fix that lands outside the file it was asked in.
    ///
    /// Every file is read and converted before any is written. A file the
    /// server names that cannot be read, or that has changed since the
    /// server read it, refuses the whole set — not the half of it that came
    /// after. Answers with the paths that changed.
    fn write_edits(&self, method: &str, by_file: Vec<(String, Vec<Value>)>) -> Result<Vec<String>> {
        let encoding = self.shared.encoding();
        let mut planned: Vec<(PathBuf, String)> = Vec::new();
        for (uri, edits) in by_file {
            let Some(file) = uri_to_absolute(&uri) else {
                return Err(Error::Server {
                    method: method.into(),
                    message: format!("rust-analyzer named a file this client cannot locate: {uri}"),
                });
            };
            let file = PathBuf::from(file);
            let text = std::fs::read_to_string(&file).map_err(|source| Error::Apply {
                path: file.display().to_string(),
                source,
            })?;
            let Some(out) = convert::apply_text_edits(&text, &edits, encoding) else {
                return Err(Error::Server {
                    method: method.into(),
                    message: format!(
                        "{} has changed since rust-analyzer last read it — save and try again",
                        file.display()
                    ),
                });
            };
            if out != text {
                planned.push((file, out));
            }
        }

        let mut changed = Vec::new();
        for (file, out) in planned {
            std::fs::write(&file, &out).map_err(|source| Error::Apply {
                path: file.display().to_string(),
                source,
            })?;
            changed.push(file.display().to_string());
        }
        changed.sort();
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four answers are kept, newest last, and the fifth forgets the first.
    /// An item is found only under its own answer's number and file, and
    /// only at an index that answer had.
    #[test]
    fn four_answers_are_kept_and_the_fifth_forgets_the_first() {
        let kept = Kept::default();
        for number in 1..=5 {
            kept.keep("a.rs", number, vec![json!(number)]);
        }
        assert_eq!(kept.item("a.rs", 1, 0), None, "the oldest is forgotten");
        assert_eq!(kept.item("a.rs", 2, 0), Some(json!(2)));
        assert_eq!(kept.item("a.rs", 5, 0), Some(json!(5)));
        assert_eq!(kept.item("b.rs", 5, 0), None, "another file's answer");
        assert_eq!(kept.item("a.rs", 5, 1), None, "an index it never had");
    }
}
