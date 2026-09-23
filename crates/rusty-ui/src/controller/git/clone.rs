//! Clone: the one git command that runs with no project open.

use super::*;

/// Open the clone dialog, empty.
pub fn open_clone_dialog(state: AppState) {
    state.git.clone.set(Some(CloneDraft::default()));
}

/// Ask the OS for the folder the clone lands in.
pub fn choose_clone_folder(state: AppState) {
    spawn_local(async move {
        match ipc::pick_folder(&t!("git.clone-into")).await {
            Ok(Some(folder)) => state.git.clone.update(|draft| {
                if let Some(draft) = draft {
                    draft.into = Some(folder);
                }
            }),
            Ok(None) => {}
            Err(error) => state.app.error.set(Some(error)),
        }
    });
}

/// Run the clone the dialog describes: into `<folder>/<name>`, where `name`
/// is what `git clone` itself would pick, streamed to the dock; the project
/// opens when git exits zero. The dialog stays up while it runs, so a second
/// click cannot start a second clone into the same directory.
pub fn clone_repository(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        url: String,
        into: String,
    }
    let Some(draft) = state.git.clone.get_untracked() else {
        return;
    };
    let url = draft.url.trim().to_string();
    let (Some(folder), Some(name)) = (draft.into.clone(), rusty_git::repo_name(&url)) else {
        return;
    };
    if draft.running {
        return;
    }
    let separator = if folder.contains('\\') && !folder.contains('/') {
        '\\'
    } else {
        '/'
    };
    let into = format!("{}{separator}{name}", folder.trim_end_matches(['/', '\\']));
    state.git.clone.update(|draft| {
        if let Some(draft) = draft {
            draft.running = true;
        }
    });
    state.dock.source.set("commands");
    let channel = stream_to_terminal(state);
    let args = Args {
        url,
        into: into.clone(),
    };
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, Option<i32>>(cmd::git::CLONE, &args, "onLine", &channel).await
        },
        move |code| {
            note_exit(state, code);
            if code == Some(0) {
                state.git.clone.set(None);
                open_project(state, into);
            } else {
                state.git.clone.update(|draft| {
                    if let Some(draft) = draft {
                        draft.running = false;
                    }
                });
            }
        },
    );
}
