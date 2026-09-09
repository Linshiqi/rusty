//! Kirchhoff, by modified nodal analysis.
//!
//! Stage 4 of `docs/kicad.md`, where the decision to write this rather than
//! bundle ngspice is argued. The short of it: stage 5 needs a solver that
//! can be *stepped* from inside this process at the firmware's own
//! timescale, and that is a different shape from "here is a netlist and a
//! duration". What this gives up is device models, and what it buys is that
//! every answer can be checked against one somebody worked out by hand.
//!
//! **Node zero is ground**, always, and every other node's voltage is
//! relative to it. MNA writes one equation per node — the currents into it
//! sum to zero, which is Kirchhoff's current law — and one more per voltage
//! source, saying what it insists on. Together:
//!
//! ```text
//! ┌       ┐ ┌   ┐   ┌   ┐
//! │ G   B │ │ v │ = │ i │      G  conductances between nodes
//! │ Bᵀ  0 │ │ j │   │ e │      B  which nodes each source is across
//! └       ┘ └   ┘   └   ┘      v  node voltages, j source currents
//! ```
//!
//! **What it refuses.** A node with no resistive path to ground has no
//! voltage — not zero, not anything — and saying so beats returning a
//! number nobody can check. Two sources insisting on different voltages
//! across one pair of nodes is a contradiction, not a circuit. Both come
//! back as [`Trouble`] naming what and where, because a solver that guesses
//! at an operating point is the worst kind of wrong here: fluent, specific,
//! and unfalsifiable by looking.

use std::collections::BTreeMap;

/// One element of a circuit, in the only three shapes the first stage needs.
///
/// A closed switch is a **zero-volt source** and not a zero-ohm resistor:
/// the conductance of the second is infinite and the first is exact. That
/// is not a trick, it is what a short is — two nodes the same voltage apart
/// carrying whatever current the rest of the circuit asks for, which is
/// precisely what a voltage source's extra equation says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Element {
    /// `ohms` between two nodes. Must be positive; a short is [`Element::Short`].
    Resistor { a: usize, b: usize, ohms: f64 },
    /// `volts` at `plus` relative to `minus`.
    Source {
        plus: usize,
        minus: usize,
        volts: f64,
    },
    /// Two nodes held together — a closed switch, a wire drawn as a part.
    Short { a: usize, b: usize },
    /// `amps` pushed *out of* `from` and *into* `into`.
    Current { from: usize, into: usize, amps: f64 },
}

impl Element {
    /// The two nodes it touches.
    fn ends(&self) -> (usize, usize) {
        match *self {
            Element::Resistor { a, b, .. } | Element::Short { a, b } => (a, b),
            Element::Source { plus, minus, .. } => (plus, minus),
            Element::Current { from, into, .. } => (from, into),
        }
    }

    /// Does current flow through it at DC, whatever the voltage across it?
    ///
    /// A current source does not: it insists on a current and says nothing
    /// about the voltage, so a node hanging off one alone still has no
    /// voltage of its own. That distinction is the whole of why the
    /// grounded check below reads this and not `ends`.
    fn conducts(&self) -> bool {
        !matches!(self, Element::Current { .. })
    }
}

/// A circuit: how many nodes, and what is between them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Circuit {
    /// Node 0 is ground; `nodes` counts it.
    pub nodes: usize,
    pub elements: Vec<Element>,
}

/// What the solver found.
#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    /// One volt figure per node, ground included and always zero.
    pub volts: Vec<f64>,
    /// The current through each voltage source and short, in the order they
    /// appear in `elements` — positive out of `plus`. MNA computes these
    /// as a by-product, and they are the answer to "how much is this
    /// drawing", which is the question a series resistor exists to settle.
    pub through: BTreeMap<usize, f64>,
}

impl Solution {
    pub fn volts_at(&self, node: usize) -> f64 {
        self.volts.get(node).copied().unwrap_or(0.0)
    }
}

