//! What the panel draws. Wire types, `wasm32`-clean.

use serde::{Deserialize, Serialize};

/// How many commits the log is cut at. Enough for the shape of a project;
/// few enough that the request is a fraction of a second on a large one.
/// Here rather than beside the `git` call because the panel names the number
/// in its "showing the newest…" line, and the panel compiles without `git`.
///
/// It was 400 while every row was a DOM node: the log draws only the rows on
/// screen now, so what bounds it is `git log` and the wire, not the page.
pub const LIMIT: usize = 1000;

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
    /// A remote-tracking branch — read off `refs/remotes/`, never off a slash
    /// in the name: `feature/x` is an ordinary local branch.
    pub remote: bool,
    /// The branch this one tracks, when it does.
    pub upstream: Option<String>,
    /// The short hash of its tip.
    pub tip: String,
    /// The full hash of its tip — what the history's rows are keyed by, so a
    /// click on the branch can find its commit.
    #[serde(default)]
    pub id: String,
    /// Commits here its upstream does not have, and the reverse.
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    /// It tracks an upstream the remote has since deleted.
    #[serde(default)]
    pub gone: bool,
    /// Its tip's commit time, seconds since the epoch.
    #[serde(default)]
    pub time: u64,
    /// Its tip's subject line.
    #[serde(default)]
    pub subject: String,
    /// For a remote-tracking branch, the remote it belongs to: `origin` for
    /// `origin/feature/x`.
    #[serde(default)]
    pub remote_name: Option<String>,
}

impl Branch {
    /// The name without its remote: `feature/x` for `origin/feature/x`, and
    /// the name itself for a local branch — what a checkout of a remote
    /// branch names the local one it creates.
    pub fn local_name(&self) -> &str {
        match &self.remote_name {
            Some(remote) => self
                .name
                .strip_prefix(remote.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
                .unwrap_or(&self.name),
            None => &self.name,
        }
    }
}

/// A tag, with the commit it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub name: String,
    /// The commit the tag names — peeled, so an annotated tag gives its commit
    /// rather than the tag object, which no row of the history is keyed by.
    pub id: String,
    /// When it was made: the tagger's date for an annotated tag, the commit's
    /// for a lightweight one.
    pub time: u64,
    /// The tag's message, or the commit's subject for a lightweight tag.
    pub subject: String,
}

/// Every branch, local and remote-tracking, and every tag — one
/// `for-each-ref`, where branches and tags were two questions before and
/// tags were not asked at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Refs {
    /// Local branches first, by name, then remote-tracking ones by name.
    pub branches: Vec<Branch>,
    /// Newest first.
    pub tags: Vec<Tag>,
}

/// A remote, as the repository's config names it — whether or not anything
/// has been fetched from it yet.
///
/// The sidebar used to know remotes only through their branches, read off
/// `refs/remotes/`. A remote just added has none until the first fetch or
/// push, so it did not exist on screen — and neither did any way to add one,
/// so a repository started with `git init` could not be connected to GitHub
/// from the panel at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    /// Where fetches come from, as configured. Not rewritten through
    /// `url.<base>.insteadOf`: this is also the text the edit field starts
    /// from, and saving a rewritten URL back would bake the rewrite into the
    /// config.
    pub url: String,
    /// Where pushes go, when that is configured apart from `url`.
    #[serde(default)]
    pub push_url: Option<String>,
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
    /// A merge, rebase, cherry-pick or revert stopped half way — on a
    /// conflict, usually. The panel says so above everything else, with the
    /// two ways out, because a repository in that state refuses most of what
    /// the panel offers and git's refusal names the state, not the way out.
    #[serde(default)]
    pub operation: Option<GitOperation>,
}

