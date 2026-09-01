//! Code folding: which regions can collapse, and what the editor shows when
//! they do.
//!
//! The whole difficulty of folding in this editor is one sentence: **the text
//! on screen stops being the text in the file.** The editing surface is a
//! transparent `<textarea>` over a highlighted `<pre>`, and they line up glyph
//! for glyph — so hiding a region means removing it from the textarea too, and
//! from that moment the caret, the selection, every diagnostic squiggle and
//! every overlay is positioned in a coordinate system that is not the file's.
//!
//! So the mapping lives here, in one place, and it is pure:
//!
//! - [`regions`] says what *can* fold, from indentation alone.
//! - [`Folded`] is a set of collapsed regions and the two conversions
//!   ([`Folded::view_of_doc`], [`Folded::doc_of_view`]) everything that draws
//!   must go through.
//! - [`Folded::view_text`] is what the textarea holds, and [`splice`] turns an
//!   edit made against that text back into an edit against the file.
//!
//! Model-side and IO-free, so the frontend can ask these questions about the
//! draft it is holding rather than about the copy the backend last read.
//!
//! **Indentation, not syntax.** VSCode's default folding is indentation-based
//! for the same reason: a file mid-edit does not parse, and a fold arrow that
//! vanishes while you are typing inside the function it belongs to is worse
//! than one that is occasionally offered a line early. Braces would do better
//! for Rust and nothing for TOML, and the editor holds both.

use serde::{Deserialize, Serialize};

/// A region that can be collapsed.
///
/// `header` stays on screen — it is the `fn` line, the `impl` line, the key
/// whose table follows. `last` is the final line that disappears. Folding a
/// region hides `header + 1 ..= last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub header: u32,
    pub last: u32,
}

impl Region {
    pub fn hidden(&self) -> u32 {
        self.last - self.header
    }
}

/// Every foldable region in `text`, outermost first, then by line.
///
/// A line opens a region when the next line with any content is indented
/// further; the region runs to the last line before indentation returns to
/// the header's level or less. Trailing blank lines belong to whatever comes
/// next, not to the region — folding a function should not swallow the space
/// before the one after it.
pub fn regions(text: &str) -> Vec<Region> {
    let indents: Vec<Option<usize>> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            (!trimmed.is_empty()).then(|| line.len() - trimmed.len())
        })
        .collect();

    let mut found = Vec::new();
    for (index, indent) in indents.iter().enumerate() {
        let Some(indent) = *indent else { continue };
        // Where does the body end? Scan forward past blanks for the first
        // line at or below this indentation.
        let mut last = index;
        let mut deeper = false;
        for (offset, other) in indents.iter().enumerate().skip(index + 1) {
            match other {
                // Blank lines do not end a region — a function with a blank
                // line in the middle is one region, not two.
                None => continue,
                Some(other) if *other > indent => {
                    deeper = true;
                    last = offset;
                }
                Some(_) => break,
            }
        }
        if deeper && last > index {
            found.push(Region {
                header: index as u32,
                last: last as u32,
            });
        }
    }
    found
}

/// The region whose header is `line`, if any.
pub fn region_at(text: &str, line: u32) -> Option<Region> {
    regions(text).into_iter().find(|r| r.header == line)
}

/// Which regions are currently collapsed.
///
/// Kept sorted and non-overlapping by construction: [`Folded::fold`] drops any
/// existing region contained by the new one, and refuses one that is contained
/// by an existing fold — its lines are already hidden, so collapsing it would
/// be a fold nobody could see or undo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Folded {
    regions: Vec<Region>,
}

impl Folded {
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// True when `line` is hidden by some fold. A header is never hidden by
    /// its own fold — it is what is left to click on.
    pub fn hides(&self, line: u32) -> bool {
        self.regions
            .iter()
            .any(|r| line > r.header && line <= r.last)
    }