/// Why there is no answer, in terms a caller can put on a sheet.
#[derive(Debug, Clone, PartialEq)]
pub enum Trouble {
    /// A node with no conducting path to ground. Its voltage is not zero
    /// and not anything — it is a question the circuit does not answer.
    Floating { node: usize },
    /// The equations contradict each other: two sources across one pair of
    /// nodes insisting on different voltages, or a loop of them.
    Contradiction,
    /// A resistance that is not a positive number of ohms.
    BadResistance { ohms: f64 },
}

impl std::fmt::Display for Trouble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Trouble::Floating { node } => write!(
                f,
                "node {node} reaches ground through nothing, so it has no voltage to report"
            ),
            Trouble::Contradiction => {
                f.write_str("two sources insist on different voltages across the same nodes")
            }
            Trouble::BadResistance { ohms } => {
                write!(
                    f,
                    "{ohms} is not a resistance a current can be worked out through"
                )
            }
        }
    }
}

impl std::error::Error for Trouble {}

/// Solve for the DC operating point.
pub fn dc(circuit: &Circuit) -> Result<Solution, Trouble> {
    if circuit.nodes == 0 {
        return Ok(Solution {
            volts: Vec::new(),
            through: BTreeMap::new(),
        });
    }
    grounded(circuit)?;

    // Every source and short gets an equation of its own, and a current to
    // solve for. Their order here is the order of their currents in the
    // unknown vector, and `through` is keyed by their index in `elements`
    // so a caller can ask about the one it placed.
    let extras: Vec<usize> = circuit
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Element::Source { .. } | Element::Short { .. }))
        .map(|(at, _)| at)
        .collect();

    let n = circuit.nodes - 1; // ground is not an unknown
    let size = n + extras.len();
    let mut a = vec![vec![0.0f64; size]; size];
    let mut b = vec![0.0f64; size];
    // Ground has no row and no column: `at` maps a node to its place, or
    // to nothing, which is what makes a stamp against ground a no-op.
    let at = |node: usize| (node != 0).then(|| node - 1);

    for element in &circuit.elements {
        match *element {
            Element::Resistor { a: p, b: m, ohms } => {
                // NaN fails the first test, so the second never sees one.
                if !ohms.is_finite() || ohms <= 0.0 {
                    return Err(Trouble::BadResistance { ohms });
                }
                let g = 1.0 / ohms;
                if let Some(p) = at(p) {
                    a[p][p] += g;
                }
                if let Some(m) = at(m) {
                    a[m][m] += g;
                }
                if let (Some(p), Some(m)) = (at(p), at(m)) {
                    a[p][m] -= g;
                    a[m][p] -= g;
                }
            }
            Element::Current { from, into, amps } => {
                if let Some(from) = at(from) {
                    b[from] -= amps;
                }
                if let Some(into) = at(into) {
                    b[into] += amps;
                }
            }
            Element::Source { .. } | Element::Short { .. } => {}
        }
    }

    for (k, index) in extras.iter().enumerate() {
        let row = n + k;
        let (plus, minus, volts) = match circuit.elements[*index] {
            Element::Source { plus, minus, volts } => (plus, minus, volts),
            Element::Short { a, b } => (a, b, 0.0),
            _ => unreachable!("extras holds only sources and shorts"),
        };
        if let Some(p) = at(plus) {
            a[p][row] += 1.0;
            a[row][p] += 1.0;
        }
        if let Some(m) = at(minus) {
            a[m][row] -= 1.0;
            a[row][m] -= 1.0;
        }
        b[row] = volts;
    }

    let x = gauss(a, b)?;
    let mut volts = vec![0.0; circuit.nodes];
    volts[1..].copy_from_slice(&x[..n]);
    let through = extras
        .iter()
        .enumerate()
        .map(|(k, index)| (*index, x[n + k]))
        .collect();
    Ok(Solution { volts, through })
}

