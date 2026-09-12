//! The project's files, readable by the model.
//!
//! The analyses answer "why does it not build"; these answer "what does
//! chapter 2 say" and "where is the PID loop". Without them the assistant
//! could name the chip a project targets and not read the README beside it,
//! and said so — "I cannot read files in this repository" — to a user whose
//! question was about a chapter of their own book. A model that cannot read
//! the file it is asked about will answer from memory, which is the failure
//! every other tool here exists to prevent.
//!
//! All three go through `rusty_edit`, so they see the project the way the
//! Files panel does: confined to the root, `.gitignore` honoured, dot
//! entries and `target/` never listed — and never written. What is returned
//! is capped, and says when it was, because a model handed the first half of
//! a file that reads as whole will describe half a file as if it were one.

use serde_json::{Value, json};

use super::{Tool, ToolContext, read_only, required_str};
use crate::{
    error::{Error, Result},
    model::ToolDef,
};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ReadFile),
        Box::new(SearchProject),
        Box::new(ListFiles),
    ]
}

/// Lines a single read hands back at most: a long chapter or a large source
/// file, short of the point where the file crowds out the question. Ask for
/// a window past that.
const MAX_LINES: usize = 400;
/// Hits a search hands back at most.
const MAX_HITS: usize = 200;
/// Entries a listing hands back at most.
const MAX_ENTRIES: usize = 400;

// ─────────────────────────────────────────────────────────────────────────────

struct ReadFile;

impl Tool for ReadFile {
    fn def(&self) -> ToolDef {
        read_only(
            "read_file",
            "Read a text file in the open project by its project-relative \
             path — `README.md`, `book/src/02-feedback-pid.md`, \
             `src/main.rs`. Returns the text with 1-based line numbers, the \
             file's total line count, and whether the read was cut short. \
             \
             Call this before answering anything about a document, a chapter, \
             a source file or a configuration file the user names or refers \
             to, and read the whole thing you are asked about — an answer \
             about a file you have not read is a guess, and the user can tell. \
             A file longer than the cap comes back as its first part with \
             `truncated: true`; ask for the rest with `start_line` and \
             `end_line`. The path must be inside the project; anything else is \
             refused by name.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Project-relative path, `/`-separated"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "First line to return, 1-based (default 1)"
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "Last line to return, 1-based, inclusive (default: as many as the cap allows)"
                    }
                },
                "required": ["path"]
            }),
        )
    }

    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let root = ctx.require_root()?;
        let path = required_str(args, "path", "read_file")?;
        let bytes =
            rusty_edit::read_bytes(root, &path).map_err(|error| Error::BadToolArguments {
                name: "read_file".into(),
                detail: error.to_string(),
            })?;
        let Ok(text) = String::from_utf8(bytes) else {
            return Err(Error::BadToolArguments {
                name: "read_file".into(),
                detail: format!("`{path}` is not a text file"),
            });
        };
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        let start = args
            .get("start_line")
            .and_then(Value::as_u64)
            .map_or(1, |n| n.max(1) as usize);
        let end = args
            .get("end_line")
            .and_then(Value::as_u64)
            .map_or(usize::MAX, |n| n as usize)
            .min(total)
            .min(start.saturating_add(MAX_LINES).saturating_sub(1));
        let window: Vec<String> = if start > total {
            Vec::new()
        } else {
            lines[start - 1..end]
                .iter()
                .enumerate()
                .map(|(offset, line)| format!("{:>5}  {line}", start + offset))
                .collect()
        };
        Ok(json!({
            "path": path,
            "total_lines": total,
            "start_line": start,
            "end_line": end.max(start.saturating_sub(1)),
            "truncated": end < total,
            "text": window.join("\n"),
        }))
    }
}

// ─────────────────────────────────────────────────────────────────────────────

struct SearchProject;

