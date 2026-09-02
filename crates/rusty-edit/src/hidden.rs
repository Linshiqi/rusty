//! The one rule for what the workbench does not look at.
//!
//! Three walkers used to carry three versions of it: the file tree hid every
//! dot entry, search excluded only `.git`, and the watcher ignored every dot
//! component. So search surfaced `.cargo/config.toml` and `.rusty/sim.toml` —
//! files the tree would never show — and a replace could rewrite them, while
//! the module doc above it promised the opposite. One predicate, three
//! callers, and a test beside each caller that the answer agrees.

/// Whether a directory entry is one the workbench never shows, searches or
/// watches: anything dot-named.
///
/// `.git` alone is thousands of files nobody is text-searching; `.cargo` and
/// `.rusty` are edited through their own panels, not by hand. There was a
/// toggle to reveal them once and it earned its keep for nobody.
pub(crate) fn hidden_entry(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_entries_are_hidden_and_nothing_else_is() {
        for hidden in [".git", ".cargo", ".rusty", ".gitignore", ".hidden.rs"] {
            assert!(hidden_entry(hidden), "{hidden}");
        }
        for shown in ["src", "Cargo.toml", "target", "a.b.c"] {
            assert!(!hidden_entry(shown), "{shown}");
        }
    }
}
