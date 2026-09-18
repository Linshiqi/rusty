//! Inlay hints, drawn after the line they are about.
//!
//! VS Code draws a hint where it belongs — `let total: f32 = both();` — and
//! this editor cannot: the text is a transparent textarea over a painted
//! copy, glyph for glyph, and nothing can make room mid-line in the one
//! without every caret after it drifting in the other. So a line's hints are
//! read at its end instead, where the hints of a method chain and of a
//! closing brace already sit in VS Code: a type a binding was given names
//! the binding (`total: f32`), everything else reads as rust-analyzer wrote
//! it. Parameter names are not asked for at all (`client.rs`): `sensor:` at
//! the end of a line names nothing.

use leptos::prelude::*;

use rusty_lsp::InlayHint;

use super::*;
use crate::state::AppState;

/// What is drawn after `text` for the hints on its line, or nothing.
fn end_of_line(text: &str, hints: &[&InlayHint]) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let content_end = text.trim_end().chars().count() as u32;
    let mut shown: Vec<String> = Vec::new();
    for hint in hints.iter().filter(|hint| !hint.parameter) {
        let label = hint.label.trim();
        if label.is_empty() {
            continue;
        }
        let read = if hint.col < content_end && label.starts_with(':') {
            // The name the type belongs to: the word the hint follows.
            let end = (hint.col as usize).min(chars.len());
            let start = chars[..end]
                .iter()
                .rposition(|c| !(c.is_alphanumeric() || *c == '_'))
                .map_or(0, |at| at + 1);
            let word: String = chars[start..end].iter().collect();
            format!("{word}{label}")
        } else {
            label.to_string()
        };
        if !shown.contains(&read) {
            shown.push(read);
        }
    }
    (!shown.is_empty()).then(|| shown.join("  "))
}

/// The hints of the lines in the window, each after its line. Nothing when
/// hints are off or none have arrived for the file on screen.
pub(super) fn inlay_hints(
    state: AppState,
    path: String,
    window: Memo<std::ops::Range<u32>>,
) -> impl IntoView {
    let zoom = state.editor.zoom;
    move || {
        if !state.editor.view.with(|view| view.inlay_hints) {
            return ().into_any();
        }
        let range = window.get();
        let z = zoom.get();
        let height = row_height(z);
        let (first, last) = state.editor.folds.with(|folds| {
            (
                folds.doc_of_view(range.start),
                folds.doc_of_view(range.end.max(1) - 1),
            )
        });
        let mut on_screen: Vec<&InlayHint> = Vec::new();
        let hints = state.editor.hints.get();
        if let Some(set) = hints.as_ref().filter(|set| set.path == path) {
            on_screen.extend(
                set.hints
                    .iter()
                    .filter(|hint| first <= hint.line && hint.line <= last),
            );
        }
        if on_screen.is_empty() {
            return ().into_any();
        }
        let draft = state.editor.draft.get();
        let size = FONT_SIZE * z * 0.9;
        let mut drawn = Vec::new();
        let mut at = 0;
        for (index, text) in draft
            .split('\n')
            .enumerate()
            .skip(first as usize)
            .take((last - first + 1) as usize)
        {
            let line = index as u32;
            let mine: Vec<&InlayHint> = on_screen[at..]
                .iter()
                .take_while(|hint| hint.line == line)
                .copied()
                .collect();
            at += mine.len();
            // Inside a collapsed region the header stands for the line.
            let row = row_for(state, line);
            if mine.is_empty() || line_of_row(state, row) != line {
                continue;
            }
            let Some(shown) = end_of_line(text, &mine) else {
                continue;
            };
            let x = PAD_PX + (line_px(text) + line_px(" ")) * z;
            let y = row_top(state, line, z);
            drawn.push(view! {
                <span
                    class="pointer-events-none absolute z-[5] rounded-[3px] bg-sunken px-1 font-mono whitespace-pre text-label-3 select-none"
                    style=format!(
                        "left: {x}px; top: {y}px; height: {height}px; line-height: {height}px; \
                         font-size: {size}px",
                    )
                >
                    {shown}
                </span>
            });
        }
        drawn.collect_view().into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(line: u32, col: u32, label: &str, parameter: bool) -> InlayHint {
        InlayHint {
            line,
            col,
            label: label.to_string(),
            parameter,
        }
    }

    #[test]
    fn a_type_names_the_binding_it_belongs_to() {
        let h = hint(0, 13, ": f32", false);
        assert_eq!(
            end_of_line("    let total = both();", &[&h]).as_deref(),
            Some("total: f32")
        );
        let a = hint(0, 10, ": i32", false);
        let b = hint(0, 13, ": u8", false);
        assert_eq!(
            end_of_line("    let (a, b) = pair();", &[&a, &b]).as_deref(),
            Some("a: i32  b: u8")
        );
    }

    /// A chain's step and a closing brace are hints at the end of the line
    /// already, and read as they were written.
    #[test]
    fn a_hint_at_the_end_of_the_line_reads_as_it_is() {
        let chain = hint(0, 12, "impl Iterator<Item = &u8>", false);
        assert_eq!(
            end_of_line("    v.iter()  ", &[&chain]).as_deref(),
            Some("impl Iterator<Item = &u8>")
        );
        let brace = hint(0, 1, "fn main", false);
        assert_eq!(end_of_line("}", &[&brace]).as_deref(), Some("fn main"));
    }

    #[test]
    fn a_parameter_name_or_an_empty_label_draws_nothing() {
        let name = hint(0, 11, "sensor:", true);
        let empty = hint(0, 3, "  ", false);
        assert_eq!(end_of_line("    sample(&gyro)", &[&name, &empty]), None);
    }

    /// A type after a multi-byte name finds the name by characters.
    #[test]
    fn a_name_is_found_by_characters() {
        let h = hint(0, 9, ": i32", false);
        assert_eq!(
            end_of_line("    let 中 = 1;", &[&h]).as_deref(),
            Some("中: i32")
        );
    }
}
