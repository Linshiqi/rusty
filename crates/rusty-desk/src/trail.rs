//! Where the editor has been, for Alt+Left and Alt+Right: VS Code's
//! navigation history, pure and tested. `app/navigate.rs` tells it where the
//! caret settles and walks it.
//!
//! **One list and a place in it.** The places are in the order they were
//! reached, and the current one is where the caret is now: every settling
//! of the caret either replaces it or adds a new place after it. Back steps
//! to the place before the current one and Forward to the one after; a new
//! place added after stepping back drops everything that was ahead, as a
//! browser's history does.
//!
//! **What is a new place is VS Code's rule** (`EditorSelectionState.
//! justifiesNewStackEntry`): another file always is; in the same file a
//! move of [`FAR`] lines or more is — a click, Ctrl+End, a line gone to, a
//! find — and a shorter one only when it was a jump (a definition, a
//! reference). Anything else, typing and the arrow keys included, replaces
//! the current place, which is how Back from the newest place goes to where
//! the caret was when it left, not where it first arrived. A place on the
//! same line as the current one is the current one, however it was reached,
//! so a place is never recorded twice in a row.
//!
//! **A walk is not a move.** Stepping back opens a file and moves its caret,
//! and that settling is the arrival, not a new place: until the caret is
//! somewhere other than where the walk left it from, nothing is recorded,
//! and the first place it settles in the walked-to file is the place walked
//! to. A file that has to be read first arrives later than a turn, which is
//! why this is a state and not a flag held across one call.
//!
//! **The caret settles once a turn.** `app/navigate.rs` records where it is
//! when the event that moved it is over, not at every move inside it: a file
//! opened at a line shows its document's own caret and then the line, and
//! recorded at each, the first would be a place nobody went to.

/// A place: a file as the project names it (or a path outside it), and a
/// line and a column, from zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    pub path: String,
    pub line: usize,
    pub col: usize,
}

/// How the caret came to settle where it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Any move: a key, a click, a file opened or picked, a find.
    Moved,
    /// A jump to what a name means: a definition, a reference, a call.
    Jumped,
}

/// How many lines a move in one file has to go to be a place of its own —
/// VS Code's `TEXT_EDITOR_SELECTION_THRESHOLD`.
pub const FAR: usize = 10;

/// How many places are kept — VS Code's `MAX_STACK_SIZE`.
pub const KEPT: usize = 50;

#[derive(Debug, Default)]
pub struct Trail {
    places: Vec<Spot>,
    /// The current place's index; meaningless while there are none.
    at: usize,
    /// A step on its way: the place stepped to, and where the caret was
    /// when it was taken.
    walking: Option<(Spot, Option<Spot>)>,
}

impl Trail {
    /// The caret settled at `here`.
    pub fn record(&mut self, here: Spot, why: Why) {
        if let Some((to, from)) = &self.walking {
            if from.as_ref() == Some(&here) {
                // Nothing has arrived yet: a file is still being read.
                return;
            }
            let arrived = here.path == to.path;
            self.walking = None;
            if arrived {
                // Where the walk landed is the place walked to, wherever the
                // text put its caret.
                self.places[self.at] = here;
                return;
            }
        }
        let Some(current) = self.places.get(self.at) else {
            self.places.push(here);
            self.at = 0;
            return;
        };
        let new = current.path != here.path
            || match current.line.abs_diff(here.line) {
                0 => false,
                near if near < FAR => why == Why::Jumped,
                _ => true,
            };
        if !new {
            self.places[self.at] = here;
            return;
        }
        self.places.truncate(self.at + 1);
        self.places.push(here);
        if self.places.len() > KEPT {
            self.places.remove(0);
        }
        self.at = self.places.len() - 1;
    }

    /// Alt+Left from `here`: the place before the current one, which
    /// becomes the current one. Places `reachable` says no to — an
    /// expansion closed since — are dropped on the way.
    pub fn back(&mut self, here: Option<Spot>, reachable: impl Fn(&Spot) -> bool) -> Option<Spot> {
        self.step(here, true, reachable)
    }

    /// Alt+Right from `here`: the place a step back left, if nothing new
    /// was recorded since.
    pub fn forward(
        &mut self,
        here: Option<Spot>,
        reachable: impl Fn(&Spot) -> bool,
    ) -> Option<Spot> {
        self.step(here, false, reachable)
    }

