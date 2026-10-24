//! `RUSTY_PROBE_MERGE`, from step 8200: a merge stopped on conflicts,
//! resolved through the Git panel and the editor's conflict bar to its
//! commit. The panel with its conflict list and the banner saying what is
//! merging; the lists as a tree and a group folded; a file only one side
//! changed taken from the other side, a file both added taken from this
//! side; a chapter opened, walked and taken whole from the incoming side by
//! the bar, saved and marked resolved by it; the code's conflict taken from
//! both sides and marked resolved from its row; then everything staged
//! taken out of the index — the mistake that left a book's merge in its
//! working tree — and Continue, which offers to stage it all again first,
//! so the merge commit still carries what it brought. Each state printed
//! and photographed; questions are answered yes, as a probe answers them.
//!
//! On a repository stopped in a merge with `book/src/01.md` and
//! `src/main.rs` changed by both sides, `new.md` added by both and
//! `notes/gone.txt` changed here and deleted there.

use slint::{ComponentHandle, Model};

use crate::{Shell, probe_surfaces::Run, work};

fn rows(shell: &Shell) -> Vec<String> {
    shell
        .get_git_rows()
        .iter()
        .map(|row| match row.kind {
            0 => format!(
                "[{} · {}{}]",
                row.group,
                row.count,
                if row.folded { " folded" } else { "" }
            ),
            1 => format!(
                "{}{}/ {}",
                "  ".repeat(row.depth as usize),
                row.name,
                row.count
            ),
            _ => format!(
                "{}{} {}{}",
                "  ".repeat(row.depth as usize),
                row.letter,
                row.path,
                if row.note.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", row.note)
                }
            ),
        })
        .collect()
}

fn conflicted(shell: &Shell, path: &str) -> bool {
    shell
        .get_git_rows()
        .iter()
        .any(|row| row.kind == 2 && row.conflict && row.path.as_str() == path)
}

fn note_of(shell: &Shell, path: &str) -> String {
    shell
        .get_git_rows()
        .iter()
        .find(|row| row.kind == 2 && row.path.as_str() == path)
        .map(|row| row.note.to_string())
        .unwrap_or_default()
}

/// `git` in the project, for what the panel does not show: the commit made.
fn git(args: &[&str]) -> String {
    let Some(root) = work::app().root_path() else {
        return String::new();
    };
    std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

pub fn merge(n: u32, run: &Run) {
    let app = work::app();
    let shell = run.window.global::<Shell>();
    let say = |what: &str| {
        eprintln!(
            "merge: {what}\n  banner {:?} / {:?} / {:?}\n  rows {:#?}",
            shell.get_git_operation(),
            shell.get_git_operation_message(),
            shell.get_git_operation_detail(),
            rows(&shell)
        );
    };
    match n {
        8200 => {
            shell.invoke_show_panel("left".into(), "git".into());
            shell.invoke_close_tab("page:welcome".into());
        }
        8201 => {
            if !run.until(
                shell.get_git_state() == 1 && shell.get_git_conflicts() > 0,
                750,
            ) {
                return;
            }
            say("stopped in a merge");
            run.snapshot("merge-panel.png");
            app.act("git.tree");
        }
        8203 => {
            say("as a tree");
            run.snapshot("merge-tree.png");
            shell.invoke_git_row("fold".into(), 1, "".into());
        }
        8204 => {
            say("the staged list folded");
            shell.invoke_git_row("fold".into(), 1, "".into());
            // Deleted there, changed here: taking theirs deletes it.
            shell.invoke_git_file("theirs".into(), "notes/gone.txt".into(), false);
        }
        8205 => {
            if !run.until(!conflicted(&shell, "notes/gone.txt"), 500) {
                return;
            }
            eprintln!(
                "merge: notes/gone.txt taken from the incoming side; in the tree: {}",
                std::path::Path::new(&git(&["rev-parse", "--show-toplevel"]).trim().to_string())
                    .join("notes/gone.txt")
                    .exists()
            );
            shell.invoke_git_file("ours".into(), "new.md".into(), false);
        }
        8206 => {
            if !run.until(!conflicted(&shell, "new.md"), 500) {
                return;
            }
            eprintln!(
                "merge: new.md taken from this side: {:?}",
                git(&["show", ":new.md"]).trim()
            );
            app.open_file("book/src/01.md".into());
        }
        8207 => {
            if !run.until(shell.get_conflict_count() > 0, 500) {
                return;
            }
            eprintln!(
                "merge: the chapter open, the bar says {:?}",
                shell.get_conflict_label()
            );
            run.snapshot("merge-bar.png");
            app.act("editor.conflict-next");
        }
        8208 => {
            eprintln!(
                "merge: after Next conflict, the bar says {:?}",
                shell.get_conflict_label()
            );
            app.act("editor.conflicts-incoming");
        }
        8209 => {
            eprintln!(
                "merge: after Accept all incoming, {} left, the bar says {:?}, unmerged {}",
                shell.get_conflict_count(),
                shell.get_conflict_label(),
                shell.get_conflict_unmerged()
            );
            app.act("file.save");
        }
        8210 => {
            if !run.until(
                note_of(&shell, "book/src/01.md")
                    .contains(&rusty_i18n::t!("git.conflict-none-left")),
                500,
            ) {
                return;
            }
            eprintln!(
                "merge: saved; its row says {:?}",
                note_of(&shell, "book/src/01.md")
            );
            run.snapshot("merge-resolved-unmarked.png");
            app.act("git.mark-resolved");
        }
        8211 => {
            if !run.until(!conflicted(&shell, "book/src/01.md"), 500) {
                return;
            }
            eprintln!(
                "merge: the chapter marked resolved; staged as {:?}",
                git(&["show", ":book/src/01.md"])
                    .lines()
                    .last()
                    .unwrap_or("")
            );
            app.open_file("src/main.rs".into());
        }
        8212 => {
            if !run.until(shell.get_conflict_count() > 0, 500) {
                return;
            }
            app.act("editor.conflicts-both");
        }
        8213 => {
            app.act("file.save");
        }
        8214 => {
            if !run.until(
                note_of(&shell, "src/main.rs").contains(&rusty_i18n::t!("git.conflict-none-left")),
                500,
            ) {
                return;
            }
            shell.invoke_git_file("resolve".into(), "src/main.rs".into(), false);
        }
        8215 => {
            if !run.until(shell.get_git_conflicts() == 0, 500) {
                return;
            }
            say("every conflict resolved");
            run.snapshot("merge-all-resolved.png");
            // The mistake: everything staged taken out of the index mid-merge
            // (asked first; a probe answers yes).
            shell.invoke_git_row("unstage".into(), 1, "".into());
        }
        8216 => {
            if !run.until(shell.get_git_staged_count() == 0, 500) {
                return;
            }
            say("everything unstaged during the merge");
            // Continue asks to stage it all first; a probe takes that.
            app.act("git.continue");
        }
        8217 => {
            if !run.until(shell.get_git_operation().is_empty(), 750) {
                return;
            }
            say("the merge continued");
            eprintln!(
                "merge: the commit made: {}",
                git(&["show", "--stat", "--format=%s (%p)", "HEAD"]).trim()
            );
            run.snapshot("merge-done.png");
        }
        8219 => app.act("app.exit"),
        _ => {}
    }
}