impl Tool for SearchProject {
    fn def(&self) -> ToolDef {
        read_only(
            "search_project",
            "Search the text of every file in the open project — the same \
             search as the Search panel, on ripgrep's engine, honouring \
             .gitignore and skipping build output. Returns matching lines \
             with their file and 1-based line number. \
             \
             Use this to find where something is defined, mentioned or \
             configured before reading the file: a function name, an error \
             string, a chapter's heading, a feature flag. Literal text by \
             default; set `regex` for a pattern. `include` narrows by glob, \
             comma-separated as the Search panel writes it: `*.rs`, \
             `book/**`. Results are capped and say so; narrow the query rather \
             than trusting a capped list as complete.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Text, or a regex when `regex` is true" },
                    "regex": { "type": "boolean", "description": "Treat `query` as a regular expression (default false)" },
                    "case_sensitive": { "type": "boolean", "description": "Match case (default false)" },
                    "include": {
                        "type": "string",
                        "description": "Comma-separated globs to search within, e.g. `*.rs, book/**` (default: everything)"
                    }
                },
                "required": ["query"]
            }),
        )
    }

    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let root = ctx.require_root()?;
        let query = rusty_edit::SearchQuery {
            text: required_str(args, "query", "search_project")?,
            case_sensitive: flag(args, "case_sensitive"),
            whole_word: false,
            regex: flag(args, "regex"),
            include: args
                .get("include")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            exclude: String::new(),
        };
        let results = rusty_edit::search(root, &query);
        if let Some(error) = results.error {
            return Err(Error::BadToolArguments {
                name: "search_project".into(),
                detail: error,
            });
        }
        let capped = results.hits.len() > MAX_HITS;
        let hits: Vec<Value> = results
            .hits
            .iter()
            .take(MAX_HITS)
            .map(|hit| {
                json!({
                    "path": hit.path,
                    "line": hit.line + 1,
                    "text": hit.text.trim_end(),
                })
            })
            .collect();
        Ok(json!({
            "hits": hits,
            "files": results.files,
            "truncated": results.truncated || capped,
        }))
    }
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

// ─────────────────────────────────────────────────────────────────────────────

struct ListFiles;

impl Tool for ListFiles {
    fn def(&self) -> ToolDef {
        read_only(
            "list_files",
            "The files in the open project as project-relative paths, the \
             way the Files panel shows them: .gitignore honoured, build \
             output and dot entries left out. Optionally only those under \
             `dir`. \
             \
             Use this to find out what a project contains before guessing at \
             a path — where the book's chapters are, whether there is a \
             firmware crate beside the library, what a directory is called. \
             The list is capped and says so; ask for a directory when the \
             whole project is too large.",
            json!({
                "type": "object",
                "properties": {
                    "dir": {
                        "type": "string",
                        "description": "A project-relative directory to list within (default: the whole project)"
                    }
                },
                "required": []
            }),
        )
    }

    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let root = ctx.require_root()?;
        let tree = rusty_edit::read_tree(root).map_err(|error| Error::BadToolArguments {
            name: "list_files".into(),
            detail: error.to_string(),
        })?;
        let prefix = args
            .get("dir")
            .and_then(Value::as_str)
            .map(|dir| {
                dir.replace('\\', "/")
                    .trim_start_matches("./")
                    .trim_end_matches('/')
                    .to_string()
            })
            .filter(|dir| !dir.is_empty());
        let mut paths = Vec::new();
        flatten(&tree, &mut paths);
        let matching: Vec<&String> = paths
            .iter()
            .filter(|path| {
                prefix
                    .as_ref()
                    .is_none_or(|dir| path.starts_with(&format!("{dir}/")))
            })
            .collect();
        let total = matching.len();
        Ok(json!({
            "files": matching.iter().take(MAX_ENTRIES).collect::<Vec<_>>(),
            "total": total,
            "truncated": total > MAX_ENTRIES,
        }))
    }
}

/// Every file under `entries`, depth first, as the tree lists them.
fn flatten(entries: &[rusty_edit::Entry], out: &mut Vec<String>) {
    for entry in entries {
        if entry.is_dir {
            flatten(&entry.children, out);
        } else {
            out.push(entry.path.clone());
        }
    }
}
