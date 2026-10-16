//! What the Git panel and the History page draw, from what `git.rs` read:
//! the two file lists, the refs down the left, the graph's rows with their
//! lines and labels, an opened commit, and a patch as rows either way.

use rusty_git::{
    ChangeKind, CommitDetail, GitOperation, RefKind, RefNameProblem, RemoteProblem, StatusEntry,
    diff::CellKind,
    log::{edge_line, hits, matches},
    plan::{PromptKind, PromptProblem},
};
use rusty_i18n::t;
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

use super::{
    App,
    git::{Git, Target},
};
use crate::{
    DiffRow, GitChange, GitCommitRow, GitFile, GitLabel, GitLane, GitRef, GitStash, Shell, format,
};

/// Fork's palette, near enough: distinct at two pixels, legible on both
/// themes, and the same in every theme — a lane that changed colour with
/// the theme would read as a different branch.
const LANES: [u32; 8] = [
    0x4f8ef7, 0xe0a33b, 0x3fbf7f, 0xd56fd0, 0xf0715b, 0x5cc8d6, 0xa98cff, 0xc7b34a,
];
const LANE_PX: f64 = 14.0;
const ROW_PX: f64 = 26.0;
/// Labels on a row before the rest are counted.
const LABELS: usize = 3;
/// Rows of a patch drawn before the rest are counted.
const DIFF_ROWS: usize = 4000;

fn rgb(value: u32) -> Color {
    Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn lane_colour(lane: u32) -> Color {
    rgb(LANES[lane as usize % LANES.len()])
}

/// A row of the refs list, and what a click on it is about.
pub(super) struct RefRow {
    /// 0 a heading, 1 a local branch, 2 a remote, 3 a remote's branch, 4 a tag.
    kind: i32,
    label: String,
    detail: String,
    tip: String,
    current: bool,
    gone: bool,
    pub(super) target: Option<Target>,
}

/// The refs down the left: local branches, each remote with its branches,
/// the tags — filtered by what is typed, headings kept.
pub(super) fn ref_rows(git: &Git) -> Vec<RefRow> {
    let needle = git.ref_filter.trim().to_lowercase();
    let keep = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
    let heading = |label: String| RefRow {
        kind: 0,
        label,
        detail: String::new(),
        tip: String::new(),
        current: false,
        gone: false,
        target: None,
    };
    let mut rows = vec![heading(t!("git.local"))];
    for branch in git.refs.branches.iter().filter(|b| !b.remote) {
        if !keep(&branch.name) {
            continue;
        }
        let mut detail = Vec::new();
        if branch.ahead > 0 {
            detail.push(format!("↑{}", branch.ahead));
        }
        if branch.behind > 0 {
            detail.push(format!("↓{}", branch.behind));
        }
        if branch.gone {
            detail.push(t!("git.gone"));
        }
        rows.push(RefRow {
            kind: 1,
            label: branch.name.clone(),
            detail: detail.join(" "),
            tip: branch.subject.clone(),
            current: branch.current,
            gone: branch.gone,
            target: Some(Target::Branch {
                name: branch.name.clone(),
                remote: false,
                current: branch.current,
            }),
        });
    }
    rows.push(heading(t!("git.remotes")));
    // Every remote the config names, then any its branches name that the
    // config does not — branches left behind by a removed remote.
    let mut remotes: Vec<String> = git.remotes.iter().map(|r| r.name.clone()).collect();
    for name in git
        .refs
        .branches
        .iter()
        .filter_map(|b| b.remote_name.clone())
    {
        if !remotes.contains(&name) {
            remotes.push(name);
        }
    }
    for remote in remotes {
        let branches: Vec<_> = git
            .refs
            .branches
            .iter()
            .filter(|b| b.remote && b.remote_name.as_deref() == Some(remote.as_str()))
            .filter(|b| keep(&b.name))
            .collect();
        let configured = git.remotes.iter().find(|r| r.name == remote);
        rows.push(RefRow {
            kind: 2,
            label: remote.clone(),
            detail: if branches.is_empty() && needle.is_empty() {
                t!("git.remote-empty")
            } else {
                String::new()
            },
            tip: match configured {
                Some(r) => t!(
                    "git.remote-urls",
                    fetch = r.url.clone(),
                    push = r.push_url.clone().unwrap_or_else(|| r.url.clone())
                ),
                None => t!("git.remote-unconfigured"),
            },
            current: false,
            gone: configured.is_none(),
            target: Some(Target::Remote {
                name: remote.clone(),
            }),
        });
        for branch in branches {
            rows.push(RefRow {
                kind: 3,
                label: branch.local_name().to_string(),
                detail: String::new(),
                tip: branch.subject.clone(),
                current: false,
                gone: false,
                target: Some(Target::Branch {
                    name: branch.name.clone(),
                    remote: true,
                    current: false,
                }),
            });
        }
    }
    rows.push(heading(t!("git.tags")));
    for tag in git.refs.tags.iter().filter(|t| keep(&t.name)) {
        rows.push(RefRow {
            kind: 4,
            label: tag.name.clone(),
            detail: String::new(),
            tip: tag.subject.clone(),
            current: false,
            gone: false,
            target: Some(Target::Tag {
                name: tag.name.clone(),
                id: tag.id.clone(),
            }),
        });
    }
    rows
}

/// The letter a file list shows for a change, its colour (0 grey, 1 green,
/// 2 amber, 3 crimson, 4 slate) and what it means.
fn glyph(
    kind: Option<ChangeKind>,
    untracked: bool,
    conflicted: bool,
) -> (&'static str, i32, String) {
    if conflicted {
        return ("!", 3, t!("git.kind-conflicted"));
    }
    if untracked {
        return ("U", 1, t!("git.kind-untracked"));
    }
    match kind {
        Some(ChangeKind::Added) => ("A", 1, t!("git.kind-added")),
        Some(ChangeKind::Modified) => ("M", 2, t!("git.kind-modified")),
        Some(ChangeKind::Deleted) => ("D", 3, t!("git.kind-deleted")),
        Some(ChangeKind::Renamed) => ("R", 4, t!("git.kind-renamed")),
        Some(ChangeKind::Other) | None => ("·", 0, t!("git.kind-other")),
    }
}

fn change_row(entry: &StatusEntry, staged: bool, chosen: Option<&(String, bool)>) -> GitChange {
    let kind = if staged { entry.staged } else { entry.unstaged };
    let (letter, tone, tip) = glyph(kind, entry.untracked && !staged, entry.conflicted);
    let path = entry.path.trim_end_matches('/');
    let (dir, name) = match path.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", path),
    };
    GitChange {
        path: entry.path.as_str().into(),
        name: name.into(),
        dir: dir.into(),
        letter: letter.into(),
        tone,
        tip: if entry.nested {
            t!("git.nested-hint").into()
        } else {
            tip.into()
        },
        staged,
        untracked: entry.untracked,
        nested: entry.nested,
        selected: chosen.is_some_and(|(p, s)| p == &entry.path && *s == staged),
    }
}

