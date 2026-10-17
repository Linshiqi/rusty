//! The repository: the Git panel in the left dock (VS Code's shape — what
//! is checked out, the commit box, the staged and unstaged files, the
//! stashes) and the History page in the editor area (Fork's — every ref
//! down the left, the graph, a commit opened below with its files and its
//! patch). `src/app/git_view.rs` draws both.
//!
//! **Reads run `git` on a thread and come back as `rusty_git`'s types;
//! writes are dock commands**, through the runner every `cargo` uses, so
//! the exact `git` line and everything git says back are in Output — a
//! checkout refused on a dirty tree is a paragraph there, not a banner
//! nobody can act on. Staging is the one quiet write: instant, reversible,
//! and a line per click would bury the commands that matter.
//!
//! **Read only what moved** (`rusty_git::GitStamp`): the stamp is the sizes
//! and times of the files git itself rewrites, read with no `git` at all,
//! and asked every few seconds while the panel or the page is showing —
//! how a commit made in a terminal arrives. One of each read is in flight
//! at a time, with at most one more asked for (`Gate`).

use std::{path::PathBuf, rc::Rc, time::Duration};

use rusty_embed::CommandPlan;
use rusty_git::{
    CommitDetail, GitIdentity, GitOperation, GitStamp, History, Refs, Remote, Stash, Status,
    plan::{PromptKind, RefPrompt, checkout_args, pick_remote},
};
use rusty_i18n::t;
use slint::ComponentHandle;

use super::{App, Tab};
use crate::{Shell, work};

/// Opened commits kept for a second look.
const CACHED: usize = 32;
/// How often the stamp is asked while the panel or the page shows.
const POLL: Duration = Duration::from_millis(2500);

/// The reads, as slots of [`Gate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    History,
    Refs,
    Status,
    Stashes,
    Remotes,
}

/// At most one of each read in flight, and one more after it when asked
/// again meanwhile — never a queue.
#[derive(Default)]
struct Gate {
    running: [bool; 5],
    again: [bool; 5],
}

impl Gate {
    fn begin(&mut self, read: Read) -> bool {
        let slot = read as usize;
        if self.running[slot] {
            self.again[slot] = true;
            false
        } else {
            self.running[slot] = true;
            true
        }
    }

    fn finish(&mut self, read: Read) -> bool {
        let slot = read as usize;
        self.running[slot] = false;
        std::mem::take(&mut self.again[slot])
    }
}

/// What a right-click was on.
#[derive(Clone, Debug)]
pub(super) enum Target {
    Branch {
        name: String,
        remote: bool,
        current: bool,
    },
    Tag {
        name: String,
        id: String,
    },
    Remote {
        name: String,
    },
    Commit {
        id: String,
        short: String,
    },
    /// A file of an opened commit: open it, copy it, nothing that writes.
    Path {
        path: String,
    },
    /// A file in one of the two lists — which list, because discard means
    /// a different thing on each.
    Change {
        path: String,
        staged: bool,
        untracked: bool,
        nested: bool,
    },
    /// A page's diff, on the side clicked: Copy and Select all.
    Diff {
        which: super::git_select::Which,
        side: usize,
    },
    /// A piece of the opened commit's message — "summary", "meta" or
    /// "body" — and what was selected in it when the menu opened.
    Text {
        which: String,
        selected: String,
        id: String,
        short: String,
        message: String,
    },
}

/// What the panel and the page hold.
pub(super) struct Git {
    /// The project the rest was read for.
    pub(super) root: Option<PathBuf>,
    pub(super) loaded: bool,
    pub(super) not_repo: bool,
    pub(super) unavailable: Option<String>,
    pub(super) history: Option<History>,
    pub(super) refs: Refs,
    pub(super) remotes: Vec<Remote>,
    pub(super) status: Option<Status>,
    pub(super) stashes: Vec<Stash>,
    pub(super) identity: Option<GitIdentity>,
    stamp: Option<GitStamp>,
    gate: Gate,
    /// The log shows this ref's history alone, when set.
    pub(super) rev: Option<String>,
    pub(super) limit: usize,
    pub(super) selected: Option<String>,
    pub(super) detail: Option<CommitDetail>,
    cache: Vec<CommitDetail>,
    /// The opened commit's file whose patch is shown.
    pub(super) file: Option<String>,
    pub(super) query: String,
    pub(super) ref_filter: String,
    /// The working-tree file whose diff the Changes page shows, and which
    /// side of the index.
    pub(super) diff_for: Option<(String, bool)>,
    pub(super) diff: Option<String>,
    pub(super) split: bool,
    pub(super) amend: bool,
    pub(super) prompt: Option<RefPrompt>,
    pub(super) menu: Option<Target>,
    /// Bumped to bring the selected commit into view.
    pub(super) reveal: i32,
    poll: Option<slint::Timer>,
    /// The two pages' diffs as drawn, and what is selected in them.
    pub(super) views: super::git_select::Views,
}

impl Default for Git {
    fn default() -> Self {
        Git {
            root: None,
            loaded: false,
            not_repo: false,
            unavailable: None,
            history: None,
            refs: Refs::default(),
            remotes: Vec::new(),
            status: None,
            stashes: Vec::new(),
            identity: None,
            stamp: None,
            gate: Gate::default(),
            rev: None,
            limit: rusty_git::LIMIT,
            selected: None,
            detail: None,
            cache: Vec::new(),
            file: None,
            query: String::new(),
            ref_filter: String::new(),
            diff_for: None,
            diff: None,
            split: false,
            amend: false,
            prompt: None,
            menu: None,
            reveal: 0,
            poll: None,
            views: Default::default(),
        }
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| (*arg).to_string()).collect()
}

/// A word as a shell would need it quoted, for the line the dock shows —
/// the arguments themselves go to git as they are.
fn shown(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || "\"'$`\\".contains(c)) {
        return arg.to_string();
    }
    let first = arg.lines().next().unwrap_or("");
    let more = if arg.lines().nth(1).is_some() {
        "…"
    } else {
        ""
    };
    format!("\"{}{more}\"", first.replace('"', "\\\""))
}

/// A yes-or-no question before something that cannot be undone. A probe
/// driving the window answers yes, since nobody is there to.
pub(super) fn confirm(question: &str) -> bool {
    if std::env::var_os("RUSTY_PROBE").is_some() {
        return true;
    }
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    let mut lines = question.splitn(2, '\n');
    let title = lines.next().unwrap_or_default();
    let detail = lines.next().unwrap_or_default();
    let answer = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(title)
        .set_description(if detail.is_empty() { title } else { detail })
        .set_buttons(MessageButtons::OkCancel)
        .show();
    matches!(answer, MessageDialogResult::Ok | MessageDialogResult::Yes)
}

impl App {
    // ---- showing ---------------------------------------------------------

