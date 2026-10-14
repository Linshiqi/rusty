//! The picker: the command palette and the file finder are one list with
//! one set of keys, and what it lists is what it was opened for.

use crate::actions::{self, Action};

/// What the picker is listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Commands,
    Files,
    /// The project chooser: the recent ones, then the ways to have another.
    Projects,
    /// Places a server named — references, implementations.
    Places,
    /// What the server offers to do at the caret.
    Fixes,
    /// A new name for the symbol at the caret, typed in the field.
    Rename,
    /// Where Run goes: the simulator, and what is plugged in.
    Targets,
}

/// What a row stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Action(&'static str),
    File(String),
    Project(String),
    /// A line of the document on screen, counted from one, and a column.
    Line(usize, Option<usize>),
    /// A row that only says something: picking it closes the picker.
    Nothing,
    /// A place a server named, to jump to.
    Place(rusty_lsp::Location),
    /// The fix at this index of the server's answer.
    Fix(usize),
    /// The symbol at the caret renamed to this.
    Rename(String),
    /// The place to run at this index of the list.
    Target(usize),
}

/// How many rows are shown. Past this the query is what narrows it.
pub const SHOWN: usize = 12;

/// The commands matching `query`, best first: (title, chord, what it does).
pub fn commands(query: &str) -> Vec<(String, String, Pick)> {
    // Not the palette itself: it is open.
    let listed: Vec<&Action> = actions::ALL
        .iter()
        .filter(|a| a.listed && a.id != "palette.open")
        .collect();
    let titles: Vec<String> = listed.iter().map(|a| rusty_i18n::lookup(a.title)).collect();
    rank(query, &titles)
        .into_iter()
        .take(SHOWN)
        .map(|index| {
            let action = listed[index];
            (
                titles[index].clone(),
                action.chord.unwrap_or_default().to_string(),
                Pick::Action(action.id),
            )
        })
        .collect()
}

/// The files matching `query`, best first: (name, folder, the file).
pub fn files(query: &str, paths: &[String]) -> Vec<(String, String, Pick)> {
    rank(query, paths)
        .into_iter()
        .take(SHOWN)
        .map(|index| {
            let path = &paths[index];
            let (dir, name) = match path.rfind('/') {
                Some(at) => (path[..at].to_string(), path[at + 1..].to_string()),
                None => (String::new(), path.clone()),
            };
            (name, dir, Pick::File(path.clone()))
        })
        .collect()
}

/// `:42`, `:42:7` or `:42,7` — VS Code's go-to-line in the finder — as a
/// line and a column counted from one. `Some((None, None))` for a `:`
/// with nothing after it yet; `None` for a query that is not one.
pub fn line_target(query: &str) -> Option<(Option<usize>, Option<usize>)> {
    let rest = query.trim_start().strip_prefix(':')?.trim();
    if rest.is_empty() {
        return Some((None, None));
    }
    let mut parts = rest.splitn(2, [':', ',']);
    let line = parts.next()?.trim().parse::<usize>().ok()?;
    let col = match parts.next().map(str::trim) {
        Some("") | None => None,
        Some(col) => Some(col.parse::<usize>().ok()?),
    };
    Some((Some(line.max(1)), col.map(|col| col.max(1))))
}

/// The recent projects matching `query`, then the ways to have another:
/// (name, where it is, what it does).
pub fn projects(query: &str, recents: &[String]) -> Vec<(String, String, Pick)> {
    let names: Vec<String> = recents.iter().map(|path| path.replace('\\', "/")).collect();
    let mut rows: Vec<(String, String, Pick)> = rank(query, &names)
        .into_iter()
        .take(SHOWN)
        .map(|index| {
            let path = &names[index];
            let name = path.rsplit('/').next().unwrap_or(path).to_string();
            (
                name,
                recents[index].clone(),
                Pick::Project(recents[index].clone()),
            )
        })
        .collect();
    for id in ["project.open", "project.new", "project.clone"] {
        if let Some(action) = actions::by_id(id) {
            let title = rusty_i18n::lookup(action.title);
            if rank(query, std::slice::from_ref(&title)).len() == 1 {
                rows.push((title, String::new(), Pick::Action(action.id)));
            }
        }
    }
    rows
}

/// The indices of `candidates` that match `query`, best first.
///
/// A match is every character of the query appearing in order, case-folded.
/// The query found whole in the last segment beats it found whole anywhere,
/// which beats it scattered through the last segment, which beats it
/// scattered anywhere; one the last segment *starts* with is better still;
/// and among equals the shorter wins — so `main` finds `src/main.rs` before
/// `book/src/22-maintenance.md`. An empty query is the list in its order.
pub fn rank(query: &str, candidates: &[String]) -> Vec<usize> {
    let needle: Vec<char> = query.trim().to_lowercase().chars().collect();
    let mut scored: Vec<(i64, usize)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, text)| score(&needle, text).map(|score| (score, index)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

fn score(needle: &[char], text: &str) -> Option<i64> {
    let length = i64::try_from(text.len()).unwrap_or(i64::MAX);
    if needle.is_empty() {
        return Some(0);
    }
    let lower = text.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let query: String = needle.iter().collect();
    let mut score = if name.contains(&query) {
        3000
    } else if lower.contains(&query) {
        2000
    } else if subsequence(needle, name) {
        1500
    } else if subsequence(needle, &lower) {
        1000
    } else {
        return None;
    };
    if name.starts_with(&query) {
        score += 500;
    }
    Some(score - length)
}

fn subsequence(needle: &[char], hay: &str) -> bool {
    let mut rest = hay.chars();
    needle.iter().all(|c| rest.any(|h| h == *c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colon_is_a_line_and_maybe_a_column() {
        assert_eq!(line_target(":42"), Some((Some(42), None)));
        assert_eq!(line_target(": 42:7"), Some((Some(42), Some(7))));
        assert_eq!(line_target(":42,7"), Some((Some(42), Some(7))));
        assert_eq!(line_target(":"), Some((None, None)));
        assert_eq!(line_target(":0"), Some((Some(1), None)));
        assert_eq!(line_target(":x"), None);
        assert_eq!(line_target("main.rs"), None);
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_name_found_whole_beats_one_scattered_and_the_shorter_wins() {
        let list = paths(&["book/src/22-maintenance.md", "src/main.rs", "src/domain.rs"]);
        assert_eq!(rank("main", &list), [1, 2, 0]);
    }

    #[test]
    fn scattered_letters_still_find_a_file() {
        let list = paths(&["src/estimator.rs", "README.md"]);
        assert_eq!(rank("estr", &list), [0]);
        assert!(rank("zzz", &list).is_empty());
    }

    #[test]
    fn an_empty_query_keeps_the_list_in_its_order() {
        let list = paths(&["b", "a", "c"]);
        assert_eq!(rank("", &list), [0, 1, 2]);
    }
}