/// Every node has a conducting path to ground, or the first one that does
/// not is named.
fn grounded(circuit: &Circuit) -> Result<(), Trouble> {
    let mut parent: Vec<usize> = (0..circuit.nodes).collect();
    fn find(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    for element in circuit.elements.iter().filter(|e| e.conducts()) {
        let (a, b) = element.ends();
        if a >= circuit.nodes || b >= circuit.nodes {
            continue;
        }
        let (a, b) = (find(&mut parent, a), find(&mut parent, b));
        if a != b {
            parent[a] = b;
        }
    }
    let ground = find(&mut parent, 0);
    for node in 1..circuit.nodes {
        if find(&mut parent, node) != ground {
            return Err(Trouble::Floating { node });
        }
    }
    Ok(())
}

/// Gaussian elimination with partial pivoting.
///
/// Dense on purpose: a sheet has tens of nodes, and a sparse factorisation
/// would be a second thing to get right for no gain anybody could measure.
/// A pivot that is not there is not a numerical wobble here — it is two
/// equations saying different things about the same unknown, which is a
/// contradiction in the drawing and is reported as one.
fn gauss(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Result<Vec<f64>, Trouble> {
    let size = b.len();
    for column in 0..size {
        let pivot = (column..size)
            .max_by(|x, y| a[*x][column].abs().total_cmp(&a[*y][column].abs()))
            .unwrap_or(column);
        if a[pivot][column].abs() < 1e-12 {
            return Err(Trouble::Contradiction);
        }
        a.swap(column, pivot);
        b.swap(column, pivot);
        for row in column + 1..size {
            let factor = a[row][column] / a[column][column];
            if factor == 0.0 {
                continue;
            }
            // `column` is always above `row`, so the split hands out the
            // pivot row and the row being reduced at once.
            let (above, from) = a.split_at_mut(row);
            for (cell, pivot) in from[0][column..].iter_mut().zip(&above[column][column..]) {
                *cell -= factor * pivot;
            }
            b[row] -= factor * b[column];
        }
    }
    let mut x = vec![0.0; size];
    for row in (0..size).rev() {
        let mut sum = b[row];
        for k in row + 1..size {
            sum -= a[row][k] * x[k];
        }
        x[row] = sum / a[row][row];
    }
    Ok(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f64, want: f64) -> bool {
        (got - want).abs() < 1e-9
    }

    /// The answer everybody can check: a divider's ratio, worked out by
    /// hand and asserted exactly. This is the whole reason for writing the
    /// solver rather than driving one — an integration can only be tested
    /// by "it ran and said something".
    #[test]
    fn a_divider_puts_its_midpoint_where_the_ratio_says() {
        // 10 V ── 20k ── mid ── 10k ── gnd
        let circuit = Circuit {
            nodes: 3,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 10.0,
                },
                Element::Resistor {
                    a: 1,
                    b: 2,
                    ohms: 20_000.0,
                },
                Element::Resistor {
                    a: 2,
                    b: 0,
                    ohms: 10_000.0,
                },
            ],
        };
        let found = dc(&circuit).expect("solved");
        assert!(close(found.volts_at(1), 10.0));
        assert!(
            close(found.volts_at(2), 10.0 / 3.0),
            "10k of 30k: {}",
            found.volts_at(2)
        );
        // And what the source is giving: 10 V across 30k.
        assert!(
            close(found.through[&0], -10.0 / 30_000.0),
            "the current out of the source's plus, which flows into it: {}",
            found.through[&0]
        );
    }

    #[test]
    fn resistors_in_parallel_draw_the_sum_of_their_currents() {
        let circuit = Circuit {
            nodes: 2,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 10.0,
                },
                Element::Resistor {
                    a: 1,
                    b: 0,
                    ohms: 10_000.0,
                },
                Element::Resistor {
                    a: 1,
                    b: 0,
                    ohms: 10_000.0,
                },
            ],
        };
        let found = dc(&circuit).expect("solved");
        assert!(close(found.through[&0].abs(), 2e-3), "1 mA each");
    }

    /// A Thévenin equivalent: the same source and series resistance answer
    /// the same way into any load, which is a property rather than a
    /// number and holds for every load at once.
    #[test]
    fn a_source_behind_a_resistor_is_its_thevenin_equivalent() {
        for load in [1_000.0, 4_700.0, 100_000.0] {
            let circuit = Circuit {
                nodes: 3,
                elements: vec![
                    Element::Source {
                        plus: 1,
                        minus: 0,
                        volts: 5.0,
                    },
                    Element::Resistor {
                        a: 1,
                        b: 2,
                        ohms: 1_000.0,
                    },
                    Element::Resistor {
                        a: 2,
                        b: 0,
                        ohms: load,
                    },
                ],
            };
            let found = dc(&circuit).expect("solved");
            let want = 5.0 * load / (1_000.0 + load);
            assert!(
                close(found.volts_at(2), want),
                "load {load}: {} against {want}",
                found.volts_at(2)
            );
        }
    }

    #[test]
    fn a_current_source_makes_its_own_voltage_across_a_resistor() {
        let circuit = Circuit {
            nodes: 2,
            elements: vec![
                Element::Current {
                    from: 0,
                    into: 1,
                    amps: 1e-3,
                },
                Element::Resistor {
                    a: 1,
                    b: 0,
                    ohms: 1_000.0,
                },
            ],
        };
        let found = dc(&circuit).expect("solved");
        assert!(close(found.volts_at(1), 1.0), "{}", found.volts_at(1));
    }

    /// A short is a zero-volt source, so the two nodes are one voltage and
    /// the current through it is whatever the rest of the circuit asks —
    /// which is the thing a zero-ohm resistor cannot express.
    #[test]
    fn a_short_holds_two_nodes_together_and_carries_what_is_asked() {
        let circuit = Circuit {
            nodes: 3,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 5.0,
                },
                Element::Short { a: 1, b: 2 },
                Element::Resistor {
                    a: 2,
                    b: 0,
                    ohms: 1_000.0,
                },
            ],
        };
        let found = dc(&circuit).expect("solved");
        assert!(close(found.volts_at(2), 5.0));
        assert!(
            close(found.through[&1].abs(), 5e-3),
            "5 mA through the short"
        );
    }

    /// The two refusals, which matter as much as the arithmetic: a solver
    /// that answered these would be fluent, specific and wrong.
    #[test]
    fn a_floating_node_and_a_contradiction_are_refused_by_name() {
        let adrift = Circuit {
            nodes: 3,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 5.0,
                },
                // Node 2 hangs off a current source and nothing else.
                Element::Current {
                    from: 0,
                    into: 2,
                    amps: 1e-3,
                },
            ],
        };
        assert_eq!(dc(&adrift), Err(Trouble::Floating { node: 2 }));

        let arguing = Circuit {
            nodes: 2,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 5.0,
                },
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 3.3,
                },
            ],
        };
        assert_eq!(dc(&arguing), Err(Trouble::Contradiction));

        let nonsense = Circuit {
            nodes: 2,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 5.0,
                },
                Element::Resistor {
                    a: 1,
                    b: 0,
                    ohms: 0.0,
                },
            ],
        };
        assert_eq!(dc(&nonsense), Err(Trouble::BadResistance { ohms: 0.0 }));
    }

    /// Two sources that agree are not a contradiction, which is the case a
    /// naive singularity check would have called one: rails drawn twice on
    /// one sheet is ordinary.
    #[test]
    fn two_sources_that_agree_are_not_arguing() {
        let circuit = Circuit {
            nodes: 2,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 3.3,
                },
                Element::Resistor {
                    a: 1,
                    b: 0,
                    ohms: 330.0,
                },
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 3.3,
                },
            ],
        };
        // Two ideal sources in parallel still leave their share of the
        // current undetermined between them, so this is a contradiction in
        // the equations even though the voltages agree — and saying so is
        // right. What must not happen is a silent answer.
        assert_eq!(dc(&circuit), Err(Trouble::Contradiction));
    }
}
