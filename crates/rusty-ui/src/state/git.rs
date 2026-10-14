//! The Git panel's state: what was read, what is selected, the prompts and
//! menus, and the gate that keeps one of each read in flight.

use super::*;

/// The repository's history, as the Git panel shows it.
///
/// Session state: which branch is being looked at, which commit is open,
/// which of its files. Reset when the project changes, because a selection
/// from one repository names nothing in another.
#[derive(Clone, Copy)]
pub struct Git {
    /// The log, laid out by the backend. `None` until asked, or when the
    /// project is not a repository — `unavailable` then says why.
    pub history: RwSignal<Option<rusty_git::History>>,
    pub branches: RwSignal<Vec<rusty_git::Branch>>,
    /// The branch the log is filtered to; `None` is every branch.
    pub rev: RwSignal<Option<String>>,
    /// The commit that is open, by full hash.
    pub selected: RwSignal<Option<String>>,
    pub detail: RwSignal<Option<rusty_git::CommitDetail>>,
    /// Which of the open commit's files is showing its patch.
    pub file: RwSignal<Option<String>>,
    /// Why there is no history — "not inside a git repository", no `git` on
    /// PATH — in the panel rather than on the banner, because a project
    /// without a repository is an ordinary thing to open.
    pub unavailable: RwSignal<Option<String>>,
    /// The one reason the panel can fix: the project is not a repository,
    /// and `git init` would make it one.
    pub not_a_repo: RwSignal<bool>,
    /// Whether the panel has asked once. The watcher refreshes the history
    /// only after that, so a project nobody looks at the history of costs no
    /// `git log` per save.
    pub loaded: RwSignal<bool>,
    /// Which of the three views is showing.
    pub mode: RwSignal<GitMode>,
    /// The working tree, for the Changes view.
    pub status: RwSignal<Option<rusty_git::Status>>,
    pub stashes: RwSignal<Vec<rusty_git::Stash>>,
    /// The diff of the working-tree path last clicked, and which path and
    /// side it is for — so an answer arriving after another click is dropped.
    pub diff: RwSignal<Option<String>>,
    pub diff_for: RwSignal<Option<(String, bool)>>,
    /// The commit message being written. Kept across re-renders and tab
    /// switches; cleared by the commit that uses it.
    pub message: RwSignal<String>,
    pub stash_note: RwSignal<String>,
    /// Every tag, newest first — the sidebar's third section.
    pub tags: RwSignal<Vec<rusty_git::Tag>>,
    /// Every remote the config names. Read on its own and only when the
    /// config moves, because the branches cannot say a remote exists until
    /// something has been fetched from it.
    pub remotes: RwSignal<Vec<rusty_git::Remote>>,
    /// The name being typed for a new branch, a rename or a new tag, while
    /// the field is open, and what it is for.
    pub prompt: RwSignal<Option<RefPrompt>>,
    /// Text to find in the log: part of a hash, an author, words of a
    /// subject. Rows that do not match are dimmed rather than hidden, so the
    /// graph's lines still join up.
    pub query: RwSignal<String>,
    /// Narrows the sidebar's branches and tags.
    pub ref_filter: RwSignal<String>,
    /// Sidebar sections folded away: `local`, `remote:<name>`, `tags`.
    pub folded: RwSignal<Vec<String>>,
    /// A commit the log should scroll into view, once — set by a click on a
    /// branch, a tag or a search hit, cleared by the log when it has.
    pub reveal: RwSignal<Option<String>>,
    /// A commit is being read while the previous one stays on screen. It
    /// used to be cleared first, and the pane collapsed to a strip and grew
    /// back on every click.
    pub detail_loading: RwSignal<bool>,
    /// Draw the whole of a diff too long to draw at once.
    pub diff_whole: RwSignal<bool>,
    /// The repository's fingerprint as of the last reads — see
    /// `rusty_git::GitStamp`. Not reactive: nothing draws it.
    pub stamp: StoredValue<Option<rusty_git::GitStamp>>,
    /// The project root the loaded state belongs to. The panel is rebuilt
    /// every time it is switched to, and read everything again each time
    /// until this told a return from a new project.
    pub root: StoredValue<Option<String>>,
    /// Commits opened recently, keyed by their full hash. A hash names one
    /// content for ever, so nothing here goes stale; a stash's name does not
    /// and is never kept.
    pub cache: StoredValue<Vec<rusty_git::CommitDetail>>,
    /// Which reads are in flight and which were asked for again meanwhile.
    pub gate: StoredValue<ReadGate>,
    /// Side by side rather than one column, for every diff the panel shows.
    /// Remembered in this window (localStorage): it is a way of reading, not
    /// a fact about the project, and re-choosing it every launch is the
    /// friction that makes a toggle feel broken.
    pub split: RwSignal<bool>,
    /// How many commits the log asks for. "Show older commits" doubles it.
    pub limit: RwSignal<usize>,
    /// Whether the next commit amends the last one instead.
    pub amend: RwSignal<bool>,
    /// The right-click menu while it is open: where, and what it is about.
    pub menu: RwSignal<Option<GitMenu>>,
    /// The clone dialog while it is open: what has been typed and chosen.
    pub clone: RwSignal<Option<CloneDraft>>,
    /// The two sides of an image being compared, once a picture is picked.
    pub images: RwSignal<Option<ImagePair>>,
    /// The opened commit's pane folded away to a strip — Fork's hide.
    pub detail_hidden: RwSignal<bool>,
    /// `Some(target)` when this window was booted with `?gitdiff=<target>`:
    /// a window showing one commit and nothing else.
    pub window_target: RwSignal<Option<String>>,
    /// Who git would sign a commit as; `None` until asked. Incomplete, and
    /// the commit box shows a form instead of letting git refuse.
    pub identity: RwSignal<Option<rusty_git::GitIdentity>>,
}

