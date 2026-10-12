//! The message catalogue.
//!
//! One crate, no IO, compiles to wasm — so the frontend looks a string up the
//! same way the backend will when the diagnostics move over.
//!
//! ## What is here and what is deliberately not
//!
//! Interface text: menus, buttons, panel headings, empty states, the setup
//! screen. **Not** translated, on purpose:
//!
//! - **Command lines, tool names, chip ids and target triples.** People type
//!   these, search for them and paste them into issues.
//! - **The dock's output.** That is cargo, espflash and QEMU speaking, not us.
//! - **The assistant's prompts and tool descriptions.** Those are written
//!   against specific failure modes and are read by a model, not a person;
//!   translating them changes what the assistant does.
//!
//! ## Missing keys fail loudly in debug and quietly in release
//!
//! A key with no entry is a bug in this repository, not in the user's day —
//! so [`t`] panics while developing and falls back to English (then to the
//! key itself) in a shipped build. A user should never see `setup.title` on
//! a button, and a contributor should never be able to add one without a
//! test failing.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// A display language.
///
/// English is not a catalogue entry that can go missing — it is the source
/// text and the fallback, so it is the enum's default and the file it reads
/// from is the one every other file is checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    English,
    SimplifiedChinese,
}

impl Locale {
    /// The tag stored in `workbench.toml` and shown in Settings.
    pub fn tag(self) -> &'static str {
        match self {
            Locale::English => "en",
            Locale::SimplifiedChinese => "zh-CN",
        }
    }

    /// The language's own name for itself. Never "Chinese (Simplified)" in
    /// English: somebody looking for their language in a list they cannot
    /// currently read finds it by how it looks, not by its English name.
    pub fn endonym(self) -> &'static str {
        match self {
            Locale::English => "English",
            Locale::SimplifiedChinese => "简体中文",
        }
    }

    pub const ALL: &'static [Locale] = &[Locale::English, Locale::SimplifiedChinese];

    /// Parse a BCP-47-ish tag, as loosely as the sources it comes from.
    ///
    /// `navigator.language` says `zh-CN`, a Windows locale says `zh_CN`, and
    /// somebody editing `workbench.toml` by hand says `zh`. All three mean
    /// the same thing, and a settings file that silently reverts to English
    /// because of a hyphen is a bug nobody can see.
    pub fn parse(tag: &str) -> Option<Locale> {
        let tag = tag.trim().to_ascii_lowercase().replace('_', "-");
        match tag.split('-').next()? {
            "en" => Some(Locale::English),
            // Only Simplified for now. `zh-TW` and `zh-HK` are *not* folded
            // in: the vocabulary genuinely differs — 軟體 against 软件,
            // 程式 against 程序 — and serving Simplified to somebody who
            // asked for Traditional is worse than serving them English,
            // which they can at least tell is not their language.
            "zh" => match tag.as_str() {
                "zh" | "zh-cn" | "zh-hans" | "zh-sg" | "zh-hans-cn" => {
                    Some(Locale::SimplifiedChinese)
                }
                _ => None,
            },
            _ => None,
        }
    }
}

/// Every catalogue, embedded. A file that does not parse is caught by
/// `every_catalogue_parses` rather than by a user opening the app.
const CATALOGUES: &[(Locale, &str)] = &[
    (Locale::English, include_str!("../locales/en.toml")),
    (
        Locale::SimplifiedChinese,
        include_str!("../locales/zh-CN.toml"),
    ),
];

