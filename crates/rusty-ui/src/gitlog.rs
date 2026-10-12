//! The Git log's arithmetic, pure: which rows to draw, where to scroll, what
//! a search matches, what a drawn row is keyed on, and the shape of every
//! line between two rows.
//!
//! The log drew every row it had — four hundred small SVGs, each with its
//! labels — and rebuilt all of them whenever the selection moved or the
//! history was read again, which was every save. It draws the rows on screen
//! now, plus a few each side. Every row is the same height, so which rows
//! those are is division, and the division is here, under tests, rather than
//! discovered in a scrolling window.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use rusty_git::{GraphRow, RefKind};

/// The rows of a list of `total` rows `row` pixels tall to draw when the
/// scroller is at `scroll_top` and `height` pixels tall: the ones in view,
/// and `spare` more each side.
///
/// The spare rows are not only for smooth scrolling. A row draws the lines
/// that *leave* it, down into the row below, so the row above the first one
/// in view has to exist for the lines arriving at the top of the view.
pub fn window(scroll_top: f64, height: f64, row: f64, total: usize, spare: usize) -> Range<usize> {
    if total == 0 || row <= 0.0 {
        return 0..0;
    }
    let first = (scroll_top.max(0.0) / row).floor() as usize;
    let shown = (height.max(0.0) / row).ceil() as usize + 1;
    let start = first.saturating_sub(spare).min(total);
    let end = first.saturating_add(shown + spare).min(total);
    start..end
}

/// Where to scroll so row `index` is in view, or `None` when it already is.
///
/// A row just out of view is brought to the nearer edge, as an arrow key in
/// any list moves; one further away than a screen is centred, because a
/// branch clicked in the sidebar lands in the middle of the page rather
/// than on its bottom line.
pub fn scroll_for(index: usize, scroll_top: f64, height: f64, row: f64) -> Option<f64> {
    let top = index as f64 * row;
    let bottom = top + row;
    let view_bottom = scroll_top + height;
    if top >= scroll_top && bottom <= view_bottom {
        return None;
    }
    let far = top + height < scroll_top || top > view_bottom + height;
    let target = if far {
        top - (height - row) / 2.0
    } else if top < scroll_top {
        top
    } else {
        bottom - height
    };
    Some(target.max(0.0))
}

/// Whether a row matches a search, already lower-cased: the start of its
/// hash, words of its subject, its author, the name of a branch or tag on it.
pub fn matches(row: &GraphRow, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let commit = &row.commit;
    commit.id.starts_with(needle)
        || commit.summary.to_lowercase().contains(needle)
        || commit.author.to_lowercase().contains(needle)
        || commit.email.to_lowercase().contains(needle)
        || commit
            .refs
            .iter()
            .any(|label| label.name.to_lowercase().contains(needle))
}

/// The indices of every row a search matches, in order. Empty for no search.
pub fn hits(rows: &[GraphRow], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .enumerate()
        .filter(|(_, row)| matches(row, &needle))
        .map(|(at, _)| at)
        .collect()
}

/// What a drawn row depends on, as one number. The log keys its rows on it,
/// so a history read again redraws only the rows whose lane, lines or labels
/// moved — or whose next row's commit moved lane, which decides where the
/// lines leaving this one turn ([`edge_line`]). The hash stands for the rest
/// of the commit: its subject, author and time cannot change without the
/// hash changing.
pub fn row_key(row: &GraphRow, lanes: u32, next_lane: Option<u32>) -> u64 {
    let mut hasher = DefaultHasher::new();
    row.commit.id.hash(&mut hasher);
    (
        row.lane,
        lanes,
        next_lane,
        row.edges.len(),
        row.commit.refs.len(),
    )
        .hash(&mut hasher);
    for edge in &row.edges {
        (edge.from, edge.to).hash(&mut hasher);
    }
    for label in &row.commit.refs {
        let kind: u8 = match label.kind {
            RefKind::Head => 0,
            RefKind::Branch => 1,
            RefKind::Remote => 2,
            RefKind::Tag => 3,
        };
        (kind, &label.name).hash(&mut hasher);
    }
    hasher.finish()
}

/// One line of the graph as drawn: an SVG path from a row's centre to the
/// next row's, and the lane whose colour it takes.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeLine {
    pub d: String,
    pub lane: u32,
}

