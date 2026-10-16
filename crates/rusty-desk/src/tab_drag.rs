//! Where a dragged tab lands, VS Code's way: along its own strip to move
//! it, onto the other group's strip or its code to move it there, onto the
//! right half of the only group's code to open the split with it there,
//! and beyond the window's edge into a window of its own. A tab dragged out
//! of a window of its own comes back by the same strips and the same code.
//!
//! Pure, and tested. In: the editor area as the window laid it out, and
//! each strip's tabs as wide as they are drawn — which is this crate's
//! say, not the layout's (`app/tab_drag.rs` measures every tab, and the
//! strip draws it that wide), so the slot a pointer falls in is the slot
//! drawn there. Out: where it lands, what to draw while it is held there,
//! and the two groups after it is let go.

/// A rectangle of the window, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// A strip's lane: its tabs' widths, how much of it shows — the strip less
/// its buttons — and how far it is scrolled.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lane {
    pub widths: Vec<f32>,
    pub shown: f32,
    pub scroll: f32,
}

impl Lane {
    /// How far the lane can scroll: the tabs that do not fit.
    fn travel(&self) -> f32 {
        (self.widths.iter().sum::<f32>() - self.shown).max(0.0)
    }

    /// Where the gap before tab `index` is drawn, from the lane's left edge
    /// as it shows — `index` past the last is the end of the last.
    fn gap(&self, index: usize) -> f32 {
        self.widths[..index.min(self.widths.len())]
            .iter()
            .sum::<f32>()
            - self.scroll
    }

    /// The gap a pointer `x` from the lane's left edge falls in, VS Code's
    /// rule: before the tab whose middle it has not reached, after the last
    /// past them all. Over the strip's buttons is the lane's end as shown.
    pub fn slot(&self, x: f32) -> usize {
        let x = x.clamp(0.0, self.shown) + self.scroll;
        let mut start = 0.0;
        for (index, width) in self.widths.iter().enumerate() {
            if x < start + width / 2.0 {
                return index;
            }
            start += width;
        }
        self.widths.len()
    }
}

/// The editor area as the window laid it out, in the window's logical
/// pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Area {
    /// The window's own size: beyond it is beyond its edge.
    pub window: (f32, f32),
    /// Both groups' panes, below the playground's bar when there is one.
    pub panes: Rect,
    /// How wide the left group is — the whole width while one group
    /// shows; the right one begins a pixel after it, past the divider.
    pub left: f32,
    /// How tall a strip is: under it is the group's code.
    pub strip: f32,
    /// The lane of the group with the keys, and the other group's.
    pub keys: Lane,
    pub other: Lane,
    pub split: bool,
    /// The group with the keys is the right one.
    pub keys_right: bool,
}

impl Area {
    fn lane(&self, other: bool) -> &Lane {
        if other { &self.other } else { &self.keys }
    }

    /// A group's pane: the group with the keys, or with `other` the other.
    pub fn pane(&self, other: bool) -> Rect {
        if !self.split {
            return self.panes;
        }
        let left = Rect {
            w: self.left,
            ..self.panes
        };
        let right = Rect {
            x: self.panes.x + self.left + 1.0,
            w: (self.panes.w - self.left - 1.0).max(0.0),
            ..self.panes
        };
        if other != self.keys_right {
            right
        } else {
            left
        }
    }

    /// The pane under a point, and whether it is the other group's. The
    /// divider's pixel is the right group's.
    fn pane_at(&self, x: f32, y: f32) -> Option<(Rect, bool)> {
        if !self.panes.contains(x, y) {
            return None;
        }
        if !self.split {
            return Some((self.panes, false));
        }
        let right = x >= self.panes.x + self.left;
        let other = right != self.keys_right;
        Some((self.pane(other), other))
    }

    /// A pane's code: all of it below its strip.
    fn code(&self, pane: Rect) -> Rect {
        Rect {
            y: pane.y + self.strip,
            h: (pane.h - self.strip).max(0.0),
            ..pane
        }
    }