/// Each row's text on each side as the patch has it — the left column's
/// (or the one column's) and the right's — `None` where the row is no line
/// of that side: a header, a note, a blank (`crate::diff_select`).
pub(super) type RowTexts = Vec<[Option<String>; 2]>;

/// A patch as rows, one column or two, what to say when there are none, and
/// the text a selection in them selects.
pub(super) fn diff_rows(patch: &str, split: bool) -> (Vec<DiffRow>, String, RowTexts) {
    let hunks = rusty_git::diff::hunks(patch);
    if hunks.is_empty() {
        let note = if patch.contains("Binary files") || patch.contains("GIT binary patch") {
            t!("git.binary")
        } else {
            t!("desk.git-no-diff")
        };
        return (Vec::new(), note, Vec::new());
    }
    let text = |s: &str| SharedString::from(crate::diff_select::drawn(s));
    // A cell's text, when it is a line of its side: a note is not.
    let line_of =
        |cell: &rusty_git::diff::Cell| (cell.kind != CellKind::Note).then(|| cell.text.clone());
    let kind = |k: CellKind| match k {
        CellKind::Context => 0,
        CellKind::Added => 1,
        CellKind::Removed => 2,
        CellKind::Note => 4,
    };
    let number = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
    let mut rows = Vec::new();
    let mut texts: RowTexts = Vec::new();
    let mut total = 0;
    for hunk in &hunks {
        total += 1 + if split {
            hunk.rows.len()
        } else {
            hunk.lines.len()
        };
        if rows.len() >= DIFF_ROWS {
            continue;
        }
        rows.push(DiffRow {
            kind: 3,
            text: text(&hunk.header),
            ..DiffRow::default()
        });
        texts.push([None, None]);
        if split {
            for row in &hunk.rows {
                let mut out = DiffRow::default();
                match &row.left {
                    Some(cell) => {
                        out.kind = kind(cell.kind);
                        out.old = number(cell.number).into();
                        out.text = text(&cell.text);
                    }
                    None => out.kind = 5,
                }
                match &row.right {
                    Some(cell) => {
                        out.right_kind = kind(cell.kind);
                        out.new = number(cell.number).into();
                        out.right_text = text(&cell.text);
                    }
                    None => out.right_kind = 5,
                }
                rows.push(out);
                texts.push([
                    row.left.as_ref().and_then(line_of),
                    row.right.as_ref().and_then(line_of),
                ]);
            }
        } else {
            for line in &hunk.lines {
                rows.push(DiffRow {
                    kind: kind(line.kind),
                    old: number(line.old).into(),
                    new: number(line.new).into(),
                    text: text(&line.text),
                    ..DiffRow::default()
                });
                texts.push([
                    (line.kind != CellKind::Note).then(|| line.text.clone()),
                    None,
                ]);
            }
        }
    }
    rows.truncate(DIFF_ROWS);
    texts.truncate(DIFF_ROWS);
    let note = if total > rows.len() {
        t!("git.diff-clipped", shown = rows.len(), count = total)
    } else {
        String::new()
    };
    (rows, note, texts)
}