/// How a line runs from lane `from` at one row to lane `to` at the next, the
/// way Fork draws it rather than as a slant: straight down within a lane,
/// and where it joins a commit on the lane to its left, a quarter circle
/// into it — a merge leaving for its second parent turns out of the commit
/// and down the parent's lane, and a branch arriving at the commit it grew
/// from comes down its own lane and turns into it. The turn is always at the
/// left lane's end, so the vertical run is the right lane's and the line
/// takes that lane's colour. `dot_above` is whether this row's commit sits
/// in `from`, `dot_below` whether the next row's sits in `to`: a line that
/// only shifts lanes, with no commit on the side it would turn at, is an S
/// instead, because a turn that ended nowhere would read as a corner.
pub fn edge_line(
    from: u32,
    to: u32,
    dot_above: bool,
    dot_below: bool,
    lane_px: f64,
    row_px: f64,
) -> EdgeLine {
    let x = |lane: u32| f64::from(lane) * lane_px + lane_px / 2.0;
    let (x1, x2) = (x(from), x(to));
    let top = row_px / 2.0;
    let bottom = top + row_px;
    let r = lane_px.min(row_px / 2.0);
    if from == to {
        return EdgeLine {
            d: format!("M {x1} {top} V {bottom}"),
            lane: to,
        };
    }
    if from < to && dot_above {
        // Out of the commit to the right, then down the parent's lane.
        let bend = x2 - r;
        let down = top + r;
        return EdgeLine {
            d: format!("M {x1} {top} H {bend} A {r} {r} 0 0 1 {x2} {down} V {bottom}"),
            lane: to,
        };
    }
    if from > to && dot_below {
        // Down its own lane, then into the commit on the left.
        let turn = bottom - r;
        let across = x1 - r;
        return EdgeLine {
            d: format!("M {x1} {top} V {turn} A {r} {r} 0 0 1 {across} {bottom} H {x2}"),
            lane: from,
        };
    }
    let middle = top + row_px / 2.0;
    EdgeLine {
        d: format!("M {x1} {top} C {x1} {middle} {x2} {middle} {x2} {bottom}"),
        lane: to,
    }
}

