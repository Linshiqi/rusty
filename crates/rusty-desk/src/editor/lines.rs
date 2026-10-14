//! Where each line of a text starts, for turning a byte into a line and a
//! column and back. Columns are characters, as people count them; bytes
//! are what the text is indexed by.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    starts: Vec<usize>,
    len: usize,
}

impl Lines {
    pub fn of(text: &str) -> Lines {
        let mut starts = Vec::with_capacity(text.len() / 32 + 1);
        starts.push(0);
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(at, _)| at + 1),
        );
        Lines {
            starts,
            len: text.len(),
        }
    }

    pub fn count(&self) -> usize {
        self.starts.len()
    }

    /// The byte line `line` starts at; past the last line, the text's end.
    pub fn start(&self, line: usize) -> usize {
        self.starts.get(line).copied().unwrap_or(self.len)
    }

    /// The byte line `line` ends at, before its `\n`.
    pub fn end(&self, line: usize) -> usize {
        self.starts
            .get(line + 1)
            .map_or(self.len, |next| next - 1)
            .max(self.start(line))
    }

    /// The line a byte is on.
    pub fn line_of(&self, at: usize) -> usize {
        self.starts
            .partition_point(|&start| start <= at)
            .saturating_sub(1)
    }

    /// A line's text.
    pub fn text<'t>(&self, text: &'t str, line: usize) -> &'t str {
        &text[self.start(line)..self.end(line)]
    }

    /// The line and character column of a byte.
    pub fn position(&self, text: &str, at: usize) -> (usize, usize) {
        let line = self.line_of(at);
        let col = text[self.start(line)..at.min(text.len())].chars().count();
        (line, col)
    }

    /// The byte at a line and character column, clamped to the line's end.
    pub fn byte(&self, text: &str, line: usize, col: usize) -> usize {
        let line = line.min(self.count() - 1);
        let (start, end) = (self.start(line), self.end(line));
        let content = &text[start..end];
        start
            + content
                .char_indices()
                .nth(col)
                .map_or(content.len(), |(at, _)| at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_columns_go_both_ways() {
        let text = "ab\n中文x\n\nend";
        let lines = Lines::of(text);
        assert_eq!(lines.count(), 4);
        assert_eq!(lines.text(text, 1), "中文x");
        assert_eq!(lines.text(text, 2), "");
        assert_eq!(lines.position(text, lines.byte(text, 1, 2)), (1, 2));
        assert_eq!(lines.byte(text, 1, 99), lines.end(1));
        assert_eq!(lines.line_of(text.len()), 3);
    }

    /// A text ending in a newline has an empty last line, as the editor
    /// shows it.
    #[test]
    fn a_final_newline_is_an_empty_last_line() {
        let lines = Lines::of("a\n");
        assert_eq!(lines.count(), 2);
        assert_eq!((lines.start(1), lines.end(1)), (2, 2));
    }
}