/// The opened commit's three pieces of text, as the page shows them and its
/// menu copies them: the subject; who wrote it, when, and its whole hash;
/// and the body under the subject. One function, so the bytes a selection
/// in the page names are the bytes the menu slices.
pub(super) fn detail_texts(detail: &CommitDetail) -> [String; 3] {
    let commit = &detail.commit;
    let body = detail
        .body
        .split_once('\n')
        .map(|(_, rest)| rest.trim())
        .unwrap_or_default();
    [
        commit.summary.clone(),
        format!(
            "{} <{}>\n{} · {}",
            commit.author,
            commit.email,
            format::full_time(commit.time),
            commit.id
        ),
        body.to_string(),
    ]
}

/// A commit's whole message, as `git log --format=%B` gives it but for the
/// blank lines at its end.
pub(super) fn message_of(detail: &CommitDetail) -> String {
    detail.body.trim_end().to_string()
}

fn problem_text(problem: PromptProblem) -> String {
    let name = |p: RefNameProblem| match p {
        RefNameProblem::Empty => t!("git.name-empty"),
        RefNameProblem::Whitespace => t!("git.name-whitespace"),
        RefNameProblem::Character(c) => t!("git.name-character", char = c.to_string()),
        RefNameProblem::Dash => t!("git.name-dash"),
        RefNameProblem::Dot => t!("git.name-dot"),
        RefNameProblem::Slash => t!("git.name-slash"),
        RefNameProblem::Lock => t!("git.name-lock"),
        RefNameProblem::Reserved => t!("git.name-reserved"),
    };
    match problem {
        PromptProblem::Name(p) | PromptProblem::Remote(RemoteProblem::Name(p)) => name(p),
        PromptProblem::Remote(RemoteProblem::Exists) => t!("git.remote-exists"),
        PromptProblem::Remote(RemoteProblem::UrlEmpty) => t!("git.url-empty"),
        PromptProblem::Remote(RemoteProblem::UrlControl) => t!("git.url-control"),
        PromptProblem::Remote(RemoteProblem::UrlDash) => t!("git.url-dash"),
    }
}

fn short(id: &str) -> String {
    id.chars().take(7).collect()
}