    /// True when the fold collapsed at `line` is this line's own.
    pub fn is_folded(&self, header: u32) -> bool {
        self.regions.iter().any(|r| r.header == header)
    }

    pub fn fold(&mut self, region: Region) {
        if region.last <= region.header || self.hides(region.header) {
            return;
        }
        self.regions
            .retain(|r| !(r.header > region.header && r.last <= region.last));
        self.regions.push(region);
        self.regions.sort();
    }

    pub fn unfold(&mut self, header: u32) {
        self.regions.retain(|r| r.header != header);
    }

    pub fn toggle(&mut self, region: Region) {
        if self.is_folded(region.header) {
            self.unfold(region.header);
        } else {
            self.fold(region);
        }
    }

    /// Document lines that are on screen, in order. Index into this is the
    /// view line.
    pub fn visible(&self, total: u32) -> Vec<u32> {
        (0..total).filter(|line| !self.hides(*line)).collect()
    }

    /// Where a document line is drawn, or `None` when it is hidden.
    ///
    /// Everything that positions itself by line — a squiggle, the caret, the
    /// completion popup — has to come through here. A diagnostic drawn at its
    /// document line while the text above it is folded lands on somebody
    /// else's code, and a squiggle on the wrong line is worse than none.
    pub fn view_of_doc(&self, line: u32) -> Option<u32> {
        if self.hides(line) {
            return None;
        }
        Some(line - self.hidden_before(line))
    }

    /// The document line drawn at view line `view`.
    ///
    /// Walks the folds in header order, pushing the answer down past each one
    /// it has already passed. Order matters: a later fold's header is only
    /// compared against a position that earlier folds have already moved.
    pub fn doc_of_view(&self, view: u32) -> u32 {
        let mut doc = view;
        for region in &self.regions {
            if region.header < doc {
                doc += region.hidden();
            }
        }
        doc
    }

    /// The row that stands for a document line on screen.
    ///
    /// Its own row when it is visible; otherwise the row of the fold hiding
    /// it. This is what every overlay anchors to, and answering with the
    /// header rather than with nothing is deliberate: an error inside a
    /// collapsed function should mark the collapsed line, which is how you
    /// find out it is in there. Every editor with folding does this, and the
    /// alternative — a diagnostic that vanishes when you fold — teaches
    /// people to distrust the margin.
    pub fn row_for(&self, line: u32) -> u32 {
        let anchor = self
            .regions
            .iter()
            .find(|r| line > r.header && line <= r.last)
            .map_or(line, |r| r.header);
        anchor - self.hidden_before(anchor)
    }

    /// Hidden lines strictly above `line`. Only meaningful for a line that is
    /// itself visible, which is the only place it is called from — a fold
    /// straddling `line` would have hidden it.
    fn hidden_before(&self, line: u32) -> u32 {
        self.regions
            .iter()
            .filter(|r| r.last < line)
            .map(Region::hidden)
            .sum()
    }

    /// What the textarea holds: the visible lines, joined.
    ///
    /// A trailing newline in the document is preserved, because the textarea's
    /// value is compared against this to detect edits and a phantom newline
    /// would read as one.
    pub fn view_text(&self, text: &str) -> String {
        if self.is_empty() {
            return text.to_string();
        }
        let lines: Vec<&str> = text.lines().collect();
        let kept: Vec<&str> = lines
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.hides(*index as u32))
            .map(|(_, line)| *line)
            .collect();
        let mut out = kept.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        out
    }

    /// Drop folds the given document line range disturbed, and shift the rest.
    ///
    /// Called after an edit. A fold whose hidden body was inside the replaced
    /// range no longer describes anything, and keeping it would hide whatever
    /// happens to be at those line numbers now.
    fn adjust(&mut self, from_line: u32, to_line: u32, delta: i64) {
        self.regions
            .retain(|r| r.last < from_line || r.header > to_line);
        for region in &mut self.regions {
            if region.header > to_line {
                region.header = (i64::from(region.header) + delta) as u32;
                region.last = (i64::from(region.last) + delta) as u32;
            }
        }
    }
}

