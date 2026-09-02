//! Deciding which language this window is in.
//!
//! The catalogue itself is `rusty-i18n`; this is the part that has to know
//! about a WebView and a settings file.
//!
//! **The setting lives in `workbench.toml`** — the backend reads it, a second
//! window has to agree with the first, and losing it must not be silent. But
//! it arrives over IPC, which is a round trip, and the language has to be
//! chosen *before the first paint* or the window flashes English and then
//! redraws. So there are two stores and they are not equals:
//!
//! - `workbench.toml` is the setting.
//! - `localStorage` is a **cache of it**, written from whatever the file said,
//!   read synchronously at boot because nothing else can be.
//!
//! Losing the cache costs one reload and then heals, which is the test the
//! repository's storage rule actually asks: only this WebView cares, and
//! losing it costs a shrug.
//!
//! **Reloading is what this must not do more than once.** The first version
//! applied the system language, then read the file, then reloaded when they
//! disagreed — and `set_locale` writes an atomic in wasm memory, which the
//! reload destroys. Every boot rediscovered the same disagreement and reloaded
//! again: a window that never finished loading, in a loop with no error in it
//! anywhere. Writing the cache *before* reloading is what terminates it, and
//! it is the whole reason the cache exists.
//!
//! **Changing it reloads**, exactly as VS Code restarts for it. The
//! alternative is making several hundred call sites reactive, which buys a
//! live swap nobody asked for and risks a window that is half translated —
//! and half translated is worse than the language you did not want, because
//! you cannot tell which half is stale.

use leptos::task::spawn_local;

use rusty_i18n::Locale;

use crate::ipc::{self, cmd};

/// Where the boot cache lives. Not the setting — see the header.
const CACHE: &str = "rusty.locale";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// What the cache says, if anything. `Some(None)` is a cached "follow the
/// system", which is different from never having chosen.
fn cached() -> Option<Option<Locale>> {
    let raw = storage()?.get_item(CACHE).ok().flatten()?;
    Some(match raw.as_str() {
        "system" => None,
        tag => Some(Locale::parse(tag)?),
    })
}

fn cache(tag: Option<&str>) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(CACHE, tag.unwrap_or("system"));
    }
}

/// What the browser says the user's language is.
///
/// `navigator.language` rather than anything from the backend because this
/// has to happen before the first paint, and every IPC call is a round trip.
pub fn system_locale() -> Option<Locale> {
    let tag = web_sys::window()?.navigator().language()?;
    Locale::parse(&tag)
}

/// Choose the language for this boot, synchronously. Called before mount.
///
/// The cache wins over the system language because it is the user's own
/// answer; absent, the system language is the best guess available this early.
pub fn apply_boot_locale() {
    let chosen = match cached() {
        Some(Some(locale)) => Some(locale),
        // A cached "follow the system" still means: ask the system.
        Some(None) | None => system_locale(),
    };
    if let Some(locale) = chosen {
        rusty_i18n::set_locale(locale);
    }
}

/// Reconcile with the file, which is the setting.
///
/// Almost always agrees and does nothing. It matters when the cache is gone
/// (a cleared WebView, a fresh profile) or stale (another window changed it,
/// somebody edited the TOML), and then it corrects the cache **before**
/// reloading, so the next boot agrees and this happens at most once.
pub fn restore_locale() {
    spawn_local(async move {
        let Ok(stored) = ipc::get::<Option<String>>(cmd::workbench::LOCALE).await else {
            return;
        };
        let wanted = match stored.as_deref() {
            None => system_locale().unwrap_or_default(),
            Some(tag) => match Locale::parse(tag) {
                Some(locale) => locale,
                // A tag the catalogue does not have is not a reason to reload
                // into the same confusion every boot.
                None => return,
            },
        };
        let known = cached();
        let agreed = known == Some(stored.as_deref().and_then(Locale::parse));
        if agreed && wanted == rusty_i18n::locale() {
            return;
        }
        cache(stored.as_deref());
        if wanted != rusty_i18n::locale() {
            reload();
        }
    });
}

/// Store a choice and reload into it.
///
/// `None` means follow the system. The cache is written first and
/// synchronously: it is what the reloaded window reads before it paints, and
/// what stops [`restore_locale`] finding the same disagreement again.
pub fn choose_locale(tag: Option<String>) {
    #[derive(serde::Serialize)]
    struct Args {
        tag: Option<String>,
    }

    let current = rusty_i18n::locale();
    let wanted = match tag.as_deref() {
        None => system_locale().unwrap_or_default(),
        Some(tag) => Locale::parse(tag).unwrap_or_default(),
    };
    cache(tag.as_deref());
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::workbench::SET_LOCALE, &Args { tag }).await;
        if wanted != current {
            reload();
        }
    });
}

/// A tool's purpose, in this window's language.
///
/// The backend names the tool and describes it in English; the name is the
/// stable half, so that is what the catalogue is keyed on. A tool with no
/// entry — one added since, or one from somebody's own catalogue file — keeps
/// the backend's own words rather than showing a key. That is
/// refuse-rather-than-guess applied to translation.
///
/// **The same binary is described twice, for two jobs.** The Toolchain panel
/// lists what a tool is; the setup sheet is asking permission to run it, and
/// says more. `espup` has a different sentence in each, so the setup sheet
/// looks in its own namespace first — one wording answering for the other
/// would be a quiet mistranslation rather than a visible one.
///
/// `target:<triple>` collapses to one key: the triple is in the *name*, and
/// the sentence itself is fixed.
pub fn tool_purpose(name: &str, english: &str) -> String {
    purpose_in("tool", name, english)
}