/// An operation git has started and not finished, read off the files it
/// leaves in the git directory while it waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// A fingerprint of the repository's own files — `HEAD` and its log, the
/// refs, the index, the stash — made from their sizes and times, with no
/// `git` run at all.
///
/// The panel compares one with the last to decide what to read again. Before
/// it, every save anywhere in the project re-ran the whole panel — nine `git`
/// processes and the history redrawn — while a commit made in a terminal,
/// which touches nothing the file watcher sees, was never noticed at all.
///
/// Each part fits in 53 bits, because it crosses the wire as a JSON number
/// and a JavaScript number holds no more: a full 64-bit hash is not a safe
/// integer there, the frontend's deserialiser refused every stamp, and every
/// probe failed without a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStamp {
    /// `HEAD`, its reflog, and the marker files of an operation in progress.
    pub head: u64,
    /// Every ref under `refs/` but the stash, and `packed-refs`.
    pub refs: u64,
    pub index: u64,
    /// `refs/stash` and its reflog, which is the stash list.
    pub stash: u64,
    /// The repository's `config`, which is where remotes live — so a remote
    /// added in a terminal reaches the sidebar the way a commit does.
    #[serde(default)]
    pub config: u64,
}

/// What a newer stamp says has to be read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stale {
    pub history: bool,
    pub refs: bool,
    pub status: bool,
    pub stashes: bool,
    pub remotes: bool,
}

impl Stale {
    pub fn any(&self) -> bool {
        self.history || self.refs || self.status || self.stashes || self.remotes
    }
}

impl GitStamp {
    /// What moved between `before` and this one. A commit, a checkout, a
    /// fetch or a new tag moves HEAD or the refs, and the log, the branches
    /// and the status's ahead-behind all follow those; staging moves only the
    /// index; a stash moves only the stash; the config, only the remotes —
    /// `git config user.name` moves it too, and costs one read of a list
    /// nobody changed, which is cheaper than a stamp per config key.
    pub fn stale_since(&self, before: &GitStamp) -> Stale {
        let moved = self.head != before.head || self.refs != before.refs;
        Stale {
            history: moved,
            refs: moved,
            status: moved || self.index != before.index,
            stashes: self.stash != before.stash,
            remotes: self.config != before.config,
        }
    }
}

/// Why git would refuse a name for a branch or a tag — the rules of `git
/// check-ref-format --branch`, checked before the command runs so the field
/// says what is wrong while it is being typed, not the dock afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefNameProblem {
    Empty,
    /// A space, a tab, a newline.
    Whitespace,
    /// One of `~ ^ : ? * [ \` or a control character.
    Character(char),
    /// A leading `-`, which git would read as an option.
    Dash,
    /// `..` anywhere, a component starting with `.`, or a trailing `.`.
    Dot,
    /// A leading or trailing `/`, or `//`.
    Slash,
    /// A component ending in `.lock`, which is git's own lock-file suffix.
    Lock,
    /// `HEAD`, `@`, or `@{`.
    Reserved,
}

/// `None` when git would take `name` as a branch or a tag.
pub fn ref_name_problem(name: &str) -> Option<RefNameProblem> {
    if name.is_empty() {
        return Some(RefNameProblem::Empty);
    }
    if name == "HEAD" || name == "@" || name.contains("@{") {
        return Some(RefNameProblem::Reserved);
    }
    if name.chars().any(char::is_whitespace) {
        return Some(RefNameProblem::Whitespace);
    }
    if let Some(bad) = name
        .chars()
        .find(|c| c.is_control() || "~^:?*[\\".contains(*c))
    {
        return Some(RefNameProblem::Character(bad));
    }
    if name.starts_with('-') {
        return Some(RefNameProblem::Dash);
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Some(RefNameProblem::Slash);
    }
    if name.contains("..")
        || name.ends_with('.')
        || name.split('/').any(|part| part.starts_with('.'))
    {
        return Some(RefNameProblem::Dot);
    }
    if name.split('/').any(|part| part.ends_with(".lock")) {
        return Some(RefNameProblem::Lock);
    }
    None
}

/// Why a remote could not be added, renamed or pointed somewhere else — said
/// beside the field while it is typed, as a branch name's problems are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteProblem {
    /// Not a name git would take. The rules are a branch's, because a remote
    /// name becomes a directory of refs: `refs/remotes/<name>/`.
    Name(RefNameProblem),
    /// Another remote already has it.
    Exists,
    UrlEmpty,
    /// A line break or another control character: a paste that brought more
    /// than the URL with it.
    UrlControl,
    /// A leading `-`, which git would take as an option.
    UrlDash,
}

