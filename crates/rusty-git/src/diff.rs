//! A unified patch read into rows, so a view can lay it out either way.
//!
//! `git` speaks unified diff and nothing else; a side-by-side view is a
//! *reading* of that text, not a second thing to ask for. The reading is done
//! here, once, pure and under tests, and the view only decides what a row
//! looks like. Pure so it compiles to wasm — the frontend does the reading as
//! the patch arrives — and so the pairing rules are tests rather than
//! something discovered by scrolling.
//!
//! The pairing follows Fork: within a hunk, a run of removed lines and the run
//! of added lines that follows it are laid side by side index for index, and
//! whichever run is shorter leaves blanks on its side. Context lines sit on
//! both sides with their own numbers. `\ No newline at end of file` belongs to
//! the side of the line before it and takes a row of its own there.

/// One `@@ … @@` section of a patch, read both ways.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The header line as git printed it, function context included.
    pub header: String,
    /// The two-column reading.
    pub rows: Vec<Row>,
    /// The one-column reading: git's own order, each line with the numbers
    /// it has on whichever side it exists. Kept alongside the rows rather
    /// than rebuilt from them, because a note pushed into a removed run
    /// would come back out after the additions it preceded.
    pub lines: Vec<Line>,
}

/// One line of the unified reading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
    pub kind: CellKind,
}

