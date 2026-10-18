//! Markdown's blocks (`crate::markdown`) as the rows `ui/markdown.slint`
//! draws: inline text as styled text, a code block's lines painted by the
//! editor's own highlighter, so a fence and the file it was copied from are
//! the same colours.
//!
//! A `.md` file's page also draws what the assistant's answers do not:
//! formulas, by RaTeX, rendered off the UI thread and kept by their source;
//! the pictures the page names, read the way a picture file opened on its
//! own is (`pictures.rs`); and every paragraph holding a formula broken into
//! lines here (`markdown::flow`), for the width its view reports back — as
//! it appears, and whenever the width or the text size changes.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use rusty_edit::Files;
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::pictures::Pic;
use crate::{
    AppWindow, CodeLine, CodeSpan, MdBlock, MdCell, MdLine, MdPiece, MdRow,
    markdown::{
        self, Block, Cell,
        flow::{self, Boxed, Measure, Piece, Run, Style, TextLine},
        math,
    },
};

/// A formula's em to the text's: KaTeX sets math a little larger than the
/// text round it, so their x-heights meet.
const MATH_SCALE: f32 = 1.2;

/// Room given to a line's text beyond what it measured, so a rounding in
/// the measure can never wrap it.
const SLACK: f32 = 2.0;

/// Formulas kept beyond the ones on the page in front.
const KEEP_FORMULAS: usize = 4000;

/// A formula as the page has it.
pub(super) enum Drawn {
    Ready {
        image: slint::Image,
        width: f32,
        height: f32,
        depth: f32,
    },
    Refused(String),
    Pending,
}

/// The page in front, and what it was made of: what a width reported back
/// lays out again without reading the file again.
#[derive(Default)]
pub(super) struct Page {
    read: Vec<Block>,
    /// The project's root and the page's folder in it.
    place: Option<(PathBuf, String)>,
    rows: Option<Rc<VecModel<MdBlock>>>,
    /// Each flowing paragraph's width and text size as its view last
    /// reported them, by key; and what it was last laid out for.
    widths: HashMap<i32, (f32, f32)>,
    laid: HashMap<i32, (f32, f32)>,
    reflow_due: bool,
    /// Text widths, by size, style and text; a line of text's height and
    /// baseline, by size.
    measures: HashMap<(u32, Style, String), f32>,
    text_lines: HashMap<u32, TextLine>,
    /// Formulas by their source and whether they are set apart.
    formulas: HashMap<(String, bool), Drawn>,
}

/// The key a flowing paragraph reports its width under: its block, and its
/// cell when it is a table's (counted from one).
fn key(block: usize, cell: usize) -> i32 {
    (block as i32) * 65536 + cell as i32
}

/// Every formula of `read`, by its source and whether it is set apart.
fn formulas_of(read: &[Block]) -> HashSet<(String, bool)> {
    let mut found = HashSet::new();
    for block in read {
        let runs: Vec<&Run> = match block {
            Block::Text { flow, .. } | Block::Heading { flow, .. } => flow.iter().collect(),
            Block::Table { rows } => rows
                .iter()
                .flatten()
                .flat_map(|cell| cell.flow.iter())
                .collect(),
            Block::Math { tex, .. } => {
                found.insert((tex.clone(), true));
                Vec::new()
            }
            _ => Vec::new(),
        };
        for run in runs {
            if let Run::Math(tex) = run {
                found.insert((tex.clone(), false));
            }
        }
    }
    found
}

/// The keys of every paragraph and cell of `read` that flows.
fn flow_keys(read: &[Block]) -> Vec<i32> {
    let mut keys = Vec::new();
    for (index, block) in read.iter().enumerate() {
        match block {
            Block::Text { flow, .. } | Block::Heading { flow, .. } if !flow.is_empty() => {
                keys.push(key(index, 0))
            }
            Block::Table { rows } => {
                for (at, cell) in rows.iter().flatten().enumerate() {
                    if !cell.flow.is_empty() {
                        keys.push(key(index, at + 1));
                    }
                }
            }
            _ => {}
        }
    }
    keys
}

