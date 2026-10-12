//! A call hierarchy as the dock's Calls tab draws it (`view/dock/calls.rs`):
//! the function asked about, and the calls into or out of it a level at a
//! time, each level asked of the server when its row is opened.
//!
//! A flat list of rows rather than a tree of nodes: a row's calls follow it
//! one level deeper, so drawing is one pass, closing a row is removing the
//! run below it, and a row is found by the number it was given, not by where
//! it is — an answer lands on the row that asked even when rows above it
//! were opened or closed while it was on the way. Pure, so what opening and
//! closing do is under tests.

use rusty_lsp::{Call, CallItem};

/// Whether a row's own calls are showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Openness {
    Closed,
    /// Asked for and not answered yet.
    Asking,
    Open,
}

/// One function in the hierarchy, and the calls that brought it there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRow {
    /// Its number in this tree, never reused.
    pub id: u64,
    /// 0 for the function asked about.
    pub depth: u32,
    /// The function, and where the calls between it and the row above are.
    /// The first row's has no sites: nothing above it called anything.
    pub call: Call,
    pub open: Openness,
}

/// Who calls a function, or what it calls, as far down as anybody opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallTree {
    /// Which ask made this tree. An answer carries the number of the tree it
    /// was asked for, so one landing after another hierarchy was started is
    /// dropped rather than put in a tree that never asked.
    pub serial: u64,
    /// Calls in — who calls it — or calls out — what it calls.
    pub incoming: bool,
    /// In the order drawn; the first is the function asked about.
    pub rows: Vec<CallRow>,
    next: u64,
}

impl CallTree {
    /// A tree holding only the function asked about, not yet opened.
    pub fn new(serial: u64, root: CallItem, incoming: bool) -> CallTree {
        CallTree {
            serial,
            incoming,
            rows: vec![CallRow {
                id: 0,
                depth: 0,
                call: Call {
                    item: root,
                    sites: Vec::new(),
                },
                open: Openness::Closed,
            }],
            next: 1,
        }
    }

    /// The function the tree is about.
    pub fn root(&self) -> &CallItem {
        &self.rows[0].call.item
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.rows.iter().position(|row| row.id == id)
    }

    /// Start opening a row. The server's description of its function comes
    /// back to be asked about, or nothing when there is nothing to ask — the
    /// row is gone, already open, or already being asked about.
    pub fn open(&mut self, id: u64) -> Option<String> {
        let at = self.index_of(id)?;
        let row = &mut self.rows[at];
        if row.open != Openness::Closed {
            return None;
        }
        row.open = Openness::Asking;
        Some(row.call.item.item.clone())
    }

    /// The calls a row asked for, put under it — sorted by where each call
    /// is made, which for calls out of one function is the order it makes
    /// them in. Dropped when the row went, or was closed while they were on
    /// the way: an answer opening a row somebody had just closed would be
    /// the row reopening itself.
    pub fn answer(&mut self, id: u64, mut calls: Vec<Call>) {
        let Some(at) = self.index_of(id) else {
            return;
        };
        if self.rows[at].open != Openness::Asking {
            return;
        }
        calls.sort_by_key(first_site);
        let depth = self.rows[at].depth + 1;
        let rows: Vec<CallRow> = calls
            .into_iter()
            .enumerate()
            .map(|(n, call)| CallRow {
                id: self.next + n as u64,
                depth,
                call,
                open: Openness::Closed,
            })
            .collect();
        self.next += rows.len() as u64;
        self.rows[at].open = Openness::Open;
        self.rows.splice(at + 1..at + 1, rows);
    }

    /// An ask that failed: the row is closed again, to be opened again.
    pub fn failed(&mut self, id: u64) {
        if let Some(at) = self.index_of(id)
            && self.rows[at].open == Openness::Asking
        {
            self.rows[at].open = Openness::Closed;
        }
    }

    /// Close a row: everything below it that is deeper goes.
    pub fn close(&mut self, id: u64) {
        let Some(at) = self.index_of(id) else {
            return;
        };
        let depth = self.rows[at].depth;
        let end = self.rows[at + 1..]
            .iter()
            .position(|row| row.depth <= depth)
            .map_or(self.rows.len(), |n| at + 1 + n);
        self.rows.drain(at + 1..end);
        self.rows[at].open = Openness::Closed;
    }

    /// Whether the row at `index` is a function already open above it on its
    /// own branch — a recursion, which opens as deep as anybody cares to
    /// click, and says so rather than looking like new callers.
    pub fn recursive(&self, index: usize) -> bool {
        let Some(row) = self.rows.get(index) else {
            return false;
        };
        let mut depth = row.depth;
        for above in self.rows[..index].iter().rev() {
            if above.depth < depth {
                if same_function(&above.call.item, &row.call.item) {
                    return true;
                }
                depth = above.depth;
            }
        }
        false
    }
}

/// Where a call is first made: the order rows are drawn in.
fn first_site(call: &Call) -> (bool, String, u32, u32) {
    let at = call
        .sites
        .first()
        .map_or(&call.item.place.location, |site| &site.location);
    (at.external, at.path.clone(), at.line, at.col)
}

