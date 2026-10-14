//! Compile the window's `.slint` files, with the interface languages bundled.
//!
//! The words live in one place, `rusty-i18n`'s TOML catalogues, which the
//! Rust side reads with `t!` and whose tests hold every language to the
//! English keys. The `.slint` files write `@tr("panel.files")` with the key
//! itself, and this turns each catalogue into the gettext file Slint bundles
//! — `<lang>/LC_MESSAGES/rusty-desk.po`, the key as the message id — so the
//! two halves of the window read the same words and a language switch is
//! `select_bundled_translation`, no reload.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

fn main() {
    let locales = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rusty-i18n/locales");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let bundle = out.join("translations");
    for (tag, file) in [("en", "en.toml"), ("zh-CN", "zh-CN.toml")] {
        let source = locales.join(file);
        println!("cargo:rerun-if-changed={}", source.display());
        let text = std::fs::read_to_string(&source).expect("the catalogue reads");
        let table: toml::Table = text.parse().expect("the catalogue parses");
        let mut entries = Vec::new();
        flatten("", &table, &mut entries);
        let dir = bundle.join(tag).join("LC_MESSAGES");
        std::fs::create_dir_all(&dir).expect("the bundle's directory");
        std::fs::write(dir.join("rusty-desk.po"), po(&entries)).expect("the .po is written");
    }
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations(&bundle)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("the .slint files compile");
}

/// Every string of a catalogue under its dotted key: `[menu.file]` `exit`
/// is `menu.file.exit`.
fn flatten(prefix: &str, table: &toml::Table, out: &mut Vec<(String, String)>) {
    for (key, value) in table {
        let name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::String(text) => out.push((name, text.clone())),
            toml::Value::Table(inner) => flatten(&name, inner, out),
            _ => {}
        }
    }
}

fn po(entries: &[(String, String)]) -> String {
    let mut out =
        String::from("msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\n");
    for (key, text) in entries {
        let _ = writeln!(
            out,
            "msgid \"{}\"\nmsgstr \"{}\"\n",
            quote(key),
            quote(text)
        );
    }
    out
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out
}
