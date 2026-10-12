//! The one thing rusty reads off a repository URL: what `git clone` will call
//! the directory. Pure, so the spellings people paste are tests.

/// The directory name `git clone <url>` creates — the last path segment with
/// `.git` and any trailing slash removed. `None` when there is no segment to
/// speak of, which is when the clone dialog has nothing to promise.
///
/// Handles the four spellings people paste: `https://host/user/repo.git`,
/// the same without `.git`, `git@host:user/repo.git`, and a bare local path.
pub fn repo_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let path = if let Some((_, rest)) = trimmed.split_once("://") {
        // A URL: the host, then the path. A host on its own names nothing.
        let (_, path) = rest.split_once('/')?;
        path
    } else if let Some((before, after)) = trimmed.split_once(':')
        && before.contains('@')
    {
        // The scp spelling, `user@host:path`.
        after
    } else {
        // A local path, drive letter and all.
        trimmed
    };
    let last = path.rsplit(['/', '\\']).next().unwrap_or(path).trim();
    let name = last.strip_suffix(".git").unwrap_or(last).trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spellings_people_paste_all_name_the_same_directory() {
        assert_eq!(
            repo_name("https://github.com/Linshiqi/rusty.git").as_deref(),
            Some("rusty")
        );
        assert_eq!(
            repo_name("https://github.com/Linshiqi/rusty").as_deref(),
            Some("rusty")
        );
        assert_eq!(
            repo_name("https://github.com/Linshiqi/rusty/").as_deref(),
            Some("rusty")
        );
        assert_eq!(
            repo_name("git@github.com:Linshiqi/rusty.git").as_deref(),
            Some("rusty")
        );
        assert_eq!(
            repo_name("ssh://git@github.com/Linshiqi/rusty.git").as_deref(),
            Some("rusty")
        );
        assert_eq!(
            repo_name("  https://gitee.com/x/cf-drone-rs.git  ").as_deref(),
            Some("cf-drone-rs")
        );
        assert_eq!(repo_name("E:\\CodeBase\\rusty").as_deref(), Some("rusty"));
    }

    #[test]
    fn a_host_or_nothing_names_no_directory() {
        assert_eq!(repo_name(""), None);
        assert_eq!(repo_name("   "), None);
        assert_eq!(repo_name("https://github.com/"), None);
        assert_eq!(repo_name("git@github.com:"), None);
    }
}