/// One line of the two-column view. A side is `None` where the other side
/// has a line with nothing to pair it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub left: Option<Cell>,
    pub right: Option<Cell>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The line's number in its file; `None` for a note, which is not a line.
    pub number: Option<u32>,
    /// The line without its leading `+`, `-` or space.
    pub text: String,
    pub kind: CellKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file` — about the line above it.
    Note,
}

/// The hunks of a unified patch, skipping the file header (`diff --git`,
/// `index`, `---`, `+++`) that precedes the first `@@`. Text before the first
/// hunk that is not a header — a binary notice, say — yields no hunks.
pub fn hunks(patch: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Building> = None;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            if let Some(done) = current.take() {
                hunks.push(done.finish());
            }
            let (old_start, new_start) = starts(rest);
            current = Some(Building::new(line.to_string(), old_start, new_start));
            continue;
        }
        let Some(building) = current.as_mut() else {
            continue;
        };
        if let Some(text) = line.strip_prefix('+') {
            building.added(text);
        } else if let Some(text) = line.strip_prefix('-') {
            building.removed(text);
        } else if let Some(text) = line.strip_prefix('\\') {
            building.note(text.trim_start());
        } else if let Some(text) = line.strip_prefix(' ') {
            building.context(text);
        } else if line.is_empty() {
            // Some tools trim the single space off an empty context line.
            building.context("");
        } else {
            // Anything else is the start of another file's header, which the
            // caller has already split off, or garbage; it ends the hunk.
            break;
        }
    }
    if let Some(done) = current.take() {
        hunks.push(done.finish());
    }
    hunks
}

/// The two starting line numbers out of ` -12,7 +12,8 @@ fn main()`. A
/// missing count means one line, and a missing number is a zero-length
/// side — the `-0,0` of a new file.
fn starts(header_rest: &str) -> (u32, u32) {
    let mut old = 1;
    let mut new = 1;
    for word in header_rest.split_whitespace().take(2) {
        if let Some(spec) = word.strip_prefix('-') {
            old = first_number(spec);
        } else if let Some(spec) = word.strip_prefix('+') {
            new = first_number(spec);
        }
    }
    (old, new)
}

fn first_number(spec: &str) -> u32 {
    spec.split(',')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
}

/// A hunk under construction: numbering both sides, and holding the current
/// removed/added runs until a context line (or the end) pairs them.
struct Building {
    header: String,
    old: u32,
    new: u32,
    removed: Vec<Cell>,
    added: Vec<Cell>,
    rows: Vec<Row>,
    lines: Vec<Line>,
}

impl Building {
    fn new(header: String, old: u32, new: u32) -> Self {
        Self {
            header,
            old,
            new,
            removed: Vec::new(),
            added: Vec::new(),
            rows: Vec::new(),
            lines: Vec::new(),
        }
    }

    fn context(&mut self, text: &str) {
        self.flush();
        self.lines.push(Line {
            old: Some(self.old),
            new: Some(self.new),
            text: text.to_string(),
            kind: CellKind::Context,
        });
        self.rows.push(Row {
            left: Some(Cell {
                number: Some(self.old),
                text: text.to_string(),
                kind: CellKind::Context,
            }),
            right: Some(Cell {
                number: Some(self.new),
                text: text.to_string(),
                kind: CellKind::Context,
            }),
        });
        self.old += 1;
        self.new += 1;
    }

    fn removed(&mut self, text: &str) {
        // A removal after additions starts a new pairing: `-a +b -c +d` is two
        // replacements, not one block of two.
        if !self.added.is_empty() {
            self.flush();
        }
        self.lines.push(Line {
            old: Some(self.old),
            new: None,
            text: text.to_string(),
            kind: CellKind::Removed,
        });
        self.removed.push(Cell {
            number: Some(self.old),
            text: text.to_string(),
            kind: CellKind::Removed,
        });
        self.old += 1;
    }

    fn added(&mut self, text: &str) {
        self.lines.push(Line {
            old: None,
            new: Some(self.new),
            text: text.to_string(),
            kind: CellKind::Added,
        });
        self.added.push(Cell {
            number: Some(self.new),
            text: text.to_string(),
            kind: CellKind::Added,
        });
        self.new += 1;
    }

    /// About the line before it: the added run if one is open, else the
    /// removed run, else the last context row — where it belongs to both
    /// sides, since the same unterminated line is in both files.
    fn note(&mut self, text: &str) {
        self.lines.push(Line {
            old: None,
            new: None,
            text: text.to_string(),
            kind: CellKind::Note,
        });
        let cell = Cell {
            number: None,
            text: text.to_string(),
            kind: CellKind::Note,
        };
        if !self.added.is_empty() {
            self.added.push(cell);
        } else if !self.removed.is_empty() {
            self.removed.push(cell);
        } else {
            self.rows.push(Row {
                left: Some(cell.clone()),
                right: Some(cell),
            });
        }
    }

    /// Lay the pending runs side by side, index for index.
    fn flush(&mut self) {
        let removed = std::mem::take(&mut self.removed);
        let added = std::mem::take(&mut self.added);
        let mut left = removed.into_iter();
        let mut right = added.into_iter();
        loop {
            match (left.next(), right.next()) {
                (None, None) => break,
                (l, r) => self.rows.push(Row { left: l, right: r }),
            }
        }
    }

    fn finish(mut self) -> Hunk {
        self.flush();
        Hunk {
            header: self.header,
            rows: self.rows,
            lines: self.lines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbers(row: &Row) -> (Option<u32>, Option<u32>) {
        (
            row.left.as_ref().and_then(|c| c.number),
            row.right.as_ref().and_then(|c| c.number),
        )
    }

    #[test]
    fn a_replacement_is_paired_index_for_index_and_the_longer_side_overhangs() {
        let patch = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n\
                     @@ -1,3 +1,4 @@\n a\n-b\n-c\n+B\n+C\n+D\n e\n";
        let hunks = hunks(patch);
        assert_eq!(hunks.len(), 1);
        let rows = &hunks[0].rows;
        assert_eq!(hunks[0].header, "@@ -1,3 +1,4 @@");
        assert_eq!(rows.len(), 5, "a, three paired rows, e");
        assert_eq!(numbers(&rows[0]), (Some(1), Some(1)));
        assert_eq!(rows[1].left.as_ref().unwrap().text, "b");
        assert_eq!(rows[1].right.as_ref().unwrap().text, "B");
        assert_eq!(rows[1].left.as_ref().unwrap().kind, CellKind::Removed);
        assert_eq!(rows[1].right.as_ref().unwrap().kind, CellKind::Added);
        assert_eq!(numbers(&rows[2]), (Some(3), Some(3)));
        assert_eq!(
            numbers(&rows[3]),
            (None, Some(4)),
            "the third addition has nothing to face"
        );
        assert!(rows[3].left.is_none());
        assert_eq!(
            numbers(&rows[4]),
            (Some(4), Some(5)),
            "context resumes on both"
        );
    }

    #[test]
    fn a_missing_newline_is_a_row_on_the_side_of_the_line_above_it() {
        // Fork's shape, from a real two-line file that grew a line and still
        // has no final newline: the left note faces the right's third line.
        let patch = "@@ -1,2 +1,3 @@\n ## fix\n-some copy\n\\ No newline at end of file\n\
                     +cert backup to 128 bits\n+badge moved\n\\ No newline at end of file\n";
        let rows = hunks(patch).remove(0).rows;
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].left.as_ref().unwrap().text, "some copy");
        assert_eq!(
            rows[1].right.as_ref().unwrap().text,
            "cert backup to 128 bits"
        );
        assert_eq!(rows[2].left.as_ref().unwrap().kind, CellKind::Note);
        assert_eq!(rows[2].left.as_ref().unwrap().number, None);
        assert_eq!(rows[2].right.as_ref().unwrap().text, "badge moved");
        assert!(rows[3].left.is_none());
        assert_eq!(rows[3].right.as_ref().unwrap().kind, CellKind::Note);
    }

    #[test]
    fn the_one_column_reading_keeps_gits_order_with_both_numbers() {
        let patch = "@@ -1,2 +1,3 @@\n ## fix\n-some copy\n\\ No newline at end of file\n\
                     +cert backup to 128 bits\n+badge moved\n\\ No newline at end of file\n";
        let lines = hunks(patch).remove(0).lines;
        let shape: Vec<(Option<u32>, Option<u32>, CellKind)> =
            lines.iter().map(|l| (l.old, l.new, l.kind)).collect();
        assert_eq!(
            shape,
            vec![
                (Some(1), Some(1), CellKind::Context),
                (Some(2), None, CellKind::Removed),
                (None, None, CellKind::Note),
                (None, Some(2), CellKind::Added),
                (None, Some(3), CellKind::Added),
                (None, None, CellKind::Note),
            ],
            "the note stays after the line it is about, not after the additions"
        );
        assert_eq!(lines[1].text, "some copy");
    }

    #[test]
    fn a_note_after_context_sits_on_both_sides() {
        let patch = "@@ -1,2 +1,2 @@\n-old\n+new\n same last\n\\ No newline at end of file\n";
        let rows = hunks(patch).remove(0).rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].left.as_ref().unwrap().kind, CellKind::Note);
        assert_eq!(rows[2].right.as_ref().unwrap().kind, CellKind::Note);
    }

    #[test]
    fn two_replacements_in_one_hunk_do_not_merge_into_one_block() {
        let patch = "@@ -1,2 +1,2 @@\n-a\n+A\n-b\n+B\n";
        let rows = hunks(patch).remove(0).rows;
        assert_eq!(rows.len(), 2, "a faces A and b faces B, no overhang");
        assert_eq!(rows[0].left.as_ref().unwrap().text, "a");
        assert_eq!(rows[0].right.as_ref().unwrap().text, "A");
        assert_eq!(rows[1].left.as_ref().unwrap().text, "b");
        assert_eq!(rows[1].right.as_ref().unwrap().text, "B");
    }

    #[test]
    fn a_new_file_numbers_only_its_right_side_from_one() {
        let patch = "diff --git a/n b/n\nnew file mode 100644\nindex 000..c0e\n--- /dev/null\n+++ b/n\n\
                     @@ -0,0 +1,2 @@\n+one\n+two\n";
        let rows = hunks(patch).remove(0).rows;
        assert_eq!(numbers(&rows[0]), (None, Some(1)));
        assert_eq!(numbers(&rows[1]), (None, Some(2)));
    }

    #[test]
    fn hunk_headers_without_counts_and_with_function_context_parse() {
        let patch = "@@ -7 +7 @@ fn main() {\n-x\n+y\n@@ -20,3 +20,3 @@\n a\n-b\n+c\n d\n";
        let hunks = hunks(patch);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].header, "@@ -7 +7 @@ fn main() {");
        assert_eq!(numbers(&hunks[0].rows[0]), (Some(7), Some(7)));
        assert_eq!(numbers(&hunks[1].rows[0]), (Some(20), Some(20)));
        assert_eq!(numbers(&hunks[1].rows[2]), (Some(22), Some(22)));
    }

    #[test]
    fn a_binary_patch_has_no_hunks() {
        assert!(hunks("diff --git a/b.png b/b.png\nBinary files differ\n").is_empty());
        assert!(hunks("").is_empty());
    }
}