/// `None` when a remote can be called `name`. `keeping` is the remote being
/// renamed, whose own name is not a clash with itself.
pub fn remote_name_problem(
    name: &str,
    remotes: &[Remote],
    keeping: Option<&str>,
) -> Option<RemoteProblem> {
    if let Some(problem) = ref_name_problem(name) {
        return Some(RemoteProblem::Name(problem));
    }
    let taken = remotes
        .iter()
        .any(|remote| remote.name == name && Some(remote.name.as_str()) != keeping);
    taken.then_some(RemoteProblem::Exists)
}

/// `None` when a remote can point at `url`, given as it will be sent —
/// trimmed. Deliberately not a URL parser: git takes `https://`, `ssh://`,
/// `git@github.com:you/repo.git` and a plain directory alike, and a check
/// that knew fewer shapes than git would refuse a remote that works.
pub fn remote_url_problem(url: &str) -> Option<RemoteProblem> {
    if url.is_empty() {
        return Some(RemoteProblem::UrlEmpty);
    }
    if url.chars().any(char::is_control) {
        return Some(RemoteProblem::UrlControl);
    }
    if url.starts_with('-') {
        return Some(RemoteProblem::UrlDash);
    }
    None
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

/// Who git would sign a commit as: `user.name` and `user.email` as `git
/// config` resolves them in the repository — local over global over system.
/// Either absent and `git commit` refuses with "Author identity unknown",
/// which is the one refusal worth asking about *before* the button.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitIdentity {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl GitIdentity {
    /// Whether a commit would carry an author.
    pub fn complete(&self) -> bool {
        let set = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
        set(&self.name) && set(&self.email)
    }
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
mod ref_tests {
    use super::*;

    #[test]
    fn names_git_would_refuse_are_refused_with_the_reason_git_has() {
        for good in ["main", "feature/x", "fix-12", "v1.2.0", "中文分支", "a_b"] {
            assert_eq!(ref_name_problem(good), None, "{good} is a fine name");
        }
        let cases = [
            ("", RefNameProblem::Empty),
            ("my branch", RefNameProblem::Whitespace),
            ("a:b", RefNameProblem::Character(':')),
            ("what?", RefNameProblem::Character('?')),
            ("-x", RefNameProblem::Dash),
            ("a..b", RefNameProblem::Dot),
            (".hidden", RefNameProblem::Dot),
            ("x/.y", RefNameProblem::Dot),
            ("end.", RefNameProblem::Dot),
            ("/x", RefNameProblem::Slash),
            ("x/", RefNameProblem::Slash),
            ("a//b", RefNameProblem::Slash),
            ("x.lock", RefNameProblem::Lock),
            ("a.lock/b", RefNameProblem::Lock),
            ("HEAD", RefNameProblem::Reserved),
            ("@", RefNameProblem::Reserved),
            ("x@{1}", RefNameProblem::Reserved),
        ];
        for (name, problem) in cases {
            assert_eq!(ref_name_problem(name), Some(problem), "{name:?}");
        }
    }

    /// A stamp that moved only in the index asks for the status and nothing
    /// else; one that moved in the refs asks for the history, the branches
    /// and the status; the stash stands alone.
    #[test]
    fn a_stamp_says_which_reads_are_stale() {
        let before = GitStamp {
            head: 1,
            refs: 2,
            index: 3,
            stash: 4,
            config: 10,
        };
        assert!(!before.stale_since(&before).any());
        let staged = GitStamp { index: 9, ..before };
        assert_eq!(
            staged.stale_since(&before),
            Stale {
                status: true,
                ..Stale::default()
            }
        );
        let committed = GitStamp {
            head: 8,
            refs: 7,
            ..before
        };
        let stale = committed.stale_since(&before);
        assert!(stale.history && stale.refs && stale.status && !stale.stashes);
        let stashed = GitStamp { stash: 5, ..before };
        assert_eq!(
            stashed.stale_since(&before),
            Stale {
                stashes: true,
                ..Stale::default()
            }
        );
        let configured = GitStamp {
            config: 6,
            ..before
        };
        assert_eq!(
            configured.stale_since(&before),
            Stale {
                remotes: true,
                ..Stale::default()
            },
            "a remote added in a terminal is read again, and nothing else is",
        );
    }

    fn remote(name: &str) -> Remote {
        Remote {
            name: name.into(),
            url: format!("https://github.com/you/{name}.git"),
            push_url: None,
        }
    }

    /// A remote's name follows a branch's rules and must not be taken —
    /// except by the remote being renamed, which is keeping its own.
    #[test]
    fn a_remote_name_is_refused_for_a_reason_and_a_clash_is_one() {
        let remotes = [remote("origin"), remote("upstream")];
        assert_eq!(remote_name_problem("fork", &remotes, None), None);
        assert_eq!(
            remote_name_problem("my.fork", &remotes, None),
            None,
            "a dot inside a name is fine, and git keeps it as one name",
        );
        assert_eq!(
            remote_name_problem("origin", &remotes, None),
            Some(RemoteProblem::Exists)
        );
        assert_eq!(
            remote_name_problem("origin", &remotes, Some("origin")),
            None,
            "renaming a remote to itself clashes with nothing",
        );
        assert_eq!(
            remote_name_problem("upstream", &remotes, Some("origin")),
            Some(RemoteProblem::Exists),
            "but renaming onto another remote does",
        );
        assert_eq!(
            remote_name_problem("", &remotes, None),
            Some(RemoteProblem::Name(RefNameProblem::Empty))
        );
        assert_eq!(
            remote_name_problem("-x", &remotes, None),
            Some(RemoteProblem::Name(RefNameProblem::Dash))
        );
    }

    /// Every shape git takes is taken; only what git would misread is not.
    #[test]
    fn a_remote_url_is_refused_only_for_what_git_would_misread() {
        for good in [
            "https://github.com/you/repo.git",
            "git@github.com:you/repo.git",
            "ssh://git@gitlab.com/you/repo.git",
            "E:/Work/My Repos/firmware.git",
            "../bare.git",
        ] {
            assert_eq!(remote_url_problem(good), None, "{good}");
        }
        assert_eq!(remote_url_problem(""), Some(RemoteProblem::UrlEmpty));
        let pasted = format!(
            "https://github.com/you/repo.git{}git push",
            char::from(10u8)
        );
        assert_eq!(
            remote_url_problem(&pasted),
            Some(RemoteProblem::UrlControl),
            "a paste that carried a second line is not a URL",
        );
        assert_eq!(
            remote_url_problem("--upload-pack=touch"),
            Some(RemoteProblem::UrlDash)
        );
    }

    #[test]
    fn a_remote_branch_names_its_local_counterpart_without_the_remote() {
        let branch = |name: &str, remote: Option<&str>| Branch {
            name: name.into(),
            current: false,
            remote: remote.is_some(),
            upstream: None,
            tip: String::new(),
            id: String::new(),
            ahead: 0,
            behind: 0,
            gone: false,
            time: 0,
            subject: String::new(),
            remote_name: remote.map(str::to_string),
        };
        assert_eq!(
            branch("origin/feature/x", Some("origin")).local_name(),
            "feature/x"
        );
        assert_eq!(branch("feature/x", None).local_name(), "feature/x");
        assert_eq!(
            branch("upstream/main", Some("upstream")).local_name(),
            "main"
        );
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    /// Both halves, and neither may be blank: git treats an empty
    /// `user.email` exactly as it treats a missing one.
    #[test]
    fn an_identity_is_complete_only_with_a_non_blank_name_and_email() {
        let id = |name: Option<&str>, email: Option<&str>| GitIdentity {
            name: name.map(str::to_string),
            email: email.map(str::to_string),
        };
        assert!(id(Some("Lin"), Some("lin@example.com")).complete());
        assert!(!id(None, Some("lin@example.com")).complete());
        assert!(!id(Some("Lin"), None).complete());
        assert!(!id(Some("  "), Some("lin@example.com")).complete());
        assert!(!GitIdentity::default().complete());
    }

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
