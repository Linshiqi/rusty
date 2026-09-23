//! Reading what `git` prints, with the formats chosen so it is unambiguous.
//!
//! Every parser here takes text and returns model types, with the real output
//! pinned in its test. The log is asked for with field and record separators
//! that cannot appear in a commit message (`%x1f` and `%x1e`), because a
//! message with a newline or a tab in it is ordinary and a parser that split
//! on either would tear commits in half.

mod diff;
mod log;
mod refs;
mod status;

pub use diff::{diff_parts, files};
pub use log::{DETAIL_FORMAT, LOG_FORMAT, STASH_FORMAT, detail, log, stashes};
pub use refs::{REFS_FORMAT, refs, remotes};
pub use status::status;