/// Turn an edit made against the folded view back into the document.
///
/// The textarea only ever holds [`Folded::view_text`], so what comes out of an
/// input event is the *view* after the edit. This finds what changed by common
/// prefix and suffix — the same technique the undo history uses to place the
/// caret — maps that span onto the document, and splices.
///
/// Deleting a selection that spanned a folded region deletes the hidden lines
/// too, which is what every editor does and what the mapping produces for
/// free: the document span between the two visible endpoints contains them.
pub fn splice(doc: &str, folds: &Folded, old_view: &str, new_view: &str) -> (String, Folded) {
    let mut folds = folds.clone();
    if old_view == new_view {
        return (doc.to_string(), folds);
    }
    if folds.is_empty() {
        return (new_view.to_string(), folds);
    }

    let prefix = common_prefix(old_view, new_view);
    let suffix = common_suffix(&old_view[prefix..], &new_view[prefix..]);
    let old_end = old_view.len() - suffix;
    let new_end = new_view.len() - suffix;

    let from = doc_offset(doc, &folds, prefix);
    let to = doc_offset(doc, &folds, old_end);

    let mut out = String::with_capacity(doc.len() + new_view.len());
    out.push_str(&doc[..from]);
    out.push_str(&new_view[prefix..new_end]);
    out.push_str(&doc[to..]);

    let from_line = line_of(doc, from);
    let to_line = line_of(doc, to);
    let removed = doc[from..to].matches('\n').count() as i64;
    let added = new_view[prefix..new_end].matches('\n').count() as i64;
    folds.adjust(from_line, to_line, added - removed);
    (out, folds)
}

/// The document byte offset that view byte offset `view` names.
fn doc_offset(doc: &str, folds: &Folded, view: usize) -> usize {
    // Walk the visible lines, spending view bytes until they run out.
    let mut remaining = view;
    let mut offset = 0usize;
    let total = doc.lines().count() as u32;
    for line in 0..total {
        let len = nth_line(doc, line).map_or(0, str::len);
        if folds.hides(line) {
            offset += len + 1;
            continue;
        }
        if remaining <= len {
            return offset + remaining;
        }
        // The newline that follows this line costs one view byte too.
        remaining -= len + 1;
        offset += len + 1;
    }
    doc.len()
}

fn nth_line(text: &str, line: u32) -> Option<&str> {
    text.lines().nth(line as usize)
}

fn line_of(text: &str, offset: usize) -> u32 {
    text[..offset.min(text.len())].matches('\n').count() as u32
}

fn common_prefix(a: &str, b: &str) -> usize {
    let mut i = 0;
    let (a, b) = (a.as_bytes(), b.as_bytes());
    while i < a.len() && i < b.len() && a[i] == b[i] {
        i += 1;
    }
    i
}

