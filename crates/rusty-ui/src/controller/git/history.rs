//! The history pane: the log, the commit opened below it, stepping
//! through either, and a commit torn off into its own window.

use super::*;

/// Show one branch's history, or every branch's when `rev` is `None`.
pub fn show_rev(state: AppState, rev: Option<String>) {
    state.git.rev.set(rev);
    load_history(state);
}

/// Ask for older commits: twice as many as now. The backend caps it.
pub fn show_more(state: AppState) {
    let limit = state.git.limit.get_untracked();
    state.git.limit.set((limit * 2).min(10_000));
    load_history(state);
}

/// Side by side or one column, for every diff the panel shows — and
/// remembered, so the choice outlives the window.
pub fn set_split(state: AppState, on: bool) {
    state.git.split.set(on);
    remember_split(on);
}

/// Whether a name is a full hash, which names one content for ever — the
/// only kind of name an opened commit is kept under. `stash@{0}` names a
/// different stash after every push or pop.
pub(super) fn is_hash(id: &str) -> bool {
    matches!(id.len(), 40 | 64) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn cached(state: AppState, id: &str) -> Option<CommitDetail> {
    state
        .git
        .cache
        .with_value(|cache| cache.iter().find(|detail| detail.commit.id == id).cloned())
}

fn remember(state: AppState, detail: CommitDetail) {
    state.git.cache.update_value(|cache| {
        cache.retain(|kept| kept.commit.id != detail.commit.id);
        cache.insert(0, detail);
        cache.truncate(CACHED_COMMITS);
    });
}

/// Open a commit: its message, its files, their patches.
///
/// The commit on screen stays there, dimmed, until the new one arrives: it
/// used to be cleared first, and the pane collapsed to a strip and grew
/// back on every click. One seen before is shown at once, from the cache.
pub fn select_commit(state: AppState, id: String) {
    #[derive(serde::Serialize)]
    struct Args {
        id: String,
    }
    state.git.selected.set(Some(id.clone()));
    if let Some(detail) = cached(state, &id) {
        state.git.detail_loading.set(false);
        show_detail(state, detail);
        return;
    }
    state.git.detail_loading.set(true);
    let args = Args { id: id.clone() };
    track(
        state,
        async move {
            let answer = ipc::call::<_, CommitDetail>(cmd::git::COMMIT, &args).await;
            if answer.is_err() {
                state.git.detail_loading.set(false);
            }
            answer
        },
        move |detail| {
            // A later click wins: the answer to an earlier one arriving after
            // it must not replace what the user is looking at now.
            if state.git.selected.get_untracked().as_deref() != Some(id.as_str()) {
                return;
            }
            state.git.detail_loading.set(false);
            if is_hash(&id) {
                remember(state, detail.clone());
            }
            show_detail(state, detail);
        },
    );
}

/// Put an opened commit on screen, keeping the file that was showing when
/// the new commit touched it too — walking down the log reading one file's
/// history is what that is for — and the first file otherwise.
fn show_detail(state: AppState, detail: CommitDetail) {
    let kept = state
        .git
        .file
        .get_untracked()
        .filter(|path| detail.files.iter().any(|file| &file.path == path));
    let path = kept.or_else(|| detail.files.first().map(|file| file.path.clone()));
    state.git.detail.set(Some(detail));
    match path {
        Some(path) => show_commit_file(state, path),
        None => state.git.file.set(None),
    }
}

/// Move the selection `step` rows through the log — the arrow keys. The
/// row is selected and scrolled to at once; it is *opened* only once the
/// keys stop, since holding an arrow down passes rows faster than a commit
/// can be read.
pub fn select_step(state: AppState, step: i64) {
    let next = state.git.history.with_untracked(|history| {
        let rows = &history.as_ref()?.rows;
        if rows.is_empty() {
            return None;
        }
        let at = state.git.selected.with_untracked(|selected| {
            selected
                .as_ref()
                .and_then(|id| rows.iter().position(|row| &row.commit.id == id))
        });
        let target = match at {
            Some(at) => (at as i64 + step).clamp(0, rows.len() as i64 - 1) as usize,
            None => 0,
        };
        Some(rows[target].commit.id.clone())
    });
    let Some(id) = next else {
        return;
    };
    if state.git.selected.get_untracked().as_deref() == Some(id.as_str()) {
        return;
    }
    state.git.selected.set(Some(id.clone()));
    state.git.reveal.set(Some(id.clone()));
    set_timeout(
        move || {
            if state.git.selected.get_untracked().as_deref() == Some(id.as_str()) {
                select_commit(state, id);
            }
        },
        Duration::from_millis(140),
    );
}

/// Go to the commit a branch or a tag names: select it and scroll the log
/// to it. A tip older than the rows loaded, or on a branch the filter
/// hides, is shown by filtering the log to `rev` and selecting it there.
pub fn reveal_ref(state: AppState, id: String, rev: Option<String>) {
    if state.git.mode.get_untracked() != GitMode::History {
        state.git.mode.set(GitMode::History);
    }
    let loaded = state.git.history.with_untracked(|history| {
        history
            .as_ref()
            .is_some_and(|history| history.rows.iter().any(|row| row.commit.id == id))
    });
    if !loaded && let Some(rev) = rev {
        show_rev(state, Some(rev));
    }
    state.git.reveal.set(Some(id.clone()));
    select_commit(state, id);
}

/// The next row the search matches after the selected one, or the one
/// before it — Enter and Shift+Enter in the search box.
pub fn step_search(state: AppState, forward: bool) {
    let query = state.git.query.get_untracked();
    let target = state.git.history.with_untracked(|history| {
        let rows = &history.as_ref()?.rows;
        let hits = crate::gitlog::hits(rows, &query);
        let at = state.git.selected.with_untracked(|selected| {
            selected
                .as_ref()
                .and_then(|id| rows.iter().position(|row| &row.commit.id == id))
        });
        let hit = crate::gitlog::step_hit(&hits, at, forward)?;
        Some(rows[hit].commit.id.clone())
    });
    if let Some(id) = target {
        state.git.reveal.set(Some(id.clone()));
        select_commit(state, id);
    }
}

/// Show one of the opened commit's files — its patch, or, for an image, the
/// picture before and after. The two sides are the commit's first parent
/// and the commit itself, less whichever side an added or deleted file does
/// not have.
pub fn show_commit_file(state: AppState, path: String) {
    state.git.diff_whole.set(false);
    state.git.file.set(Some(path.clone()));
    if !rusty_git::is_image_path(&path) {
        return;
    }
    let Some(detail) = state.git.detail.get_untracked() else {
        return;
    };
    let kind = detail.files.iter().find(|f| f.path == path).map(|f| f.kind);
    let old = match (kind, detail.commit.parents.first()) {
        (Some(ChangeKind::Added), _) | (_, None) => None,
        (_, Some(parent)) => Some(ImageSource::Rev(parent.clone())),
    };
    let new = match kind {
        Some(ChangeKind::Deleted) => None,
        _ => Some(ImageSource::Rev(detail.commit.id.clone())),
    };
    load_images(state, path, old, new);
}

/// Fetch an image's two sides as `data:` URLs. Each side answers on its own,
/// so a missing old side never delays the new one, and an answer for a
/// picture no longer showing is dropped.
pub fn load_images(
    state: AppState,
    path: String,
    old: Option<ImageSource>,
    new: Option<ImageSource>,
) {
    #[derive(serde::Serialize)]
    struct Args {
        spec: Option<String>,
        path: String,
    }
    let side = |source: &Option<ImageSource>| match source {
        None => ImageSide::Absent,
        Some(_) => ImageSide::Loading,
    };
    state.git.images.set(Some(ImagePair {
        path: path.clone(),
        old: side(&old),
        new: side(&new),
    }));
    let mime = rusty_git::image_mime(&path).unwrap_or("application/octet-stream");
    for (is_old, source) in [(true, old), (false, new)] {
        let Some(source) = source else {
            continue;
        };
        let args = Args {
            spec: match source {
                ImageSource::Worktree => None,
                ImageSource::Rev(rev) => Some(rev),
            },
            path: path.clone(),
        };
        let path = path.clone();
        spawn_local(async move {
            let side = match ipc::call::<_, String>(cmd::git::BLOB, &args).await {
                Ok(base64) => ImageSide::Ready {
                    // Base64 is four characters for three bytes.
                    bytes: base64.trim_end_matches('=').len() * 3 / 4,
                    url: format!("data:{mime};base64,{base64}"),
                },
                Err(error) => ImageSide::Failed(error.message),
            };
            state.git.images.update(|pair| {
                if let Some(pair) = pair
                    && pair.path == path
                {
                    if is_old {
                        pair.old = side;
                    } else {
                        pair.new = side;
                    }
                }
            });
        });
    }
}

/// Fold the opened commit's pane down to a strip, or bring it back.
pub fn toggle_detail(state: AppState) {
    state.git.detail_hidden.update(|hidden| *hidden = !*hidden);
}

/// Tear the opened commit off into a window of its own.
pub fn open_commit_window(state: AppState, target: String) {
    #[derive(serde::Serialize)]
    struct Args {
        target: String,
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::git::WINDOW, &Args { target }).await },
        |()| {},
    );
}