/// What the clone dialog holds while it is open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloneDraft {
    pub url: String,
    /// The folder the repository's directory is created *in*.
    pub into: Option<String>,
    /// Set while `git clone` runs, so the button cannot start a second.
    pub running: bool,
}

/// An image's two sides, each resolved on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePair {
    pub path: String,
    pub old: ImageSide,
    pub new: ImageSide,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSide {
    /// Nothing to show: the file did not exist on this side.
    Absent,
    Loading,
    /// A `data:` URL ready for an `<img>`, and the size in bytes.
    Ready {
        url: String,
        bytes: usize,
    },
    Failed(String),
}

/// Where an image's side comes from, as the backend is asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSource {
    /// The file as it is on disk.
    Worktree,
    /// `git show <rev>:<path>` — a hash, `HEAD`, or `:0` for the index.
    Rev(String),
}

/// A right-click in the Git panel: the pointer, and what was under it.
#[derive(Clone, Debug, PartialEq)]
pub struct GitMenu {
    pub x: f64,
    pub y: f64,
    pub target: GitTarget,
}

pub use rusty_git::plan::{PromptKind, PromptProblem, RefPrompt};

/// The reads the panel makes, as indices into [`ReadGate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitRead {
    History,
    Refs,
    Status,
    Stashes,
    Remotes,
}

impl GitRead {
    fn slot(self) -> usize {
        match self {
            GitRead::History => 0,
            GitRead::Refs => 1,
            GitRead::Status => 2,
            GitRead::Stashes => 3,
            GitRead::Remotes => 4,
        }
    }
}

/// At most one of each read in flight, and one more after it when asked
/// again meanwhile — never a queue. A burst of saves asked for the status
/// once per save, and each answer arrived, was compared and was drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadGate {
    running: [bool; 5],
    again: [bool; 5],
}

impl ReadGate {
    /// Whether to start the read now. When one is already running, it is
    /// marked to run once more when that one finishes, and this is no.
    pub fn begin(&mut self, read: GitRead) -> bool {
        let slot = read.slot();
        if self.running[slot] {
            self.again[slot] = true;
            false
        } else {
            self.running[slot] = true;
            true
        }
    }

    /// The read finished. Whether it was asked for again while it ran.
    pub fn finish(&mut self, read: GitRead) -> bool {
        let slot = read.slot();
        self.running[slot] = false;
        std::mem::take(&mut self.again[slot])
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    /// A second ask while one read runs is one more run afterwards, however
    /// many times it is asked; a different read is its own.
    #[test]
    fn a_read_asked_for_while_running_runs_once_more_and_no_more() {
        let mut gate = ReadGate::default();
        assert!(gate.begin(GitRead::Status));
        assert!(!gate.begin(GitRead::Status));
        assert!(!gate.begin(GitRead::Status));
        assert!(gate.begin(GitRead::History), "another read is not held up");
        assert!(gate.finish(GitRead::Status), "asked again while it ran");
        assert!(gate.begin(GitRead::Status));
        assert!(!gate.finish(GitRead::Status), "and not again after that");
        assert!(!gate.finish(GitRead::History));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitTarget {
    /// A branch in the sidebar, local or remote-tracking.
    Branch {
        name: String,
        remote: bool,
        current: bool,
    },
    /// A tag in the sidebar, with the commit it names.
    Tag { name: String, id: String },
    /// A remote's heading in the sidebar.
    Remote { name: String },
    /// A commit, by full hash — or a stash by its `stash@{n}` name.
    Commit { id: String },
    /// A path in a commit's file list, relative to the root: open it, copy
    /// it, and nothing that writes.
    Path { path: String },
    /// A path in one of the Changes view's two lists, with which list it was
    /// clicked in — stage or unstage, discard and stash mean different
    /// things on the two sides.
    Change {
        path: String,
        staged: bool,
        untracked: bool,
    },
}

/// The Git panel's three views.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GitMode {
    History,
    Changes,
    Stashes,
}