/// Bytes shared at the end, never overlapping the prefix already taken and
/// never splitting a character.
fn common_suffix(a: &str, b: &str) -> usize {
    let mut i = 0;
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    while i < ab.len()
        && i < bb.len()
        && ab[ab.len() - 1 - i] == bb[bb.len() - 1 - i]
        // A byte boundary in both, or the splice cuts a character in half and
        // the result is not valid UTF-8.
        && a.is_char_boundary(a.len() - 1 - i)
        && b.is_char_boundary(b.len() - 1 - i)
    {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: &str = "\
fn main() {
    let x = 1;
    if x > 0 {
        println!(\"hi\");
    }
}

fn other() {
    ok();
}
";

    #[test]
    fn a_function_body_is_foldable() {
        let found = regions(RUST);
        // The closing brace stays on screen: it is at the header's own
        // indentation, so it is not part of the body. `fn main() {` … `}` is
        // what an indentation-folding editor leaves behind, and hiding the
        // brace would make a folded function look unbalanced.
        assert!(found.contains(&Region { header: 0, last: 4 }), "{found:?}");
        assert!(found.contains(&Region { header: 7, last: 8 }), "{found:?}");
    }

    #[test]
    fn a_nested_block_is_its_own_region() {
        assert!(regions(RUST).contains(&Region { header: 2, last: 3 }));
    }

    /// A blank line inside a function is not the end of it. Splitting there
    /// would make every function with a paragraph break fold in two pieces.
    #[test]
    fn a_blank_line_does_not_end_a_region() {
        let text = "fn f() {\n    a();\n\n    b();\n}\n";
        assert!(regions(text).contains(&Region { header: 0, last: 3 }));
    }

    /// And a blank line *after* one belongs to what follows — folding a
    /// function must not swallow the space before the next.
    #[test]
    fn trailing_blank_lines_are_not_part_of_the_region() {
        let text = "fn f() {\n    a();\n}\n\n\nfn g() {}\n";
        let found = regions(text);
        assert!(found.contains(&Region { header: 0, last: 1 }), "{found:?}");
    }

    #[test]
    fn flat_text_folds_nothing() {
        assert_eq!(regions("one\ntwo\nthree\n"), vec![]);
        assert_eq!(regions(""), vec![]);
    }

    #[test]
    fn folding_hides_the_body_and_keeps_the_header() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        assert!(!folds.hides(0), "the header is what is left to click");
        assert!(folds.hides(1) && folds.hides(5));
        assert!(!folds.hides(6));
    }

    #[test]
    fn the_view_text_is_the_visible_lines() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        assert_eq!(
            folds.view_text(RUST),
            "fn main() {\n\nfn other() {\n    ok();\n}\n"
        );
    }

    /// The document's own trailing newline must survive, or every fold looks
    /// like an edit that removed one.
    #[test]
    fn a_trailing_newline_survives_folding() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        assert!(folds.view_text(RUST).ends_with('\n'));
        let no_newline = "fn f() {\n    a();\n}";
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 2 });
        assert_eq!(folds.view_text(no_newline), "fn f() {");
    }

    /// The conversion every overlay depends on. A squiggle drawn at its
    /// document line while text above it is folded lands on somebody else's
    /// code.
    #[test]
    fn lines_map_both_ways() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        assert_eq!(folds.view_of_doc(0), Some(0));
        assert_eq!(folds.view_of_doc(3), None, "hidden lines are not drawn");
        assert_eq!(folds.view_of_doc(6), Some(1));
        assert_eq!(folds.view_of_doc(7), Some(2));

        assert_eq!(folds.doc_of_view(0), 0);
        assert_eq!(folds.doc_of_view(1), 6);
        assert_eq!(folds.doc_of_view(2), 7);
    }

    #[test]
    fn two_folds_compose() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        folds.fold(Region { header: 7, last: 9 });
        assert_eq!(folds.view_of_doc(6), Some(1));
        assert_eq!(folds.view_of_doc(7), Some(2));
        assert_eq!(folds.view_of_doc(8), None);
        assert_eq!(folds.doc_of_view(2), 7);
    }

    /// Round-tripping is the property, not any particular number.
    #[test]
    fn every_visible_line_round_trips() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        folds.fold(Region { header: 7, last: 9 });
        for doc in folds.visible(11) {
            let view = folds.view_of_doc(doc).expect("visible");
            assert_eq!(folds.doc_of_view(view), doc, "doc line {doc}");
        }
    }

    /// A fold inside a fold is invisible and could never be undone.
    #[test]
    fn a_fold_inside_a_folded_region_is_refused() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        folds.fold(Region { header: 2, last: 4 });
        assert_eq!(folds.regions().len(), 1);
    }

    /// Folding the outer region absorbs the inner one, so unfolding the outer
    /// does not leave an orphan collapsed inside it.
    #[test]
    fn folding_an_outer_region_absorbs_the_inner_ones() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 2, last: 4 });
        folds.fold(Region { header: 0, last: 5 });
        assert_eq!(folds.regions(), &[Region { header: 0, last: 5 }]);
    }

    #[test]
    fn typing_in_visible_text_lands_in_the_document() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        let view = folds.view_text(RUST);
        let edited = view.replace("    ok();", "    ok(); // done");
        let (doc, _) = splice(RUST, &folds, &view, &edited);
        assert!(doc.contains("    ok(); // done"));
        assert!(
            doc.contains("println!(\"hi\");"),
            "the folded body must survive: {doc}"
        );
    }

    /// The line count changing is the case that breaks a naive mapping.
    #[test]
    fn adding_a_line_below_a_fold_keeps_the_fold_over_the_same_code() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 7, last: 9 });
        let view = folds.view_text(RUST);
        let edited = view.replace("fn main() {", "fn main() {\n    // new");
        let (doc, folds) = splice(RUST, &folds, &view, &edited);
        assert_eq!(
            folds.regions(),
            &[Region {
                header: 8,
                last: 10
            }]
        );
        let lines: Vec<&str> = doc.lines().collect();
        assert_eq!(lines[8], "fn other() {");
    }

    /// A fold whose body was edited no longer describes anything, and keeping
    /// it would hide whatever now sits at those line numbers.
    #[test]
    fn an_edit_through_a_fold_drops_it() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        let view = folds.view_text(RUST);
        // Select from inside the header line through the line after the fold
        // and delete — the folded body goes with it, as in every editor.
        let edited = view.replacen("fn main() {\n\nfn other", "fn gone", 1);
        let (doc, folds) = splice(RUST, &folds, &view, &edited);
        assert!(folds.is_empty(), "the fold cannot survive its own deletion");
        assert!(!doc.contains("println!"), "the hidden body goes too: {doc}");
        assert!(doc.contains("fn gone"));
    }

    #[test]
    fn an_unfolded_document_splices_to_itself() {
        let folds = Folded::default();
        let (doc, _) = splice(RUST, &folds, RUST, "changed\n");
        assert_eq!(doc, "changed\n");
    }

    /// Multibyte text must not be cut in half by the prefix/suffix scan.
    #[test]
    fn a_cjk_comment_survives_an_edit_under_a_fold() {
        let text = "fn f() {\n    // 中文注释\n}\nfn g() {\n    ok();\n}\n";
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 2 });
        let view = folds.view_text(text);
        let edited = view.replace("    ok();", "    ok2();");
        let (doc, _) = splice(text, &folds, &view, &edited);
        assert!(doc.contains("// 中文注释"), "{doc}");
        assert!(doc.contains("ok2();"));
    }

    /// An error inside a collapsed function has to mark the collapsed line —
    /// a squiggle that vanishes when you fold teaches people to distrust the
    /// margin, and a squiggle drawn at a stale row marks somebody else's code.
    #[test]
    fn a_hidden_line_is_represented_by_the_line_that_hides_it() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 4 });
        assert_eq!(folds.row_for(3), 0, "inside the fold, shown on its header");
        assert_eq!(folds.row_for(0), 0);
        // Line 5 is the closing brace, still visible, one row below.
        assert_eq!(folds.row_for(5), 1);
        assert_eq!(folds.row_for(7), 3);
    }

    #[test]
    fn row_for_agrees_with_view_of_doc_on_every_visible_line() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 4 });
        folds.fold(Region { header: 7, last: 8 });
        for doc in 0..11 {
            if let Some(view) = folds.view_of_doc(doc) {
                assert_eq!(folds.row_for(doc), view, "doc line {doc}");
            }
        }
    }

    #[test]
    fn unfolding_puts_everything_back() {
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 5 });
        folds.unfold(0);
        assert_eq!(folds.view_text(RUST), RUST);
        assert_eq!(folds.view_of_doc(3), Some(3));
    }
}
