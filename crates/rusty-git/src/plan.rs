//! The decisions a write makes before it is a `git` line: which remote a
//! push goes to and how a branch is checked out. Pure, so the tests pin what
//! each answers.

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| (*arg).to_string()).collect()
}

/// The remote a push or a new upstream goes to — `None` when the repository
/// has none, which is the case a push has to stop and ask about.
///
/// The current branch's upstream's remote first, then `origin`, then the
/// first there is. `known` is every remote name there is evidence of: the
/// config's, and the ones remote-tracking branches name, so a click in the
/// moment before the remotes have been read does not ask to add a remote
/// that plainly exists.
pub fn pick_remote(upstream: Option<&str>, known: &[String]) -> Option<String> {
    if let Some(upstream) = upstream {
        // The longest name that begins the upstream: a remote may be called
        // `team` and another `team/fw`, and `team/fw/main` is the second's.
        let owner = known
            .iter()
            .filter(|name| {
                upstream
                    .strip_prefix(name.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|name| name.len());
        return owner
            .cloned()
            .or_else(|| upstream.split('/').next().map(str::to_string));
    }
    known
        .iter()
        .find(|name| *name == "origin")
        .or_else(|| known.first())
        .cloned()
}

/// The arguments that check out `name`. A local branch by name; a remote
/// one through the local branch of the same name when there is one, and a
/// new local branch tracking it otherwise — checking a remote branch out by
/// its own name would detach HEAD, which is never what a double-click on
/// `origin/feature` means.
pub fn checkout_args(name: &str, remote: bool, local: &str, local_exists: bool) -> Vec<String> {
    if !remote {
        args(&["checkout", name])
    } else if local_exists {
        args(&["checkout", local])
    } else {
        args(&["checkout", "--track", name])
    }
}

/// The field for a branch, a tag or a remote: what it will make, and what has
/// been typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefPrompt {
    pub kind: PromptKind,
    /// The name — or, for a remote's new URL, the URL.
    pub value: String,
    /// The URL of a new remote: the one prompt with two fields.
    pub url: String,
}

/// Why a prompt's OK is disabled, in the terms its field says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptProblem {
    Name(crate::RefNameProblem),
    Remote(crate::RemoteProblem),
}

impl RefPrompt {
    /// What stops the prompt being carried out, by the rules of what it
    /// makes: a branch's or a tag's name, a remote's name and URL, or a URL
    /// alone. Trimmed, as the submit trims.
    pub fn problem(&self, remotes: &[crate::Remote]) -> Option<PromptProblem> {
        let value = self.value.trim();
        match &self.kind {
            PromptKind::Branch { .. } | PromptKind::Rename { .. } | PromptKind::Tag { .. } => {
                crate::ref_name_problem(value).map(PromptProblem::Name)
            }
            PromptKind::Remote { .. } => crate::remote_name_problem(value, remotes, None)
                .or_else(|| crate::remote_url_problem(self.url.trim()))
                .map(PromptProblem::Remote),
            PromptKind::RenameRemote { from } => {
                crate::remote_name_problem(value, remotes, Some(from)).map(PromptProblem::Remote)
            }
            PromptKind::RemoteUrl { .. } => {
                crate::remote_url_problem(value).map(PromptProblem::Remote)
            }
        }
    }