/// The setup sheet's wording for the same tool. Falls through to
/// [`tool_purpose`]'s namespace, then to the backend's English.
pub fn setup_purpose(name: &str, english: &str) -> String {
    let name = if name.starts_with("target:") {
        "target"
    } else {
        name
    };
    rusty_i18n::translate(&format!("setup.purpose.{name}"))
        .unwrap_or_else(|| purpose_in("tool", name, english))
}

/// What the setup sheet shows instead of a purpose when a step is manual.
pub fn setup_manual(name: &str, english: &str) -> String {
    purpose_in("setup.manual", name, english)
}

fn purpose_in(section: &str, name: &str, english: &str) -> String {
    rusty_i18n::translate(&format!("{section}.{name}")).unwrap_or_else(|| english.to_string())
}

/// A diagnostic's title and detail, in this window's language.
///
/// The backend sends both the English and the `kind` that names *which*
/// diagnostic this is, with the interpolated values kept apart in `args`. So
/// the sentence can be looked up and refilled; a `kind` with no entry — a
/// diagnostic added since, or a language that has not caught up — keeps the
/// English, which is a sentence rather than a key.
pub fn problem_text(problem: &rusty_embed::Problem) -> (String, String) {
    let one = |suffix: &str, english: &str| {
        rusty_i18n::translate(&format!("problem.{}-{suffix}", problem.kind)).map_or_else(
            || english.to_string(),
            |text| {
                let args: Vec<(&str, String)> = problem
                    .args
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.clone()))
                    .collect();
                rusty_i18n::fill(&text, &args)
            },
        )
    };
    (one("title", &problem.title), one("detail", &problem.detail))
}

fn reload() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}

#[cfg(test)]
mod tests {
    /// Every diagnostic the backend can emit has a catalogue entry.
    ///
    /// Falling back to the backend's English is the *correct* behaviour for a
    /// `kind` nobody has translated — but for rusty's own diagnostics it is a
    /// gap somebody should be told about, and silently showing English is how
    /// a gap survives a release. So the check reads `rusty-embed`'s source for
    /// the `kind` each `Problem::new` names, and requires both halves.
    #[test]
    fn every_diagnostic_kind_is_in_the_catalogue() {
        let embed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("rusty-embed")
            .join("src");
        let known = rusty_i18n::keys(rusty_i18n::Locale::English);

        let mut kinds = Vec::new();
        for entry in std::fs::read_dir(&embed).expect("read rusty-embed/src") {
            let path = entry.expect("entry").path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source");
            // `Problem::new(severity, "kind", …)` — the kind is the first
            // literal in the argument list. Bounded at the first builder call,
            // or `.arg("crates", …)` answers for the kind and the test starts
            // demanding entries for argument names.
            for (index, _) in text.match_indices("Problem::new(") {
                let rest = &text[index..];
                let end = rest.find(".arg(").unwrap_or(rest.len());
                let end = end.min(rest.find(".fix(").unwrap_or(rest.len()));
                let args = &rest[..end];
                let Some(open) = args.find('"') else { continue };
                let Some(close) = args[open + 1..].find('"') else {
                    continue;
                };
                kinds.push(args[open + 1..open + 1 + close].to_string());
            }
        }

        assert!(
            !kinds.is_empty(),
            "found no `Problem::new` call sites — the scan is broken, not the catalogue"
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
            "these diagnostics have no catalogue entry:\n  {}",
            missing.join("\n  ")
        );
    }

    /// Every `t!("…")` in this crate names a key the catalogue defines.
    ///
    /// The macro cannot check this — it has no view of the TOML at expansion
    /// time — and `lookup`'s debug assertion only fires if somebody happens
    /// to open the screen the key is on. So the check reads the source
    /// instead: a key added to a call site without a catalogue entry fails
    /// here, in CI, rather than showing `setup.title` on a button.
    #[test]
    fn every_key_used_in_the_source_exists_in_the_catalogue() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let known = rusty_i18n::keys(rusty_i18n::Locale::English);

        let mut used = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                // Comments are dropped first: a doc comment showing what a
                // call site looks like is documentation, not a call site,
                // and this test's own header is the proof.
                let text: String = std::fs::read_to_string(&path)
                    .expect("read source")
                    .lines()
                    .map(|line| match line.find("//") {
                        Some(at) => &line[..at],
                        None => line,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                for (index, _) in text.match_indices("t!(\"") {
                    // `format!("…")` ends in `t!("` too. The macro's name is
                    // one character long, so what precedes it has to not be
                    // part of an identifier.
                    let preceded_by_a_name = text[..index]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_');
                    if preceded_by_a_name {
                        continue;
                    }
                    let rest = &text[index + 4..];
                    if let Some(end) = rest.find('"') {
                        used.push((path.clone(), rest[..end].to_string()));
                    }
                }
            }
        }

        assert!(
            !used.is_empty(),
            "found no `t!` call sites at all — the scan is broken, not the catalogue"
        );
        let missing: Vec<String> = used
            .iter()
            .filter(|(_, key)| !known.contains(key))
            .map(|(path, key)| {
                format!(
                    "{key}  ({})",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )
            })
            .collect();
        assert!(
            missing.is_empty(),
            "these keys are used but not defined in locales/en.toml:\n  {}",
            missing.join("\n  ")
        );
    }
}
