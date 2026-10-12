//! The echo: the highlighted text under the transparent textarea, a window
//! of rows between spacers, each row keyed on what is drawn on it.

use super::*;

/// The painted layer the textarea sits over.
#[component]
pub(super) fn Echo(pane: Pane) -> impl IntoView {
    let Pane {
        unlinked,
        zoom,
        rows_total,
        window,
        widest,
        lit_guide,
        metrics,
        ..
    } = pane;
    view! {
        <pre
            class=move || {
                let base = "pointer-events-none m-0 overflow-visible py-2 pr-4 \
                            pl-2 whitespace-pre";
                // Drained when no `mod` declares the file, because
                // rust-analyzer is not analysing a word of it. The
                // name being dim in the tree, the tab and the header
                // is missable — on a selected row it is a shade
                // against a highlight — and the code is where the
                // eye actually is.
                //
                // This goes past VS Code deliberately, and the
                // protocol is why: `unlinked-file` arrives as a
                // Hint over *two characters* with no `Unnecessary`
                // tag, so there is nothing for VS Code's
                // `editorUnnecessaryCode.opacity` to act on and it
                // dims nothing. rusty knows more than the
                // diagnostic does — it reads the `mod` lines
                // itself, before the file is ever opened.
                //
                // The squiggle dims with the text rather than being
                // exempted: opacity compounds through a parent, and
                // a two-character mark at 60% is still plainly a
                // mark. Hovering it is unaffected — that is the
                // textarea's job, and the textarea is not dimmed.
                if unlinked.get() {
                    format!("{base} opacity-60")
                } else {
                    base.to_string()
                }
            }
            // At least as wide as the widest line, drawn or not:
            // the textarea over this column holds every line, and
            // one wider than the column scrolls inside itself —
            // the caret drifting off its glyph.
            style=move || {
                format!(
                    "{}; min-width: {}px",
                    metrics.get(),
                    widest.get() * zoom.get() + PAD_PX + 16.0,
                )
            }
            aria-hidden="true"
        >
            <div style=move || pane.spacer(window.get().start)></div>
            // Keyed by the line and everything drawn on it, so a
            // keystroke rebuilds the row it changed and a scroll
            // the rows it brought in (`EchoRow`).
            <For
                each=move || pane.echo_rows()
                key=|row| row.key
                children=move |row: EchoRow| {
                    // A collapsed header says how much is
                    // underneath it. A bare `…` gives no sense of
                    // whether unfolding costs three lines or three
                    // hundred.
                    let summary = row
                        .folded
                        .map(|n| {
                            let unit = if n == 1 { "line" } else { "lines" };
                            view! {
                                <span class="rounded-[3px] bg-selection px-1 text-label-3">
                                    {format!(" ⋯ {n} {unit} ")}
                                </span>
                            }
                        });
                    // A line one pixel wide at each stop the
                    // line is indented past, in `ch` so a zoom
                    // moves it with the text; brighter for the
                    // block the caret is in.
                    let index = row.index as usize;
                    let guides = (0..row.guides)
                        .map(|stop| {
                            let class = move || {
                                let lit = lit_guide.with(|lit| {
                                    lit.is_some_and(|(at, first, last)| {
                                        at == stop && (first..=last).contains(&index)
                                    })
                                });
                                if lit {
                                    "pointer-events-none absolute inset-y-0 w-px bg-label-4"
                                } else {
                                    "pointer-events-none absolute inset-y-0 w-px bg-line"
                                }
                            };
                            let left = u32::from(stop) * TAB_SIZE as u32;
                            view! { <span class=class style=format!("left: {left}ch") /> }
                        })
                        .collect_view();
                    view! {
                        <div class="relative">
                            {guides}
                            {decorate(row.line, row.index, &row.diags, &row.hints, row.link)}
                            {summary}
                            // An empty line still occupies one, or
                            // the caret above sits a row too high
                            // for the rest of the file.
                            {"\u{200b}"}
                        </div>
                    }
                }
            />
            <div style=move || pane.spacer(rows_total.get().saturating_sub(window.get().end))></div>
        </pre>
    }
}
