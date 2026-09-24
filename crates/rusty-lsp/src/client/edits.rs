//! Answers that change text: completions and the import an accepted one
//! brings, quick fixes and refactorings, renames — and the writing of edits
//! that land in files other than the one being edited, which the frontend
//! holds no buffer for.

use std::{
    collections::VecDeque,
    io::Write,
    path::{Path, PathBuf},
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
    /// All of it or none of it, as far as a filesystem allows. Every file is
    /// read and converted before any is written, so a file the server names
    /// that cannot be read, or that has changed since the server read it,
    /// refuses the whole set — not the half of it that came after. A write
    /// that fails once others have succeeded puts those back as they were;
    /// only when that fails as well is anything left changed, and the error
    /// then names it. Answers with the paths that changed.
    fn write_edits(&self, method: &str, by_file: Vec<(String, Vec<Value>)>) -> Result<Vec<String>> {
        let encoding = self.shared.encoding();
        let mut planned: Vec<Planned> = Vec::new();
        for (uri, edits) in by_file {
            let Some(file) = uri_to_absolute(&uri) else {
                return Err(Error::Server {
                    method: method.into(),
                    message: format!("rust-analyzer named a file this client cannot locate: {uri}"),
                });
            };
            let file = PathBuf::from(file);
            let before = std::fs::read_to_string(&file).map_err(|source| Error::Apply {
                path: file.display().to_string(),
                source,
                undone: Vec::new(),
            })?;
            let Some(after) = convert::apply_text_edits(&before, &edits, encoding) else {
                return Err(Error::Server {
                    method: method.into(),
                    message: format!(
                        "{} has changed since rust-analyzer last read it — save and try again",
                        file.display()
                    ),
                });
            };
            if after != before {
                planned.push(Planned {
                    file,
                    before,
                    after,
                });
            }
        }
        write_all(&planned, write_file)
    }
}

/// One file's part of an edit: what it holds, and what it is to hold.
struct Planned {
    file: PathBuf,
    before: String,
    after: String,
}

/// A write that failed, and whether it got far enough to change the file:
/// opening one for writing empties it, so a failure after the open leaves
/// it holding part of what was being written, and one at the open leaves
/// it as it was.
struct WriteFailure {
    error: std::io::Error,
    touched: bool,
}

/// Put `text` in `file`, in place. Writing a copy and renaming it over the
/// file would make a failed write harmless, but it would be another file:
/// default permissions, no link to the old one, and a rename to every
/// watcher where this is a change to a file that was already there.
fn write_file(file: &Path, text: &str) -> std::result::Result<(), WriteFailure> {
    let mut out = std::fs::File::create(file).map_err(|error| WriteFailure {
        error,
        touched: false,
    })?;
    out.write_all(text.as_bytes())
        .map_err(|error| WriteFailure {
            error,
            touched: true,
        })
}

/// Write every planned file through `write`, in order, or leave them all as
/// they were: when one fails, the files already written — and the failing
/// one, if the failure came after it was emptied — get their old text back
/// through the same `write`. Answers with the paths written, sorted.
fn write_all(
    planned: &[Planned],
    mut write: impl FnMut(&Path, &str) -> std::result::Result<(), WriteFailure>,
) -> Result<Vec<String>> {
    for (at, failing) in planned.iter().enumerate() {
        let Err(failure) = write(&failing.file, &failing.after) else {
            continue;
        };
        let emptied = failure.touched.then_some(failing);
        let mut undone = Vec::new();
        let mut left = Vec::new();
        for plan in planned[..at].iter().chain(emptied) {
            let name = plan.file.display().to_string();
            match write(&plan.file, &plan.before) {
                Ok(()) => undone.push(name),
                Err(_) => left.push(name),
            }
        }
        let path = failing.file.display().to_string();
        let source = failure.error;
        return Err(if left.is_empty() {
            Error::Apply {
                path,
                source,
                undone,
            }
        } else {
            Error::PartlyApplied { path, source, left }
        });
    }
    let mut changed: Vec<String> = planned
        .iter()
        .map(|plan| plan.file.display().to_string())
        .collect();
    changed.sort();
    Ok(changed)
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

    /// Three files on disk, each planned to gain a `pub`.
    fn three_files(dir: &Path) -> Vec<Planned> {
        ["a", "b", "c"]
            .into_iter()
            .map(|name| {
                let file = dir.join(format!("{name}.rs"));
                let before = format!("fn {name}() {{}}\n");
                std::fs::write(&file, &before).unwrap();
                let after = format!("pub {before}");
                Planned {
                    file,
                    before,
                    after,
                }
            })
            .collect()
    }

    fn on_disk(plan: &Planned) -> String {
        std::fs::read_to_string(&plan.file).unwrap()
    }

    fn name(plan: &Planned) -> String {
        plan.file.display().to_string()
    }

    /// When putting back fails as well, the edit is half on disk, and the
    /// error names exactly the files that are: the one written and not put
    /// back — not the one whose write was refused, nor the one never
    /// reached.
    #[test]
    fn a_file_that_cannot_be_put_back_is_named_as_left_changed() {
        let dir = tempfile::tempdir().unwrap();
        let planned = three_files(dir.path());
        let outcome = write_all(&planned, |file, text| {
            // b's write is refused at the open, and so is a's way back.
            if file == planned[1].file || (file == planned[0].file && text == planned[0].before) {
                return Err(WriteFailure {
                    error: std::io::ErrorKind::PermissionDenied.into(),
                    touched: false,
                });
            }
            write_file(file, text)
        });
        let error = outcome.unwrap_err();
        match &error {
            Error::PartlyApplied { path, left, .. } => {
                assert_eq!(*path, name(&planned[1]));
                assert_eq!(*left, [name(&planned[0])]);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            error
                .to_string()
                .ends_with(&format!("left changed: {}", name(&planned[0]))),
            "{error}"
        );
        assert_eq!(on_disk(&planned[0]), planned[0].after, "a keeps the edit");
        assert_eq!(on_disk(&planned[1]), planned[1].before);
        assert_eq!(
            on_disk(&planned[2]),
            planned[2].before,
            "c was never reached"
        );
    }

    /// A write can fail after the open has emptied its file — a full disk
    /// — and then that file is as changed as the ones before it, and is put
    /// back with them.
    #[test]
    fn a_write_that_fails_after_emptying_its_file_puts_that_file_back_too() {
        let dir = tempfile::tempdir().unwrap();
        let planned = three_files(dir.path());
        let outcome = write_all(&planned, |file, text| {
            if file == planned[1].file && text == planned[1].after {
                // Half the new text lands, and then the disk is full.
                std::fs::write(file, &text[..text.len() / 2]).unwrap();
                return Err(WriteFailure {
                    error: std::io::ErrorKind::StorageFull.into(),
                    touched: true,
                });
            }
            write_file(file, text)
        });
        match outcome {
            Err(Error::Apply { path, undone, .. }) => {
                assert_eq!(path, name(&planned[1]));
                assert_eq!(undone, [name(&planned[0]), name(&planned[1])]);
            }
            other => panic!("{other:?}"),
        }
        for plan in &planned {
            assert_eq!(on_disk(plan), plan.before, "{}", name(plan));
        }
    }
}
