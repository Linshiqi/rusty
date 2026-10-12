//! One union-find for every question here of the form "which of these are
//! joined": the nets a sheet's wires make, the nodes a circuit reaches
//! ground through, and the nets a KiCad schematic draws.
//!
//! Joining hangs the first set's root under the second's, so which member
//! stands for a set is decided by the order of the joins alone — a caller
//! that names a net by its root gets the same name whatever the lookups in
//! between did to the paths.

#[derive(Debug, Clone, Default)]
pub(crate) struct UnionFind(Vec<usize>);

impl UnionFind {
    /// `n` elements, each on its own.
    pub(crate) fn new(n: usize) -> Self {
        UnionFind((0..n).collect())
    }

    /// One more element, on its own, and its index — for the KiCad readers,
    /// which meet their points as they go.
    #[cfg(feature = "backend")]
    pub(crate) fn push(&mut self) -> usize {
        let index = self.0.len();
        self.0.push(index);
        index
    }

    /// The element standing for `i`'s set.
    pub(crate) fn find(&mut self, i: usize) -> usize {
        let mut root = i;
        while self.0[root] != root {
            root = self.0[root];
        }
        let mut at = i;
        while self.0[at] != root {
            let next = self.0[at];
            self.0[at] = root;
            at = next;
        }
        root
    }

    /// Join the sets of `a` and `b`.
    pub(crate) fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a] = b;
        }
    }
}
