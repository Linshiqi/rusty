//! Three rules about the window's source, held by reading it.
//!
//! The window's words are keys of `rusty-i18n`'s catalogue — `t!("…")` in
//! the Rust, `@tr("…")` in the `.slint` files — and a key missing from the
//! catalogue would show as the key itself. This holds every key the window
//! names to the catalogue.
//!
//! Every diagnostic `rusty-embed` can raise has its words there too. The
//! window translates a `Problem` by its `kind` (`problem.<kind>-title` and
//! `-detail`) and falls back to the English it carries — correct for a kind
//! nobody has translated, and exactly how a gap in rusty's own survives a
//! release unnoticed.
//!
//! And the window names no part: everything that differs between two
//! chips is a key of the chip catalogue (CLAUDE.md, rule 6), and a chip
//! id in the window's code is a `match` that does the wrong thing for the
//! next part added — silently, which is how seventy-five of them once sent
//! every STM32 to esp-generate.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn files(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("the directory reads") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                files(&path, extension, out);
            } else if path.extension().is_some_and(|e| e == extension) {
                out.push(path);
            }
        }
    }

    /// Every string literal right after `opener`, skipping the whitespace
    /// rustfmt puts between a macro's paren and its first argument. Only
    /// where `opener` starts a word: `format!(` ends in `t!(` too, and read
    /// as a translation every format string was a missing key.
    fn keys_after(text: &str, opener: &str) -> Vec<String> {
        let mut keys = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find(opener) {
            let word = rest[..at]
                .chars()
                .next_back()
                .is_none_or(|before| !(before.is_alphanumeric() || before == '_'));
            rest = &rest[at + opener.len()..];
            if !word {
                continue;
            }
            let after = rest.trim_start();
            if let Some(quoted) = after.strip_prefix('"')
                && let Some(end) = quoted.find('"')
            {
                keys.push(quoted[..end].to_string());
            }
        }
        keys
    }

    fn without_comments(text: &str) -> String {
        text.lines()
            .map(|line| match line.find("//") {
                Some(at) => &line[..at],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_key_the_window_names_is_in_the_catalogue() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let known = rusty_i18n::keys(rusty_i18n::Locale::English);
        let mut used = Vec::new();
        let mut rust = Vec::new();
        files(&root.join("src"), "rs", &mut rust);
        // Not this file: its own scanning names the openers in strings.
        rust.retain(|path| !path.ends_with("catalogue.rs"));
        for path in rust {
            let text = without_comments(&std::fs::read_to_string(&path).unwrap());
            for key in keys_after(&text, "t!(") {
                used.push((path.clone(), key));
            }
        }
        let mut slint = Vec::new();
        files(&root.join("ui"), "slint", &mut slint);
        for path in slint {
            let text = without_comments(&std::fs::read_to_string(&path).unwrap());
            for key in keys_after(&text, "@tr(") {
                used.push((path.clone(), key));
            }
        }
        assert!(used.len() > 20, "the scan found almost nothing: {used:?}");
        let missing: Vec<_> = used
            .iter()
            .filter(|(_, key)| !known.contains(key))
            .map(|(path, key)| format!("{key} ({})", path.display()))
            .collect();
        assert!(missing.is_empty(), "not in the catalogue: {missing:#?}");
    }

    /// Every `Problem::new` in `rusty-embed` names a kind with both halves
    /// in the catalogue. The kind is the first string literal of the call,
    /// read up to the first builder call, or `.arg("crates", …)` would
    /// answer for it.
    #[test]
    fn every_diagnostic_kind_is_in_the_catalogue() {
        let embed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rusty-embed/src");
        let known = rusty_i18n::keys(rusty_i18n::Locale::English);
        let mut sources = Vec::new();
        files(&embed, "rs", &mut sources);
        let mut kinds = Vec::new();
        for path in sources {
            let text = std::fs::read_to_string(&path).unwrap();
            for (at, _) in text.match_indices("Problem::new(") {
                let rest = &text[at..];
                let end = [".arg(", ".fix("]
                    .iter()
                    .filter_map(|call| rest.find(call))
                    .min()
                    .unwrap_or(rest.len());
                let args = &rest[..end];
                let Some(open) = args.find('"') else { continue };
                let Some(close) = args[open + 1..].find('"') else {
                    continue;
                };
                kinds.push(args[open + 1..open + 1 + close].to_string());
            }
        }
        assert!(
            kinds.len() > 10,
            "found almost no `Problem::new` — the scan is broken, not the catalogue"
        );
        let missing: Vec<String> = kinds
            .iter()
            .flat_map(|kind| {
                [
                    format!("problem.{kind}-title"),
                    format!("problem.{kind}-detail"),
                ]
            })
            .filter(|key| !known.contains(key))
            .collect();
        assert!(
            missing.is_empty(),
            "diagnostics with no words: {missing:#?}"
        );
    }

    /// A chip's family, as its id starts: `esp32` of `esp32c3`, `stm32` of
    /// `stm32f411ce`, `ch32` of `ch32v003j4m6`.
    fn family(id: &str) -> String {
        let letters = id.chars().take_while(|c| c.is_ascii_alphabetic()).count();
        let digits = id[letters..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .count();
        id[..letters + digits].to_string()
    }

    /// The names of every part and vendor the catalogue knows — ids,
    /// families — appear nowhere in the window's code. Comments may tell
    /// the history; the code may not decide by it.
    #[test]
    fn the_window_names_no_part() {
        let catalog = rusty_embed::catalog::Catalog::builtin();
        let mut names: Vec<String> = Vec::new();
        for chip in catalog.chips() {
            names.push(chip.id.to_lowercase());
            names.push(family(&chip.id.to_lowercase()));
        }
        names.extend(catalog.vendors().iter().map(|v| v.id.to_lowercase()));
        names.retain(|name| name.len() >= 3);
        names.sort();
        names.dedup();
        assert!(
            names.len() > 10,
            "the catalogue named almost nothing: {names:?}"
        );

        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut sources = Vec::new();
        files(&root.join("src"), "rs", &mut sources);
        files(&root.join("ui"), "slint", &mut sources);
        sources.retain(|path| !path.ends_with("catalogue.rs"));
        let mut found = Vec::new();
        for path in sources {
            let text = without_comments(&std::fs::read_to_string(&path).unwrap());
            for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                let word = word.to_lowercase();
                if names.binary_search(&word).is_ok() {
                    found.push(format!("{word} ({})", path.display()));
                }
            }
        }
        assert!(found.is_empty(), "the window names parts: {found:#?}");
    }
}