    /// The right half of a pane's code, which a drop splits off.
    fn right_half(&self, pane: Rect) -> Rect {
        let code = self.code(pane);
        let half = (code.w / 2.0).round();
        Rect {
            x: code.x + half,
            w: code.w - half,
            ..code
        }
    }
}

/// Where a dragged tab is let go of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// Nothing moves: back where it was, over chrome that is not the editor
    /// area, or out of a window of its own onto no window.
    Nowhere,
    /// Into a group's strip, before the tab at `index` of it as it is now:
    /// `index` as long as the strip is after its last tab.
    Strip { other: bool, index: usize },
    /// Onto a group's code: at the end of its strip, or in front where it
    /// is when it is there already.
    Group { other: bool },
    /// Onto the right half of the only group's code: the split, with the
    /// tab in a group of its own on the right.
    Split,
    /// Beyond the window's edge: a window of its own there.
    Out,
}

/// Where a tab let go of at `x`, `y` in the window lands. `from` is the
/// group it is dragged from — whether it is the other group's, and where it
/// is in that group's strip — or `None` for a window of its own's tab.
pub fn land(area: &Area, from: Option<(bool, usize)>, x: f32, y: f32) -> Landing {
    let (width, height) = area.window;
    if !(0.0..width).contains(&x) || !(0.0..height).contains(&y) {
        // Out of a window of its own and onto no window: it stays there.
        return if from.is_some() {
            Landing::Out
        } else {
            Landing::Nowhere
        };
    }
    let Some((pane, other)) = area.pane_at(x, y) else {
        return Landing::Nowhere;
    };
    let lane = area.lane(other);
    if y < pane.y + area.strip {
        let index = lane.slot(x - pane.x);
        // Either side of itself is where it is.
        if let Some((from_other, at)) = from
            && from_other == other
            && (index == at || index == at + 1)
        {
            return Landing::Nowhere;
        }
        return Landing::Strip { other, index };
    }
    let split_off = !area.split && area.right_half(pane).contains(x, y);
    match from {
        // Its own group's code is where it is — but the right half of the
        // only group's splits it off, while a tab is left on the left: the
        // split never shows an empty pane.
        Some((from_other, _)) if from_other == other => {
            if split_off && lane.widths.len() > 1 {
                Landing::Split
            } else {
                Landing::Nowhere
            }
        }
        Some(_) => Landing::Group { other },
        None if split_off && !lane.widths.is_empty() => Landing::Split,
        None => Landing::Group { other },
    }
}

/// What the window draws while a dragged tab is held over a landing: the
/// marker in a strip's gap, the overlay over the code it would take.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shown {
    pub marker: Option<Rect>,
    pub overlay: Option<Rect>,
}

/// How wide the marker in a strip's gap is.
const MARKER: f32 = 2.0;

pub fn shown(area: &Area, landing: Landing) -> Shown {
    match landing {
        Landing::Strip { other, index } => {
            let pane = area.pane(other);
            let lane = area.lane(other);
            // On the gap, kept on the part of the lane that shows.
            let gap = lane.gap(index) - MARKER / 2.0;
            let x = gap.clamp(0.0, (lane.shown - MARKER).max(0.0));
            Shown {
                marker: Some(Rect {
                    x: pane.x + x,
                    y: pane.y,
                    w: MARKER,
                    h: area.strip,
                }),
                overlay: None,
            }
        }
        Landing::Group { other } => Shown {
            marker: None,
            overlay: Some(area.code(area.pane(other))),
        },
        Landing::Split => Shown {
            marker: None,
            overlay: Some(area.right_half(area.panes)),
        },
        Landing::Nowhere | Landing::Out => Shown::default(),
    }
}

/// How near an end of a lane a dragged tab scrolls it.
const EDGE: f32 = 28.0;