/// The active locale, as a discriminant.
///
/// A plain atomic rather than a signal: changing the display language
/// reloads the window, exactly as VS Code restarts for it. The alternative —
/// making every one of several hundred call sites reactive — buys a live
/// swap nobody asked for and risks a half-translated screen.
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Set the language. Takes effect for text rendered after this point, which
/// in practice means: call it before the first paint.
pub fn set_locale(locale: Locale) {
    CURRENT.store(
        match locale {
            Locale::English => 0,
            Locale::SimplifiedChinese => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn locale() -> Locale {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Locale::SimplifiedChinese,
        _ => Locale::English,
    }
}

type Catalogue = HashMap<&'static str, HashMap<String, String>>;

fn catalogues() -> &'static Catalogue {
    static PARSED: OnceLock<Catalogue> = OnceLock::new();
    PARSED.get_or_init(|| {
        CATALOGUES
            .iter()
            .map(|(locale, text)| (locale.tag(), flatten(text)))
            .collect()
    })
}

/// TOML tables become dotted keys: `[setup] title = "…"` is `setup.title`.
///
/// Nested rather than flat in the file because a flat list of four hundred
/// dotted keys is a file nobody can find anything in, and the grouping is
/// the only structure a translator gets.
fn flatten(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(table) = text.parse::<toml::Table>() else {
        return out;
    };
    walk(&table, String::new(), &mut out);
    out
}

fn walk(table: &toml::Table, prefix: String, out: &mut HashMap<String, String>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::String(text) => {
                out.insert(path, text.clone());
            }
            toml::Value::Table(inner) => walk(inner, path, out),
            _ => {}
        }
    }
}

/// Look one key up, answering `None` when the catalogue does not have it.
///
/// For text that arrives from the *backend* carrying its own English. The
/// backend sends a stable name — a tool's, a chip's — and the frontend
/// translates it if it can; a name added later, or one from somebody's own
/// TOML, has no entry here and must fall back to what the backend said rather
/// than to a missing-key placeholder. That is refuse-rather-than-guess applied
/// to translation: no entry means no claim, not an invented one.
///
/// Distinct from [`lookup`] on purpose — [`lookup`]'s missing key is a bug and
/// asserts, and an absent optional entry must not trip that assertion.
pub fn translate(key: &str) -> Option<String> {
    let catalogues = catalogues();
    catalogues
        .get(locale().tag())
        .and_then(|c| c.get(key))
        .or_else(|| {
            catalogues
                .get(Locale::English.tag())
                .and_then(|c| c.get(key))
        })
        .cloned()
}

/// Look one key up in the active language.
///
/// Falls back to English, then to the key itself. The key is the last resort
/// and is never meant to be seen: [`t!`] panics on it in debug builds.
pub fn lookup(key: &str) -> String {
    let catalogues = catalogues();
    if let Some(text) = catalogues.get(locale().tag()).and_then(|c| c.get(key)) {
        return text.clone();
    }
    if let Some(text) = catalogues
        .get(Locale::English.tag())
        .and_then(|c| c.get(key))
    {
        // A missing translation shows the English, which is a language the
        // reader may not have but is at least a sentence. A missing *key* is
        // a bug, and the debug assertion below is where it surfaces.
        return text.clone();
    }
    debug_assert!(false, "no catalogue entry for `{key}`");
    key.to_string()
}