    /// The panel or the page came on screen. A project shown before is only
    /// probed; a new one starts clean.
    pub(super) fn git_shown(self: &Rc<Self>) {
        let root = self.root();
        if root.is_none() {
            return;
        }
        let known = {
            let state = self.state.borrow();
            state.git.root == root && state.git.loaded
        };
        if known {
            self.git_probe();
            self.git_status();
        } else {
            {
                let mut state = self.state.borrow_mut();
                let poll = state.git.poll.take();
                state.git = Git {
                    root,
                    poll,
                    ..Git::default()
                };
            }
            self.git_load();
        }
        self.git_poll();
        self.sync_git();
    }

    /// Whether either surface is on screen: the reason to keep asking.
    fn git_visible(&self) -> bool {
        let state = self.state.borrow();
        let panel =
            state.layout.left.open && crate::docks::Dock::Left.front(&state.layout) == "git";
        let page = matches!(
            state.active.and_then(|at| state.tabs.get(at)),
            Some(Tab::Page("git" | "gitdiff"))
        );
        panel || page
    }

    fn git_poll(self: &Rc<Self>) {
        if self.state.borrow().git.poll.is_some() {
            return;
        }
        let timer = slint::Timer::default();
        let weak = Rc::downgrade(self);
        timer.start(slint::TimerMode::Repeated, POLL, move || {
            if let Some(app) = weak.upgrade()
                && app.git_visible()
                && app.state.borrow().git.loaded
            {
                app.git_status();
                app.git_probe();
            }
        });
        self.state.borrow_mut().git.poll = Some(timer);
    }

    /// The History page, opened or brought forward.
    pub(super) fn git_open_history(self: &Rc<Self>) {
        self.open_tab(Tab::Page("git"));
        self.git_shown();
    }

    // ---- reading ---------------------------------------------------------

    /// Everything at once — the panel opening, or Refresh — with the stamp
    /// taken beside it as the baseline later probes are compared with.
    pub(super) fn git_load(self: &Rc<Self>) {
        self.git_stamp();
        self.git_history();
        self.git_refs();
        self.git_status();
        self.git_stashes();
        self.git_remotes();
        self.git_identity();
    }

    fn git_stamp(self: &Rc<Self>) {
        let Some(root) = self.root() else { return };
        work::spawn(
            move || rusty_git::repo::stamp(&root).ok(),
            |app, stamp| {
                if stamp.is_some() {
                    app.state.borrow_mut().git.stamp = stamp;
                }
            },
        );
    }

    /// Whether the repository moved since it was last read, and the reads
    /// of what did.
    fn git_probe(self: &Rc<Self>) {
        let Some(root) = self.root() else { return };
        work::spawn(
            move || rusty_git::repo::stamp(&root).ok(),
            |app, stamp| {
                let Some(stamp) = stamp else { return };
                let before = app.state.borrow_mut().git.stamp.replace(stamp);
                let Some(before) = before else { return };
                let stale = stamp.stale_since(&before);
                if stale.history {
                    app.git_history();
                }
                if stale.refs {
                    app.git_refs();
                }
                if stale.status {
                    app.git_status();
                }
                if stale.stashes {
                    app.git_stashes();
                }
                if stale.remotes {
                    app.git_remotes();
                }
            },
        );
    }

    fn git_begin(&self, read: Read) -> Option<PathBuf> {
        let root = self.root()?;
        self.state.borrow_mut().git.gate.begin(read).then_some(root)
    }

    /// Done with a read; true when it was asked for again meanwhile.
    fn git_finish(&self, read: Read) -> bool {
        self.state.borrow_mut().git.gate.finish(read)
    }

    /// The log for the chosen ref, or for every one. Not a repository is an
    /// ordinary thing to open, and the panel says so rather than a banner.
    fn git_history(self: &Rc<Self>) {
        let Some(root) = self.git_begin(Read::History) else {
            return;
        };
        let asked = {
            let state = self.state.borrow();
            (state.git.rev.clone(), state.git.limit)
        };
        let (rev, limit) = asked.clone();
        work::spawn(
            move || {
                rusty_git::repo::history(&root, rev.as_deref(), limit).map_err(|error| {
                    let not_repo = matches!(error, rusty_git::Error::NotARepository { .. });
                    (not_repo, error.to_string())
                })
            },
            move |app, answer| {
                let again = app.git_finish(Read::History);
                let current = {
                    let mut state = app.state.borrow_mut();
                    let git = &mut state.git;
                    let current = (git.rev.clone(), git.limit) == asked;
                    if current {
                        match answer {
                            Ok(history) => {
                                git.history = Some(history);
                                git.unavailable = None;
                                git.not_repo = false;
                            }
                            Err((not_repo, why)) => {
                                git.history = None;
                                git.not_repo = not_repo;
                                git.unavailable = Some(why);
                            }
                        }
                        git.loaded = true;
                    }
                    current
                };
                if current {
                    app.sync_git();
                }
                if again || !current {
                    app.git_history();
                }
            },
        );
    }

    fn git_refs(self: &Rc<Self>) {
        let Some(root) = self.git_begin(Read::Refs) else {
            return;
        };
        work::spawn(
            move || rusty_git::repo::refs(&root).ok(),
            |app, refs| {
                let again = app.git_finish(Read::Refs);
                if let Some(refs) = refs {
                    app.state.borrow_mut().git.refs = refs;
                    app.sync_git();
                }
                if again {
                    app.git_refs();
                }
            },
        );
    }

    pub(super) fn git_status(self: &Rc<Self>) {
        let Some(root) = self.git_begin(Read::Status) else {
            return;
        };
        work::spawn(
            move || rusty_git::repo::status(&root).ok(),
            |app, status| {
                let again = app.git_finish(Read::Status);
                if let Some(status) = status {
                    let changed = app.state.borrow().git.status.as_ref() != Some(&status);
                    if changed {
                        app.state.borrow_mut().git.status = Some(status);
                        app.sync_git();
                    }
                }
                if again {
                    app.git_status();
                }
            },
        );
    }

    fn git_stashes(self: &Rc<Self>) {
        let Some(root) = self.git_begin(Read::Stashes) else {
            return;
        };
        work::spawn(
            move || rusty_git::repo::stashes(&root).ok(),
            |app, stashes| {
                let again = app.git_finish(Read::Stashes);
                if let Some(stashes) = stashes {
                    app.state.borrow_mut().git.stashes = stashes;
                    app.sync_git();
                }
                if again {
                    app.git_stashes();
                }
            },
        );
    }

    fn git_remotes(self: &Rc<Self>) {
        let Some(root) = self.git_begin(Read::Remotes) else {
            return;
        };
        work::spawn(
            move || rusty_git::repo::remotes(&root).ok(),
            |app, remotes| {
                let again = app.git_finish(Read::Remotes);
                if let Some(remotes) = remotes {
                    app.state.borrow_mut().git.remotes = remotes;
                    app.sync_git();
                }
                if again {
                    app.git_remotes();
                }
            },
        );
    }

