//! What a failing test said, beside the line it failed at, as VS Code
//! writes a test's message after its line: the first line of the panic,
//! over the red line the echo draws there (`state::Tests::marks_in`), with
//! the whole of it on the card. An overlay like the lens, so the text is
//! never moved by it.

use super::*;

/// The notes on the lines in the window.
#[component]
pub(super) fn FailureNotes(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        zoom,
        window,
        ..
    } = pane;
    let path = pane.path.get_value();
    move || {
        let notes: Vec<(u32, String)> = state.tests.failures.with(|failures| {
            failures
                .iter()
                .filter_map(|failure| {
                    let (file, line, _) = failure.at.as_ref()?;
                    (*file == path).then(|| (*line, note_of(&failure.message)))
                })
                .collect()
        });
        if notes.is_empty() {
            return ().into_any();
        }
        let draft = state.editor.draft.get();
        let lines: Vec<&str> = draft.split('\n').collect();
        let z = zoom.get();
        let height = row_height(z);
        let range = window.get();
        notes
            .into_iter()
            .filter_map(|(line, note)| {
                // Inside a collapsed region the header stands for the line,
                // and its own text is what the note would sit after.
                let row = row_for(state, line);
                if line_of_row(state, row) != line || !range.contains(&row) {
                    return None;
                }
                let content = lines.get(line as usize).copied()?;
                let x = line_right(content, &hints_on(state, &path, line), z) + line_px("   ") * z;
                let y = row_top(state, line, z);
                Some(view! {
                    <div
                        class="pointer-events-none absolute z-10 flex max-w-[60ch] items-center font-sans text-footnote leading-none text-crimson opacity-80 select-none"
                        style=format!("left: {x}px; top: {y}px; height: {height}px")
                    >
                        <span class="min-w-0 truncate">{note}</span>
                    </div>
                })
            })
            .collect_view()
            .into_any()
    }
}

/// The first line of what a test said — `assertion `left == right`
/// failed`, `Vector { x: 0.0, y: -1.5707963, z: 0.0 }` — which is the line
/// that says what went wrong; the rest is on the card.
fn note_of(message: &str) -> String {
    let first = message
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    format!("✗ {}", first.trim())
}

#[cfg(test)]
mod tests {
    use super::note_of;

    #[test]
    fn a_note_is_the_first_line_that_says_something() {
        assert_eq!(
            note_of("assertion `left == right` failed\n  left: 3\n right: 4"),
            "✗ assertion `left == right` failed"
        );
        assert_eq!(note_of("\n  boom  "), "✗ boom");
        assert_eq!(note_of(""), "✗ ");
    }
}
