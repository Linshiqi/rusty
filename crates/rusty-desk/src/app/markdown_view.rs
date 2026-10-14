//! Markdown's blocks (`crate::markdown`) as the rows `ui/markdown.slint`
//! draws: inline text as styled text, a code block's lines painted by the
//! editor's own highlighter, so a fence and the file it was copied from are
//! the same colours.

use rusty_edit::Files;
use slint::{ModelRc, VecModel};

use crate::{
    CodeLine, CodeSpan, MdBlock, MdCell, MdRow,
    markdown::{self, Block},
};

/// A document's blocks, ready to draw.
pub(super) fn blocks(files: &Files, text: &str) -> ModelRc<MdBlock> {
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