    fn step(
        &mut self,
        here: Option<Spot>,
        back: bool,
        reachable: impl Fn(&Spot) -> bool,
    ) -> Option<Spot> {
        // Where the caret is now is the current place, settled or not.
        if let Some(here) = &here {
            self.record(here.clone(), Why::Moved);
        }
        loop {
            let next = if back {
                self.at.checked_sub(1)?
            } else {
                Some(self.at + 1).filter(|&next| next < self.places.len())?
            };
            if reachable(&self.places[next]) {
                self.at = next;
                let to = self.places[next].clone();
                self.walking = Some((to.clone(), here));
                return Some(to);
            }
            self.places.remove(next);
            if back {
                self.at -= 1;
            }
        }
    }

    /// The place the caret is at, or was last at in a file.
    pub fn current(&self) -> Option<&Spot> {
        self.places.get(self.at)
    }

    /// Lines `first..first + removed` of `path` became `first..first +
    /// added`: the places below moved with their lines, and one inside
    /// what changed stays inside it.
    pub fn follow(&mut self, path: &str, first: usize, removed: usize, added: usize) {
        let spots = self
            .places
            .iter_mut()
            .chain(self.walking.iter_mut().map(|(to, _)| to));
        for spot in spots.filter(|spot| spot.path == path) {
            if spot.line >= first + removed {
                spot.line = spot.line + added - removed;
            } else if spot.line >= first {
                spot.line = spot.line.min(first + added.saturating_sub(1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, line: usize) -> Spot {
        Spot {
            path: path.into(),
            line,
            col: 0,
        }
    }

    fn anywhere(_: &Spot) -> bool {
        true
    }

    /// Walk back as far as the trail goes, arriving each time, and say
    /// where each step landed.
    fn walked_back(trail: &mut Trail, mut here: Spot) -> Vec<Spot> {
        let mut seen = Vec::new();
        while let Some(to) = trail.back(Some(here.clone()), anywhere) {
            trail.record(to.clone(), Why::Moved);
            seen.push(to.clone());
            here = to;
        }
        seen
    }

    /// Another file is a place; a move of ten lines or more in the same
    /// file is; anything shorter replaces where the caret was.
    #[test]
    fn what_is_a_place_is_vs_codes_rule() {
        let mut trail = Trail::default();
        trail.record(at("a.rs", 0), Why::Moved);
        trail.record(at("a.rs", 3), Why::Moved);
        trail.record(at("a.rs", 9), Why::Moved);
        trail.record(at("a.rs", 19), Why::Moved);
        trail.record(at("b.rs", 19), Why::Moved);
        assert_eq!(
            walked_back(&mut trail, at("b.rs", 19)),
            [at("a.rs", 19), at("a.rs", 9)],
            "a.rs:0 moved to a.rs:9 a few lines at a time, and a.rs:19 is a \
             place of its own — each move is measured from the last"
        );
    }

    /// A jump is a place however near it lands, unless it is the same line.
    #[test]
    fn a_jump_is_a_place_even_nearby() {
        let mut trail = Trail::default();
        trail.record(at("a.rs", 10), Why::Moved);
        trail.record(at("a.rs", 12), Why::Jumped);
        trail.record(at("a.rs", 12), Why::Jumped);
        assert_eq!(walked_back(&mut trail, at("a.rs", 12)), [at("a.rs", 10)]);
    }

    /// Back from the newest place is where the caret was when it left, and
    /// Forward undoes Back — to where the caret was there too.
    #[test]
    fn back_returns_to_where_you_were_and_forward_undoes_it() {
        let mut trail = Trail::default();
        trail.record(at("a.rs", 0), Why::Moved);
        trail.record(at("a.rs", 4), Why::Moved);
        trail.record(at("b.rs", 30), Why::Jumped);
        trail.record(at("b.rs", 33), Why::Moved);

        let back = trail.back(Some(at("b.rs", 35)), anywhere);
        assert_eq!(back, Some(at("a.rs", 4)), "where the caret left a.rs");
        trail.record(at("a.rs", 4), Why::Moved);
        assert_eq!(trail.back(Some(at("a.rs", 4)), anywhere), None);

        let forward = trail.forward(Some(at("a.rs", 4)), anywhere);
        assert_eq!(forward, Some(at("b.rs", 35)), "where the caret was in b.rs");
        trail.record(at("b.rs", 35), Why::Moved);
        assert_eq!(trail.forward(Some(at("b.rs", 35)), anywhere), None);
    }

    /// A place recorded after stepping back drops what was ahead.
    #[test]
    fn a_new_place_after_back_clears_forward() {
        let mut trail = Trail::default();
        for path in ["a.rs", "b.rs", "c.rs"] {
            trail.record(at(path, 0), Why::Moved);
        }
        assert_eq!(
            trail.back(Some(at("c.rs", 0)), anywhere),
            Some(at("b.rs", 0))
        );
        trail.record(at("b.rs", 0), Why::Moved);
        trail.record(at("d.rs", 0), Why::Moved);
        assert_eq!(trail.forward(Some(at("d.rs", 0)), anywhere), None);
        assert_eq!(
            walked_back(&mut trail, at("d.rs", 0)),
            [at("b.rs", 0), at("a.rs", 0)]
        );
    }

    /// Records of the same place in a row are one place: picking the tab
    /// that is already in front, a caret put back where it was.
    #[test]
    fn the_same_place_twice_is_one_place() {
        let mut trail = Trail::default();
        trail.record(at("a.rs", 0), Why::Moved);
        trail.record(at("b.rs", 5), Why::Moved);
        trail.record(at("b.rs", 5), Why::Moved);
        trail.record(at("b.rs", 5), Why::Jumped);
        assert_eq!(walked_back(&mut trail, at("b.rs", 5)), [at("a.rs", 0)]);
    }

    /// The caret still where it was while the walked-to file is read is not
    /// a place, and the arrival is the place walked to — even where the
    /// file, edited on disk since, put the caret ten lines away.
    #[test]
    fn arriving_is_not_a_move() {
        let mut trail = Trail::default();
        trail.record(at("a.rs", 40), Why::Moved);
        trail.record(at("b.rs", 0), Why::Moved);
        let here = at("b.rs", 0);
        assert_eq!(
            trail.back(Some(here.clone()), anywhere),
            Some(at("a.rs", 40))
        );
        trail.record(here, Why::Moved);
        trail.record(at("a.rs", 25), Why::Moved);
        assert_eq!(trail.current(), Some(&at("a.rs", 25)));
        assert_eq!(
            trail.forward(Some(at("a.rs", 25)), anywhere),
            Some(at("b.rs", 0)),
            "the arrival dropped nothing ahead"
        );
    }

    /// Two steps before the first has arrived go two places back.
    #[test]
    fn steps_taken_before_arriving_add_up() {
        let mut trail = Trail::default();
        for path in ["a.rs", "b.rs", "c.rs"] {
            trail.record(at(path, 0), Why::Moved);
        }
        let here = at("c.rs", 0);
        assert_eq!(
            trail.back(Some(here.clone()), anywhere),
            Some(at("b.rs", 0))
        );
        assert_eq!(trail.back(Some(here), anywhere), Some(at("a.rs", 0)));
    }

    /// A place nobody can go back to any more is dropped on the way, in
    /// either direction.
    #[test]
    fn unreachable_places_are_passed_over_and_forgotten() {
        let mut trail = Trail::default();
        for path in ["a.rs", "gone", "b.rs", "gone", "c.rs"] {
            trail.record(at(path, 0), Why::Moved);
        }
        let open = |spot: &Spot| spot.path != "gone";
        assert_eq!(trail.back(Some(at("c.rs", 0)), open), Some(at("b.rs", 0)));
        trail.record(at("b.rs", 0), Why::Moved);
        assert_eq!(trail.back(Some(at("b.rs", 0)), open), Some(at("a.rs", 0)));
        trail.record(at("a.rs", 0), Why::Moved);
        assert_eq!(
            trail.forward(Some(at("a.rs", 0)), open),
            Some(at("b.rs", 0))
        );
        trail.record(at("b.rs", 0), Why::Moved);
        assert_eq!(
            trail.forward(Some(at("b.rs", 0)), open),
            Some(at("c.rs", 0))
        );
    }

    /// Lines added above a place move it down; taken away, up; a place on a
    /// line an edit replaced stays inside what replaced it; another file's
    /// places stay put.
    #[test]
    fn places_follow_the_edits() {
        let mut trail = Trail::default();
        for spot in [
            at("a.rs", 2),
            at("a.rs", 30),
            at("b.rs", 30),
            at("a.rs", 60),
        ] {
            trail.record(spot, Why::Moved);
        }
        // Lines 10..15 of a.rs became 10..12: three lines gone.
        trail.follow("a.rs", 10, 5, 2);
        trail.follow("a.rs", 50, 12, 1);
        assert_eq!(
            walked_back(&mut trail, at("a.rs", 50)),
            [at("b.rs", 30), at("a.rs", 27), at("a.rs", 2)]
        );
    }

    /// Only the newest places are kept.
    #[test]
    fn the_oldest_places_are_let_go() {
        let mut trail = Trail::default();
        for n in 0..KEPT + 10 {
            trail.record(at(&format!("{n}.rs"), 0), Why::Moved);
        }
        let back = walked_back(&mut trail, at(&format!("{}.rs", KEPT + 9), 0));
        assert_eq!(back.len(), KEPT - 1);
        assert_eq!(back.last(), Some(&at("10.rs", 0)));
    }
}
