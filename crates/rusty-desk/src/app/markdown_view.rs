//! Markdown's blocks (`crate::markdown`) as the rows `ui/markdown.slint`
//! draws: inline text as styled text, a code block's lines painted by the
//! editor's own highlighter, so a fence and the file it was copied from are
//! the same colours.

use std::path::Path;

use rusty_edit::Files;
use rusty_i18n::t;
use slint::{ModelRc, VecModel};

use crate::{
    CodeLine, CodeSpan, MdBlock, MdCell, MdRow,
    markdown::{self, Block},
};

/// A document's blocks, ready to draw. `place` is the project's root and
/// the document's folder in it, for the pictures it names: a picture is
/// found from the document's folder and never above the root, and one on
/// another machine is named rather than fetched — fetching it would tell its
/// host who opened the file.
pub(super) fn blocks(files: &Files, text: &str, place: Option<(&Path, &str)>) -> ModelRc<MdBlock> {
    let rows: Vec<MdBlock> = markdown::blocks(text)
        .into_iter()
        .map(|block| match block {
            Block::Text {
                inline,
                quote,
                item,
            } => MdBlock {
                kind: 0,
                text: markdown::styled(&inline),
                quote: quote as i32,
                depth: item.as_ref().map_or(0, |i| i.depth as i32 + 1),
                marker: item.map(|i| i.marker).unwrap_or_default().into(),
                ..MdBlock::default()
            },
            Block::Heading { level, inline } => MdBlock {
                kind: 1,
                level: level as i32,
                text: markdown::styled(&inline),
                ..MdBlock::default()
            },
            Block::Code { lang, lines } => {
                let painted = files.highlight_snippet(&lang, &lines.join("\n"));
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
            Block::Rule => MdBlock {
                kind: 3,
                ..MdBlock::default()
            },
            Block::Image { src, alt } => {
                let remote = src.contains("://");
                let picture = place.filter(|_| !remote).and_then(|(root, dir)| {
                    let relative = if dir.is_empty() {
                        src.clone()
                    } else {
                        format!("{dir}/{src}")
                    };
                    let path = rusty_edit::absolute(root, &relative).ok()?;
                    slint::Image::load_from_path(&path).ok()
                });
                let note = match (&picture, remote) {
                    (Some(_), _) => alt,
                    (None, true) => format!("{alt} — {}", t!("markdown.image-remote")),
                    (None, false) => t!("markdown.image-failed", path = src.clone(), error = alt),
                };
                MdBlock {
                    kind: 5,
                    shown: picture.is_some(),
                    image: picture.unwrap_or_default(),
                    marker: note.into(),
                    ..MdBlock::default()
                }
            }
            Block::Table { rows } => {
                let rows: Vec<MdRow> = rows
                    .iter()
                    .enumerate()
                    .map(|(at, cells)| MdRow {
                        cells: ModelRc::new(VecModel::from(
                            cells
                                .iter()
                                .map(|cell| MdCell {
                                    text: markdown::styled(cell),
                                    header: at == 0,
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
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// Whether a file is Markdown, by its name.
fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx")
}

impl super::App {
    /// A Markdown file in front is read as a page unless its source was
    /// asked for: reading is the common case in a workbench — a README, a
    /// book — so the page is the default, and the toggle says which way it
    /// will go.
    pub(super) fn sync_markdown(&self) {
        use slint::ComponentHandle;
        let Some(window) = self.window() else { return };
        let shell = window.global::<crate::Shell>();
        let path = match self.active_tab() {
            Some(super::Tab::File(path)) if is_markdown(&path) => Some(path),
            _ => None,
        };
        let page = path
            .as_ref()
            .is_some_and(|p| !self.state.borrow().markdown_source.contains(p));
        shell.set_md_file(path.is_some());
        shell.set_md_page(page);
        let (Some(path), true) = (path, page) else {
            return;
        };
        let Some((text, ..)) = self.editor.draft_of(&path) else {
            return;
        };
        let root = self.root();
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        shell.set_md_blocks(blocks(
            &self.files,
            &text,
            root.as_deref().map(|root| (root, dir)),
        ));
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
