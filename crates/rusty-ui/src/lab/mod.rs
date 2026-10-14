//! The signal lab's arithmetic is `rusty_embed::lab`, shared by both
//! windows; this is the words for what it refuses, in the window's
//! language.

use std::collections::BTreeMap;

use rusty_embed::model::Sheet;
use rusty_i18n::t;

pub use rusty_embed::lab::{Played, record, sources_of, start_of, sweep};

use crate::state::Source;

mod words;

pub use words::{dsp_refusal_text, preset_name, signal_error_text};

/// What `source` plays, or why it plays nothing, worded.
pub fn played(
    sheet: &Sheet,
    source: &Source,
    texts: &BTreeMap<Source, String>,
) -> Result<Played, String> {
    rusty_embed::lab::played(sheet, source, texts).map_err(|why| match why {
        rusty_embed::lab::Unplayable::Gone(part) => t!("lab.gone", part = part),
        rusty_embed::lab::Unplayable::Signal(error) => signal_error_text(&error),
        rusty_embed::lab::Unplayable::Rate(text) => text,
    })
}
