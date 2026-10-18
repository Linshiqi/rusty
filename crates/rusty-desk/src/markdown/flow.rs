//! A paragraph that holds a formula, broken into lines here rather than by
//! Slint: `StyledText` wraps text and nothing else, and a formula is a
//! picture. So a paragraph with one is cut into atoms — words, spaces, a
//! Chinese character each, formulas — measured, and set line by line: the
//! text of a line as one `StyledText` that does not wrap, every formula as a
//! picture beside it, each standing on the line's baseline.
//!
//! Pure: what a piece of text measures is asked of the caller, which asks
//! the window's own text layout, so a line broken here is a line that fits
//! there.

/// How a run of text is set: what `StyledText` reads as Markdown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
}

/// A paragraph's content, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    Text {
        text: String,
        style: Style,
    },
    /// An inline formula: its TeX.
    Math(String),
    /// A line break the text asked for.
    Break,
}

/// What a formula is, to the line it stands in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Boxed<'a> {
    /// Drawn: its box in pixels — width, and how far it reaches above the
    /// baseline and below.
    Drawn {
        width: f32,
        ascent: f32,
        descent: f32,
    },
    /// Not drawn, or not yet: its source stands in for it, as code.
    Source(&'a str),
}

/// How text is measured, in pixels, by whoever will draw it.
pub trait Measure {
    fn width(&mut self, text: &str, style: &Style) -> f32;
}

/// A line of text's height and where its baseline is, from its top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextLine {
    pub height: f32,
    pub baseline: f32,
}

/// A line, laid out: its height and what is on it, each placed from the
/// line's top left — the pieces the view draws, and every atom on the line
/// with where it stands, which is what a selection is measured against.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub height: f32,
    pub pieces: Vec<Piece>,
    pub placed: Vec<Placed>,
}

/// One atom of a paragraph on a line: which atom, where it stands across
/// the line, what it is copied as, and how its text is measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The atom's index in its paragraph (`atom_texts`), which a selection
    /// names it by: a line's worth of atoms changes with the width, an
    /// atom does not.
    pub atom: usize,
    pub x: f32,
    pub width: f32,
    /// A word, a space, or a formula's TeX.
    pub text: String,
    pub style: Style,
    /// A formula, drawn or standing as its source: selected whole or not at
    /// all, never a character of it.
    pub formula: bool,
}

