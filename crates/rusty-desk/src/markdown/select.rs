//! Selecting on a Markdown page: a selection as two places in the page's
//! blocks, and what a point, a drag and a copy mean against the lines the
//! page laid out.
//!
//! The page's text is drawn by Slint and cannot be selected there, so the
//! selection is the window's own, as the diff's is (`crate::diff_select`):
//! every paragraph, heading and cell is laid out into lines by rusty
//! (`super::flow`), so where each atom — a word, a space, a Chinese
//! character, a formula — stands is known, and a character inside a word is
//! found by measuring the word's prefixes with the same measure that placed
//! it. A code block is lines of one atom each. A place is the atom and a
//! byte offset into its text rather than a line and an x: the lines change
//! with the width and the atoms do not, so a selection outlives a reflow.
//! Pure, so what a drag across a wrap or a formula selects is a test.

use super::flow::{Line, Measure, Placed};

/// A place in one block's text: an atom, and a byte offset into its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub atom: usize,
    pub offset: usize,
}

/// A place on the page: the block or cell, by the key its lines are laid
/// out under (reading order), then the spot in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    pub key: i32,
    pub spot: Spot,
}

/// Where a selection began and where its moving end is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Place,
    pub head: Place,
}

impl Selection {
    /// The two ends in reading order.
    pub fn ordered(&self) -> (Place, Place) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// What the selection covers of the block under `key`, as spots in it:
    /// `None` at an end that lies in another block — the whole of this one
    /// that way — and nothing at all when the block is outside it.
    pub fn in_block(&self, key: i32) -> Option<(Option<Spot>, Option<Spot>)> {
        let (from, to) = self.ordered();
        if key < from.key || key > to.key {
            return None;
        }
        Some((
            (from.key == key).then_some(from.spot),
            (to.key == key).then_some(to.spot),
        ))
    }
}

/// A text as it is measured: a tab is the four spaces a code block draws
/// it as.
fn shown(text: &str) -> String {
    text.replace('\t', "    ")
}

/// How far into `placed` its first `offset` bytes reach.
fn advance(placed: &Placed, offset: usize, measure: &mut dyn Measure) -> f32 {
    if offset == 0 {
        return 0.0;
    }
    if offset >= placed.text.len() {
        return placed.width;
    }
    measure.width(&shown(&placed.text[..offset]), &placed.style)
}

/// The line of `lines` a point `y` down the block falls on: above the
/// first is the first, below the last the last.
fn line_at(lines: &[Line], y: f32) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let mut top = 0.0;
    for (at, line) in lines.iter().enumerate() {
        if y < top + line.height {
            return Some(at);
        }
        top += line.height;
    }
    Some(lines.len() - 1)
}

