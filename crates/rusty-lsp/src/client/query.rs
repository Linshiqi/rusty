//! Questions that change nothing: what the thing under a position is, the
//! signature of the call around it, and how the server colours a document.

use serde_json::{Value, json};

use super::LspClient;
use crate::{
    convert,
    error::Result,
    model::{HoverInfo, SemanticSpan, SignatureInfo},
};

impl LspClient {
    /// What the thing under this position is, as prose, and how much text the
    /// answer covers — the range is what lets a tooltip stay up while the
    /// pointer moves within the same token.
    pub fn hover(&self, path: &str, line: u32, col: u32) -> Result<Option<HoverInfo>> {
        let result = self
            .shared
            .request("textDocument/hover", self.position_params(path, line, col))?;
        let text = self.shared.open_text(path);
        Ok(convert::hover_info(
            &result,
            text.as_deref(),
            self.shared.encoding(),
        ))
    }

    /// The signature of the call around this position, if the caret is inside
    /// one.
    pub fn signature_help(&self, path: &str, line: u32, col: u32) -> Result<Option<SignatureInfo>> {
        let result = self.shared.request(
            "textDocument/signatureHelp",
            self.position_params(path, line, col),
        )?;
        Ok(convert::signature_info(&result))
    }

    /// A document's semantic colouring, as the server sees it — for an open
    /// document; there is nothing to convert against otherwise. The whole of
    /// it, or the lines `from..to`: all of a 24,000-line file was three and a
    /// half megabytes and 600 ms after every pause in typing, nearly all of it
    /// for lines nobody was looking at.
    pub fn semantic_tokens(
        &self,
        path: &str,
        lines: Option<(u32, u32)>,
    ) -> Result<Vec<SemanticSpan>> {
        let document = json!({ "uri": self.shared.uri(path) });
        let result = match lines {
            Some((from, to)) => self.shared.request(
                "textDocument/semanticTokens/range",
                json!({
                    "textDocument": document,
                    "range": {
                        "start": { "line": from, "character": 0 },
                        "end": { "line": to, "character": 0 },
                    },
                }),
            )?,
            None => self.shared.request(
                "textDocument/semanticTokens/full",
                json!({ "textDocument": document }),
            )?,
        };
        let data: Vec<u32> = result["data"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|v| v as u32)
                    .collect()
            })
            .unwrap_or_default();
        let Some(text) = self.shared.open_text(path) else {
            return Ok(Vec::new());
        };
        Ok(convert::semantic_spans(
            &data,
            &text,
            &self.shared.legend(),
            self.shared.encoding(),
        ))
    }
}