    fn git_identity(self: &Rc<Self>) {
        let Some(root) = self.root() else { return };
        work::spawn(
            move || rusty_git::repo::identity(&root).ok(),
            |app, identity| {
                if identity.is_some() {
                    app.state.borrow_mut().git.identity = identity;
                    app.sync_git();
                }
            },
        );
    }

    /// After a command that changed the repository: everything read back,
    /// a new baseline, the tree, and the open files that changed on disk.
    fn after_git(self: &Rc<Self>) {
        self.git_load();
        self.reread_project();
        self.follow_disk();
    }

    /// Open files nobody has edited take what the disk has now — a checkout,
    /// a pull or a discard wrote it. An edited one is left alone: its draft
    /// is somebody's work.
    fn follow_disk(self: &Rc<Self>) {
        let open: Vec<String> = self
            .state
            .borrow()
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Tab::File(path) => Some(path.clone()),
                _ => None,
            })
            .collect();
        self.follow_changed(open);
    }

    // ---- writing ---------------------------------------------------------

    /// A `git` line in the dock, then everything read back.
    fn git(self: &Rc<Self>, list: Vec<String>) {
        self.git_then(list, |app, _| app.after_git());
    }

    fn git_then(
        self: &Rc<Self>,
        list: Vec<String>,
        then: impl FnOnce(&Rc<App>, crate::run::Ended) + Send + 'static,
    ) {
        let mut plan = CommandPlan::new("git", list.clone(), String::new());
        plan.display = std::iter::once("git".to_string())
            .chain(list.iter().map(|arg| shown(arg)))
            .collect::<Vec<_>>()
            .join(" ");
        let root = self.root();
        self.run(&plan, root, then);
    }

    /// A verb from a button, the menu or a key.
    pub(super) fn git_act(self: &Rc<Self>, verb: &str) {
        match verb {
            "shown" => self.git_shown(),
            "refresh" => self.git_load(),
            "history" => self.git_open_history(),
            "fetch" => self.git(args(&["fetch", "--all", "--prune"])),
            "pull" => self.git(args(&["pull"])),
            "push" => self.git_push(),
            "init" => self.git_then(args(&["init"]), |app, ended| {
                if ended == crate::run::Ended::Done {
                    app.after_git();
                }
            }),
            "branch" => self.git_prompt(PromptKind::Branch { from: None }),
            "remote" => self.git_prompt(PromptKind::Remote { push: false }),
            "stage-all" => self.git_stage_all(true),
            "unstage-all" => self.git_stage_all(false),
            "commit" => self.git_commit(),
            "amend" => self.git_amend(),
            "stash" => self.git_stash_save(),
            "continue" => self.git_continue(),
            "abort" => self.git_abort(),
            "more" => {
                self.state.borrow_mut().git.limit += rusty_git::LIMIT;
                self.git_history();
            }
            "filter-clear" => {
                self.state.borrow_mut().git.rev = None;
                self.git_history();
            }
            "split" => {
                {
                    let mut state = self.state.borrow_mut();
                    state.git.split = !state.git.split;
                }
                self.sync_git();
            }
            "detail-close" => {
                {
                    let mut state = self.state.borrow_mut();
                    state.git.selected = None;
                    state.git.detail = None;
                    state.git.file = None;
                }
                self.sync_git();
            }
            "head" => self.git_reveal_head(),
            _ => {}
        }
    }

    /// Push the branch checked out. With no upstream yet, set one; with no
    /// remote at all there is nowhere to push to, so the remote form opens
    /// and the push carries on once it is added.
    fn git_push(self: &Rc<Self>) {
        let tracked = self
            .state
            .borrow()
            .git
            .status
            .as_ref()
            .is_some_and(|s| s.upstream.is_some());
        if tracked {
            return self.git(args(&["push"]));
        }
        match self.git_default_remote() {
            Some(remote) => self.git_push_to(remote),
            None => self.git_prompt(PromptKind::Remote { push: true }),
        }
    }

    fn git_push_to(self: &Rc<Self>, remote: String) {
        let (head, upstream) = {
            let state = self.state.borrow();
            state
                .git
                .status
                .as_ref()
                .map(|s| (s.head.clone(), s.upstream.clone()))
                .unwrap_or_default()
        };
        let mut list = args(&["push"]);
        if upstream.is_none()
            && let Some(head) = head
        {
            list.extend(args(&["-u", &remote, &head]));
        }
        self.git(list);
    }

    /// Every remote name there is evidence of: the config's, then any a
    /// remote-tracking branch names.
    fn git_known_remotes(&self) -> Vec<String> {
        let state = self.state.borrow();
        let mut known: Vec<String> = state.git.remotes.iter().map(|r| r.name.clone()).collect();
        for name in state
            .git
            .refs
            .branches
            .iter()
            .filter_map(|b| b.remote_name.clone())
        {
            if !known.contains(&name) {
                known.push(name);
            }
        }
        known
    }

    fn git_default_remote(&self) -> Option<String> {
        let upstream = self
            .state
            .borrow()
            .git
            .status
            .as_ref()
            .and_then(|s| s.upstream.clone());
        pick_remote(upstream.as_deref(), &self.git_known_remotes())
    }

    fn git_stage_all(self: &Rc<Self>, on: bool) {
        let paths: Vec<String> = {
            let state = self.state.borrow();
            state
                .git
                .status
                .as_ref()
                .map(|s| {
                    s.entries
                        .iter()
                        .filter(|e| {
                            if on {
                                (e.unstaged.is_some() || e.untracked || e.conflicted) && !e.nested
                            } else {
                                e.staged.is_some()
                            }
                        })
                        .map(|e| e.path.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        self.git_stage(paths, on);
    }

    /// Put paths in the index or take them out — quiet, then the status
    /// read back whichever way it went, since a stage that failed on one
    /// path still staged the others.
    fn git_stage(self: &Rc<Self>, paths: Vec<String>, on: bool) {
        let Some(root) = self.root() else { return };
        if paths.is_empty() {
            return;
        }
        work::spawn(
            move || {
                if on {
                    rusty_git::repo::stage(&root, &paths)
                } else {
                    rusty_git::repo::unstage(&root, &paths)
                }
                .map_err(|error| error.to_string())
            },
            |app, done| {
                if let Err(why) = done {
                    app.note(1, why);
                }
                app.git_status();
                app.git_diff_again();
            },
        );
    }

    /// A verb on one file of the two lists.
    pub(super) fn git_file(self: &Rc<Self>, verb: &str, path: String, staged: bool) {
        let entry = {
            let state = self.state.borrow();
            state
                .git
                .status
                .as_ref()
                .and_then(|s| s.entries.iter().find(|e| e.path == path).cloned())
        };
        let untracked = entry.as_ref().is_some_and(|e| e.untracked);
        match verb {
            "diff" => self.git_show_diff(path, staged),
            "stage" => self.git_stage(vec![path], true),
            "unstage" => self.git_stage(vec![path], false),
            "open" => self.open_file(path),
            "discard" => self.git_discard(path, staged, untracked),
            "stash" => {
                self.git_forget_diff(&path);
                self.git(args(&["stash", "push", "--include-untracked", "--", &path]));
            }
            "include" => self.git_include_nested(path),
            "copy" => copy_text(&path),
            "copy-full" => {
                if let Some(root) = self.root() {
                    copy_text(&root.join(&path).to_string_lossy());
                }
            }
            _ => {}
        }
    }

    /// Throw a file's changes away — asked first, in words that say what
    /// happens to *this* file: from the unstaged list the tree goes back to
    /// the index, from the staged list the file goes back to the last
    /// commit, and an untracked file has nothing to go back to.
    fn git_discard(self: &Rc<Self>, path: String, staged: bool, untracked: bool) {
        let (question, list) = if untracked {
            (
                t!("git.discard-untracked-confirm", path = path.clone()),
                args(&["clean", "-f", "--", &path]),
            )
        } else if staged {
            (
                t!("git.discard-staged-confirm", path = path.clone()),
                args(&[
                    "restore",
                    "--source=HEAD",
                    "--staged",
                    "--worktree",
                    "--",
                    &path,
                ]),
            )
        } else {
            (
                t!("git.discard-unstaged-confirm", path = path.clone()),
                args(&["restore", "--", &path]),
            )
        };
        if confirm(&question) {
            self.git_forget_diff(&path);
            self.git(list);
        }
    }

    /// Fold an empty repository inside the project into it: its `.git`
    /// goes to the recycle bin, after checking again it has no commits.
    fn git_include_nested(self: &Rc<Self>, path: String) {
        let asked = if cfg!(windows) {
            t!("git.include-nested-confirm", path = path.clone())
        } else {
            t!("git.include-nested-confirm-trash", path = path.clone())
        };
        if !confirm(&asked) {
            return;
        }
        let Some(root) = self.root() else { return };
        work::spawn(
            move || -> Result<(), String> {
                let folder = path.trim_end_matches('/');
                let dir = rusty_edit::absolute(&root, folder).map_err(|e| e.to_string())?;
                if !rusty_git::repo::is_empty_repository(&dir) {
                    return Err(format!(
                        "{folder}/ has commits of its own, so rusty will not remove its .git"
                    ));
                }
                rusty_edit::delete_entry(&root, &format!("{folder}/.git"))
                    .map_err(|e| e.to_string())
            },
            |app, done| {
                if let Err(why) = done {
                    app.note(2, why);
                }
                app.git_status();
                app.reread_project();
            },
        );
    }

    /// Commit what is staged with the message being written, or amend the
    /// last commit with it. The message is one argument however many lines
    /// it has; only an amend may go without one.
    fn git_commit(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        if self
            .state
            .borrow()
            .git
            .identity
            .as_ref()
            .is_some_and(|id| !id.complete())
        {
            return;
        }
        let message = shell.get_git_message().to_string();
        let amend = self.state.borrow().git.amend;
        let mut list = args(&["commit"]);
        if amend {
            list.push("--amend".into());
        }
        if message.trim().is_empty() {
            if !amend {
                return;
            }
            list.push("--no-edit".into());
        } else {
            list.push("-m".into());
            list.push(message);
        }
        self.git_then(list, |app, ended| {
            if ended == crate::run::Ended::Done {
                app.state.borrow_mut().git.amend = false;
                if let Some(window) = app.window() {
                    window.global::<Shell>().set_git_message("".into());
                }
            }
            app.after_git();
        });
    }

    /// Amending on or off. On over an empty box fills it with HEAD's whole
    /// message, because `-m` with only a summary typed would cut an essay
    /// down to its first line.
    fn git_amend(self: &Rc<Self>) {
        let on = {
            let mut state = self.state.borrow_mut();
            state.git.amend = !state.git.amend;
            state.git.amend
        };
        self.sync_git();
        let empty = self
            .window()
            .is_some_and(|w| w.global::<Shell>().get_git_message().trim().is_empty());
        if !on || !empty {
            return;
        }
        let Some(root) = self.root() else { return };
        work::spawn(
            move || rusty_git::repo::commit(&root, "HEAD").ok(),
            |app, detail| {
                let Some(detail) = detail else { return };
                let Some(window) = app.window() else { return };
                let shell = window.global::<Shell>();
                if app.state.borrow().git.amend && shell.get_git_message().trim().is_empty() {
                    shell.set_git_message(detail.body.trim_end().into());
                }
            },
        );
    }

    /// `git config user.name` and `user.email`, then the identity read back
    /// so the form goes away on git's word.
    pub(super) fn git_identity_save(self: &Rc<Self>, name: String, email: String, local: bool) {
        let scope = |mut list: Vec<String>| {
            if !local {
                list.insert(1, "--global".into());
            }
            list
        };
        let for_name = scope(vec!["config".into(), "user.name".into(), name]);
        let for_email = scope(vec!["config".into(), "user.email".into(), email]);
        self.git_then(for_name, move |app, ended| {
            if ended != crate::run::Ended::Done {
                return app.git_identity();
            }
            app.git_then(for_email, |app, _| app.git_identity());
        });
    }

    fn git_stash_save(self: &Rc<Self>) {
        let note = self
            .window()
            .map(|w| w.global::<Shell>().get_git_stash_note().to_string())
            .unwrap_or_default();
        let mut list = args(&["stash", "push", "--include-untracked"]);
        if !note.trim().is_empty() {
            list.push("-m".into());
            list.push(note);
        }
        self.git_then(list, |app, ended| {
            if ended == crate::run::Ended::Done
                && let Some(window) = app.window()
            {
                window.global::<Shell>().set_git_stash_note("".into());
            }
            app.after_git();
        });
    }

    /// Apply, pop or drop a stash — drop asks first: it is gone after.
    pub(super) fn git_stash(self: &Rc<Self>, verb: &str, index: i32) {
        if !matches!(verb, "apply" | "pop" | "drop") || index < 0 {
            return;
        }
        let name = format!("stash@{{{index}}}");
        if verb == "drop" && !confirm(&t!("desk.git-drop-confirm", name = name.clone())) {
            return;
        }
        self.git(vec!["stash".into(), verb.into(), name]);
    }

    fn git_operation(&self) -> Option<GitOperation> {
        self.state
            .borrow()
            .git
            .status
            .as_ref()
            .and_then(|s| s.operation)
    }

    /// Carry on with the merge, rebase, cherry-pick or revert that stopped.
    fn git_continue(self: &Rc<Self>) {
        let list = match self.git_operation() {
            Some(GitOperation::Merge) => args(&["commit", "--no-edit"]),
            Some(GitOperation::Rebase) => args(&["rebase", "--continue"]),
            Some(GitOperation::CherryPick) => args(&["cherry-pick", "--continue"]),
            Some(GitOperation::Revert) => args(&["revert", "--continue"]),
            None => return,
        };
        self.git(list);
    }

    fn git_abort(self: &Rc<Self>) {
        let verb = match self.git_operation() {
            Some(GitOperation::Merge) => "merge",
            Some(GitOperation::Rebase) => "rebase",
            Some(GitOperation::CherryPick) => "cherry-pick",
            Some(GitOperation::Revert) => "revert",
            None => return,
        };
        if confirm(&t!("git.abort-confirm")) {
            self.git(args(&[verb, "--abort"]));
        }
    }

    // ---- the working tree's diff ------------------------------------------

    /// One working-tree file's diff, on the Changes page.
    fn git_show_diff(self: &Rc<Self>, path: String, staged: bool) {
        {
            let mut state = self.state.borrow_mut();
            state.git.diff_for = Some((path, staged));
            state.git.diff = None;
        }
        self.open_tab(Tab::Page("gitdiff"));
        self.git_diff_again();
    }

    /// The diff showing read again — staging moved the side it is of.
    fn git_diff_again(self: &Rc<Self>) {
        let Some(root) = self.root() else { return };
        let (asked, untracked) = {
            let state = self.state.borrow();
            let Some(asked) = state.git.diff_for.clone() else {
                return;
            };
            let untracked = state
                .git
                .status
                .as_ref()
                .and_then(|s| s.entries.iter().find(|e| e.path == asked.0))
                .is_some_and(|e| e.untracked);
            (asked, untracked)
        };
        let (path, staged) = asked.clone();
        work::spawn(
            move || {
                rusty_git::repo::diff_file(&root, &path, staged, untracked)
                    .unwrap_or_else(|error| error.to_string())
            },
            move |app, text| {
                {
                    let mut state = app.state.borrow_mut();
                    if state.git.diff_for.as_ref() != Some(&asked) {
                        return;
                    }
                    state.git.diff = Some(text);
                }
                app.sync_git();
            },
        );
    }

    fn git_forget_diff(&self, path: &str) {
        let mut state = self.state.borrow_mut();
        if state
            .git
            .diff_for
            .as_ref()
            .is_some_and(|(shown, _)| shown == path)
        {
            state.git.diff_for = None;
            state.git.diff = None;
        }
    }

    // ---- the history -------------------------------------------------------

    /// A row of the log clicked: its commit opened below.
    pub(super) fn git_pick(self: &Rc<Self>, index: usize) {
        let id = {
            let state = self.state.borrow();
            state
                .git
                .history
                .as_ref()
                .and_then(|h| h.rows.get(index))
                .map(|row| row.commit.id.clone())
        };
        if let Some(id) = id {
            self.git_select(id, false);
        }
    }

    /// Open a commit: from the cache when it was opened before (a full hash
    /// names one content for ever), read otherwise.
    fn git_select(self: &Rc<Self>, id: String, reveal: bool) {
        let cached = {
            let mut state = self.state.borrow_mut();
            let git = &mut state.git;
            git.selected = Some(id.clone());
            if reveal {
                git.reveal += 1;
            }
            git.cache.iter().find(|d| d.commit.id == id).cloned()
        };
        if let Some(detail) = cached {
            self.git_detail(detail);
            return;
        }
        self.sync_git();
        let Some(root) = self.root() else { return };
        work::spawn(
            move || rusty_git::repo::commit(&root, &id).map_err(|e| e.to_string()),
            |app, detail| match detail {
                Ok(detail) => {
                    {
                        let mut state = app.state.borrow_mut();
                        let git = &mut state.git;
                        git.cache.retain(|d| d.commit.id != detail.commit.id);
                        git.cache.push(detail.clone());
                        if git.cache.len() > CACHED {
                            git.cache.remove(0);
                        }
                        if git.selected.as_deref() != Some(&detail.commit.id) {
                            return;
                        }
                    }
                    app.git_detail(detail);
                }
                Err(why) => app.note(1, why),
            },
        );
    }

    fn git_detail(self: &Rc<Self>, detail: CommitDetail) {
        {
            let mut state = self.state.borrow_mut();
            let git = &mut state.git;
            // The file shown stays shown when the next commit touched it
            // too; otherwise the first file is.
            let keep = git
                .file
                .as_ref()
                .is_some_and(|f| detail.files.iter().any(|c| &c.path == f));
            if !keep {
                git.file = detail.files.first().map(|f| f.path.clone());
            }
            git.detail = Some(detail);
        }
        self.sync_git();
    }

    pub(super) fn git_detail_file(self: &Rc<Self>, index: usize) {
        {
            let mut state = self.state.borrow_mut();
            let path = state
                .git
                .detail
                .as_ref()
                .and_then(|d| d.files.get(index))
                .map(|f| f.path.clone());
            if path.is_none() {
                return;
            }
            state.git.file = path;
        }
        self.sync_git();
    }

    /// Up or down the log from the commit opened.
    pub(super) fn git_step(self: &Rc<Self>, step: i32) {
        let id = {
            let state = self.state.borrow();
            let Some(history) = state.git.history.as_ref() else {
                return;
            };
            let at = state
                .git
                .selected
                .as_ref()
                .and_then(|id| history.rows.iter().position(|r| &r.commit.id == id));
            let next = match at {
                Some(at) => at as i64 + i64::from(step),
                None => 0,
            };
            let next = next.clamp(0, history.rows.len() as i64 - 1);
            history.rows.get(next as usize).map(|r| r.commit.id.clone())
        };
        if let Some(id) = id {
            self.git_select(id, true);
        }
    }

    pub(super) fn git_search(self: &Rc<Self>, text: &str) {
        self.state.borrow_mut().git.query = text.to_string();
        self.sync_git();
    }

    /// The next or the previous commit the search matches, opened.
    pub(super) fn git_search_step(self: &Rc<Self>, forward: bool) {
        let id = {
            let state = self.state.borrow();
            let Some(history) = state.git.history.as_ref() else {
                return;
            };
            let hits = rusty_git::log::hits(&history.rows, &state.git.query);
            let at = state
                .git
                .selected
                .as_ref()
                .and_then(|id| history.rows.iter().position(|r| &r.commit.id == id));
            rusty_git::log::step_hit(&hits, at, forward)
                .and_then(|hit| history.rows.get(hit))
                .map(|r| r.commit.id.clone())
        };
        if let Some(id) = id {
            self.git_select(id, true);
        }
    }

    fn git_reveal_head(self: &Rc<Self>) {
        let id = {
            let state = self.state.borrow();
            state
                .git
                .history
                .as_ref()
                .and_then(|h| {
                    h.rows.iter().find(|r| {
                        r.commit
                            .refs
                            .iter()
                            .any(|l| l.kind == rusty_git::RefKind::Head)
                    })
                })
                .map(|r| r.commit.id.clone())
        };
        if let Some(id) = id {
            self.git_select(id, true);
        }
    }

    /// A ref in the sidebar clicked: its commit; double-clicked: a branch
    /// checked out.
    pub(super) fn git_ref(self: &Rc<Self>, index: usize, double: bool) {
        let Some(target) = self.git_ref_target(index) else {
            return;
        };
        match (target, double) {
            (Target::Branch { name, remote, .. }, true) => self.git_checkout(name, remote),
            (Target::Branch { name, .. }, false) => {
                let id = {
                    let state = self.state.borrow();
                    state
                        .git
                        .refs
                        .branches
                        .iter()
                        .find(|b| b.name == name)
                        .map(|b| b.id.clone())
                };
                if let Some(id) = id {
                    self.git_select(id, true);
                }
            }
            (Target::Tag { id, .. }, _) => self.git_select(id, true),
            (Target::Remote { .. }, _) => {}
            _ => {}
        }
    }

    pub(super) fn git_ref_filter(self: &Rc<Self>, text: &str) {
        self.state.borrow_mut().git.ref_filter = text.to_string();
        self.sync_git();
    }

    /// Check a branch out; a remote one through a local branch of its name.
    fn git_checkout(self: &Rc<Self>, name: String, remote: bool) {
        let (local, exists) = {
            let state = self.state.borrow();
            let branches = &state.git.refs.branches;
            let local = branches
                .iter()
                .find(|b| b.name == name)
                .map(|b| b.local_name().to_string())
                .unwrap_or_else(|| name.clone());
            let exists = branches.iter().any(|b| !b.remote && b.name == local);
            (local, exists)
        };
        let list = checkout_args(&name, remote, &local, exists);
        self.git_then(list, |app, _| {
            app.state.borrow_mut().git.rev = None;
            app.after_git();
        });
    }

    // ---- the name field ----------------------------------------------------

    /// Open the field for a branch, a tag or a remote — filled with what is
    /// being changed, or with `origin` for a first remote.
    fn git_prompt(self: &Rc<Self>, kind: PromptKind) {
        let value = {
            let state = self.state.borrow();
            let remotes = &state.git.remotes;
            match &kind {
                PromptKind::Rename { from } | PromptKind::RenameRemote { from } => from.clone(),
                PromptKind::Remote { .. } if !remotes.iter().any(|r| r.name == "origin") => {
                    "origin".to_string()
                }
                PromptKind::RemoteUrl { name } => remotes
                    .iter()
                    .find(|r| &r.name == name)
                    .map(|r| r.url.clone())
                    .unwrap_or_default(),
                _ => String::new(),
            }
        };
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_git_prompt_value(value.clone().into());
            shell.set_git_prompt_url("".into());
        }
        self.state.borrow_mut().git.prompt = Some(RefPrompt {
            kind,
            value,
            url: String::new(),
        });
        self.sync_git();
    }

    pub(super) fn git_prompt_edited(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        if let Some(prompt) = self.state.borrow_mut().git.prompt.as_mut() {
            prompt.value = shell.get_git_prompt_value().to_string();
            prompt.url = shell.get_git_prompt_url().to_string();
        }
        self.sync_git();
    }

    /// The field carried out — never when git would refuse what is in it —
    /// or put away.
    pub(super) fn git_prompt_done(self: &Rc<Self>, ok: bool) {
        self.git_prompt_edited();
        let prompt = self.state.borrow_mut().git.prompt.take();
        self.sync_git();
        let Some(prompt) = prompt.filter(|_| ok) else {
            return;
        };
        let remotes = self.state.borrow().git.remotes.clone();
        if prompt.problem(&remotes).is_some() {
            self.state.borrow_mut().git.prompt = Some(prompt);
            self.sync_git();
            return;
        }
        let name = prompt.value.trim().to_string();
        match prompt.kind {
            PromptKind::Branch { from } => {
                let mut list = args(&["checkout", "-b", &name]);
                list.extend(from);
                self.git_then(list, |app, _| {
                    app.state.borrow_mut().git.rev = None;
                    app.after_git();
                });
            }
            PromptKind::Rename { from } => {
                if from != name {
                    self.git(args(&["branch", "-m", &from, &name]));
                }
            }
            PromptKind::Tag { at } => self.git(args(&["tag", &name, &at])),
            PromptKind::Remote { push } => {
                let url = prompt.url.trim().to_string();
                self.git_then(
                    args(&["remote", "add", "--", &name, &url]),
                    move |app, ended| {
                        app.after_git();
                        // The remote list is being read again and will not
                        // have arrived yet — so the push is told where to go.
                        if push && ended == crate::run::Ended::Done {
                            app.git_push_to(name);
                        }
                    },
                );
            }
            PromptKind::RenameRemote { from } => {
                if from != name {
                    self.git(args(&["remote", "rename", "--", &from, &name]));
                }
            }
            PromptKind::RemoteUrl { name: remote } => {
                self.git(args(&["remote", "set-url", "--", &remote, &name]));
            }
        }
    }

    // ---- the right-click menus -----------------------------------------------

    pub(super) fn git_ref_target(&self, index: usize) -> Option<Target> {
        let state = self.state.borrow();
        super::git_view::ref_rows(&state.git)
            .into_iter()
            .nth(index)
            .and_then(|row| row.target)
    }

    /// A right-click on a file of the two lists.
    pub(super) fn git_change_menu(self: &Rc<Self>, path: &str, staged: bool, x: f32, y: f32) {
        let entry = {
            let state = self.state.borrow();
            state
                .git
                .status
                .as_ref()
                .and_then(|s| s.entries.iter().find(|e| e.path == path).cloned())
        };
        let Some(entry) = entry else { return };
        let target = Target::Change {
            path: entry.path,
            staged,
            untracked: entry.untracked,
            nested: entry.nested,
        };
        self.git_menu(target, x, y);
    }

    pub(super) fn git_ref_menu(self: &Rc<Self>, index: i32, x: f32, y: f32) {
        let target = usize::try_from(index)
            .ok()
            .and_then(|index| self.git_ref_target(index));
        if let Some(target) = target {
            self.git_menu(target, x, y);
        }
    }

    pub(super) fn git_commit_menu(self: &Rc<Self>, index: i32, x: f32, y: f32) {
        let target = {
            let state = self.state.borrow();
            usize::try_from(index)
                .ok()
                .and_then(|at| state.git.history.as_ref()?.rows.get(at).cloned())
                .map(|row| Target::Commit {
                    id: row.commit.id,
                    short: row.commit.short,
                })
        };
        if let Some(target) = target {
            self.git_menu(target, x, y);
        }
    }

    /// A right-click on a file of the opened commit.
    pub(super) fn git_file_menu(self: &Rc<Self>, index: i32, x: f32, y: f32) {
        let target = {
            let state = self.state.borrow();
            usize::try_from(index)
                .ok()
                .and_then(|at| state.git.detail.as_ref()?.files.get(at).cloned())
                .map(|file| Target::Path { path: file.path })
        };
        if let Some(target) = target {
            self.git_menu(target, x, y);
        }
    }

    /// A right-click: what it was on, and the menu for that.
    pub(super) fn git_menu(self: &Rc<Self>, target: Target, x: f32, y: f32) {
        let items = self.git_menu_items(&target);
        self.state.borrow_mut().git.menu = Some(target);
        self.open_menu("git", items, x, y);
    }

    fn git_menu_items(&self, target: &Target) -> Vec<(String, String, bool)> {
        let item = |id: &str, label: String| (id.to_string(), label, false);
        let danger = |id: &str, label: String| (id.to_string(), label, true);
        let divider = || (String::new(), String::new(), false);
        let current = {
            let state = self.state.borrow();
            state
                .git
                .refs
                .branches
                .iter()
                .find(|b| b.current)
                .map(|b| b.name.clone())
                .unwrap_or_default()
        };
        match target {
            Target::Branch {
                remote,
                current: is_current,
                ..
            } => {
                let mut items = Vec::new();
                if !is_current {
                    items.push(item(
                        "checkout",
                        if *remote {
                            t!("git.checkout-remote")
                        } else {
                            t!("git.checkout")
                        },
                    ));
                    if !current.is_empty() {
                        items.push(item(
                            "merge",
                            t!("git.merge-into", current = current.clone()),
                        ));
                        items.push(item(
                            "rebase",
                            t!("git.rebase-onto", current = current.clone()),
                        ));
                    }
                    items.push(divider());
                }
                items.push(item("branch-from", t!("git.branch-from")));
                if !remote {
                    items.push(item("rename", t!("git.rename-branch")));
                    items.push(item("push", t!("git.push-branch")));
                }
                items.push(item("filter", t!("git.filter-branch")));
                items.push(item("copy-name", t!("git.copy-name")));
                if !is_current {
                    items.push(divider());
                    if *remote {
                        items.push(danger("delete-remote", t!("git.delete-remote")));
                    } else {
                        items.push(danger("delete", t!("git.delete-branch")));
                    }
                }
                items
            }
            Target::Tag { .. } => vec![
                item("checkout", t!("git.checkout-tag")),
                item("branch-from", t!("git.branch-from")),
                item("push", t!("git.push-tag")),
                item("copy-name", t!("git.copy-name")),
                divider(),
                danger("delete", t!("git.delete-tag")),
            ],
            Target::Remote { .. } => vec![
                item("fetch", t!("git.remote-fetch")),
                item("url", t!("git.remote-edit-url")),
                item("rename", t!("git.remote-rename")),
                item("copy-url", t!("git.remote-copy-url")),
                divider(),
                danger("remove", t!("git.remote-remove")),
            ],
            Target::Commit { .. } => vec![
                item("copy-hash", t!("git.copy-hash")),
                item("copy-short", t!("git.copy-short-hash")),
                item("copy-message", t!("git.copy-message")),
                divider(),
                item("branch-here", t!("git.branch-here")),
                item("tag-here", t!("git.tag-here")),
                item("checkout", t!("git.checkout-commit")),
                divider(),
                item("cherry-pick", t!("git.cherry-pick")),
                item("revert", t!("git.revert")),
            ],
            Target::Path { .. } => vec![
                item("open", t!("git.open-file")),
                item("copy", t!("git.copy-path")),
                item("copy-full", t!("git.copy-full-path")),
            ],
            Target::Change {
                staged,
                untracked,
                nested,
                ..
            } => {
                let mut items = Vec::new();
                if *nested {
                    items.push(item("include", t!("git.include-nested")));
                } else if *staged {
                    items.push(item("unstage", t!("git.unstage")));
                } else {
                    items.push(item("stage", t!("git.stage")));
                }
                items.push(item("open", t!("git.open-file")));
                items.push(item("stage-all", t!("git.stage-all")));
                items.push(item("stash", t!("git.stash-file")));
                items.push(item("copy", t!("git.copy-path")));
                items.push(item("copy-full", t!("git.copy-full-path")));
                items.push(divider());
                items.push(danger(
                    "discard",
                    if *untracked {
                        t!("git.delete-untracked")
                    } else {
                        t!("git.discard")
                    },
                ));
                items
            }
            Target::Diff { which, side } => self.diff_menu_items(*which, *side),
            Target::Text { selected, .. } => App::text_menu_items(selected),
        }
    }

    /// A row of a Git menu picked.
    pub(super) fn git_menu_picked(self: &Rc<Self>, id: &str) {
        let Some(target) = self.state.borrow_mut().git.menu.take() else {
            return;
        };
        match target {
            Target::Branch { name, remote, .. } => match id {
                "checkout" => self.git_checkout(name, remote),
                "merge" => self.git(args(&["merge", "--no-edit", &name])),
                "rebase" => {
                    let current = self
                        .state
                        .borrow()
                        .git
                        .refs
                        .branches
                        .iter()
                        .find(|b| b.current)
                        .map(|b| b.name.clone())
                        .unwrap_or_default();
                    if confirm(&t!(
                        "git.rebase-confirm",
                        current = current,
                        onto = name.clone()
                    )) {
                        self.git(args(&["rebase", &name]));
                    }
                }
                "branch-from" => self.git_prompt(PromptKind::Branch { from: Some(name) }),
                "rename" => self.git_prompt(PromptKind::Rename { from: name }),
                "push" => self.git_push_branch(name),
                "filter" => {
                    self.state.borrow_mut().git.rev = Some(name);
                    self.git_history();
                }
                "copy-name" => copy_text(&name),
                "delete" => self.git_then(args(&["branch", "-d", &name]), move |app, _| {
                    {
                        let mut state = app.state.borrow_mut();
                        if state.git.rev.as_deref() == Some(name.as_str()) {
                            state.git.rev = None;
                        }
                    }
                    app.after_git();
                }),
                "delete-remote" => self.git_delete_remote_branch(name),
                _ => {}
            },
            Target::Tag { name, .. } => match id {
                "checkout" => self.git(args(&["checkout", "--detach", &name])),
                "branch-from" => self.git_prompt(PromptKind::Branch { from: Some(name) }),
                "push" => match self.git_default_remote() {
                    Some(remote) => {
                        self.git(args(&["push", &remote, &format!("refs/tags/{name}")]))
                    }
                    None => self.git_prompt(PromptKind::Remote { push: false }),
                },
                "copy-name" => copy_text(&name),
                "delete" if confirm(&t!("git.delete-tag-confirm", name = name.clone())) => {
                    self.git(args(&["tag", "-d", &name]));
                }
                _ => {}
            },
            Target::Remote { name } => match id {
                "fetch" => self.git(args(&["fetch", "--prune", "--", &name])),
                "url" => self.git_prompt(PromptKind::RemoteUrl { name }),
                "rename" => self.git_prompt(PromptKind::RenameRemote { from: name }),
                "copy-url" => {
                    let url = self
                        .state
                        .borrow()
                        .git
                        .remotes
                        .iter()
                        .find(|r| r.name == name)
                        .map(|r| r.url.clone());
                    if let Some(url) = url {
                        copy_text(&url);
                    }
                }
                "remove" if confirm(&t!("git.remote-remove-confirm", name = name.clone())) => {
                    self.git_then(args(&["remote", "remove", "--", &name]), move |app, _| {
                        {
                            let mut state = app.state.borrow_mut();
                            let prefix = format!("{name}/");
                            if state
                                .git
                                .rev
                                .as_ref()
                                .is_some_and(|r| r.starts_with(&prefix))
                            {
                                state.git.rev = None;
                            }
                        }
                        app.after_git();
                    });
                }
                _ => {}
            },
            Target::Commit { id: commit, short } => match id {
                "copy-hash" => copy_text(&commit),
                "copy-short" => copy_text(&short),
                "copy-message" => self.git_copy_message(commit),
                "branch-here" => self.git_prompt(PromptKind::Branch { from: Some(commit) }),
                "tag-here" => self.git_prompt(PromptKind::Tag { at: commit }),
                "checkout" => self.git(args(&["checkout", "--detach", &commit])),
                "cherry-pick" => self.git(args(&["cherry-pick", &commit])),
                "revert" => self.git(args(&["revert", "--no-edit", &commit])),
                _ => {}
            },
            Target::Path { path } => match id {
                "open" => self.open_file(path),
                "copy" => copy_text(&path),
                "copy-full" => {
                    if let Some(root) = self.root() {
                        copy_text(&root.join(&path).to_string_lossy());
                    }
                }
                _ => {}
            },
            Target::Change { path, staged, .. } => {
                if id == "stage-all" {
                    self.git_stage_all(true);
                } else {
                    self.git_file(id, path, staged);
                }
            }
            Target::Diff { which, side } => {
                self.diff_menu_picked(which, side, id);
                self.diff_refocus(which);
            }
            Target::Text {
                which,
                selected,
                id: commit,
                short,
                message,
            } => match id {
                "copy-message" => copy_text(&message),
                "copy-hash" => copy_text(&commit),
                "copy-short" => copy_text(&short),
                _ => self.text_menu_picked(id, &which, &selected),
            },
        }
    }

    /// A Git menu put away with nothing picked: a diff's keys back to it.
    pub(super) fn git_menu_dismissed(self: &Rc<Self>) {
        let target = self.state.borrow_mut().git.menu.take();
        if let Some(Target::Diff { which, .. }) = target {
            self.diff_refocus(which);
        }
    }

    /// A commit's whole message onto the clipboard: from what was read of
    /// it when it was opened, or read now — the log holds only subjects.
    fn git_copy_message(self: &Rc<Self>, id: String) {
        let known = {
            let state = self.state.borrow();
            let git = &state.git;
            git.detail
                .iter()
                .chain(git.cache.iter())
                .find(|d| d.commit.id == id)
                .map(super::git_view::message_of)
        };
        if let Some(message) = known {
            copy_text(&message);
            return;
        }
        let Some(root) = self.root() else { return };
        work::spawn(
            move || rusty_git::repo::commit(&root, &id).map_err(|e| e.to_string()),
            |app, detail| match detail {
                Ok(detail) => {
                    copy_text(&super::git_view::message_of(&detail));
                    let mut state = app.state.borrow_mut();
                    let git = &mut state.git;
                    git.cache.retain(|d| d.commit.id != detail.commit.id);
                    git.cache.push(detail);
                    if git.cache.len() > CACHED {
                        git.cache.remove(0);
                    }
                }
                Err(why) => app.note(1, why),
            },
        );
    }

    /// Push a local branch that is not the one checked out.
    fn git_push_branch(self: &Rc<Self>, name: String) {
        let upstream = self
            .state
            .borrow()
            .git
            .refs
            .branches
            .iter()
            .find(|b| !b.remote && b.name == name)
            .and_then(|b| b.upstream.clone());
        let owner = upstream
            .as_deref()
            .and_then(|upstream| pick_remote(Some(upstream), &self.git_known_remotes()));
        let list = match owner {
            Some(remote) => args(&["push", &remote, &name]),
            None => match self.git_default_remote() {
                Some(remote) => args(&["push", "-u", &remote, &name]),
                None => return self.git_prompt(PromptKind::Remote { push: false }),
            },
        };
        self.git(list);
    }

    /// Delete a branch on its remote — everybody's copy, so it asks.
    fn git_delete_remote_branch(self: &Rc<Self>, name: String) {
        let (remote, branch) = {
            let state = self.state.borrow();
            state
                .git
                .refs
                .branches
                .iter()
                .find(|b| b.name == name)
                .map(|b| {
                    (
                        b.remote_name.clone().unwrap_or_else(|| "origin".into()),
                        b.local_name().to_string(),
                    )
                })
                .unwrap_or_else(|| ("origin".into(), name.clone()))
        };
        if confirm(&t!("git.delete-remote-confirm", name = name)) {
            self.git(args(&["push", &remote, "--delete", &branch]));
        }
    }
}

/// Text onto the clipboard.
pub(super) fn copy_text(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second ask while one read runs is one more run afterwards, however
    /// many times it is asked; a different read is its own.
    #[test]
    fn a_read_asked_for_while_running_runs_once_more_and_no_more() {
        let mut gate = Gate::default();
        assert!(gate.begin(Read::Status));
        assert!(!gate.begin(Read::Status));
        assert!(!gate.begin(Read::Status));
        assert!(gate.begin(Read::History), "another read is not held up");
        assert!(gate.finish(Read::Status), "asked again while it ran");
        assert!(gate.begin(Read::Status));
        assert!(!gate.finish(Read::Status), "and not again after that");
        assert!(!gate.finish(Read::History));
    }

    /// The dock's line quotes what a shell would need quoted, and a message
    /// of many lines shows its first.
    #[test]
    fn the_dock_line_quotes_what_a_shell_would_split() {
        assert_eq!(shown("main"), "main");
        assert_eq!(shown("fix the thing"), "\"fix the thing\"");
        assert_eq!(shown("first\nsecond"), "\"first…\"");
        assert_eq!(shown(""), "\"\"");
    }
}