/// The byte offset of `placed`'s text nearest to `dx` into it: a character
/// boundary, the one whose prefix is nearest. A formula is one thing, so
/// its nearest end.
fn offset_at(placed: &Placed, dx: f32, measure: &mut dyn Measure) -> usize {
    let len = placed.text.len();
    if placed.formula || len == 0 {
        return if dx * 2.0 < placed.width { 0 } else { len };
    }
    let boundaries: Vec<usize> = placed
        .text
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(len))
        .collect();
    // The last boundary whose prefix fits in `dx`; the widths grow with the
    // prefix, so it is found by halving.
    let (mut low, mut high) = (0, boundaries.len() - 1);
    while low < high {
        let mid = (low + high).div_ceil(2);
        if advance(placed, boundaries[mid], measure) <= dx {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let before = boundaries[low];
    let Some(&after) = boundaries.get(low + 1) else {
        return before;
    };
    let (x0, x1) = (
        advance(placed, before, measure),
        advance(placed, after, measure),
    );
    if dx - x0 <= x1 - dx { before } else { after }
}

/// The spot a point of the block falls on: left of a line's text is its
/// start, right of it its end, and inside a word the character boundary
/// nearest the point.
pub fn hit(lines: &[Line], x: f32, y: f32, measure: &mut dyn Measure) -> Option<Spot> {
    let at = line_at(lines, y)?;
    let line = &lines[at];
    let (Some(first), Some(last)) = (line.placed.first(), line.placed.last()) else {
        // A line with nothing on it: the end of the line before, or the
        // block's start.
        return Some(
            lines[..at]
                .iter()
                .rev()
                .find_map(|l| l.placed.last())
                .map_or(Spot::default(), |p| Spot {
                    atom: p.atom,
                    offset: p.text.len(),
                }),
        );
    };
    if x <= first.x {
        return Some(Spot {
            atom: first.atom,
            offset: 0,
        });
    }
    if x >= last.x + last.width {
        return Some(Spot {
            atom: last.atom,
            offset: last.text.len(),
        });
    }
    for placed in &line.placed {
        if x < placed.x {
            // In a gap before this atom: the atom's start.
            return Some(Spot {
                atom: placed.atom,
                offset: 0,
            });
        }
        if x < placed.x + placed.width {
            return Some(Spot {
                atom: placed.atom,
                offset: offset_at(placed, x - placed.x, measure),
            });
        }
    }
    Some(Spot {
        atom: last.atom,
        offset: last.text.len(),
    })
}

/// The atom a point of the block is over, when it is over one — for the
/// pointer's shape and the link under it, with nothing measured.
pub fn under(lines: &[Line], x: f32, y: f32) -> Option<&Placed> {
    let at = line_at(lines, y)?;
    let mut top = 0.0;
    for line in &lines[..at] {
        top += line.height;
    }
    let line = &lines[at];
    if y < top || y >= top + line.height {
        return None;
    }
    line.placed.iter().find(|p| x >= p.x && x < p.x + p.width)
}

/// The whole of the atom `spot` is in, for a double-click.
pub fn word_at(lines: &[Line], spot: Spot) -> (Spot, Spot) {
    let len = lines
        .iter()
        .flat_map(|l| l.placed.iter())
        .find(|p| p.atom == spot.atom)
        .map_or(spot.offset, |p| p.text.len());
    (
        Spot {
            atom: spot.atom,
            offset: 0,
        },
        Spot {
            atom: spot.atom,
            offset: len,
        },
    )
}

/// The spot after the last atom of a block whose atoms are `texts`.
pub fn end_of(texts: &[String]) -> Spot {
    match texts.last() {
        Some(last) => Spot {
            atom: texts.len() - 1,
            offset: last.len(),
        },
        None => Spot::default(),
    }
}

/// Where the selection from `from` to `to` is drawn on each line: an x and
/// a width, or nothing on a line it does not cross. An end of `None` is
/// the block's own end that way. A line's share runs from its first
/// selected atom to its last — to the end of its text, not of the column,
/// as an editor draws it.
pub fn drawn(
    lines: &[Line],
    from: Option<Spot>,
    to: Option<Spot>,
    measure: &mut dyn Measure,
) -> Vec<Option<(f32, f32)>> {
    let from = from.unwrap_or_default();
    lines
        .iter()
        .map(|line| {
            let (first, last) = (line.placed.first()?, line.placed.last()?);
            let line_end = Spot {
                atom: last.atom,
                offset: last.text.len(),
            };
            let line_start = Spot {
                atom: first.atom,
                offset: 0,
            };
            if from >= line_end || to.is_some_and(|to| to <= line_start) {
                return None;
            }
            let x1 = if from.atom < first.atom {
                0.0
            } else {
                match line.placed.iter().find(|p| p.atom == from.atom) {
                    Some(p) => p.x + advance(p, from.offset, measure),
                    // An atom between two of the line's that stands on it
                    // nowhere — the space the line before was broken at:
                    // this line's start.
                    None => line
                        .placed
                        .iter()
                        .find(|p| p.atom > from.atom)
                        .map_or(0.0, |p| p.x),
                }
            };
            let x2 = match to {
                Some(to) if to.atom <= last.atom => {
                    match line.placed.iter().find(|p| p.atom == to.atom) {
                        Some(p) => p.x + advance(p, to.offset, measure),
                        None => line
                            .placed
                            .iter()
                            .rev()
                            .find(|p| p.atom < to.atom)
                            .map_or(0.0, |p| p.x + p.width),
                    }
                }
                _ => last.x + last.width,
            };
            (x2 > x1).then_some((x1, x2 - x1))
        })
        .collect()
}

/// The text between `from` and `to` of a block whose atoms are `texts`,
/// an end of `None` being the block's own.
pub fn text(texts: &[String], from: Option<Spot>, to: Option<Spot>) -> String {
    let from = from.unwrap_or_default();
    let to = to.unwrap_or_else(|| end_of(texts));
    if from >= to {
        return String::new();
    }
    let mut out = String::new();
    for (at, atom) in texts.iter().enumerate() {
        if at < from.atom || at > to.atom {
            continue;
        }
        let start = if at == from.atom {
            boundary(atom, from.offset)
        } else {
            0
        };
        let end = if at == to.atom {
            boundary(atom, to.offset)
        } else {
            atom.len()
        };
        if start < end {
            out.push_str(&atom[start..end]);
        }
    }
    out
}

/// `at`, moved back to a character boundary of `text` and within it.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::flow::{Boxed, Run, Style, TextLine, atom_texts, is_cjk, lay_out};

    /// Every character one unit wide, a Chinese one two.
    struct Units;

    impl Measure for Units {
        fn width(&mut self, text: &str, _: &Style) -> f32 {
            text.chars()
                .map(|c| if is_cjk(c) { 2.0 } else { 1.0 })
                .sum()
        }
    }

    const TEXT: TextLine = TextLine {
        height: 10.0,
        baseline: 8.0,
    };

    fn plain(text: &str) -> Run {
        Run::Text {
            text: text.into(),
            style: Style::default(),
        }
    }

    fn laid(runs: &[Run], width: f32) -> Vec<Line> {
        lay_out(runs, &[], width, TEXT, &mut Units)
    }

    fn spot(atom: usize, offset: usize) -> Spot {
        Spot { atom, offset }
    }

    /// "one two" / "three" / "four": a point lands on the character
    /// boundary nearest it, the start left of the text and the end right
    /// of it, and a point under the last line is on the last line.
    #[test]
    fn a_point_lands_on_the_nearest_character_boundary() {
        let runs = [plain("one two three four")];
        let lines = laid(&runs, 9.0);
        let hit = |x, y| hit(&lines, x, y, &mut Units).unwrap();
        assert_eq!(hit(0.4, 5.0), spot(0, 0));
        assert_eq!(hit(1.6, 5.0), spot(0, 2), "nearer the second boundary");
        assert_eq!(hit(4.0, 5.0), spot(2, 0), "the start of `two`");
        assert_eq!(hit(50.0, 5.0), spot(2, 3), "right of the text: its end");
        assert_eq!(hit(-5.0, 15.0), spot(4, 0), "the second line's start");
        assert_eq!(
            hit(2.0, 99.0),
            spot(6, 2),
            "below everything: the last line"
        );
    }

    /// A drag from the middle of `one` to the middle of `three` copies the
    /// space the line was broken at, which stands on no line.
    #[test]
    fn a_selection_across_a_wrap_copies_the_space_the_line_broke_at() {
        let runs = [plain("one two three four")];
        let lines = laid(&runs, 9.0);
        let texts = atom_texts(&runs);
        let from = hit(&lines, 1.0, 5.0, &mut Units).unwrap();
        let to = hit(&lines, 2.0, 15.0, &mut Units).unwrap();
        assert_eq!(text(&texts, Some(from), Some(to)), "ne two th");
        let marks = drawn(&lines, Some(from), Some(to), &mut Units);
        assert_eq!(marks, [Some((1.0, 6.0)), Some((0.0, 2.0)), None]);
        assert_eq!(text(&texts, None, None), "one two three four");
    }

    /// Chinese: each character is an atom, so a point is never inside one,
    /// and a selection across the wrap copies nothing between the lines.
    #[test]
    fn chinese_selects_by_the_character() {
        let runs = [plain("偏航角俯仰角")];
        let lines = laid(&runs, 6.0);
        let texts = atom_texts(&runs);
        let from = hit(&lines, 2.9, 5.0, &mut Units).unwrap();
        let to = hit(&lines, 2.0, 15.0, &mut Units).unwrap();
        assert_eq!((from, to), (spot(1, 0), spot(4, 0)));
        assert_eq!(text(&texts, Some(from), Some(to)), "航角俯");
        assert_eq!(
            drawn(&lines, Some(from), Some(to), &mut Units),
            [Some((2.0, 4.0)), Some((0.0, 2.0))],
            "the second line's first character is in it"
        );
    }

    /// A formula is selected whole and copied as its TeX between dollars.
    #[test]
    fn a_formula_is_one_thing_copied_as_its_source() {
        let runs = [plain("a "), Run::Math("x^2".into()), plain(" b")];
        let lines = lay_out(&runs, &[Boxed::Source("x^2")], 100.0, TEXT, &mut Units);
        let texts = atom_texts(&runs);
        // `a`, space, formula (3 wide, as its source), space, `b`.
        assert_eq!(hit(&lines, 2.5, 5.0, &mut Units).unwrap(), spot(2, 0));
        assert_eq!(hit(&lines, 4.0, 5.0, &mut Units).unwrap(), spot(2, 5));
        assert_eq!(text(&texts, Some(spot(0, 0)), Some(spot(2, 5))), "a $x^2$");
        assert_eq!(word_at(&lines, spot(2, 3)), (spot(2, 0), spot(2, 5)));
    }

    /// A selection that reaches past a block's ends covers it to those ends.
    #[test]
    fn an_end_in_another_block_is_this_blocks_own_end() {
        let runs = [plain("one two three four")];
        let lines = laid(&runs, 9.0);
        assert_eq!(
            drawn(&lines, None, None, &mut Units),
            [Some((0.0, 7.0)), Some((0.0, 5.0)), Some((0.0, 4.0))]
        );
        assert_eq!(
            drawn(&lines, Some(spot(4, 2)), None, &mut Units),
            [None, Some((2.0, 3.0)), Some((0.0, 4.0))]
        );
    }

    /// A selection is read in order whichever way it was dragged, and says
    /// which blocks it reaches into.
    #[test]
    fn a_selection_knows_its_order_and_its_blocks() {
        let s = Selection {
            anchor: Place {
                key: 3,
                spot: spot(1, 0),
            },
            head: Place {
                key: 1,
                spot: spot(4, 2),
            },
        };
        assert_eq!(s.ordered().0.key, 1);
        assert_eq!(s.in_block(0), None);
        assert_eq!(s.in_block(1), Some((Some(spot(4, 2)), None)));
        assert_eq!(s.in_block(2), Some((None, None)));
        assert_eq!(s.in_block(3), Some((None, Some(spot(1, 0)))));
        assert_eq!(s.in_block(4), None);
        assert!(!s.is_empty());
    }

    /// The pointer's shape asks what is under it, which is nothing between
    /// the lines' text and the margin.
    #[test]
    fn what_is_under_a_point_is_an_atom_or_nothing() {
        let runs = [plain("one two three four")];
        let lines = laid(&runs, 9.0);
        assert_eq!(
            under(&lines, 5.0, 5.0).map(|p| p.text.as_str()),
            Some("two")
        );
        assert!(under(&lines, 8.0, 5.0).is_none(), "past the line's text");
        assert!(under(&lines, 1.0, 40.0).is_none(), "below the block");
    }
}