/// Two items name one function when they are the same name in the same
/// place — the server's descriptions of it may differ in what they carry.
fn same_function(a: &CallItem, b: &CallItem) -> bool {
    a.name == b.name && a.place.location == b.place.location
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_lsp::{Location, Place};

    fn place(path: &str, line: u32) -> Place {
        Place {
            location: Location {
                path: path.to_string(),
                line,
                col: 4,
                external: false,
            },
            end_col: 8,
            text: String::new(),
        }
    }

    fn item(name: &str, path: &str, line: u32) -> CallItem {
        CallItem {
            name: name.to_string(),
            kind: "function".to_string(),
            detail: None,
            place: place(path, line),
            item: format!("{{\"name\":\"{name}\"}}"),
        }
    }

    fn call(name: &str, path: &str, line: u32, at: u32) -> Call {
        Call {
            item: item(name, path, line),
            sites: vec![place(path, at)],
        }
    }

    /// One line per row: indentation for depth, then the name.
    fn drawn(tree: &CallTree) -> Vec<String> {
        tree.rows
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth as usize), row.call.item.name))
            .collect()
    }

    fn tree() -> CallTree {
        CallTree::new(7, item("gain", "src/lib.rs", 0), true)
    }

    #[test]
    fn opening_a_row_asks_once_and_puts_its_calls_under_it() {
        let mut tree = tree();
        assert_eq!(tree.open(0).as_deref(), Some("{\"name\":\"gain\"}"));
        assert_eq!(tree.open(0), None, "already being asked");
        // The server's order is not the drawn one: by where each call is.
        tree.answer(
            0,
            vec![
                call("main", "src/main.rs", 0, 2),
                call("step", "src/lib.rs", 4, 6),
            ],
        );
        assert_eq!(drawn(&tree), ["gain", "  step", "  main"]);
        assert_eq!(tree.rows[0].open, Openness::Open);
        assert_eq!(tree.open(0), None, "already open");
    }

    /// An answer finds the row that asked by its number, wherever it is now.
    #[test]
    fn an_answer_lands_on_its_row_after_the_rows_above_it_moved() {
        let mut tree = tree();
        tree.open(0);
        tree.answer(
            0,
            vec![call("a", "src/a.rs", 0, 1), call("b", "src/b.rs", 0, 1)],
        );
        let b = tree.rows[2].id;
        let a = tree.rows[1].id;
        assert!(tree.open(b).is_some());
        // `a` opens above `b` while `b`'s answer is on the way.
        tree.open(a);
        tree.answer(a, vec![call("a1", "src/a.rs", 9, 10)]);
        tree.answer(b, vec![call("b1", "src/b.rs", 9, 10)]);
        assert_eq!(drawn(&tree), ["gain", "  a", "    a1", "  b", "    b1"]);
    }

    #[test]
    fn closing_a_row_takes_everything_under_it_and_nothing_beside_it() {
        let mut tree = tree();
        tree.open(0);
        tree.answer(
            0,
            vec![call("a", "src/a.rs", 0, 1), call("b", "src/b.rs", 0, 1)],
        );
        let a = tree.rows[1].id;
        tree.open(a);
        tree.answer(a, vec![call("a1", "src/a.rs", 9, 10)]);
        let a1 = tree.rows[2].id;
        tree.open(a1);
        tree.answer(a1, vec![call("a2", "src/a.rs", 20, 21)]);
        assert_eq!(drawn(&tree), ["gain", "  a", "    a1", "      a2", "  b"]);
        tree.close(a);
        assert_eq!(drawn(&tree), ["gain", "  a", "  b"]);
        assert_eq!(tree.rows[1].open, Openness::Closed);
        // Reopened, it asks again: what it held is gone.
        assert!(tree.open(a).is_some());
    }

    /// Closed while its answer was on the way, a row stays closed; an ask
    /// that failed can be made again.
    #[test]
    fn a_late_answer_does_not_reopen_a_closed_row() {
        let mut tree = tree();
        tree.open(0);
        tree.close(0);
        tree.answer(0, vec![call("main", "src/main.rs", 0, 2)]);
        assert_eq!(drawn(&tree), ["gain"]);
        assert_eq!(tree.rows[0].open, Openness::Closed);

        tree.open(0);
        tree.failed(0);
        assert_eq!(tree.rows[0].open, Openness::Closed);
        assert!(tree.open(0).is_some());
        tree.answer(99, vec![call("x", "src/x.rs", 0, 0)]);
        assert_eq!(drawn(&tree), ["gain"], "no row 99");
    }

    /// A function that calls itself turns up under itself, and says so; a
    /// function of the same name elsewhere is not a recursion.
    #[test]
    fn a_function_under_itself_is_a_recursion() {
        let mut tree = tree();
        tree.open(0);
        tree.answer(
            0,
            vec![
                call("gain", "src/lib.rs", 0, 3),
                call("step", "src/lib.rs", 5, 6),
            ],
        );
        let step = tree.rows[2].id;
        tree.open(step);
        tree.answer(step, vec![call("gain", "src/other.rs", 0, 7)]);
        assert_eq!(drawn(&tree), ["gain", "  gain", "  step", "    gain"]);
        assert!(!tree.recursive(0));
        assert!(tree.recursive(1));
        assert!(!tree.recursive(2));
        assert!(!tree.recursive(3), "another file's gain");
    }
}