/// A strip to scroll while a dragged tab is held near an end of its lane,
/// when there is more of the lane that way: whether it is the other
/// group's, and which way, -1 towards the start and 1 towards the end.
pub fn edge_scroll(area: &Area, x: f32, y: f32) -> Option<(bool, i32)> {
    let (pane, other) = area.pane_at(x, y)?;
    if y >= pane.y + area.strip {
        return None;
    }
    let lane = area.lane(other);
    let at = x - pane.x;
    if at < EDGE && lane.scroll > 0.0 {
        Some((other, -1))
    } else if at > lane.shown - EDGE && at < lane.shown && lane.scroll < lane.travel() {
        Some((other, 1))
    } else {
        None
    }
}

/// The two editor groups' tabs as the window keeps them: the group with
/// the keys first, each with the tab in front, and which side the group
/// with the keys is on — 0 left, 1 right.
#[derive(Clone, Debug, PartialEq)]
pub struct Groups<T> {
    pub keys: Vec<T>,
    pub keys_front: Option<usize>,
    pub other: Vec<T>,
    pub other_front: Option<usize>,
    pub split: bool,
    pub focus: usize,
}

impl<T: Clone + PartialEq> Groups<T> {
    /// `tab` let go of at `landing`, out of the group it was in — `from`,
    /// whether it is the other group — or from a window of its own
    /// (`None`). The group it lands in takes the keys, with it in front,
    /// as VS Code's does; a group left with no tabs ends the split. One
    /// file is one tab of a group, so landing in a group that has it
    /// already moves that one. False when nothing moved.
    pub fn land(&mut self, tab: &T, from: Option<bool>, landing: Landing) -> bool {
        let (to, index) = match landing {
            Landing::Strip { other, index } => (other, Some(index)),
            Landing::Group { other } => (other, None),
            Landing::Split => return self.split_off(tab, from),
            Landing::Nowhere | Landing::Out => return false,
        };
        if to && !self.split {
            return false;
        }
        if let Some(from) = from
            && from != to
        {
            self.take(from, tab);
        }
        let (list, front) = if to {
            (&mut self.other, &mut self.other_front)
        } else {
            (&mut self.keys, &mut self.keys_front)
        };
        let had = list.iter().position(|t| t == tab);
        let at = match (had, index) {
            // Onto the code of a group that has it: in front where it is.
            (Some(had), None) => had,
            (None, None) => {
                list.push(tab.clone());
                list.len() - 1
            }
            (had, Some(index)) => {
                let mut index = index.min(list.len());
                if let Some(had) = had {
                    list.remove(had);
                    if had < index {
                        index -= 1;
                    }
                }
                list.insert(index, tab.clone());
                index
            }
        };
        *front = Some(at);
        if to {
            self.swap();
        }
        self.settle();
        true
    }

    /// The split, with `tab` in a group of its own on the right and the
    /// keys there — the only group's other tabs staying on the left. Not
    /// when there is a split already, nor when nothing would be left.
    fn split_off(&mut self, tab: &T, from: Option<bool>) -> bool {
        let left = self.keys.iter().filter(|t| *t != tab).count();
        if self.split || (from.is_some() && left == 0) || (from.is_none() && self.keys.is_empty()) {
            return false;
        }
        if from.is_some() {
            self.take(false, tab);
        }
        self.other = std::mem::take(&mut self.keys);
        self.other_front = self.keys_front.take();
        self.keys = vec![tab.clone()];
        self.keys_front = Some(0);
        self.split = true;
        self.focus = 1;
        true
    }

    /// `tab` out of a group, the tab in front moving as a close moves it:
    /// the next one along, or the last.
    fn take(&mut self, other: bool, tab: &T) {
        let (list, front) = if other {
            (&mut self.other, &mut self.other_front)
        } else {
            (&mut self.keys, &mut self.keys_front)
        };
        let Some(at) = list.iter().position(|t| t == tab) else {
            return;
        };
        list.remove(at);
        *front = match *front {
            _ if list.is_empty() => None,
            Some(shown) if shown > at => Some(shown - 1),
            Some(shown) if shown == at => Some(at.min(list.len() - 1)),
            shown => shown,
        };
    }