/// What each atom of a paragraph is copied as, by index: a word, a space,
/// a formula as `$…$`, a hard break as a line break. The same atoms
/// `lay_out` places, so a selection in atoms reads the same text whatever
/// width the paragraph was laid out for — the spaces a line was broken at
/// included, which stand on no line.
pub fn atom_texts(runs: &[Run]) -> Vec<String> {
    atoms(runs)
        .into_iter()
        .map(|atom| match atom.kind {
            Kind::Formula(_) | Kind::Source(_) => format!("${}$", atom.text.trim()),
            Kind::Break => "\n".to_string(),
            Kind::Word | Kind::Space => atom.text,
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    /// Text, as Markdown for one `StyledText`.
    Text {
        markdown: String,
        x: f32,
        y: f32,
        width: f32,
    },
    /// The `index`th formula of the paragraph: `x` and `y` are its box's top
    /// left, and `scale` what it was shrunk by to fit the line.
    Formula {
        index: usize,
        x: f32,
        y: f32,
        scale: f32,
    },
    /// The `index`th formula's source, standing in for it.
    Source {
        index: usize,
        markdown: String,
        x: f32,
        y: f32,
        width: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Word,
    Space,
    Formula(usize),
    Source(usize),
    Break,
}

#[derive(Clone, Debug)]
struct Atom {
    kind: Kind,
    text: String,
    style: Style,
    width: f32,
    ascent: f32,
    descent: f32,
    scale: f32,
}

impl Atom {
    fn first(&self) -> Option<char> {
        match self.kind {
            Kind::Word => self.text.chars().next(),
            _ => None,
        }
    }

    fn last(&self) -> Option<char> {
        match self.kind {
            Kind::Word => self.text.chars().last(),
            _ => None,
        }
    }
}

/// Whether `c` is set the way Chinese, Japanese and Korean are: a line may
/// break before or after it without a space.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// The room kept above and below a formula taller than the text, as a
/// share of a line of text: about the leading the text has round its own
/// letters.
const LEAD: f32 = 0.1;

/// Punctuation no line may begin with.
fn closes(c: char) -> bool {
    "，。、；：？！）》」』】〕〉…‥％‰°”’,.;:?!)]}%".contains(c)
}

/// Punctuation no line may end with.
fn opens(c: char) -> bool {
    "（《「『【〔〈“‘([{".contains(c)
}

/// Whether a line may break between `before` and `after`.
fn breakable(before: &Atom, after: &Atom) -> bool {
    if matches!(after.kind, Kind::Space | Kind::Break) {
        return false;
    }
    if before.kind == Kind::Space {
        return true;
    }
    if after.first().is_some_and(closes) || before.last().is_some_and(opens) {
        return false;
    }
    before.last().is_some_and(is_cjk) || after.first().is_some_and(is_cjk)
}

/// The paragraph's atoms: words, spaces, characters that break anywhere,
/// formulas and the breaks the text asked for.
fn atoms(runs: &[Run]) -> Vec<Atom> {
    let mut out = Vec::new();
    let mut formula = 0;
    let atom = |kind, text: String, style: &Style| Atom {
        kind,
        text,
        style: style.clone(),
        width: 0.0,
        ascent: 0.0,
        descent: 0.0,
        scale: 1.0,
    };
    for run in runs {
        match run {
            Run::Text { text, style } => {
                let mut word = String::new();
                let mut space = false;
                for c in text.chars() {
                    if c.is_whitespace() {
                        if !word.is_empty() {
                            out.push(atom(Kind::Word, std::mem::take(&mut word), style));
                        }
                        space = true;
                        continue;
                    }
                    if space {
                        out.push(atom(Kind::Space, " ".into(), style));
                        space = false;
                    }
                    if is_cjk(c) {
                        if !word.is_empty() {
                            out.push(atom(Kind::Word, std::mem::take(&mut word), style));
                        }
                        out.push(atom(Kind::Word, c.to_string(), style));
                    } else {
                        word.push(c);
                    }
                }
                if !word.is_empty() {
                    out.push(atom(Kind::Word, word, style));
                }
                if space {
                    out.push(atom(Kind::Space, " ".into(), style));
                }
            }
            Run::Math(tex) => {
                out.push(atom(Kind::Formula(formula), tex.clone(), &Style::default()));
                formula += 1;
            }
            Run::Break => out.push(atom(Kind::Break, String::new(), &Style::default())),
        }
    }
    // Two spaces in a row — one run ending in one, the next starting with
    // one — are one space.
    out.dedup_by(|b, a| a.kind == Kind::Space && b.kind == Kind::Space);
    out
}

/// Lay a paragraph out in lines no wider than `width`.
///
/// `formulas` says, for each `Run::Math` in order, what it is to the line.
pub fn lay_out(
    runs: &[Run],
    formulas: &[Boxed],
    width: f32,
    text: TextLine,
    measure: &mut dyn Measure,
) -> Vec<Line> {
    let mut atoms = atoms(runs);
    let code = Style {
        code: true,
        ..Style::default()
    };
    for atom in &mut atoms {
        match atom.kind {
            Kind::Word | Kind::Space => atom.width = measure.width(&atom.text, &atom.style),
            Kind::Formula(index) => match formulas.get(index) {
                Some(Boxed::Drawn {
                    width: w,
                    ascent,
                    descent,
                }) => {
                    // Wider than the line: shrunk to fit it, never cut.
                    atom.scale = if *w > width && *w > 0.0 {
                        width / w
                    } else {
                        1.0
                    };
                    atom.width = w * atom.scale;
                    atom.ascent = ascent * atom.scale;
                    atom.descent = descent * atom.scale;
                }
                Some(Boxed::Source(_)) | None => {
                    let tex = match formulas.get(index) {
                        Some(Boxed::Source(tex)) => tex.trim().to_string(),
                        _ => String::new(),
                    };
                    atom.width = measure.width(&tex, &code);
                    atom.text = tex;
                    atom.style = code.clone();
                    atom.kind = Kind::Source(index);
                }
            },
            Kind::Source(_) | Kind::Break => {}
        }
    }
    lines(&atoms, width)
        .into_iter()
        .map(|range| place(&atoms[range.clone()], range.start, text))
        .collect()
}

/// Where each line begins and ends: greedily as many atoms as fit, broken
/// at the last place a break is allowed, with no space at either end. A
/// line with no such place runs over, to the first place there is one — as
/// a browser lets a long word run over rather than put a comma at the start
/// of a line.
fn lines(atoms: &[Atom], width: f32) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let skip = |mut at: usize| {
        while at < atoms.len() && atoms[at].kind == Kind::Space {
            at += 1;
        }
        at
    };
    let mut start = skip(0);
    let mut at = start;
    let mut used = 0.0;
    let mut last_break: Option<usize> = None;
    let mut over = false;
    while at < atoms.len() {
        let atom = &atoms[at];
        let mut end = None;
        if atom.kind == Kind::Break {
            out.push(start..at);
            start = skip(at + 1);
            at = start;
            used = 0.0;
            last_break = None;
            over = false;
            continue;
        }
        let may_break = at > start && breakable(&atoms[at - 1], atom);
        if may_break {
            last_break = Some(at);
            if over {
                end = Some(at);
            }
        }
        if end.is_none() && atom.kind != Kind::Space && at > start && used + atom.width > width {
            match last_break {
                Some(at) => end = Some(at),
                None => over = true,
            }
        }
        if let Some(end) = end {
            out.push(start..end);
            start = skip(end);
            at = start;
            used = 0.0;
            last_break = None;
            over = false;
            continue;
        }
        used += atom.width;
        at += 1;
    }
    if start < atoms.len() {
        out.push(start..atoms.len());
    }
    // No line ends in a space.
    out.into_iter()
        .map(|range| {
            let mut end = range.end;
            while end > range.start && atoms[end - 1].kind == Kind::Space {
                end -= 1;
            }
            range.start..end
        })
        .filter(|range| !range.is_empty())
        .collect()
}

/// One line's atoms, placed: text gathered into pieces, every piece on the
/// line's baseline. A formula taller than the text keeps the text's own
/// leading above and below it, so it never touches the line next to it.
fn place(atoms: &[Atom], first: usize, text: TextLine) -> Line {
    let lead = text.height * LEAD;
    let mut ascent = text.baseline;
    let mut descent = text.height - text.baseline;
    for atom in atoms {
        if let Kind::Formula(_) = atom.kind {
            ascent = ascent.max(atom.ascent + lead);
            descent = descent.max(atom.descent + lead);
        }
    }
    let text_y = ascent - text.baseline;
    let mut pieces = Vec::new();
    let mut placed = Vec::new();
    let mut x = 0.0;
    let mut gathered: Vec<&Atom> = Vec::new();
    let mut gathered_x = 0.0;
    // A piece neither begins nor ends with a space: Markdown drops a space
    // at either end of what it reads, and the text after it would then
    // stand a space early. A space between two pieces is the gap between
    // where they stand.
    let flush = |gathered: &mut Vec<&Atom>, at: f32, pieces: &mut Vec<Piece>| {
        while gathered.last().is_some_and(|a| a.kind == Kind::Space) {
            gathered.pop();
        }
        if gathered.is_empty() {
            return;
        }
        let width = gathered.iter().map(|a| a.width).sum();
        pieces.push(Piece::Text {
            markdown: markdown(gathered),
            x: at,
            y: text_y,
            width,
        });
        gathered.clear();
    };
    for (at, atom) in atoms.iter().enumerate() {
        if atom.kind != Kind::Break {
            let formula = matches!(atom.kind, Kind::Formula(_) | Kind::Source(_));
            placed.push(Placed {
                atom: first + at,
                x,
                width: atom.width,
                // As `atom_texts` copies it, so a spot's offset means one
                // thing in both.
                text: if formula {
                    format!("${}$", atom.text.trim())
                } else {
                    atom.text.clone()
                },
                style: atom.style.clone(),
                formula,
            });
        }
        match atom.kind {
            Kind::Space if gathered.is_empty() => {}
            Kind::Word | Kind::Space => {
                if gathered.is_empty() {
                    gathered_x = x;
                }
                gathered.push(atom);
            }
            Kind::Formula(index) => {
                flush(&mut gathered, gathered_x, &mut pieces);
                pieces.push(Piece::Formula {
                    index,
                    x,
                    y: ascent - atom.ascent,
                    scale: atom.scale,
                });
            }
            Kind::Source(index) => {
                flush(&mut gathered, gathered_x, &mut pieces);
                pieces.push(Piece::Source {
                    index,
                    markdown: markdown(&[atom]),
                    x,
                    y: text_y,
                    width: atom.width,
                });
            }
            Kind::Break => {}
        }
        x += atom.width;
    }
    flush(&mut gathered, gathered_x, &mut pieces);
    Line {
        height: ascent + descent,
        pieces,
        placed,
    }
}

/// Atoms as the Markdown `StyledText` reads: each stretch of one style
/// marked up once, its spaces outside the marks, which CommonMark requires.
fn markdown(atoms: &[&Atom]) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < atoms.len() {
        let style = &atoms[at].style;
        let mut text = String::new();
        while at < atoms.len() && atoms[at].style == *style {
            text.push_str(&atoms[at].text);
            at += 1;
        }
        out.push_str(&marked(&text, style));
    }
    out
}

/// A stretch of text in one style, as Markdown.
pub fn marked(text: &str, style: &Style) -> String {
    let core = text.trim();
    if core.is_empty() {
        return text.to_string();
    }
    let lead = &text[..text.len() - text.trim_start().len()];
    let trail = &text[text.trim_end().len()..];
    let mut inner = if style.code {
        super::code_span(core)
    } else {
        super::escape(core)
    };
    match (style.strong, style.emphasis) {
        (true, true) => inner = format!("***{inner}***"),
        (true, false) => inner = format!("**{inner}**"),
        (false, true) => inner = format!("*{inner}*"),
        (false, false) => {}
    }
    if style.strike {
        inner = format!("~~{inner}~~");
    }
    if let Some(url) = &style.link {
        inner = format!("[{inner}]({})", super::link_target(url));
    }
    format!("{lead}{inner}{trail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character one unit wide, a Chinese one two — the shape of a
    /// real font's widths, without a font.
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

    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.pieces
                    .iter()
                    .map(|p| match p {
                        Piece::Text { markdown, .. } => markdown.clone(),
                        Piece::Formula { index, .. } => format!("<{index}>"),
                        Piece::Source { markdown, .. } => markdown.clone(),
                    })
                    .collect()
            })
            .collect()
    }

    fn boxed(width: f32) -> Boxed<'static> {
        Boxed::Drawn {
            width,
            ascent: 6.0,
            descent: 1.0,
        }
    }

    /// Every atom on a line stands where the text before it ends, under
    /// its index in the paragraph; the space a line was broken at stands
    /// on no line and is still in the paragraph's texts, so a selection
    /// across the break copies it.
    #[test]
    fn placed_atoms_keep_their_index_and_the_break_keeps_its_space() {
        let runs = [plain("one two three")];
        let lines = lay_out(&runs, &[], 9.0, TEXT, &mut Units);
        let placed: Vec<(usize, f32, &str)> = lines
            .iter()
            .flat_map(|l| l.placed.iter().map(|p| (p.atom, p.x, p.text.as_str())))
            .collect();
        assert_eq!(
            placed,
            [
                (0, 0.0, "one"),
                (1, 3.0, " "),
                (2, 4.0, "two"),
                (4, 0.0, "three")
            ]
        );
        assert_eq!(atom_texts(&runs), ["one", " ", "two", " ", "three"]);
        let with_formula = [
            plain("a "),
            Run::Math(" x^2 ".into()),
            Run::Break,
            plain("b"),
        ];
        assert_eq!(atom_texts(&with_formula), ["a", " ", "$x^2$", "\n", "b"]);
    }

    /// Latin text breaks at spaces and never inside a word; no line begins
    /// or ends with the space it broke at.
    #[test]
    fn words_break_at_spaces() {
        let lines = lay_out(&[plain("one two three four")], &[], 9.0, TEXT, &mut Units);
        assert_eq!(texts(&lines), ["one two", "three", "four"]);
    }

    /// Chinese breaks between any two characters — except before a closing
    /// mark, which stays with the character before it.
    #[test]
    fn chinese_breaks_anywhere_but_before_its_punctuation() {
        let lines = lay_out(&[plain("偏航角俯仰角。滚转")], &[], 12.0, TEXT, &mut Units);
        // Six units of three characters fit twice over; the 。 may not
        // begin the second line, so 角 goes down with it.
        assert_eq!(texts(&lines), ["偏航角俯仰", "角。滚转"]);
    }

    /// A formula is one unbreakable thing that stands on the baseline: its
    /// box reaches above and below, and the line grows to hold it.
    #[test]
    fn a_formula_stands_on_the_baseline_of_its_line() {
        let runs = [plain("角 "), Run::Math(r"\psi".into()), plain(" 是")];
        let tall = Boxed::Drawn {
            width: 3.0,
            ascent: 12.0,
            descent: 4.0,
        };
        let lines = lay_out(&runs, &[tall], 100.0, TEXT, &mut Units);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        // Text's baseline at 8 from its top; the formula reaches 12 above it
        // and 4 below, with the text's leading kept round it.
        let lead = TEXT.height * LEAD;
        assert!((line.height - (12.0 + 4.0 + 2.0 * lead)).abs() < 1e-4);
        let Piece::Text { y: text_y, .. } = &line.pieces[0] else {
            panic!("text first")
        };
        let Piece::Formula { y, x, scale, .. } = &line.pieces[1] else {
            panic!("then the formula")
        };
        assert_eq!(*text_y + TEXT.baseline, *y + 12.0, "one baseline");
        assert_eq!((*x, *scale), (3.0, 1.0), "after 角 and its space");
    }

    /// A line may break beside a formula among Chinese text, but never
    /// between it and the comma after it, nor the bracket before it — a
    /// line with nowhere to break runs over rather than begin with a comma.
    #[test]
    fn a_formula_keeps_its_punctuation() {
        let runs = [plain("角（"), Run::Math("x".into()), plain("），")];
        let lines = lay_out(&runs, &[boxed(4.0)], 7.0, TEXT, &mut Units);
        assert_eq!(texts(&lines), ["角", "（<0>），"]);
        let runs = [plain("一二"), Run::Math("x".into()), plain("，三")];
        let lines = lay_out(&runs, &[boxed(4.0)], 6.0, TEXT, &mut Units);
        assert_eq!(texts(&lines), ["一二", "<0>，", "三"]);
    }

    /// A formula wider than the line is shrunk to fit it, never cut, and it
    /// takes a line of its own.
    #[test]
    fn a_formula_wider_than_the_line_is_shrunk_to_fit() {
        let runs = [plain("a "), Run::Math("x".into()), plain(" b")];
        let lines = lay_out(&runs, &[boxed(40.0)], 10.0, TEXT, &mut Units);
        assert_eq!(texts(&lines), ["a", "<0>", "b"]);
        let Piece::Formula { scale, .. } = lines[1].pieces[0] else {
            panic!("the formula")
        };
        assert_eq!(scale, 0.25);
    }

    /// A formula that could not be drawn stands as its source, as code, and
    /// is never dropped.
    #[test]
    fn a_formula_not_drawn_stands_as_its_source() {
        let runs = [plain("see "), Run::Math(r"\frac{1}".into())];
        let lines = lay_out(
            &runs,
            &[Boxed::Source(r"\frac{1}")],
            100.0,
            TEXT,
            &mut Units,
        );
        let line = &lines[0];
        assert!(matches!(
            &line.pieces[1],
            Piece::Source { index: 0, markdown, .. } if markdown == r"`\frac{1}`"
        ));
    }

    /// Styles carry over the line they are broken across: each line marks
    /// its own part up, spaces outside the marks.
    #[test]
    fn a_style_is_marked_up_again_on_every_line() {
        let bold = Style {
            strong: true,
            ..Style::default()
        };
        let link = Style {
            link: Some("https://example.com/a b".into()),
            ..Style::default()
        };
        let runs = [
            plain("x "),
            Run::Text {
                text: "bold words here ".into(),
                style: bold,
            },
            Run::Text {
                text: "link".into(),
                style: link,
            },
        ];
        let lines = lay_out(&runs, &[], 11.0, TEXT, &mut Units);
        assert_eq!(
            texts(&lines),
            [
                "x **bold**",
                "**words here**",
                "[link](https://example.com/a%20b)"
            ]
        );
    }

    /// A break the text asked for is a line end, whatever room is left.
    #[test]
    fn a_hard_break_ends_the_line() {
        let lines = lay_out(
            &[plain("a"), Run::Break, plain("b")],
            &[],
            100.0,
            TEXT,
            &mut Units,
        );
        assert_eq!(texts(&lines), ["a", "b"]);
    }

    /// Pieces stand where the measured widths before them end, and a space
    /// beside a formula is the gap before the next piece, never a space at
    /// the edge of a piece's text, which Markdown would drop.
    #[test]
    fn pieces_stand_where_the_text_before_them_ends() {
        let runs = [plain("ab "), Run::Math("x".into()), plain(" 中 d")];
        let lines = lay_out(&runs, &[boxed(5.0)], 100.0, TEXT, &mut Units);
        let xs: Vec<f32> = lines[0]
            .pieces
            .iter()
            .map(|p| match p {
                Piece::Text { x, .. } | Piece::Formula { x, .. } | Piece::Source { x, .. } => *x,
            })
            .collect();
        assert_eq!(xs, [0.0, 3.0, 9.0]);
        assert_eq!(texts(&lines), ["ab<0>中 d"]);
    }
}