/// The next hit after `at` (the first when nothing is selected), or the one
/// before it, wrapping at either end.
pub fn step_hit(hits: &[usize], at: Option<usize>, forward: bool) -> Option<usize> {
    let (first, last) = (*hits.first()?, *hits.last()?);
    let Some(at) = at else {
        return Some(if forward { first } else { last });
    };
    if forward {
        Some(hits.iter().copied().find(|&hit| hit > at).unwrap_or(first))
    } else {
        Some(
            hits.iter()
                .rev()
                .copied()
                .find(|&hit| hit < at)
                .unwrap_or(last),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_git::{Commit, Edge, RefLabel};

    fn row(id: &str, summary: &str, author: &str, refs: &[&str]) -> GraphRow {
        GraphRow {
            commit: Commit {
                id: id.into(),
                short: id.chars().take(7).collect(),
                parents: Vec::new(),
                author: author.into(),
                email: format!("{}@example.com", author.to_lowercase()),
                time: 0,
                summary: summary.into(),
                refs: refs
                    .iter()
                    .map(|name| RefLabel {
                        kind: RefKind::Branch,
                        name: (*name).into(),
                    })
                    .collect(),
            },
            lane: 0,
            edges: Vec::new(),
        }
    }

    #[test]
    fn the_window_is_the_rows_in_view_and_some_either_side() {
        // 26px rows, a 260px view: ten rows in view, one part-row, spare 5.
        assert_eq!(window(0.0, 260.0, 26.0, 1000, 5), 0..16);
        assert_eq!(window(2600.0, 260.0, 26.0, 1000, 5), 95..116);
        assert_eq!(
            window(25_990.0, 260.0, 26.0, 1000, 5),
            994..1000,
            "clamped at the end"
        );
        assert_eq!(
            window(0.0, 260.0, 26.0, 3, 5),
            0..3,
            "a short list is all of it"
        );
        assert_eq!(window(0.0, 260.0, 26.0, 0, 5), 0..0);
        assert!(
            window(1300.0, 260.0, 26.0, 1000, 5).start < 50,
            "the row above the first in view is drawn, for the lines that arrive from it"
        );
    }

    #[test]
    fn a_row_out_of_view_is_brought_to_the_near_edge_or_centred_when_far() {
        // In view: nothing to do.
        assert_eq!(scroll_for(3, 0.0, 260.0, 26.0), None);
        // Just below: its bottom meets the view's.
        assert_eq!(scroll_for(10, 0.0, 260.0, 26.0), Some(26.0));
        // Just above: its top meets the view's.
        assert_eq!(scroll_for(9, 260.0, 260.0, 26.0), Some(234.0));
        // A screen and more away: centred.
        assert_eq!(
            scroll_for(500, 0.0, 260.0, 26.0),
            Some(500.0 * 26.0 - 117.0)
        );
        assert_eq!(
            scroll_for(0, 5000.0, 260.0, 26.0),
            Some(0.0),
            "never above the top"
        );
    }

    #[test]
    fn a_search_matches_hash_starts_subjects_authors_and_labels() {
        let rows = vec![
            row("20d12f8aaa", "Fix the mixer", "Lin", &["main"]),
            row("59ea8cd0bb", "Add a tachometer", "Wang", &[]),
            row("aa00ff11cc", "Tidy", "Lin", &["feature/tacho"]),
        ];
        assert_eq!(hits(&rows, "20d1"), vec![0], "the start of a hash");
        assert_eq!(
            hits(&rows, "d12f8"),
            Vec::<usize>::new(),
            "not the middle of one"
        );
        assert_eq!(
            hits(&rows, "TACHO"),
            vec![1, 2],
            "subject and label, any case"
        );
        assert_eq!(hits(&rows, "lin"), vec![0, 2], "the author");
        assert_eq!(hits(&rows, "wang@"), vec![1], "the email");
        assert!(hits(&rows, "  ").is_empty(), "no search is no hits");
    }

    #[test]
    fn a_row_is_keyed_on_what_it_draws() {
        let plain = row("20d12f8aaa", "Fix the mixer", "Lin", &[]);
        assert_eq!(
            row_key(&plain, 2, None),
            row_key(&plain.clone(), 2, None),
            "the same row, the same key"
        );
        let mut moved = plain.clone();
        moved.lane = 1;
        assert_ne!(
            row_key(&plain, 2, None),
            row_key(&moved, 2, None),
            "its dot moved lane"
        );
        let mut joined = plain.clone();
        joined.edges.push(Edge { from: 0, to: 1 });
        assert_ne!(
            row_key(&plain, 2, None),
            row_key(&joined, 2, None),
            "a line leaves it"
        );
        let labelled = row("20d12f8aaa", "Fix the mixer", "Lin", &["main"]);
        assert_ne!(
            row_key(&plain, 2, None),
            row_key(&labelled, 2, None),
            "a branch arrived on it"
        );
        assert_ne!(
            row_key(&plain, 2, None),
            row_key(&plain, 3, None),
            "the graph widened"
        );
        assert_ne!(
            row_key(&plain, 2, Some(0)),
            row_key(&plain, 2, Some(1)),
            "the commit below moved lane, and the lines turn at it"
        );
        let other = row("59ea8cd0bb", "Fix the mixer", "Lin", &[]);
        assert_ne!(
            row_key(&plain, 2, None),
            row_key(&other, 2, None),
            "another commit"
        );
    }

    /// Straight within a lane; a quarter circle where a line joins a commit
    /// on the lane to its left — out of a merge above, into the commit a
    /// branch grew from below — coloured as the lane it runs down; an S
    /// where it only shifts over.
    #[test]
    fn lines_turn_into_commits_and_bend_between_lanes() {
        let straight = edge_line(1, 1, true, true, 14.0, 26.0);
        assert_eq!(straight.d, "M 21 13 V 39");
        assert_eq!(straight.lane, 1);

        let merge = edge_line(0, 1, true, true, 14.0, 26.0);
        assert_eq!(merge.d, "M 7 13 H 8 A 13 13 0 0 1 21 26 V 39");
        assert_eq!(merge.lane, 1, "the branch's colour, down the branch's lane");

        let grew_from = edge_line(1, 0, true, true, 14.0, 26.0);
        assert_eq!(grew_from.d, "M 21 13 V 26 A 13 13 0 0 1 8 39 H 7");
        assert_eq!(grew_from.lane, 1, "the branch's colour, into the commit");

        let far = edge_line(0, 2, true, false, 14.0, 26.0);
        assert!(far.d.starts_with("M 7 13 H 22 A 13 13"), "{}", far.d);

        let shift = edge_line(2, 1, false, false, 14.0, 26.0);
        assert_eq!(shift.d, "M 35 13 C 35 26 21 26 21 39");
        assert_eq!(shift.lane, 1);
        // Leaving a commit leftwards with nothing to turn into below.
        assert!(edge_line(1, 0, true, false, 14.0, 26.0).d.contains(" C "));
    }

    #[test]
    fn stepping_through_hits_wraps_at_both_ends() {
        let hits = [2, 5, 9];
        assert_eq!(step_hit(&hits, None, true), Some(2));
        assert_eq!(step_hit(&hits, None, false), Some(9));
        assert_eq!(step_hit(&hits, Some(2), true), Some(5));
        assert_eq!(step_hit(&hits, Some(9), true), Some(2), "wraps forward");
        assert_eq!(step_hit(&hits, Some(2), false), Some(9), "wraps back");
        assert_eq!(
            step_hit(&hits, Some(6), false),
            Some(5),
            "from a row between hits"
        );
        assert_eq!(step_hit(&[], Some(1), true), None);
    }
}