    /// The problem worth saying out loud: one about a field with something
    /// in it. An empty field only disables OK — the remote form opens with
    /// its name filled in and its URL empty, and a form that greets somebody
    /// with "enter a URL" in red is scolding them for not having typed yet.
    pub fn shown_problem(&self, remotes: &[crate::Remote]) -> Option<PromptProblem> {
        let (value, url) = (self.value.trim(), self.url.trim());
        match &self.kind {
            PromptKind::Remote { .. } => {
                let name = (!value.is_empty())
                    .then(|| crate::remote_name_problem(value, remotes, None))
                    .flatten();
                let url = (!url.is_empty())
                    .then(|| crate::remote_url_problem(url))
                    .flatten();
                name.or(url).map(PromptProblem::Remote)
            }
            _ => (!value.is_empty()).then(|| self.problem(remotes)).flatten(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// A new branch from a commit or a branch (`None`: HEAD), checked out
    /// once it is made.
    Branch { from: Option<String> },
    /// A new name for a local branch.
    Rename { from: String },
    /// A new tag on a commit.
    Tag { at: String },
    /// A new remote, by name and URL. `push` when a push with nowhere to go
    /// asked for it — once the remote exists, the push carries on, because
    /// that is what the click was for.
    Remote { push: bool },
    /// A new name for a remote.
    RenameRemote { from: String },
    /// A new URL for a remote.
    RemoteUrl { name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_string()).collect()
    }

    /// Where a push goes: the upstream's remote, then `origin`, then any —
    /// and nowhere at all when there is none, which is when the panel asks
    /// for one instead of running a push git can only refuse.
    #[test]
    fn a_push_goes_to_the_remote_there_is_evidence_for() {
        assert_eq!(
            pick_remote(Some("fork/main"), &names(&["origin", "fork"])).as_deref(),
            Some("fork"),
            "the branch's own upstream decides first",
        );
        assert_eq!(
            pick_remote(Some("team/fw/main"), &names(&["team", "team/fw"])).as_deref(),
            Some("team/fw"),
            "the longest remote name that begins the upstream owns it",
        );
        assert_eq!(
            pick_remote(None, &names(&["upstream", "origin"])).as_deref(),
            Some("origin")
        );
        assert_eq!(
            pick_remote(None, &names(&["github"])).as_deref(),
            Some("github")
        );
        assert_eq!(pick_remote(None, &[]), None);
        assert_eq!(
            pick_remote(Some("origin/main"), &[]).as_deref(),
            Some("origin"),
            "an upstream is evidence of its remote before the list has arrived",
        );
    }

    #[test]
    fn a_remote_branch_is_checked_out_through_a_local_one() {
        assert_eq!(
            checkout_args("main", false, "main", true),
            ["checkout", "main"]
        );
        assert_eq!(
            checkout_args("origin/feature/x", true, "feature/x", true),
            ["checkout", "feature/x"],
            "the local branch of that name, when there is one"
        );
        assert_eq!(
            checkout_args("origin/feature/x", true, "feature/x", false),
            ["checkout", "--track", "origin/feature/x"],
            "a new one tracking it, when there is not — never a detached HEAD"
        );
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;
    use crate::{RefNameProblem, Remote, RemoteProblem};

    fn prompt(kind: PromptKind, value: &str, url: &str) -> RefPrompt {
        RefPrompt {
            kind,
            value: value.into(),
            url: url.into(),
        }
    }

    fn remotes() -> Vec<Remote> {
        vec![Remote {
            name: "origin".into(),
            url: "https://github.com/you/firmware.git".into(),
            push_url: None,
        }]
    }

    /// Each kind is judged by the rules of what it makes: a second `origin`
    /// is refused, a fine name with no URL cannot be sent, and a URL field is
    /// never held to a branch name's rules — a URL is full of `:` and `/`.
    #[test]
    fn each_prompt_is_held_to_the_rules_of_what_it_makes() {
        let remotes = remotes();
        let add = |name: &str, url: &str| prompt(PromptKind::Remote { push: false }, name, url);
        assert_eq!(
            add("origin", "https://github.com/you/other.git").problem(&remotes),
            Some(PromptProblem::Remote(RemoteProblem::Exists))
        );
        assert_eq!(
            add("fork", "").problem(&remotes),
            Some(PromptProblem::Remote(RemoteProblem::UrlEmpty))
        );
        assert_eq!(
            add(" fork ", " https://github.com/you/fork.git ").problem(&remotes),
            None,
            "both fields are trimmed, as the submit trims them",
        );
        let url = prompt(
            PromptKind::RemoteUrl {
                name: "origin".into(),
            },
            "git@github.com:you/firmware.git",
            "",
        );
        assert_eq!(url.problem(&remotes), None);
        let rename = |to: &str| {
            prompt(
                PromptKind::RenameRemote {
                    from: "origin".into(),
                },
                to,
                "",
            )
        };
        assert_eq!(rename("origin").problem(&remotes), None);
        assert_eq!(rename("upstream").problem(&remotes), None);
        let branch = prompt(PromptKind::Branch { from: None }, "a:b", "");
        assert_eq!(
            branch.problem(&remotes),
            Some(PromptProblem::Name(RefNameProblem::Character(':')))
        );
    }

    /// The remote form opens with `origin` filled in and nothing in the URL.
    /// That disables OK and says nothing; a URL typed wrong says why, and a
    /// clashing name says so before any URL is typed.
    #[test]
    fn an_empty_field_disables_ok_without_a_word() {
        let remotes = remotes();
        let fresh = prompt(PromptKind::Remote { push: true }, "upstream", "");
        assert!(fresh.problem(&remotes).is_some());
        assert_eq!(fresh.shown_problem(&remotes), None);
        let dashed = prompt(PromptKind::Remote { push: true }, "upstream", "-x");
        assert_eq!(
            dashed.shown_problem(&remotes),
            Some(PromptProblem::Remote(RemoteProblem::UrlDash))
        );
        let clash = prompt(PromptKind::Remote { push: false }, "origin", "");
        assert_eq!(
            clash.shown_problem(&remotes),
            Some(PromptProblem::Remote(RemoteProblem::Exists))
        );
    }
}