impl App {
    /// Both surfaces, from the state.
    pub(super) fn sync_git(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        // The two diffs, kept while what they were made from is the same, so
        // a selection in one outlives every read that changes nothing there.
        self.sync_diff_views(&window);
        let state = self.state.borrow();
        let git = &state.git;

        // Where the repository stands.
        shell.set_git_state(if git.root.is_none() || !git.loaded {
            0
        } else if git.not_repo {
            2
        } else if git.unavailable.is_some() {
            3
        } else {
            1
        });
        shell.set_git_problem(git.unavailable.clone().unwrap_or_default().into());
        let status = git.status.as_ref();
        let head = match status {
            Some(s) if s.detached => t!("git.detached"),
            Some(s) => s.head.clone().unwrap_or_default(),
            None => String::new(),
        };
        shell.set_git_head(head.into());
        let (ahead, behind) = status.map(|s| (s.ahead, s.behind)).unwrap_or_default();
        shell.set_git_ahead(ahead as i32);
        shell.set_git_behind(behind as i32);
        let upstream = match status.and_then(|s| s.upstream.clone()) {
            Some(upstream) => t!(
                "git.ahead-behind",
                ahead = ahead,
                behind = behind,
                upstream = upstream
            ),
            None => t!("git.no-upstream"),
        };
        shell.set_git_upstream(upstream.into());
        shell.set_git_pull_tip(
            if behind > 0 {
                t!("git.pull-count", count = behind)
            } else {
                t!("git.pull")
            }
            .into(),
        );
        shell.set_git_push_tip(
            if ahead > 0 {
                t!("git.push-count", count = ahead)
            } else if status.is_some_and(|s| s.upstream.is_none()) {
                t!("git.push-upstream")
            } else {
                t!("git.push")
            }
            .into(),
        );

        // A merge, rebase, cherry-pick or revert waiting.
        let conflicts = status
            .map(|s| s.entries.iter().filter(|e| e.conflicted).count())
            .unwrap_or(0);
        let (operation, detail) = match status.and_then(|s| s.operation) {
            Some(op) => (
                match op {
                    GitOperation::Merge => t!("git.op-merge"),
                    GitOperation::Rebase => t!("git.op-rebase"),
                    GitOperation::CherryPick => t!("git.op-cherry-pick"),
                    GitOperation::Revert => t!("git.op-revert"),
                },
                if conflicts > 0 {
                    t!("git.op-conflicts", count = conflicts)
                } else {
                    t!("git.op-resolved")
                },
            ),
            None => (String::new(), String::new()),
        };
        shell.set_git_operation(operation.into());
        shell.set_git_operation_detail(detail.into());
        shell.set_git_conflicts(conflicts as i32);
        shell.set_git_identity_needed(git.identity.as_ref().is_some_and(|id| !id.complete()));
        shell.set_git_amend(git.amend);

        // The two lists.
        let chosen = git.diff_for.as_ref();
        let entries = status.map(|s| s.entries.as_slice()).unwrap_or_default();
        let staged: Vec<GitChange> = entries
            .iter()
            .filter(|e| e.staged.is_some())
            .map(|e| change_row(e, true, chosen))
            .collect();
        let unstaged: Vec<GitChange> = entries
            .iter()
            .filter(|e| e.unstaged.is_some() || e.untracked || e.conflicted)
            .map(|e| change_row(e, false, chosen))
            .collect();
        shell.set_git_clean(staged.is_empty() && unstaged.is_empty());
        shell.set_git_staged(ModelRc::new(VecModel::from(staged)));
        shell.set_git_unstaged(ModelRc::new(VecModel::from(unstaged)));
        let stashes: Vec<GitStash> = git
            .stashes
            .iter()
            .map(|s| GitStash {
                index: s.index as i32,
                label: s.label.as_str().into(),
                message: s.message.as_str().into(),
                when: format::commit_when(s.time).into(),
            })
            .collect();
        shell.set_git_stashes(ModelRc::new(VecModel::from(stashes)));

        // The Changes page: the chosen file's diff, whose rows are
        // `sync_diff_views`'.
        shell.set_git_diff_title(
            git.diff_for
                .as_ref()
                .map(|(path, staged)| {
                    format!(
                        "{path} · {}",
                        if *staged {
                            t!("git.staged")
                        } else {
                            t!("git.unstaged")
                        }
                    )
                })
                .unwrap_or_default()
                .into(),
        );
        shell.set_git_diff_staged(git.diff_for.as_ref().is_some_and(|(_, s)| *s));
        shell.set_git_split(git.split);

        // The refs.
        let refs: Vec<GitRef> = ref_rows(git)
            .into_iter()
            .enumerate()
            .map(|(index, row)| GitRef {
                index: index as i32,
                kind: row.kind,
                label: row.label.into(),
                detail: row.detail.into(),
                tip: row.tip.into(),
                current: row.current,
                gone: row.gone,
                filtered: row
                    .target
                    .as_ref()
                    .is_some_and(|t| matches!(t, Target::Branch { name, .. } if git.rev.as_deref() == Some(name))),
            })
            .collect();
        shell.set_git_refs(ModelRc::new(VecModel::from(refs)));
        shell.set_git_filter(
            git.rev
                .as_ref()
                .map(|rev| t!("git.filter-on", name = rev.clone()))
                .unwrap_or_default()
                .into(),
        );

        // The log.
        let needle = git.query.trim().to_lowercase();
        let (log, lanes, truncated, selected_at) = match &git.history {
            Some(history) => {
                let rows = &history.rows;
                let edges = |at: usize| -> Vec<GitLane> {
                    let row = &rows[at];
                    let next = rows.get(at + 1).map(|r| r.lane);
                    row.edges
                        .iter()
                        .map(|edge| {
                            let line = edge_line(
                                edge.from,
                                edge.to,
                                edge.from == row.lane,
                                Some(edge.to) == next,
                                LANE_PX,
                                ROW_PX,
                            );
                            GitLane {
                                commands: line.d.into(),
                                color: lane_colour(line.lane),
                            }
                        })
                        .collect()
                };
                let mut selected_at = -1;
                let log: Vec<GitCommitRow> = rows
                    .iter()
                    .enumerate()
                    .map(|(at, row)| {
                        let commit = &row.commit;
                        let selected = git.selected.as_deref() == Some(commit.id.as_str());
                        if selected {
                            selected_at = at as i32;
                        }
                        let labels: Vec<GitLabel> = commit
                            .refs
                            .iter()
                            .take(LABELS)
                            .map(|label| {
                                let (text, kind) = match label.kind {
                                    RefKind::Head if label.name.is_empty() => {
                                        (format!("✓ {}", t!("git.head")), 0)
                                    }
                                    RefKind::Head => (format!("✓ {}", label.name), 0),
                                    RefKind::Branch => (label.name.clone(), 1),
                                    RefKind::Remote => (label.name.clone(), 2),
                                    RefKind::Tag => (label.name.clone(), 3),
                                };
                                GitLabel {
                                    text: text.into(),
                                    color: lane_colour(row.lane),
                                    kind,
                                }
                            })
                            .collect();
                        GitCommitRow {
                            index: at as i32,
                            short: commit.short.as_str().into(),
                            summary: commit.summary.as_str().into(),
                            author: commit.author.as_str().into(),
                            when: format::commit_when(commit.time).into(),
                            lane: row.lane as i32,
                            dot: lane_colour(row.lane),
                            merge: commit.parents.len() > 1,
                            edges: ModelRc::new(VecModel::from(edges(at))),
                            above: ModelRc::new(VecModel::from(if at == 0 {
                                Vec::new()
                            } else {
                                edges(at - 1)
                            })),
                            labels: ModelRc::new(VecModel::from(labels)),
                            more: commit.refs.len().saturating_sub(LABELS) as i32,
                            selected,
                            dim: !needle.is_empty() && !matches(row, &needle),
                        }
                    })
                    .collect();
                (log, history.lanes, history.truncated, selected_at)
            }
            None => (Vec::new(), 0, false, -1),
        };
        shell.set_git_log(ModelRc::new(VecModel::from(log)));
        shell.set_git_lanes(lanes.max(1) as i32);
        shell.set_git_truncated(truncated);
        shell.set_git_selected(selected_at);
        shell.set_git_reveal(git.reveal);
        let found = git
            .history
            .as_ref()
            .map(|h| hits(&h.rows, &git.query))
            .unwrap_or_default();
        shell.set_git_search_count(
            if git.query.trim().is_empty() {
                String::new()
            } else if found.is_empty() {
                t!("git.search-none")
            } else if let Some(at) = found.iter().position(|&hit| hit as i32 == selected_at) {
                t!("git.search-count", at = at + 1, count = found.len())
            } else {
                t!("git.search-total", count = found.len())
            }
            .into(),
        );

        // The opened commit.
        match &git.detail {
            Some(detail) if git.selected.as_deref() == Some(detail.commit.id.as_str()) => {
                shell.set_git_detail_open(true);
                let [summary, meta, body] = detail_texts(detail);
                shell.set_git_detail_summary(summary.into());
                shell.set_git_detail_meta(meta.into());
                shell.set_git_detail_body(body.into());
                let files: Vec<GitFile> = detail
                    .files
                    .iter()
                    .enumerate()
                    .map(|(index, file)| {
                        let (letter, tone, tip) = glyph(Some(file.kind), false, false);
                        GitFile {
                            index: index as i32,
                            path: file.path.as_str().into(),
                            letter: letter.into(),
                            tone,
                            tip: tip.into(),
                            added: file
                                .added
                                .map(|n| format!("+{n}"))
                                .unwrap_or_default()
                                .into(),
                            removed: file
                                .removed
                                .map(|n| format!("−{n}"))
                                .unwrap_or_else(|| t!("git.binary"))
                                .into(),
                            selected: git.file.as_deref() == Some(file.path.as_str()),
                        }
                    })
                    .collect();
                shell.set_git_detail_files(ModelRc::new(VecModel::from(files)));
            }
            _ => {
                shell.set_git_detail_open(git.selected.is_some());
                shell.set_git_detail_summary("".into());
                shell.set_git_detail_body("".into());
                shell.set_git_detail_meta(
                    if git.selected.is_some() {
                        t!("git.loading")
                    } else {
                        String::new()
                    }
                    .into(),
                );
                shell.set_git_detail_files(ModelRc::default());
            }
        }

        // The name field.
        match &git.prompt {
            Some(prompt) => {
                shell.set_git_prompt_open(true);
                let title = match &prompt.kind {
                    PromptKind::Branch { from: None } => t!("git.prompt-branch"),
                    PromptKind::Branch { from: Some(from) } => t!(
                        "git.prompt-branch-from",
                        from = if from.len() == 40 {
                            short(from)
                        } else {
                            from.clone()
                        }
                    ),
                    PromptKind::Rename { from } => t!("git.prompt-rename", from = from.clone()),
                    PromptKind::Tag { at } => t!("git.prompt-tag", at = short(at)),
                    PromptKind::Remote { push: true } => t!("git.prompt-remote-push"),
                    PromptKind::Remote { push: false } => t!("git.prompt-remote"),
                    PromptKind::RenameRemote { from } => {
                        t!("git.prompt-remote-rename", from = from.clone())
                    }
                    PromptKind::RemoteUrl { name } => {
                        t!("git.prompt-remote-url", name = name.clone())
                    }
                };
                shell.set_git_prompt_title(title.into());
                shell.set_git_prompt_has_url(matches!(prompt.kind, PromptKind::Remote { .. }));
                shell.set_git_prompt_problem(
                    prompt
                        .shown_problem(&git.remotes)
                        .map(problem_text)
                        .unwrap_or_default()
                        .into(),
                );
                shell.set_git_prompt_ok(prompt.problem(&git.remotes).is_none());
            }
            None => shell.set_git_prompt_open(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A patch reads as a header and its lines, numbered on the side each
    /// exists, and side by side the removed and added runs pair up.
    #[test]
    fn a_patch_is_rows_either_way() {
        let patch = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n";
        let (rows, note, texts) = diff_rows(patch, false);
        assert!(note.is_empty());
        assert_eq!(
            texts,
            [
                [None, None],
                [Some("keep".to_string()), None],
                [Some("old".to_string()), None],
                [Some("new".to_string()), None],
            ],
            "the header is no line, and the markers are not text"
        );
        let kinds: Vec<i32> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [3, 0, 2, 1]);
        assert_eq!(rows[2].old.as_str(), "2");
        assert_eq!(rows[3].new.as_str(), "2");
        let (split, _, texts) = diff_rows(patch, true);
        assert_eq!(
            texts[2],
            [Some("old".to_string()), Some("new".to_string())],
            "each side its own text"
        );
        assert_eq!(
            split.len(),
            3,
            "the header, the kept line, and old beside new"
        );
        assert_eq!((split[2].kind, split[2].right_kind), (2, 1));
        assert_eq!(split[2].text.as_str(), "old");
        assert_eq!(split[2].right_text.as_str(), "new");
    }

    /// Git's `?` is not a letter anybody reads: an untracked file is a
    /// green U, and a conflict outranks whatever else the file is.
    #[test]
    fn a_change_wears_the_letter_that_says_what_it_is() {
        assert_eq!(glyph(None, true, false).0, "U");
        assert_eq!(glyph(Some(ChangeKind::Modified), false, true).0, "!");
        assert_eq!(glyph(Some(ChangeKind::Deleted), false, false).1, 3);
    }
}
