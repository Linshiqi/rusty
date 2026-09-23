//! The branches, the tags and the remotes: one `for-each-ref`, and the
//! remotes out of the config.

use std::collections::BTreeMap;

use crate::model::{Branch, Refs, Remote, Tag};

/// The remotes out of `git config -z --get-regexp` over the `url` and
/// `pushurl` keys: every entry ends in NUL, and is its key, a newline, then
/// its value — so a URL holding a space, or a path on a drive with one in
/// its name, arrives whole.
///
/// A remote's name may itself hold dots (`my.fork`), so the name is what
/// lies between `remote.` and the trailing `.url` or `.pushurl`, never a
/// split on dots. git fetches from the first of several `url` lines, which
/// is the one kept; a remote with a `pushurl` and no `url` is nothing a
/// fetch can use and is left out rather than shown with a URL it lacks.
pub fn remotes(text: &str) -> Vec<Remote> {
    let mut found: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
    for entry in text.split('\0') {
        let Some((key, value)) = entry.split_once('\n') else {
            continue;
        };
        let Some(rest) = key.strip_prefix("remote.") else {
            continue;
        };
        let (name, push) = if let Some(name) = rest.strip_suffix(".pushurl") {
            (name, true)
        } else if let Some(name) = rest.strip_suffix(".url") {
            (name, false)
        } else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let (url, push_url) = found.entry(name.to_string()).or_default();
        let slot = if push { push_url } else { url };
        if slot.is_none() {
            *slot = Some(value.to_string());
        }
    }
    found
        .into_iter()
        .filter_map(|(name, (url, push_url))| {
            Some(Remote {
                name,
                url: url?,
                push_url,
            })
        })
        .collect()
}

/// The format [`refs`] reads, one ref a line and fields on `\x1f`: the full
/// ref name, `*` when checked out, the upstream's full name, how it tracks
/// it (`ahead 1, behind 2`, or `gone`), the hash, the commit an annotated
/// tag peels to, when it was made, what it points at if it is symbolic, and
/// its subject — last, so nothing after it can be torn by what it contains.
pub const REFS_FORMAT: &str = "%(refname)%1f%(HEAD)%1f%(upstream)%1f%(upstream:track,nobracket)%1f%(objectname)%1f%(*objectname)%1f%(creatordate:unix)%1f%(symref)%1f%(subject)";

/// Branches and tags out of `git for-each-ref --format=REFS_FORMAT
/// refs/heads refs/remotes refs/tags`.
///
/// Local or remote is the ref's namespace, never a slash in its name, and a
/// symbolic ref — `origin/HEAD` — points at a branch rather than being one.
pub fn refs(text: &str) -> Refs {
    let mut out = Refs::default();
    for line in text.lines() {
        let fields: Vec<&str> = line.splitn(9, '\x1f').collect();
        let [
            refname,
            head,
            upstream,
            track,
            id,
            peeled,
            time,
            symref,
            subject,
        ] = fields[..]
        else {
            continue;
        };
        if !symref.trim().is_empty() {
            continue;
        }
        let time = time.trim().parse().unwrap_or(0);
        let id = id.trim().to_string();
        let subject = subject.trim_end().to_string();
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            let (ahead, behind, gone) = tracking(track);
            out.branches.push(Branch {
                name: name.to_string(),
                current: head.trim() == "*",
                remote: false,
                upstream: short_ref(upstream),
                tip: id.chars().take(7).collect(),
                id,
                ahead,
                behind,
                gone,
                time,
                subject,
                remote_name: None,
            });
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            out.branches.push(Branch {
                name: name.to_string(),
                current: false,
                remote: true,
                upstream: None,
                tip: id.chars().take(7).collect(),
                id,
                ahead: 0,
                behind: 0,
                gone: false,
                time,
                subject,
                remote_name: name.split('/').next().map(str::to_string),
            });
        } else if let Some(name) = refname.strip_prefix("refs/tags/") {
            let peeled = peeled.trim();
            out.tags.push(Tag {
                name: name.to_string(),
                id: if peeled.is_empty() {
                    id
                } else {
                    peeled.to_string()
                },
                time,
                subject,
            });
        }
    }
    out.branches
        .sort_by(|a, b| a.remote.cmp(&b.remote).then_with(|| a.name.cmp(&b.name)));
    out.tags
        .sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.name.cmp(&b.name)));
    out
}