/// Substitute `{name}` placeholders.
///
/// Left in place when an argument is missing rather than blanked: `{count}`
/// on screen says which argument the caller forgot, where an empty space
/// says a number is zero.
pub fn fill(template: &str, args: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Every key a catalogue defines, for the cross-checking tests.
pub fn keys(locale: Locale) -> Vec<String> {
    let mut keys: Vec<String> = catalogues()
        .get(locale.tag())
        .map(|c| c.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}

/// Translate.
///
/// ```ignore
/// t!("dock.problems")
/// t!("setup.missing.body", count = 4)
/// ```
#[macro_export]
macro_rules! t {
    ($key:literal) => {{ $crate::lookup($key) }};
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        $crate::fill(
            &$crate::lookup($key),
            &[$((stringify!($name), ::std::string::ToString::to_string(&$value))),+],
        )
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A catalogue that does not parse would fall back to English silently
    /// and completely — the whole language quietly missing, with nothing to
    /// see. This is the check that makes it a build-time failure instead.
    #[test]
    fn every_catalogue_parses_and_is_not_empty() {
        for (locale, text) in CATALOGUES {
            assert!(
                text.parse::<toml::Table>().is_ok(),
                "{} does not parse as TOML",
                locale.tag()
            );
            assert!(
                !flatten(text).is_empty(),
                "{} parsed to nothing",
                locale.tag()
            );
        }
    }

    /// English is the source text, so every other catalogue is measured
    /// against it: an extra key is a leftover from a string that was
    /// reworded, and a missing one is a screen that silently reverts.
    #[test]
    fn every_language_has_exactly_the_english_keys() {
        let english = keys(Locale::English);
        for locale in Locale::ALL.iter().filter(|l| **l != Locale::English) {
            let theirs = keys(*locale);
            let missing: Vec<&String> = english.iter().filter(|k| !theirs.contains(k)).collect();
            let extra: Vec<&String> = theirs.iter().filter(|k| !english.contains(k)).collect();
            assert!(
                missing.is_empty(),
                "{} is missing {} keys: {:?}",
                locale.tag(),
                missing.len(),
                &missing[..missing.len().min(10)]
            );
            assert!(
                extra.is_empty(),
                "{} has keys English does not: {:?}",
                locale.tag(),
                extra
            );
        }
    }

    /// A translation that dropped a `{count}` renders a sentence with a hole
    /// in it, and one that invented a `{name}` renders the braces. Neither
    /// shows up until somebody switches language and looks at that one
    /// screen, which is exactly the bug this catches.
    #[test]
    fn placeholders_survive_translation() {
        let placeholder = |text: &str| -> Vec<String> {
            let mut found: Vec<String> = text
                .split('{')
                .skip(1)
                .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_string()))
                .collect();
            found.sort();
            found
        };
        let english = flatten(CATALOGUES[0].1);
        for (locale, text) in &CATALOGUES[1..] {
            for (key, translated) in flatten(text) {
                let Some(source) = english.get(&key) else {
                    continue;
                };
                assert_eq!(
                    placeholder(source),
                    placeholder(&translated),
                    "{} `{key}` does not carry the same placeholders",
                    locale.tag()
                );
            }
        }
    }

    /// The things that must stay in English, checked rather than trusted: a
    /// translated command line is one somebody cannot run, and a translated
    /// target triple is one cargo has never heard of.
    #[test]
    fn no_catalogue_translates_a_command_or_a_triple() {
        const LITERAL: &[&str] = &[
            "cargo",
            "rustup",
            "espflash",
            "espup",
            "probe-rs",
            "xtensa-esp32-none-elf",
            "riscv32imc-unknown-none-elf",
        ];
        for (locale, text) in &CATALOGUES[1..] {
            let english = flatten(CATALOGUES[0].1);
            for (key, translated) in flatten(text) {
                let Some(source) = english.get(&key) else {
                    continue;
                };
                for word in LITERAL {
                    assert_eq!(
                        source.contains(word),
                        translated.contains(word),
                        "{} `{key}`: `{word}` must appear in both or neither",
                        locale.tag()
                    );
                }
            }
        }
    }

    #[test]
    fn a_locale_tag_is_read_as_loosely_as_it_arrives() {
        assert_eq!(Locale::parse("zh-CN"), Some(Locale::SimplifiedChinese));
        assert_eq!(Locale::parse("zh_CN"), Some(Locale::SimplifiedChinese));
        assert_eq!(Locale::parse("zh"), Some(Locale::SimplifiedChinese));
        assert_eq!(Locale::parse("en-GB"), Some(Locale::English));
        assert_eq!(Locale::parse("  EN  "), Some(Locale::English));
        assert_eq!(Locale::parse("fr"), None);
    }

    /// Traditional Chinese is a different language here, not a font choice.
    /// 軟體/软件 and 程式/程序 are different words, and serving Simplified
    /// to somebody who asked for Traditional is worse than serving English,
    /// which they can at least tell is not theirs.
    #[test]
    fn traditional_chinese_is_not_served_the_simplified_catalogue() {
        assert_eq!(Locale::parse("zh-TW"), None);
        assert_eq!(Locale::parse("zh-HK"), None);
        assert_eq!(Locale::parse("zh-Hant"), None);
    }

    #[test]
    fn a_missing_argument_leaves_its_name_visible() {
        assert_eq!(fill("{a} and {b}", &[("a", "1".into())]), "1 and {b}");
    }
}