    /// The keys to the other group.
    fn swap(&mut self) {
        std::mem::swap(&mut self.keys, &mut self.other);
        std::mem::swap(&mut self.keys_front, &mut self.other_front);
        self.focus = 1 - self.focus;
    }

    /// A group left with no tabs ends the split: the other's tabs are the
    /// window's, on the left.
    fn settle(&mut self) {
        if !self.split || (!self.keys.is_empty() && !self.other.is_empty()) {
            return;
        }
        if self.keys.is_empty() {
            self.keys = std::mem::take(&mut self.other);
            self.keys_front = self.other_front.take();
        }
        self.other.clear();
        (self.split, self.other_front, self.focus) = (false, None, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window 1000 by 700 with its editor area from 200 across and 40
    /// down, 600 by 500, and strips 36 tall — one group, or split at 300
    /// with the keys on the right.
    fn area(keys: &[f32], other: &[f32], split: bool) -> Area {
        Area {
            window: (1000.0, 700.0),
            panes: Rect {
                x: 200.0,
                y: 40.0,
                w: 600.0,
                h: 500.0,
            },
            left: if split { 300.0 } else { 600.0 },
            strip: 36.0,
            keys: Lane {
                widths: keys.to_vec(),
                shown: if split { 299.0 } else { 560.0 },
                scroll: 0.0,
            },
            other: Lane {
                widths: other.to_vec(),
                shown: 300.0,
                scroll: 0.0,
            },
            split,
            keys_right: split,
        }
    }

    /// A strip's middle, down the window.
    const STRIP: f32 = 58.0;

    #[test]
    fn a_pointer_falls_in_the_gap_before_the_tab_whose_middle_it_has_not_reached() {
        let lane = Lane {
            widths: vec![100.0, 80.0, 120.0],
            shown: 400.0,
            scroll: 0.0,
        };
        assert_eq!(lane.slot(0.0), 0);
        assert_eq!(lane.slot(49.0), 0);
        assert_eq!(lane.slot(51.0), 1);
        assert_eq!(lane.slot(139.0), 1);
        assert_eq!(lane.slot(141.0), 2);
        assert_eq!(lane.slot(240.0), 3);
        assert_eq!(lane.slot(399.0), 3);
    }

    /// Scrolled, a point of the strip is further along the lane; and over
    /// the strip's buttons is the end of the lane that shows, not a tab
    /// hidden under them.
    #[test]
    fn a_scrolled_lane_counts_from_where_it_is_scrolled_to() {
        let lane = Lane {
            widths: vec![100.0; 10],
            shown: 300.0,
            scroll: 450.0,
        };
        assert_eq!(lane.slot(0.0), 5);
        assert_eq!(lane.slot(10.0), 5);
        assert_eq!(lane.slot(120.0), 6);
        assert_eq!(lane.slot(10_000.0), 8);
    }

    #[test]
    fn along_its_own_strip_a_tab_moves_and_either_side_of_itself_is_nowhere() {
        let area = area(&[100.0, 100.0, 100.0, 100.0], &[], false);
        // The first tab, over the third's right half: after the third.
        assert_eq!(
            land(&area, Some((false, 0)), 200.0 + 260.0, STRIP),
            Landing::Strip {
                other: false,
                index: 3
            }
        );
        // Over its own right half, and the next tab's left half.
        assert_eq!(
            land(&area, Some((false, 1)), 200.0 + 160.0, STRIP),
            Landing::Nowhere
        );
        assert_eq!(
            land(&area, Some((false, 1)), 200.0 + 230.0, STRIP),
            Landing::Nowhere
        );
        // Past every tab: after the last.
        assert_eq!(
            land(&area, Some((false, 1)), 200.0 + 500.0, STRIP),
            Landing::Strip {
                other: false,
                index: 4
            }
        );
    }

    #[test]
    fn the_other_groups_strip_and_its_code_take_a_tab_from_this_one() {
        // Split at 300: the other group on the left, the keys on the right.
        let area = area(&[100.0, 100.0], &[100.0, 100.0, 100.0], true);
        assert_eq!(
            land(&area, Some((false, 0)), 200.0 + 120.0, STRIP),
            Landing::Strip {
                other: true,
                index: 1
            }
        );
        assert_eq!(
            land(&area, Some((false, 0)), 300.0, 300.0),
            Landing::Group { other: true }
        );
        // Its own group's code is where it is.
        assert_eq!(
            land(&area, Some((false, 0)), 700.0, 300.0),
            Landing::Nowhere
        );
        // From the other group, the keys group's strip — which begins a
        // pixel past the divider.
        assert_eq!(
            land(&area, Some((true, 2)), 200.0 + 301.0 + 10.0, STRIP),
            Landing::Strip {
                other: false,
                index: 0
            }
        );
    }

    #[test]
    fn the_right_half_of_the_only_groups_code_splits_while_a_tab_is_left() {
        let two = area(&[100.0, 100.0], &[], false);
        assert_eq!(land(&two, Some((false, 1)), 650.0, 300.0), Landing::Split);
        assert_eq!(land(&two, Some((false, 1)), 450.0, 300.0), Landing::Nowhere);
        // The only tab: split off, the left would be empty.
        let one = area(&[100.0], &[], false);
        assert_eq!(land(&one, Some((false, 0)), 650.0, 300.0), Landing::Nowhere);
        // Split already: there is no third group.
        let split = area(&[100.0, 100.0], &[100.0], true);
        assert_eq!(
            land(&split, Some((false, 0)), 790.0, 300.0),
            Landing::Nowhere
        );
    }

    #[test]
    fn beyond_the_edge_is_a_window_of_its_own_and_chrome_is_nowhere() {
        let area = area(&[100.0, 100.0], &[], false);
        assert_eq!(land(&area, Some((false, 0)), -5.0, 300.0), Landing::Out);
        assert_eq!(land(&area, Some((false, 0)), 500.0, 700.0), Landing::Out);
        // The tree beside the editor area, the status bar under it.
        assert_eq!(
            land(&area, Some((false, 0)), 100.0, 300.0),
            Landing::Nowhere
        );
        assert_eq!(
            land(&area, Some((false, 0)), 500.0, 600.0),
            Landing::Nowhere
        );
    }

    /// Out of a window of its own: the strips and the code take it back,
    /// the right half of the only group's code splits — and off every
    /// window it stays where it is.
    #[test]
    fn a_window_of_its_owns_tab_comes_back_by_the_same_ways() {
        let area = area(&[100.0, 100.0], &[], false);
        assert_eq!(
            land(&area, None, 200.0 + 120.0, STRIP),
            Landing::Strip {
                other: false,
                index: 1
            }
        );
        assert_eq!(
            land(&area, None, 300.0, 300.0),
            Landing::Group { other: false }
        );
        assert_eq!(land(&area, None, 650.0, 300.0), Landing::Split);
        assert_eq!(land(&area, None, -50.0, 300.0), Landing::Nowhere);
        // Nothing open: the whole code takes it.
        let empty = Area {
            keys: Lane::default(),
            ..area.clone()
        };
        assert_eq!(
            land(&empty, None, 650.0, 300.0),
            Landing::Group { other: false }
        );
    }

    #[test]
    fn the_marker_sits_on_the_gap_and_the_overlay_on_the_code_it_takes() {
        let area = area(&[100.0, 80.0], &[], false);
        let marker = shown(
            &area,
            Landing::Strip {
                other: false,
                index: 1,
            },
        )
        .marker
        .unwrap();
        assert_eq!(
            (marker.x, marker.y, marker.w, marker.h),
            (299.0, 40.0, 2.0, 36.0)
        );
        // Before the first: not off the lane's start.
        let first = shown(
            &area,
            Landing::Strip {
                other: false,
                index: 0,
            },
        );
        assert_eq!(first.marker.unwrap().x, 200.0);
        let split = shown(&area, Landing::Split).overlay.unwrap();
        assert_eq!(
            split,
            Rect {
                x: 500.0,
                y: 76.0,
                w: 300.0,
                h: 464.0
            }
        );
        let two = self::area(&[100.0], &[100.0], true);
        assert_eq!(
            shown(&two, Landing::Group { other: true }).overlay.unwrap(),
            Rect {
                x: 200.0,
                y: 76.0,
                w: 300.0,
                h: 464.0
            }
        );
        assert_eq!(shown(&area, Landing::Out), Shown::default());
    }

    /// A marker for a gap scrolled out of view is kept at the lane's edge.
    #[test]
    fn a_marker_never_leaves_the_lane() {
        let mut area = area(&[100.0; 10], &[], false);
        area.keys.scroll = 300.0;
        let at = |index| {
            shown(
                &area,
                Landing::Strip {
                    other: false,
                    index,
                },
            )
            .marker
            .unwrap()
            .x
        };
        assert_eq!(at(0), 200.0);
        assert_eq!(at(4), 299.0);
        assert_eq!(at(10), 200.0 + 558.0);
    }

    #[test]
    fn a_lane_scrolls_near_an_end_only_where_it_has_more() {
        let mut area = area(&[100.0; 10], &[], false);
        assert_eq!(edge_scroll(&area, 200.0 + 550.0, STRIP), Some((false, 1)));
        assert_eq!(edge_scroll(&area, 210.0, STRIP), None);
        area.keys.scroll = 440.0;
        assert_eq!(edge_scroll(&area, 210.0, STRIP), Some((false, -1)));
        assert_eq!(edge_scroll(&area, 200.0 + 550.0, STRIP), None);
        // The code under the strip scrolls nothing.
        assert_eq!(edge_scroll(&area, 210.0, 300.0), None);
        // Everything fits: nothing to scroll.
        let fits = self::area(&[100.0, 100.0], &[], false);
        assert_eq!(edge_scroll(&fits, 200.0 + 550.0, STRIP), None);
    }

    fn groups(keys: &[&'static str], other: &[&'static str]) -> Groups<&'static str> {
        Groups {
            keys: keys.to_vec(),
            keys_front: (!keys.is_empty()).then_some(0),
            other: other.to_vec(),
            other_front: (!other.is_empty()).then_some(0),
            split: !other.is_empty(),
            focus: if other.is_empty() { 0 } else { 1 },
        }
    }

    #[test]
    fn a_tab_moved_along_its_strip_lands_in_the_gap_and_comes_to_front() {
        let mut g = groups(&["a", "b", "c", "d"], &[]);
        let to = |index| Landing::Strip {
            other: false,
            index,
        };
        assert!(g.land(&"a", Some(false), to(3)));
        assert_eq!(g.keys, ["b", "c", "a", "d"]);
        assert_eq!(g.keys_front, Some(2));
        assert!(g.land(&"d", Some(false), to(0)));
        assert_eq!(g.keys, ["d", "b", "c", "a"]);
        assert_eq!(g.keys_front, Some(0));
        assert!(g.land(&"b", Some(false), to(4)));
        assert_eq!(g.keys, ["d", "c", "a", "b"]);
        assert_eq!(g.keys_front, Some(3));
    }

    /// Moved to the other group: out of this one, into that one's gap with
    /// the keys, and the group it left keeps a tab in front.
    #[test]
    fn a_tab_moved_to_the_other_group_takes_the_keys_there() {
        let mut g = groups(&["a", "b"], &["x", "y"]);
        g.keys_front = Some(0);
        assert!(g.land(
            &"a",
            Some(false),
            Landing::Strip {
                other: true,
                index: 1
            }
        ));
        assert_eq!(g.keys, ["x", "a", "y"]);
        assert_eq!(g.keys_front, Some(1));
        assert_eq!(g.other, ["b"]);
        assert_eq!(g.other_front, Some(0));
        assert_eq!(g.focus, 0);
        assert!(g.split);
        // Onto the other group's code: after its last tab.
        assert!(g.land(&"y", Some(false), Landing::Group { other: true }));
        assert_eq!(g.keys, ["b", "y"]);
        assert_eq!(g.other, ["x", "a"]);
        assert_eq!(g.focus, 1);
    }

    /// The last tab of a group moved out of it ends the split: never an
    /// empty pane beside the work.
    #[test]
    fn the_last_tab_moved_out_of_a_group_ends_the_split() {
        let mut g = groups(&["a"], &["x", "y"]);
        assert!(g.land(&"a", Some(false), Landing::Group { other: true }));
        assert!(!g.split);
        assert_eq!(g.keys, ["x", "y", "a"]);
        assert_eq!(g.keys_front, Some(2));
        assert!(g.other.is_empty());
        assert_eq!((g.other_front, g.focus), (None, 0));
        // And the other way: the other group's last tab into this one.
        let mut g = groups(&["a", "b"], &["x"]);
        assert!(g.land(
            &"x",
            Some(true),
            Landing::Strip {
                other: false,
                index: 1
            }
        ));
        assert!(!g.split);
        assert_eq!(g.keys, ["a", "x", "b"]);
        assert_eq!(g.keys_front, Some(1));
    }

    /// One file is one tab of a group: moved into a group that has it, the
    /// tab there is the one that moves, and one is left.
    #[test]
    fn a_file_both_groups_have_is_one_tab_of_the_group_it_lands_in() {
        let mut g = groups(&["a", "b"], &["x", "a", "y"]);
        assert!(g.land(
            &"a",
            Some(false),
            Landing::Strip {
                other: true,
                index: 3
            }
        ));
        assert_eq!(g.keys, ["x", "y", "a"]);
        assert_eq!(g.keys_front, Some(2));
        assert_eq!(g.other, ["b"]);
    }

    #[test]
    fn a_split_off_tab_is_alone_on_the_right_with_the_keys() {
        let mut g = groups(&["a", "b", "c"], &[]);
        g.keys_front = Some(1);
        assert!(g.land(&"b", Some(false), Landing::Split));
        assert_eq!(g.keys, ["b"]);
        assert_eq!(g.keys_front, Some(0));
        assert_eq!(g.other, ["a", "c"]);
        assert_eq!(g.other_front, Some(1));
        assert!(g.split);
        assert_eq!(g.focus, 1);
        // Split already, or nothing left on the left: nothing moves.
        let before = g.clone();
        assert!(!g.land(&"a", Some(true), Landing::Split));
        assert_eq!(g, before);
        let mut alone = groups(&["a"], &[]);
        assert!(!alone.land(&"a", Some(false), Landing::Split));
        assert_eq!(alone.keys, ["a"]);
    }

    /// From a window of its own: into a gap, onto the code, split off —
    /// and onto a group that has the file, in front where it is.
    #[test]
    fn a_tab_from_a_window_of_its_own_joins_the_groups() {
        let mut g = groups(&["a", "b"], &[]);
        assert!(g.land(
            &"t",
            None,
            Landing::Strip {
                other: false,
                index: 1
            }
        ));
        assert_eq!(g.keys, ["a", "t", "b"]);
        assert_eq!(g.keys_front, Some(1));
        assert!(g.land(&"b", None, Landing::Group { other: false }));
        assert_eq!(g.keys, ["a", "t", "b"]);
        assert_eq!(g.keys_front, Some(2));
        assert!(g.land(&"u", None, Landing::Split));
        assert_eq!(g.keys, ["u"]);
        assert_eq!(g.other, ["a", "t", "b"]);
        assert_eq!(g.other_front, Some(2));
        assert!(!g.land(&"v", None, Landing::Nowhere));
        assert!(!g.land(&"v", None, Landing::Out));
    }
}