/// `%(upstream:track,nobracket)`: empty in step, `ahead 1`, `behind 2`,
/// `ahead 1, behind 2`, or `gone`.
fn tracking(track: &str) -> (u32, u32, bool) {
    let track = track.trim();
    if track == "gone" {
        return (0, 0, true);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in track.split(',').map(str::trim) {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

/// `refs/remotes/origin/main` → `origin/main`; `refs/heads/x` → `x`.
fn short_ref(full: &str) -> Option<String> {
    let full = full.trim();
    let short = full
        .strip_prefix("refs/remotes/")
        .or_else(|| full.strip_prefix("refs/heads/"))
        .unwrap_or(full);
    (!short.is_empty()).then(|| short.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real answer's shape: a remote whose name holds a dot, a push URL
    /// set apart from the fetch URL, a path with a space in it, and the
    /// trailing NUL git ends on.
    #[test]
    fn remotes_are_read_whole_with_dotted_names_and_push_urls() {
        let text = concat!(
            "remote.origin.url\nhttps://github.com/you/firmware.git\0",
            "remote.my.fork.url\ngit@github.com:you/fork.git\0",
            "remote.my.fork.pushurl\nE:/Work/My Repos/fork.git\0",
        );
        assert_eq!(
            remotes(text),
            vec![
                Remote {
                    name: "my.fork".into(),
                    url: "git@github.com:you/fork.git".into(),
                    push_url: Some("E:/Work/My Repos/fork.git".into()),
                },
                Remote {
                    name: "origin".into(),
                    url: "https://github.com/you/firmware.git".into(),
                    push_url: None,
                },
            ],
        );
    }

    /// No remotes is an empty answer (git exits 1 with nothing printed); a
    /// second `url` does not replace the first, which is the one git fetches
    /// from; and a remote that only has somewhere to push is left out.
    #[test]
    fn nothing_is_nothing_and_the_first_url_is_the_fetch_url() {
        assert!(remotes("").is_empty());
        let text = concat!(
            "remote.origin.url\nhttps://one.example/r.git\0",
            "remote.origin.url\nhttps://two.example/r.git\0",
            "remote.pushonly.pushurl\nhttps://three.example/r.git\0",
            "core.bare\nfalse\0",
        );
        let read = remotes(text);
        assert_eq!(read.len(), 1, "{read:?}");
        assert_eq!(read[0].url, "https://one.example/r.git");
    }

    /// A real `for-each-ref` answer: the current branch ahead and behind, a
    /// slashed local branch whose upstream is gone, a remote branch, the
    /// remote's HEAD pointer, an annotated tag peeling to its commit and a
    /// lightweight one.
    #[test]
    fn refs_read_branches_by_namespace_and_tags_peeled() {
        let line = |fields: [&str; 9]| format!("{}\n", fields.join("\x1f"));
        let text = [
            line([
                "refs/heads/main",
                "*",
                "refs/remotes/origin/main",
                "ahead 2, behind 1",
                "aaaaaaaa11",
                "",
                "1756940000",
                "",
                "The tip",
            ]),
            line([
                "refs/heads/feature/x",
                " ",
                "refs/remotes/origin/feature/x",
                "gone",
                "bbbbbbbb22",
                "",
                "1756930000",
                "",
                "Work",
            ]),
            line([
                "refs/remotes/origin/main",
                " ",
                "",
                "",
                "cccccccc33",
                "",
                "1756920000",
                "",
                "Theirs",
            ]),
            line([
                "refs/remotes/origin/HEAD",
                " ",
                "",
                "",
                "cccccccc33",
                "",
                "1756920000",
                "refs/remotes/origin/main",
                "Theirs",
            ]),
            line([
                "refs/tags/v1.0",
                " ",
                "",
                "",
                "tagobject44",
                "dddddddd44",
                "1756910000",
                "",
                "Release one",
            ]),
            line([
                "refs/tags/v0.9",
                " ",
                "",
                "",
                "eeeeeeee55",
                "",
                "1756900000",
                "",
                "An old commit",
            ]),
        ]
        .concat();
        let refs = refs(&text);
        let names: Vec<(&str, bool)> = refs
            .branches
            .iter()
            .map(|b| (b.name.as_str(), b.remote))
            .collect();
        assert_eq!(
            names,
            vec![("feature/x", false), ("main", false), ("origin/main", true)],
            "locals by name, then remotes; origin/HEAD is not a branch"
        );
        let main = &refs.branches[1];
        assert!(main.current);
        assert_eq!((main.ahead, main.behind, main.gone), (2, 1, false));
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert_eq!(main.tip, "aaaaaaa");
        assert!(refs.branches[0].gone);
        assert_eq!(refs.branches[2].remote_name.as_deref(), Some("origin"));
        assert_eq!(refs.branches[2].local_name(), "main");
        assert_eq!(refs.tags[0].name, "v1.0");
        assert_eq!(
            refs.tags[0].id, "dddddddd44",
            "an annotated tag names its commit"
        );
        assert_eq!(refs.tags[1].id, "eeeeeeee55");
    }
}
