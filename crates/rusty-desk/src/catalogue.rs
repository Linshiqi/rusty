//! The window's words are keys of `rusty-i18n`'s catalogue — `t!("…")` in
//! the Rust, `@tr("…")` in the `.slint` files — and a key missing from the
//! catalogue would show as the key itself. This holds every key the window
//! names to the catalogue, as `rusty-ui`'s test did for the WebView.

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
}
