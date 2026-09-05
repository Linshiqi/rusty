//! What the panel draws. Wire types, `wasm32`-clean.

use serde::{Deserialize, Serialize};

/// How many commits the log is cut at. Enough for the shape of a project;
/// few enough that the request is a fraction of a second on a large one.
/// Here rather than beside the `git` call because the panel names the number
/// in its "showing the newest…" line, and the panel compiles without `git`.
pub const LIMIT: usize = 400;

/// What a decoration on a commit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefKind {
    /// Where the working tree is.
    Head,
    /// A local branch.
    Branch,
    /// A remote-tracking branch, `origin/main`.
    Remote,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefLabel {
    pub kind: RefKind,
    pub name: String,
}

/// One commit, as the log lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    /// The full hash — the identity everything else keys on.
    pub id: String,
    /// The seven-character form people read and type.
    pub short: String,
    /// Full hashes, first parent first. Empty for a root.
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Author time, seconds since the epoch.
    pub time: u64,
    /// The first line of the message.
    pub summary: String,
    pub refs: Vec<RefLabel>,
}

/// A line drawn from this row's centre to the next row's: `from` is a lane at
/// this row, `to` a lane at the row below. A lane that continues is
/// `from == to`; a branch leaving a merge commit is `from == commit lane`;
/// a branch arriving at the commit below is `to == its lane`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub from: u32,
    pub to: u32,
}

/// One row of the graph: the commit, which lane its dot sits in, and the
/// lines running down out of this row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRow {
    pub commit: Commit,
    pub lane: u32,
    pub edges: Vec<Edge>,
}

/// The log, laid out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub rows: Vec<GraphRow>,
    /// How many lanes the widest row needs — the graph column's width.
    pub lanes: u32,
    /// True when the log was cut at the limit and older commits exist.
    pub truncated: bool,
    /// What `HEAD` names: a branch, or a detached hash.
    pub head: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// A mode change, a copy, a type change — real, rare, and not worth a
    /// glyph each.
    Other,
}

/// One file a commit touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Repository-relative, `/`-separated. For a rename, the new name.
    pub path: String,
    pub kind: ChangeKind,
    /// Lines added and removed. `None` for a binary file, which has neither.
    pub added: Option<u32>,
    pub removed: Option<u32>,
    /// This file's part of the commit's patch, `diff --git` header included.
    pub patch: String,
}

/// A commit opened: the log's row plus everything the log leaves out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub commit: Commit,
    /// The whole message, summary line included.
    pub body: String,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    /// `main`, or `origin/main` for a remote-tracking one.
    pub name: String,
    /// Checked out.
    pub current: bool,
    pub remote: bool,
    /// The branch this one tracks, when it does.
    pub upstream: Option<String>,
    /// The short hash of its tip.
    pub tip: String,
}

/// One path the working tree or the index differs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    /// Repository-relative, `/`-separated. For a rename, the new name.
    pub path: String,
    /// How the index differs from HEAD — what the next commit would carry.
    pub staged: Option<ChangeKind>,
    /// How the working tree differs from the index.
    pub unstaged: Option<ChangeKind>,
    /// Not in the index at all.
    pub untracked: bool,
    /// A merge left it with conflict markers.
    pub conflicted: bool,
}

/// Where the working tree stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The branch checked out, or `None` when HEAD is detached.
    pub head: Option<String>,
    pub detached: bool,
    /// The branch this one tracks, when it does.
    pub upstream: Option<String>,
    /// Commits here the upstream does not have, and the reverse.
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<StatusEntry>,
}

/// One stash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stash {
    /// Its position, newest first — what `stash@{n}` counts.
    pub index: u32,
    /// `stash@{0}`, as git names it.
    pub label: String,
    /// The note it was saved with, or git's own `WIP on main: …`.
    pub message: String,
    pub time: u64,
}

/// Whether a path names an image the panel shows as pictures — old beside
/// new — rather than as a patch git can only call binary.
pub fn is_image_path(path: &str) -> bool {
    image_mime(path).is_some()
}

/// The MIME type an image path's extension implies, for a `data:` URL; `None`
/// for anything that is not an image this panel draws. SVG is text to git and
/// a picture to a person, so it is here.
pub fn image_mime(path: &str) -> Option<&'static str> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        _ => return None,
    })
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn images_are_told_by_extension_case_blind_and_nothing_else_is_one() {
        assert_eq!(
            image_mime("book/src/figures/fig-23-osd.svg"),
            Some("image/svg+xml")
        );
        assert_eq!(image_mime("logo.PNG"), Some("image/png"));
        assert_eq!(image_mime("a/b/photo.JPEG"), Some("image/jpeg"));
        assert!(is_image_path("icon.ico"));
        assert!(!is_image_path("src/main.rs"));
        assert!(!is_image_path("Makefile"));
        assert!(!is_image_path("images/README"));
        assert!(
            !is_image_path("dir.png/notes.txt"),
            "the extension is the file's, not a folder's"
        );
    }
}