/// A document's blocks, ready to draw, as the assistant and the release
/// notes show them: no formula drawn — its source stands for it — and no
/// picture read.
pub(super) fn blocks(files: &Files, text: &str, _place: Option<(&Path, &str)>) -> ModelRc<MdBlock> {
    let rows: Vec<MdBlock> = markdown::blocks(text)
        .iter()
        .map(|block| plain(files, block))
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// A doc comment's blocks as the hover card draws them: read as rustdoc
/// reads one (`markdown::doc_blocks`, a link only to the web) and drawn as
/// the assistant's answers are, a block fenced as nothing being in `lang`,
/// the language of the file the name is in — rustdoc's own rule for Rust.
pub(crate) fn doc_blocks(files: &Files, text: &str, lang: &str) -> ModelRc<MdBlock> {
    let rows: Vec<MdBlock> = markdown::doc_blocks(text)
        .iter()
        .map(|block| match block {
            Block::Code { lang: none, lines } if none.is_empty() => code(files, lang, lines),
            block => plain(files, block),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// A block drawn without the page's formulas, pictures or flowing lines.
fn plain(files: &Files, block: &Block) -> MdBlock {
    match block {
        Block::Text {
            inline,
            quote,
            item,
            caption,
            ..
        } => MdBlock {
            kind: 0,
            text: markdown::styled(inline),
            quote: *quote as i32,
            depth: item.as_ref().map_or(0, |i| i.depth as i32 + 1),
            marker: item
                .as_ref()
                .map(|i| i.marker.clone())
                .unwrap_or_default()
                .into(),
            caption: *caption,
            ..MdBlock::default()
        },
        Block::Heading { level, inline, .. } => MdBlock {
            kind: 1,
            level: *level as i32,
            text: markdown::styled(inline),
            ..MdBlock::default()
        },
        Block::Code { lang, lines } => code(files, lang, lines),
        Block::Rule => MdBlock {
            kind: 3,
            ..MdBlock::default()
        },
        Block::Table { rows } => table(rows, |_, _| None),
        Block::Image { alt, src, figure } => MdBlock {
            kind: 5,
            marker: if alt.is_empty() {
                src.clone()
            } else {
                alt.clone()
            }
            .into(),
            figure: *figure,
            ..MdBlock::default()
        },
        Block::Math { tex, quote } => MdBlock {
            kind: 6,
            quote: *quote as i32,
            text: markdown::styled(&code_of(tex)),
            ..MdBlock::default()
        },
        Block::Withheld { tag } => MdBlock {
            kind: 7,
            marker: t!("markdown.withheld", tag = tag.clone()).into(),
            ..MdBlock::default()
        },
    }
}

/// A formula's source as a code span.
fn code_of(tex: &str) -> String {
    flow::marked(
        tex.trim(),
        &Style {
            code: true,
            ..Style::default()
        },
    )
}

fn code(files: &Files, lang: &str, lines: &[String]) -> MdBlock {
    let painted = files.highlight_snippet(lang, &lines.join("\n"));
    let lines: Vec<CodeLine> = painted
        .into_iter()
        .map(|line| CodeLine {
            spans: ModelRc::new(VecModel::from(
                line.spans
                    .into_iter()
                    .map(|span| CodeSpan {
                        text: span.text.replace('\t', "    ").into(),
                        kind: crate::editor::token_kind(span.token),
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    MdBlock {
        kind: 2,
        marker: lang.into(),
        lines: ModelRc::new(VecModel::from(lines)),
        ..MdBlock::default()
    }
}

/// A table, each cell's lines asked of `flowed` by its row and column —
/// `None` for a cell drawn as plain styled text.
fn table(
    rows: &[Vec<Cell>],
    mut flowed: impl FnMut(usize, &Cell) -> Option<(i32, Vec<MdLine>)>,
) -> MdBlock {
    let mut at = 0;
    let rows: Vec<MdRow> = rows
        .iter()
        .enumerate()
        .map(|(row, cells)| MdRow {
            cells: ModelRc::new(VecModel::from(
                cells
                    .iter()
                    .map(|cell| {
                        at += 1;
                        let lines = flowed(at, cell);
                        MdCell {
                            text: markdown::styled(&cell.inline),
                            header: row == 0,
                            flowing: lines.is_some(),
                            key: lines.as_ref().map_or(-1, |(key, _)| *key),
                            flow: ModelRc::new(VecModel::from(
                                lines.map(|(_, lines)| lines).unwrap_or_default(),
                            )),
                        }
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    MdBlock {
        kind: 4,
        rows: ModelRc::new(VecModel::from(rows)),
        ..MdBlock::default()
    }
}

/// Text measured by the window's own text layout, at one size.
struct Probe<'a> {
    window: &'a AppWindow,
    size: f32,
    measures: &'a mut HashMap<(u32, Style, String), f32>,
}

impl Probe<'_> {
    fn raw(&self, markdown: &str) -> f32 {
        self.window.set_md_probe_size(self.size);
        self.window.set_md_probe_text(markdown::styled(markdown));
        self.window.get_md_probe_width()
    }
}

impl Measure for Probe<'_> {
    fn width(&mut self, text: &str, style: &Style) -> f32 {
        let key = (self.size.to_bits(), style.clone(), text.to_string());
        if let Some(width) = self.measures.get(&key) {
            return *width;
        }
        // A lone space lays out as nothing; the space is the difference it
        // makes between two letters.
        let width = if text.trim().is_empty() {
            let spaced = flow::marked("a a", style);
            let tight = flow::marked("aa", style);
            (self.raw(&spaced) - self.raw(&tight)).max(0.0)
        } else {
            self.raw(&flow::marked(text, style))
        };
        self.measures.insert(key, width);
        width
    }
}

/// A line of text's height and baseline at `size`, as the window lays one
/// out.
fn text_line(window: &AppWindow, size: f32, known: &mut HashMap<u32, TextLine>) -> TextLine {
    *known.entry(size.to_bits()).or_insert_with(|| {
        window.set_md_probe_size(size);
        window.set_md_probe_text(markdown::styled("Hg中"));
        let height = window.get_md_probe_height();
        let ascent = window.get_md_probe_ascent();
        let descent = -window.get_md_probe_descent().abs();
        if height <= 0.0 || ascent <= 0.0 {
            return TextLine {
                height: size * 1.4,
                baseline: size * 1.1,
            };
        }
        // What the line has beyond the font's own extent is shared above
        // and below it.
        TextLine {
            height,
            baseline: (height - (ascent - descent)) / 2.0 + ascent,
        }
    })
}

impl Page {
    /// A paragraph's lines for the width and size its view reported, or
    /// none until it has.
    fn lines(&mut self, window: &AppWindow, key: i32, runs: &[Run]) -> Vec<MdLine> {
        let Some(&(width, size)) = self.widths.get(&key) else {
            return Vec::new();
        };
        if width < 1.0 || size <= 0.0 {
            return Vec::new();
        }
        self.laid.insert(key, (width, size));
        let em = size * MATH_SCALE;
        let pad = math::PAD;
        let sources: Vec<&str> = runs
            .iter()
            .filter_map(|run| match run {
                Run::Math(tex) => Some(tex.as_str()),
                _ => None,
            })
            .collect();
        let boxes: Vec<Boxed> = sources
            .iter()
            .map(|tex| match self.formulas.get(&(tex.to_string(), false)) {
                Some(Drawn::Ready {
                    width,
                    height,
                    depth,
                    ..
                }) => Boxed::Drawn {
                    width: width * em,
                    ascent: height * em,
                    descent: depth * em,
                },
                _ => Boxed::Source(tex),
            })
            .collect();
        let text = text_line(window, size, &mut self.text_lines);
        let scale = window.window().scale_factor();
        let laid = flow::lay_out(
            runs,
            &boxes,
            width,
            text,
            &mut Probe {
                window,
                size,
                measures: &mut self.measures,
            },
        );
        laid.into_iter()
            .map(|line| MdLine {
                height: line.height,
                pieces: ModelRc::new(VecModel::from(
                    line.pieces
                        .into_iter()
                        .map(|piece| self.piece(piece, &sources, em, pad, scale))
                        .collect::<Vec<_>>(),
                )),
            })
            .collect()
    }

    fn piece(&self, piece: Piece, sources: &[&str], em: f32, pad: f32, scale: f32) -> MdPiece {
        // A picture drawn across a pixel boundary is resampled, and a hairline
        // glyph comes out grey and soft: formulas stand on whole device
        // pixels.
        let snap = |v: f32| (v * scale).round() / scale.max(0.01);
        match piece {
            Piece::Text {
                markdown,
                x,
                y,
                width,
            } => MdPiece {
                kind: 0,
                text: markdown::styled(&markdown),
                x,
                y,
                w: width + SLACK,
                ..MdPiece::default()
            },
            Piece::Formula {
                index,
                x,
                y,
                scale: shrunk,
            } => {
                let found = sources
                    .get(index)
                    .and_then(|tex| self.formulas.get(&(tex.to_string(), false)));
                let Some(Drawn::Ready {
                    image,
                    width,
                    height,
                    depth,
                }) = found
                else {
                    return MdPiece::default();
                };
                let unit = em * shrunk;
                MdPiece {
                    kind: 1,
                    x: snap(x - pad * unit),
                    y: snap(y - pad * unit),
                    w: snap((width + 2.0 * pad) * unit),
                    h: snap((height + depth + 2.0 * pad) * unit),
                    image: image.clone(),
                    ..MdPiece::default()
                }
            }
            Piece::Source {
                index,
                markdown,
                x,
                y,
                width,
            } => {
                let tip = match sources
                    .get(index)
                    .and_then(|tex| self.formulas.get(&(tex.to_string(), false)))
                {
                    Some(Drawn::Refused(reason)) => {
                        t!("markdown.formula-refused", reason = reason.clone())
                    }
                    _ => String::new(),
                };
                MdPiece {
                    kind: 2,
                    text: markdown::styled(&markdown),
                    x,
                    y,
                    w: width + SLACK,
                    tip: tip.into(),
                    ..MdPiece::default()
                }
            }
        }
    }
}

/// Whether a file is Markdown, by its name.
fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx")
}

/// Whether a file has a page over its source: Markdown read, an SVG drawn.
fn has_page(path: &str) -> bool {
    is_markdown(path) || crate::picture::is_svg(path)
}

/// Where a picture a page names is, or why it will not be read: never
/// above the project's root, and never from another machine — fetching it
/// would tell its host who opened the file.
fn picture_path(place: Option<&(PathBuf, String)>, src: &str) -> Result<PathBuf, String> {
    if src.contains("://") || src.starts_with("//") {
        return Err(t!("markdown.image-remote"));
    }
    let Some((root, dir)) = place else {
        return Err(t!("markdown.image-no-project"));
    };
    let written = src;
    let src = src.split(['?', '#']).next().unwrap_or_default();
    let src = percent_decoded(src);
    let relative = match src.strip_prefix('/') {
        Some(from_root) => from_root.to_string(),
        None if dir.is_empty() => src,
        None => format!("{dir}/{src}"),
    };
    rusty_edit::absolute(root, &relative).map_err(|e| {
        t!(
            "markdown.image-failed",
            path = written.to_string(),
            error = e.to_string()
        )
    })
}

/// `%20` and friends, as a link writes a path with spaces in it — the
/// language server's decoder, which refuses a broken escape; a link that
/// holds one is kept as written rather than mangled.
fn percent_decoded(s: &str) -> String {
    rusty_lsp::percent_decode(s).unwrap_or_else(|| s.to_string())
}

impl super::App {
    /// A Markdown file in front is read as a page unless its source was
    /// asked for: reading is the common case in a workbench — a README, a
    /// book — so the page is the default, and the toggle says which way it
    /// will go. An SVG is drawn over its source the same way.
    pub(super) fn sync_markdown(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<crate::Shell>();
        let path = match self.active_tab() {
            Some(super::Tab::File(path)) if has_page(&path) => Some(path),
            _ => None,
        };
        let page = path
            .as_ref()
            .is_some_and(|p| !self.state.borrow().markdown_source.contains(p));
        shell.set_md_file(path.is_some());
        shell.set_md_page(page);
        let svg = path.as_deref().is_some_and(crate::picture::is_svg);
        shell.set_picture_page(page && svg);
        let (Some(path), true) = (path, page) else {
            return;
        };
        let Some((text, ..)) = self.editor.draft_of(&path) else {
            return;
        };
        if svg {
            self.show_svg(&text);
            return;
        }
        let root = self.root();
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let read = markdown::blocks(&text);
        let place = root.map(|root| (root, dir.to_string()));
        self.want_formulas(&read);
        self.want_pictures(&read, place.as_ref());
        {
            let mut state = self.state.borrow_mut();
            state.page.read = read;
            state.page.place = place;
        }
        let count = self.state.borrow().page.read.len();
        let rows: Vec<MdBlock> = (0..count)
            .map(|index| self.page_block(&window, index))
            .collect();
        let model = Rc::new(VecModel::from(rows));
        self.state.borrow_mut().page.rows = Some(model.clone());
        shell.set_md_blocks(ModelRc::from(model));
    }

    /// The page's block at `index`, as its formulas, pictures and reported
    /// widths have it now.
    fn page_block(&self, window: &AppWindow, index: usize) -> MdBlock {
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        let (page, pictures) = (&mut state.page, &state.pictures);
        let Some(block) = page.read.get(index).cloned() else {
            return MdBlock::default();
        };
        match &block {
            Block::Text { flow: runs, .. } if !runs.is_empty() => {
                let key = key(index, 0);
                let lines = page.lines(window, key, runs);
                MdBlock {
                    flowing: true,
                    key,
                    flow: ModelRc::new(VecModel::from(lines)),
                    ..plain(&self.files, &block)
                }
            }
            Block::Heading { flow: runs, .. } if !runs.is_empty() => {
                let key = key(index, 0);
                let lines = page.lines(window, key, runs);
                MdBlock {
                    flowing: true,
                    key,
                    flow: ModelRc::new(VecModel::from(lines)),
                    ..plain(&self.files, &block)
                }
            }
            Block::Table { rows } => table(rows, |at, cell| {
                if cell.flow.is_empty() {
                    return None;
                }
                let key = key(index, at);
                Some((key, page.lines(window, key, &cell.flow)))
            }),
            Block::Image { src, alt, figure } => {
                let shown_as = if alt.is_empty() {
                    src.clone()
                } else {
                    alt.clone()
                };
                let found = picture_path(page.place.as_ref(), src).and_then(|path| match pictures
                    .get(&path)
                {
                    Some(Pic::Ready { image, .. }) => Ok(Some(image.clone())),
                    Some(Pic::Failed(reason)) => Err(t!(
                        "markdown.image-failed",
                        path = src.clone(),
                        error = reason.clone()
                    )),
                    _ => Ok(None),
                });
                match found {
                    Ok(Some(image)) => MdBlock {
                        kind: 5,
                        shown: true,
                        image,
                        marker: alt.clone().into(),
                        figure: *figure,
                        ..MdBlock::default()
                    },
                    // On its way: its words stand in for it until it is read.
                    Ok(None) => MdBlock {
                        kind: 5,
                        marker: shown_as.into(),
                        figure: *figure,
                        ..MdBlock::default()
                    },
                    Err(why) => MdBlock {
                        kind: 5,
                        marker: shown_as.into(),
                        tip: why.into(),
                        figure: *figure,
                        ..MdBlock::default()
                    },
                }
            }
            Block::Math { tex, .. } => match page.formulas.get(&(tex.clone(), true)) {
                Some(Drawn::Ready {
                    image,
                    width,
                    height,
                    depth,
                }) => MdBlock {
                    shown: true,
                    image: image.clone(),
                    mw: (width + 2.0 * math::PAD) * MATH_SCALE,
                    mh: (height + depth + 2.0 * math::PAD) * MATH_SCALE,
                    ..plain(&self.files, &block)
                },
                Some(Drawn::Refused(reason)) => MdBlock {
                    tip: t!("markdown.formula-refused", reason = reason.clone()).into(),
                    ..plain(&self.files, &block)
                },
                _ => plain(&self.files, &block),
            },
            _ => plain(&self.files, &block),
        }
    }

    /// Draw every formula of `read` not drawn yet, off the UI thread and on
    /// every core, and show the page again with them.
    fn want_formulas(self: &Rc<Self>, read: &[Block]) {
        let mut wanted: Vec<(String, bool)> = Vec::new();
        let on_page = formulas_of(read);
        {
            let mut state = self.state.borrow_mut();
            let formulas = &mut state.page.formulas;
            if formulas.len() > KEEP_FORMULAS {
                formulas.retain(|key, _| on_page.contains(key));
            }
            for key in on_page {
                if !formulas.contains_key(&key) {
                    formulas.insert(key.clone(), Drawn::Pending);
                    wanted.push(key);
                }
            }
        }
        if wanted.is_empty() {
            return;
        }
        crate::work::spawn(
            move || render_all(wanted),
            |app, drawn| {
                {
                    let mut state = app.state.borrow_mut();
                    for (key, result) in drawn {
                        let drawn = match result.and_then(|formula| {
                            slint::Image::load_from_svg_data(formula.svg.as_bytes())
                                .map(|image| (image, formula))
                                .map_err(|_| "the formula's picture could not be read".to_string())
                        }) {
                            Ok((image, formula)) => Drawn::Ready {
                                image,
                                width: formula.width,
                                height: formula.height,
                                depth: formula.depth,
                            },
                            Err(reason) => Drawn::Refused(reason),
                        };
                        state.page.formulas.insert(key, drawn);
                    }
                }
                app.sync_markdown();
            },
        );
    }

    /// Read every picture `read` names that is not read yet.
    fn want_pictures(self: &Rc<Self>, read: &[Block], place: Option<&(PathBuf, String)>) {
        for block in read {
            if let Block::Image { src, .. } = block
                && let Ok(path) = picture_path(place, src)
            {
                self.want_picture(path);
            }
        }
    }

    /// A flowing paragraph's view says how wide it is and at what text size:
    /// lay it out again when either changed, once for all the reports one
    /// layout pass makes.
    pub(super) fn md_flow_width(self: &Rc<Self>, key: i32, width: f32, size: f32) {
        // A view not laid out yet says nothing about its width: the width it
        // gets is reported when it gets it.
        if width < 1.0 || size <= 0.0 {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            let page = &mut state.page;
            if page
                .widths
                .get(&key)
                .is_some_and(|(w, s)| (w - width).abs() < 0.5 && (s - size).abs() < 0.01)
            {
                return;
            }
            page.widths.insert(key, (width, size));
            if page.reflow_due {
                return;
            }
            page.reflow_due = true;
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(app) = weak.upgrade() {
                app.md_reflow();
            }
        });
    }

    /// Lay out again every paragraph whose width or size moved.
    fn md_reflow(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let (blocks, rows) = {
            let mut state = self.state.borrow_mut();
            let page = &mut state.page;
            page.reflow_due = false;
            let mut blocks: Vec<usize> = page
                .widths
                .iter()
                .filter(|(key, at)| {
                    page.laid.get(key).is_none_or(|laid| {
                        (laid.0 - at.0).abs() >= 0.5 || (laid.1 - at.1).abs() >= 0.01
                    })
                })
                .map(|(key, _)| (*key / 65536) as usize)
                .collect();
            blocks.sort_unstable();
            blocks.dedup();
            (blocks, page.rows.clone())
        };
        let Some(rows) = rows else { return };
        for index in blocks {
            if index < slint::Model::row_count(&*rows) {
                let block = self.page_block(&window, index);
                slint::Model::set_row_data(&*rows, index, block);
            }
        }
    }

    /// Whether everything the page in front asked for has arrived: every
    /// formula drawn or refused, every picture read or failed, every
    /// paragraph with a formula laid out. For the probe.
    pub fn page_settled(&self) -> bool {
        let state = self.state.borrow();
        let page = &state.page;
        let pending = page.formulas.values().any(|f| matches!(f, Drawn::Pending));
        let reading = page.read.iter().any(|block| match block {
            Block::Image { src, .. } => picture_path(page.place.as_ref(), src)
                .is_ok_and(|path| matches!(state.pictures.get(&path), Some(Pic::Pending) | None)),
            _ => false,
        });
        let unlaid = flow_keys(&page.read)
            .iter()
            .any(|key| !page.laid.contains_key(key));
        !pending && !reading && !unlaid && !page.reflow_due
    }

    /// What the page in front holds, counted: for the probe.
    pub fn page_report(&self) -> String {
        let state = self.state.borrow();
        let page = &state.page;
        let count = |f: &dyn Fn(&Block) -> bool| page.read.iter().filter(|b| f(b)).count();
        let flowing = count(&|b| match b {
            Block::Text { flow, .. } | Block::Heading { flow, .. } => !flow.is_empty(),
            Block::Table { rows } => rows.iter().flatten().any(|c| !c.flow.is_empty()),
            _ => false,
        });
        let (mut ready, mut refused) = (0, Vec::new());
        let on_page = formulas_of(&page.read);
        for ((tex, display), drawn) in &page.formulas {
            if !on_page.contains(&(tex.clone(), *display)) {
                continue;
            }
            match drawn {
                Drawn::Ready { .. } => ready += 1,
                Drawn::Refused(reason) => refused.push(format!("{tex:?}: {reason}")),
                Drawn::Pending => {}
            }
        }
        let pictures: Vec<String> = page
            .read
            .iter()
            .filter_map(|block| match block {
                Block::Image { src, .. } => Some(format!(
                    "{src} {}",
                    match picture_path(page.place.as_ref(), src)
                        .map(|path| state.pictures.get(&path))
                    {
                        Ok(Some(Pic::Ready { width, height, .. })) => format!("{width}x{height}"),
                        Ok(Some(Pic::Failed(reason))) => format!("failed: {reason}"),
                        Ok(_) => "reading".to_string(),
                        Err(why) => why,
                    }
                )),
                _ => None,
            })
            .collect();
        format!(
            "{} blocks, {} with formulas in lines ({} laid), {} display; formulas drawn {}, refused {} {:?}; pictures {:?}",
            page.read.len(),
            flowing,
            flow_keys(&page.read)
                .iter()
                .filter(|key| page.laid.contains_key(key))
                .count(),
            count(&|b| matches!(b, Block::Math { .. })),
            ready,
            refused.len(),
            refused,
            pictures
        )
    }

    /// The page and its source, swapped for the file in front.
    pub(super) fn markdown_toggle(self: &std::rc::Rc<Self>) {
        let Some(super::Tab::File(path)) = self.active_tab() else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let source = &mut state.markdown_source;
            if !source.remove(&path) {
                source.insert(path);
            }
        }
        self.sync_markdown();
    }
}

/// Every formula drawn, the work shared out over the machine's cores.
fn render_all(wanted: Vec<(String, bool)>) -> Vec<((String, bool), Result<math::Formula, String>)> {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let share = wanted.len().div_ceil(cores).max(8);
    std::thread::scope(|scope| {
        let workers: Vec<_> = wanted
            .chunks(share)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(tex, display)| {
                            ((tex.clone(), *display), math::render(tex, *display))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect()
    })
}

/// A link clicked on a page: a web address goes to the system's browser,
/// and nothing else is opened — a page does not get to run a program.
pub(super) fn open_link(link: &str) {
    if !(link.starts_with("https://") || link.starts_with("http://")) {
        return;
    }
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("explorer.exe");
        c.arg(link);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(link);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(link);
        c
    };
    let _ = command.spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture is found from the page's folder, never above the project's
    /// root, and never fetched from another machine.
    #[test]
    fn a_picture_is_found_from_its_page_and_stays_in_the_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("book/src/figures")).unwrap();
        std::fs::write(dir.path().join("book/src/figures/fig 1.svg"), "<svg/>").unwrap();
        std::fs::write(dir.path().join("logo.png"), "x").unwrap();
        let place = (dir.path().to_path_buf(), "book/src".to_string());
        let found = picture_path(Some(&place), "figures/fig%201.svg").unwrap();
        assert!(found.ends_with("fig 1.svg"));
        assert!(
            picture_path(Some(&place), "/logo.png").is_ok(),
            "from the root"
        );
        assert!(picture_path(Some(&place), "../../../outside.png").is_err());
        let remote = picture_path(Some(&place), "https://example.com/a.png").unwrap_err();
        assert_eq!(remote, t!("markdown.image-remote"));
        assert!(picture_path(None, "figures/fig 1.svg").is_err());
    }
}
