//! Lanes for the commit graph.
//!
//! The input is the log in topological order — every commit before any of its
//! parents, which is what `git log --topo-order` promises and what the
//! algorithm relies on. Each lane holds the hash it is waiting to see next;
//! a commit takes the lane that was waiting for it, hands the lane on to its
//! first parent, and opens a new lane for every other parent. A lane whose
//! commit turns out to be one another lane also reached has converged, and
//! is closed at that row.
//!
//! Computed here, on the backend, rather than in the view: the frontend
//! draws rows and lines and never decides where they go, so the lane a commit
//! sits in is one fact rather than two opinions. The whole thing is pure and
//! the shapes that go wrong — a merge, two branches converging on one
//! commit, a root — are each a test.

use crate::model::{Commit, Edge, GraphRow, History};

/// Lay the log out. `rows.len() == commits.len()`, in the same order.
pub fn lay_out(commits: Vec<Commit>) -> History {
    // What each lane is waiting for. `None` is a free lane.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows: Vec<GraphRow> = Vec::with_capacity(commits.len());
    let mut widest = 0u32;

    for commit in commits {
        // The lane waiting for this commit, or a free one, or a new one.
        let lane = match lanes
            .iter()
            .position(|slot| slot.as_deref() == Some(&commit.id))
        {
            Some(at) => at,
            None => match lanes.iter().position(Option::is_none) {
                Some(free) => free,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            },
        };

        // Every other lane that was also waiting for this commit has
        // converged here. Its line, drawn from the row above, was aimed at
        // its own lane; redirect it into this dot and close it.
        let converging: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(at, slot)| *at != lane && slot.as_deref() == Some(&commit.id))
            .map(|(at, _)| at)
            .collect();
        for at in &converging {
            lanes[*at] = None;
            if let Some(previous) = rows.last_mut()
                && let Some(edge) = previous.edges.iter_mut().find(|e| e.to == *at as u32)
            {
                edge.to = lane as u32;
            }
        }

        // Which lanes were already running before this row touched them.
        // Only those pass *through* the row; a lane this commit opens for a
        // parent starts at the dot and has no line arriving from above.
        let running_before: Vec<bool> = lanes.iter().map(Option::is_some).collect();

        let mut edges = Vec::new();
        let mut parents = commit.parents.iter();

        // First parent inherits this lane — unless another lane is already
        // waiting for it, in which case this line joins that one below.
        match parents.next() {
            None => lanes[lane] = None,
            Some(first) => match lanes
                .iter()
                .position(|slot| slot.as_deref() == Some(first.as_str()))
            {
                Some(other) if other != lane => {
                    lanes[lane] = None;
                    edges.push(Edge {
                        from: lane as u32,
                        to: other as u32,
                    });
                }
                _ => {
                    lanes[lane] = Some(first.clone());
                    edges.push(Edge {
                        from: lane as u32,
                        to: lane as u32,
                    });
                }
            },
        }

        // Every further parent is a line leaving this dot: into the lane
        // already waiting for it, or into a new one.
        for parent in parents {
            let to = match lanes
                .iter()
                .position(|slot| slot.as_deref() == Some(parent.as_str()))
            {
                Some(at) => at,
                None => {
                    let at = lanes.iter().position(Option::is_none).unwrap_or_else(|| {
                        lanes.push(None);
                        lanes.len() - 1
                    });
                    lanes[at] = Some(parent.clone());
                    at
                }
            };
            edges.push(Edge {
                from: lane as u32,
                to: to as u32,
            });
        }

        // Every lane that was running before this row and still is runs
        // straight through it — including one a line from this dot joins
        // below, which is why this is not "lanes no edge names".
        for (at, slot) in lanes.iter().enumerate() {
            if at != lane && slot.is_some() && running_before.get(at).copied().unwrap_or(false) {
                let to = at as u32;
                edges.push(Edge { from: to, to });
            }
        }

        widest = widest.max(lanes.len() as u32);
        rows.push(GraphRow {
            commit,
            lane: lane as u32,
            edges,
        });
    }

    History {
        rows,
        lanes: widest.max(1),
        truncated: false,
        head: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.to_string(),
            short: id.chars().take(7).collect(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: "a".into(),
            email: "a@x".into(),
            time: 0,
            summary: id.to_string(),
            refs: Vec::new(),
        }
    }

    fn lanes_of(history: &History) -> Vec<u32> {
        history.rows.iter().map(|r| r.lane).collect()
    }

    /// Three commits in a line: one lane, every dot in it, one line down
    /// out of each but the root.
    #[test]
    fn a_linear_history_is_one_lane() {
        let h = lay_out(vec![
            commit("c", &["b"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ]);
        assert_eq!(lanes_of(&h), vec![0, 0, 0]);
        assert_eq!(h.lanes, 1);
        assert_eq!(h.rows[0].edges, vec![Edge { from: 0, to: 0 }]);
        assert_eq!(h.rows[2].edges, vec![], "a root has nothing below it");
    }

    /// A merge opens a second lane for its second parent; the two branches
    /// sit side by side; they converge on their common ancestor, and the
    /// line from the second branch is redirected into that ancestor's dot.
    #[test]
    fn a_merge_opens_a_lane_and_the_branches_converge_on_their_ancestor() {
        let h = lay_out(vec![
            commit("m", &["a", "b"]),
            commit("a", &["c"]),
            commit("b", &["c"]),
            commit("c", &[]),
        ]);
        assert_eq!(lanes_of(&h), vec![0, 0, 1, 0]);
        assert_eq!(h.lanes, 2);
        // The merge: one line straight down to `a`, one out to lane 1 for `b`.
        assert_eq!(
            h.rows[0].edges,
            vec![Edge { from: 0, to: 0 }, Edge { from: 0, to: 1 }]
        );
        // `a`'s row: its own line down, and lane 1 passing through.
        assert!(h.rows[1].edges.contains(&Edge { from: 0, to: 0 }));
        assert!(h.rows[1].edges.contains(&Edge { from: 1, to: 1 }));
        // `b`'s row: its line was aimed at lane 1 and is redirected into
        // `c`, which sits in lane 0 — while `a`'s lane runs on through.
        assert!(h.rows[2].edges.contains(&Edge { from: 1, to: 0 }));
        assert!(h.rows[2].edges.contains(&Edge { from: 0, to: 0 }));
        assert_eq!(h.rows[2].edges.len(), 2);
    }

    /// A branch tip that is not the newest commit starts its own lane at its
    /// own row rather than being drawn as if it descended from the row above.
    #[test]
    fn a_second_tip_takes_a_free_lane_where_it_appears() {
        let h = lay_out(vec![
            commit("x", &["a"]),
            commit("y", &["a"]),
            commit("a", &[]),
        ]);
        assert_eq!(lanes_of(&h), vec![0, 1, 0]);
        // `x` goes down its own lane; `y`, having no lane waiting for it,
        // takes lane 1 and joins `a` below, with `x`'s lane passing by.
        assert!(h.rows[1].edges.contains(&Edge { from: 1, to: 0 }));
        assert!(h.rows[1].edges.contains(&Edge { from: 0, to: 0 }));
        assert_eq!(h.rows[1].edges.len(), 2);
    }

    /// The first parent joins a lane already waiting for it — the classic
    /// "merged into main" shape, where the feature line bends back in.
    #[test]
    fn a_first_parent_already_waited_for_bends_into_that_lane() {
        let h = lay_out(vec![
            commit("m", &["a", "f"]),
            commit("f", &["a"]),
            commit("a", &[]),
        ]);
        assert_eq!(lanes_of(&h), vec![0, 1, 0]);
        assert!(h.rows[1].edges.contains(&Edge { from: 1, to: 0 }));
        assert!(h.rows[1].edges.contains(&Edge { from: 0, to: 0 }));
        // And the merge itself opened lane 1 at its own row: no line arrives
        // at that dot from above, so no `1 → 1` is drawn there.
        assert_eq!(
            h.rows[0].edges,
            vec![Edge { from: 0, to: 0 }, Edge { from: 0, to: 1 }]
        );
    }

    /// Lanes are reused once free, so a long history with short-lived
    /// branches does not grow a column per branch.
    #[test]
    fn a_closed_lane_is_reused() {
        let h = lay_out(vec![
            commit("m2", &["m1", "g"]),
            commit("g", &["m1"]),
            commit("m1", &["m0", "f"]),
            commit("f", &["m0"]),
            commit("m0", &[]),
        ]);
        assert_eq!(h.lanes, 2, "two branches in turn need two lanes, not three");
        assert_eq!(lanes_of(&h), vec![0, 1, 0, 1, 0]);
    }
}
