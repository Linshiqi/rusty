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
    /// A PN junction, by Shockley: `I = Is·(exp(V / (n·Vt)) − 1)`.
    ///
    /// The first element here that is not linear, which is what brings
    /// Newton–Raphson and everything that can go wrong with it.
    /// `saturation` is `Is` and `ideality` is `n`: a silicon signal diode
    /// is about `1e-14` A and 1, which puts it near 0.7 V at a few
    /// milliamps, and a red lamp about `1e-20` A and 2, which puts it near
    /// 2 V. Both of those are asserted below rather than asserted here —
    /// the drop is what a person recognises, and the parameters are only
    /// how it is reached.
    Diode {
        anode: usize,
        cathode: usize,
        saturation: f64,
        ideality: f64,
    },
    /// `farads` between two nodes. An open circuit at DC and a conductance
    /// of `C/h` in a transient step, which is what makes it remember.
    Capacitor { a: usize, b: usize, farads: f64 },
    /// `henries` between two nodes. A short at DC, and the mirror image of
    /// the capacitor in a step.
    Inductor { a: usize, b: usize, henries: f64 },
}

impl Element {
    /// The two nodes it touches.
    fn ends(&self) -> (usize, usize) {
        match *self {
            Element::Resistor { a, b, .. } | Element::Short { a, b } => (a, b),
            Element::Source { plus, minus, .. } => (plus, minus),
            Element::Current { from, into, .. } => (from, into),
            Element::Diode { anode, cathode, .. } => (anode, cathode),
            Element::Capacitor { a, b, .. } | Element::Inductor { a, b, .. } => (a, b),
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

    /// Does it hold energy — and so carry something from one step to the
    /// next, and behave differently at DC than in a transient?
    fn remembers(&self) -> bool {
        matches!(self, Element::Capacitor { .. } | Element::Inductor { .. })
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
    /// Newton-Raphson did not settle, even after stepping `gmin` down from
    /// a conductance that makes every junction behave.
    ///
    /// **This is a real answer**, and the reason writing the solver is
    /// defensible at all: the circuits here are small and mostly linear, so
    /// a refusal is rare — and a refusal is what the rules of this project
    /// demand over an operating point nobody can check. SPICE's forty years
    /// are largely in not reaching this, and rusty has neither those years
    /// nor a reason to pretend it does.
    DidNotConverge { after: usize },
    /// A transient step that is not a positive length of time.
    BadStep { seconds: f64 },
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
            Trouble::DidNotConverge { after } => write!(
                f,
                "the operating point did not settle after {after} attempts, so there is no answer to report"
            ),
            Trouble::BadStep { seconds } => {
                write!(f, "{seconds} is not a length of time to step through")
            }
        }
    }
}

impl std::error::Error for Trouble {}

/// What one transient step needs beyond the circuit: how long it is, and
/// where each energy-storing element was at the end of the last one.
#[derive(Debug, Clone, Copy)]
struct Dynamic<'a> {
    seconds: f64,
    /// Indexed by element: a capacitor's voltage, an inductor's current.
    memory: &'a [f64],
}

/// A circuit being walked through time.
///
/// **This is the shape stage 5 needs**, and the reason the solver is
/// written here rather than driven as a subprocess: advance to the next
/// instant, read the voltages, change what the firmware is driving, advance
/// again. A netlist handed to something else and run to completion cannot
/// be asked that.
///
/// Backward Euler, and not the trapezoidal rule: it is unconditionally
/// stable and it damps rather than rings. A schematic here is switched hard
/// — a GPIO goes from nothing to the rail in one step — and the trapezoidal
/// rule answers a step edge with an oscillation that is arithmetic rather
/// than circuit, which is exactly the kind of confident wrong answer this
/// simulator exists not to give. The price is first-order accuracy, which
/// is a smaller timestep, and the test below measures that the error really
/// does halve with the step rather than asserting a tolerance nobody can
/// justify.
#[derive(Debug, Clone)]
pub struct Transient {
    circuit: Circuit,
    memory: Vec<f64>,
    now: Solution,
}

impl Transient {
    /// Start from the DC operating point — capacitors open, inductors
    /// shorted — which is what a circuit that has been sitting there is at.
    pub fn settled(circuit: Circuit) -> Result<Self, Trouble> {
        let now = solve(&circuit, None)?;
        let mut started = Transient {
            memory: vec![0.0; circuit.elements.len()],
            circuit,
            now,
        };
        started.remember(None);
        Ok(started)
    }

    /// Start from rest: every capacitor uncharged, every inductor
    /// carrying nothing.
    ///
    /// The other constructor and not a fallback from it, because the two
    /// are different claims. [`Transient::settled`] says the circuit has
    /// been sitting there — and for a node reachable only through a
    /// capacitor there is no such answer, which it reports rather than
    /// inventing a zero. This one says the power has just come on, which is
    /// a statement about the world that only the caller can make.
    pub fn at_rest(circuit: Circuit) -> Self {
        Transient {
            memory: vec![0.0; circuit.elements.len()],
            now: Solution {
                volts: vec![0.0; circuit.nodes],
                through: BTreeMap::new(),
            },
            circuit,
        }
    }

    /// Advance by `seconds`.
    pub fn step(&mut self, seconds: f64) -> Result<&Solution, Trouble> {
        // NaN fails the first test, so the second never sees one.
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(Trouble::BadStep { seconds });
        }
        self.now = solve(
            &self.circuit,
            Some(Dynamic {
                seconds,
                memory: &self.memory,
            }),
        )?;
        self.remember(Some(seconds));
        Ok(&self.now)
    }

    /// What a source is insisting on — how the firmware reaches the
    /// circuit between one step and the next.
    pub fn drive(&mut self, element: usize, volts: f64) {
        if let Some(Element::Source { volts: at, .. }) = self.circuit.elements.get_mut(element) {
            *at = volts;
        }
    }

    pub fn now(&self) -> &Solution {
        &self.now
    }

    pub fn volts_at(&self, node: usize) -> f64 {
        self.now.volts_at(node)
    }

    /// Carry each element's state across the boundary: a capacitor keeps
    /// the voltage it ended at, an inductor the current it ended at.
    fn remember(&mut self, seconds: Option<f64>) {
        for (index, element) in self.circuit.elements.iter().enumerate() {
            match *element {
                Element::Capacitor { a, b, .. } => {
                    self.memory[index] = self.now.volts_at(a) - self.now.volts_at(b);
                }
                Element::Inductor { a, b, henries } => {
                    // At DC it was a short, and its current is the one the
                    // extra equation solved for. In a step it is not in
                    // that list at all -- it is a conductance and a source
                    // like the capacitor -- so its new current comes from
                    // the companion's own equation, `i = i_before + h·v/L`.
                    // Reading `through` here instead leaves the current at
                    // zero for ever, and the voltage across it never
                    // decays: the circuit looks like an open one.
                    self.memory[index] = match seconds {
                        Some(h) => {
                            let across = self.now.volts_at(a) - self.now.volts_at(b);
                            self.memory[index] + h / henries.max(1e-300) * across
                        }
                        None => self.now.through.get(&index).copied().unwrap_or(0.0),
                    };
                }
                _ => {}
            }
        }
    }
}

/// The thermal voltage at room temperature, `kT/q` at 300.15 K.
const THERMAL: f64 = 0.025_865;

/// How close two successive guesses must be before the answer is taken:
/// SPICE's own shape — a relative part for large voltages and a floor for
/// small ones — and tighter than its defaults, because these circuits are
/// small enough to afford it and a test asserting against Shockley wants
/// the digits.
const RELTOL: f64 = 1e-9;
const VNTOL: f64 = 1e-12;

/// And how closely the currents have to agree.
///
/// **Voltages settling is not the same as the circuit being solved**, and
/// the difference is not academic: with limiting active, successive guesses
/// can stop moving while the junction's own equation is out by two orders
/// of magnitude, because each round linearises at the same clamped point
/// and hands back the same answer. That is a fixed point of the *limited*
/// map and not a solution of the circuit. So the residual is checked too —
/// what the junction's curve says at the answer against what the straight
/// line through the linearisation point said — and it is what makes the
/// convergence claim mean anything.
const ABSTOL: f64 = 1e-14;
const RELTOL_I: f64 = 1e-9;

/// Solve for the DC operating point.
///
/// A linear circuit is one solve. A circuit with a junction in it is
/// Newton-Raphson: guess the voltages, replace each diode with the
/// conductance and current source that match it *at that guess*, solve the
/// linear system that makes, and repeat until the guess stops moving.
///
/// **When it will not settle, `gmin` steps.** A tiny conductance across
/// every junction makes the circuit easier to solve and the answer slightly
/// wrong; starting with a large one, solving, and carrying that answer into
/// the next round with a smaller one walks the solver down to `gmin = 0`,
/// which is the real circuit. It is the oldest trick in SPICE, and it is
/// here because the alternative is a refusal on circuits that have answers.
pub fn dc(circuit: &Circuit) -> Result<Solution, Trouble> {
    solve(circuit, None)
}

/// One operating point, static or one step into a transient.
fn solve(circuit: &Circuit, dynamic: Option<Dynamic<'_>>) -> Result<Solution, Trouble> {
    if circuit.nodes == 0 {
        return Ok(Solution {
            volts: Vec::new(),
            through: BTreeMap::new(),
        });
    }
    grounded(circuit, dynamic.is_some())?;

    let nonlinear = circuit
        .elements
        .iter()
        .any(|e| matches!(e, Element::Diode { .. }));
    let mut guess = vec![0.0; circuit.nodes];
    if !nonlinear {
        return step(circuit, &guess, 0.0, &mut [], dynamic).map(|(found, _)| found);
    }

    // Straight at the answer first: most circuits here converge from zero
    // and pay nothing for the ladder below.
    let mut attempts = 0usize;
    if let Ok(found) = newton(circuit, &mut guess.clone(), 0.0, &mut attempts, dynamic) {
        return Ok(found);
    }
    // And when they do not, walk gmin down, each answer seeding the next.
    for gmin in [1e-3, 1e-4, 1e-6, 1e-8, 1e-10, 1e-12, 0.0] {
        match newton(circuit, &mut guess, gmin, &mut attempts, dynamic) {
            Ok(found) if gmin == 0.0 => return Ok(found),
            Ok(_) => {}
            Err(Trouble::DidNotConverge { .. }) => {}
            Err(other) => return Err(other),
        }
    }
    Err(Trouble::DidNotConverge { after: attempts })
}

/// Newton-Raphson at one `gmin`, leaving its answer in `guess`.
///
/// `last` carries each junction's voltage from the previous round, because
/// limiting is about the *step* and not about the voltage — see [`limited`].
fn newton(
    circuit: &Circuit,
    guess: &mut Vec<f64>,
    gmin: f64,
    attempts: &mut usize,
    dynamic: Option<Dynamic<'_>>,
) -> Result<Solution, Trouble> {
    const ROUNDS: usize = 200;
    let mut last = vec![0.0; circuit.elements.len()];
    for _ in 0..ROUNDS {
        *attempts += 1;
        let (found, residual) = step(circuit, guess, gmin, &mut last, dynamic)?;
        let settled = found.volts.iter().zip(guess.iter()).all(|(now, before)| {
            (now - before).abs() <= RELTOL * now.abs().max(before.abs()) + VNTOL
        });
        guess.clone_from(&found.volts);
        // Both, or neither counts: see ABSTOL.
        if settled && residual.solved() {
            return Ok(found);
        }
    }
    Err(Trouble::DidNotConverge { after: *attempts })
}

/// The worst disagreement between a junction's own curve at the answer and
/// the straight line the solve was built on.
#[derive(Debug, Clone, Copy, Default)]
struct Residual {
    off_by: f64,
    largest: f64,
}

impl Residual {
    fn saw(&mut self, off_by: f64, current: f64) {
        self.off_by = self.off_by.max(off_by.abs());
        self.largest = self.largest.max(current.abs());
    }

    fn solved(&self) -> bool {
        self.off_by <= ABSTOL + RELTOL_I * self.largest
    }
}

/// One linear solve: every element stamped, each diode at the operating
/// point `guess` puts it at.
fn step(
    circuit: &Circuit,
    guess: &[f64],
    gmin: f64,
    last: &mut [f64],
    dynamic: Option<Dynamic<'_>>,
) -> Result<(Solution, Residual), Trouble> {
    let mut residual = Residual::default();
    // Every source and short gets an equation of its own, and a current to
    // solve for. Their order here is the order of their currents in the
    // unknown vector, and `through` is keyed by their index in `elements`
    // so a caller can ask about the one it placed.
    let extras: Vec<usize> = circuit
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| match e {
            Element::Source { .. } | Element::Short { .. } => true,
            // At DC an inductor is a piece of wire, and a piece of wire is
            // an equation of its own for the same reason a short is: the
            // current through it is what the rest of the circuit asks for.
            Element::Inductor { .. } => dynamic.is_none(),
            _ => false,
        })
        .map(|(at, _)| at)
        .collect();

    let n = circuit.nodes - 1; // ground is not an unknown
    let size = n + extras.len();
    let mut a = vec![vec![0.0f64; size]; size];
    let mut b = vec![0.0f64; size];
    // Ground has no row and no column: `at` maps a node to its place, or
    // to nothing, which is what makes a stamp against ground a no-op.
    let at = |node: usize| (node != 0).then(|| node - 1);
    // A conductance and a current between two nodes, which is what every
    // element below reduces to. `i` flows out of `p` and into `m`.
    let pair = |p: usize, m: usize, g: f64, i: f64, a: &mut [Vec<f64>], b: &mut [f64]| {
        if let Some(p) = at(p) {
            a[p][p] += g;
            b[p] -= i;
        }
        if let Some(m) = at(m) {
            a[m][m] += g;
            b[m] += i;
        }
        if let (Some(p), Some(m)) = (at(p), at(m)) {
            a[p][m] -= g;
            a[m][p] -= g;
        }
    };

    for (index, element) in circuit.elements.iter().enumerate() {
        match *element {
            Element::Resistor { a: p, b: m, ohms } => {
                // NaN fails the first test, so the second never sees one.
                if !ohms.is_finite() || ohms <= 0.0 {
                    return Err(Trouble::BadResistance { ohms });
                }
                pair(p, m, 1.0 / ohms, 0.0, &mut a, &mut b);
            }
            Element::Current { from, into, amps } => {
                pair(from, into, 0.0, amps, &mut a, &mut b);
            }
            Element::Diode {
                anode,
                cathode,
                saturation,
                ideality,
            } => {
                let thermal = ideality.max(1e-3) * THERMAL;
                let wanted = guess.get(anode).copied().unwrap_or(0.0)
                    - guess.get(cathode).copied().unwrap_or(0.0);
                let across = limited(wanted, last[index], saturation, thermal);
                last[index] = across;
                let exponent = (across / thermal).exp();
                let current = saturation * (exponent - 1.0);
                let slope = saturation / thermal * exponent + gmin;
                // What the curve really has at the voltage the *previous*
                // solve produced, against what the line through this point
                // claims there. Zero when limiting is off and the guess has
                // stopped moving, which is exactly when the answer is real.
                let truly = saturation * ((wanted / thermal).exp() - 1.0);
                residual.saw(truly - (current + slope * (wanted - across)), truly);
                // The companion model: a conductance equal to the curve's
                // slope here, and a source carrying whatever the real curve
                // has that the straight line through this point does not.
                pair(
                    anode,
                    cathode,
                    slope,
                    current - slope * across,
                    &mut a,
                    &mut b,
                );
            }
            Element::Capacitor { a: p, b: m, farads } => {
                // Backward Euler: `i = C·(v − v_before) / h`, which is a
                // conductance of `C/h` in parallel with a source carrying
                // where the capacitor was. With no step it contributes
                // nothing, which is what an open circuit is.
                if let Some(now) = dynamic {
                    let g = farads / now.seconds;
                    pair(p, m, g, -g * now.memory[index], &mut a, &mut b);
                }
            }
            Element::Inductor {
                a: p,
                b: m,
                henries,
            } => {
                // The mirror image: `i = i_before + h·v / L`. The current
                // is out of `p` and into `m`, which is the sign `pair`
                // takes, so the companion's constant part is what it was
                // already carrying.
                if let Some(now) = dynamic {
                    pair(
                        p,
                        m,
                        now.seconds / henries.max(1e-300),
                        now.memory[index],
                        &mut a,
                        &mut b,
                    );
                }
            }
            Element::Source { .. } | Element::Short { .. } => {}
        }
    }

    for (k, index) in extras.iter().enumerate() {
        let row = n + k;
        let (plus, minus, volts) = match circuit.elements[*index] {
            Element::Source { plus, minus, volts } => (plus, minus, volts),
            Element::Short { a, b } | Element::Inductor { a, b, .. } => (a, b, 0.0),
            _ => unreachable!("extras holds sources, shorts and static inductors"),
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
    Ok((Solution { volts, through }, residual))
}

/// SPICE's junction limiting (`pnjlim`), and the whole reason a diode
/// converges.
///
/// The exponential doubles every eighteen millivolts, so a Newton step that
/// overshoots by a quarter of a volt asks the curve for `e^10` times the
/// current, and the next guess overshoots further: it runs away to infinity
/// in about three iterations. The naive loop is not slow, it is useless.
///
/// **It limits the step, not the voltage**, and that distinction is the
/// whole of it. A version that clamped the new voltage alone looked right
/// and converged on a lie: past the knee it mapped every guess to the same
/// point, so the iteration stopped moving while Kirchhoff was out by two
/// orders of magnitude, and — worse — an answer that genuinely sits above
/// the critical voltage could never be reached, because limiting never
/// switched off. Comparing against `previous` means that once the guesses
/// settle, the step is small, no limiting applies, and the linearisation is
/// at the real operating point.
fn limited(wanted: f64, previous: f64, saturation: f64, thermal: f64) -> f64 {
    if !wanted.is_finite() {
        return previous;
    }
    let critical = thermal * (thermal / (saturation.max(1e-300) * std::f64::consts::SQRT_2)).ln();
    if wanted <= critical || (wanted - previous).abs() <= 2.0 * thermal {
        return wanted;
    }
    if previous > 0.0 {
        let arg = 1.0 + (wanted - previous) / thermal;
        if arg > 0.0 {
            previous + thermal * arg.ln()
        } else {
            critical
        }
    } else {
        thermal * (wanted / thermal).ln()
    }
}

/// Every node has a conducting path to ground, or the first one that does
/// not is named.
fn grounded(circuit: &Circuit, stepping: bool) -> Result<(), Trouble> {
    let mut parent: Vec<usize> = (0..circuit.nodes).collect();
    fn find(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    // A capacitor conducts during a step and not at DC, which is exactly
    // the difference between a node that has a voltage while something is
    // changing and one that never had one.
    for element in circuit
        .elements
        .iter()
        .filter(|e| e.conducts() && (stepping || !e.remembers()))
    {
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

    /// Shockley's own equation, evaluated on the answer.
    ///
    /// The whole point of the closed-form gates: this asserts that the
    /// operating point satisfies the *physics* — the current the resistor
    /// carries and the current the junction carries are the same number —
    /// rather than that it equals a figure somebody once wrote down. A
    /// solver could match a memorised constant while being wrong about
    /// everything around it; it cannot satisfy Kirchhoff at the junction by
    /// accident.
    fn shockley(volts: f64, saturation: f64, ideality: f64) -> f64 {
        saturation * ((volts / (ideality * super::THERMAL)).exp() - 1.0)
    }

    #[test]
    fn a_junction_lands_where_shockley_and_the_resistor_agree() {
        // 5 V ── 1k ── node 2 ──▶|── ground
        let (saturation, ideality) = (1e-14, 1.0);
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
                Element::Diode {
                    anode: 2,
                    cathode: 0,
                    saturation,
                    ideality,
                },
            ],
        };
        let found = dc(&circuit).expect("solved");
        let across = found.volts_at(2);

        let through_resistor = (5.0 - across) / 1_000.0;
        let through_junction = shockley(across, saturation, ideality);
        assert!(
            (through_resistor - through_junction).abs() < 1e-12,
            "Kirchhoff at the node: {through_resistor} A through the resistor \
             against {through_junction} A through the junction, at {across} V"
        );
        // And the band a person would recognise, so a solver that satisfied
        // its own arithmetic in the wrong units could not pass.
        assert!(
            (0.6..0.8).contains(&across),
            "a silicon diode at a few milliamps sits near 0.7 V: {across}"
        );
    }

    #[test]
    fn a_junction_the_wrong_way_round_blocks() {
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
                // The cathode at the resistor: reverse-biased.
                Element::Diode {
                    anode: 0,
                    cathode: 2,
                    saturation: 1e-14,
                    ideality: 1.0,
                },
            ],
        };
        let found = dc(&circuit).expect("solved");
        assert!(
            (found.volts_at(2) - 5.0).abs() < 1e-6,
            "only the saturation current flows, so the resistor drops nothing \
             measurable and the node sits at the rail: {}",
            found.volts_at(2)
        );
    }

    #[test]
    fn two_junctions_in_series_drop_more_than_one_and_less_than_twice() {
        let of = |count: usize| {
            let mut elements = vec![
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
            ];
            // A chain from node 2 down to ground, one node per junction.
            for k in 0..count {
                elements.push(Element::Diode {
                    anode: 2 + k,
                    cathode: if k + 1 == count { 0 } else { 3 + k },
                    saturation: 1e-14,
                    ideality: 1.0,
                });
            }
            Circuit {
                nodes: 2 + count,
                elements,
            }
        };
        let one = dc(&of(1)).expect("one").volts_at(2);
        let two = dc(&of(2)).expect("two").volts_at(2);
        assert!(
            two > one && two < 2.0 * one,
            "each of two carries less current than one alone did, so it drops \
             a little less than it did: {one} then {two}"
        );
    }

    /// The test that limiting exists for.
    ///
    /// One ohm between a five-volt rail and a junction: the first Newton
    /// step from a zero guess asks the exponential for `e^193`, which is
    /// not a large number, it is infinity, and the iteration after it is
    /// arithmetic on NaN. Unlimited, this does not converge slowly — it
    /// never converges at all.
    #[test]
    fn a_junction_driven_hard_still_settles() {
        let (saturation, ideality) = (1e-20, 2.0);
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
                    ohms: 1.0,
                },
                Element::Diode {
                    anode: 2,
                    cathode: 0,
                    saturation,
                    ideality,
                },
            ],
        };
        let found = dc(&circuit).expect("a hard drive still has an answer");
        let across = found.volts_at(2);
        assert!(across.is_finite(), "{across}");
        let through_resistor = (5.0 - across) / 1.0;
        let through_junction = shockley(across, saturation, ideality);
        let scale = through_resistor.abs().max(through_junction.abs());
        assert!(
            (through_resistor - through_junction).abs() <= 1e-9 * scale,
            "Kirchhoff still holds at {across} V: {through_resistor} against {through_junction}"
        );
    }

    /// A lamp on a rail through its resistor, which is the circuit this
    /// solver exists for — every rusty sheet has one.
    #[test]
    fn a_lamp_on_a_rail_draws_a_few_milliamps_through_its_resistor() {
        let (saturation, ideality) = (1e-20, 2.0);
        let circuit = Circuit {
            nodes: 3,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 3.3,
                },
                Element::Resistor {
                    a: 1,
                    b: 2,
                    ohms: 330.0,
                },
                Element::Diode {
                    anode: 2,
                    cathode: 0,
                    saturation,
                    ideality,
                },
            ],
        };
        let found = dc(&circuit).expect("solved");
        let across = found.volts_at(2);
        let current = (3.3 - across) / 330.0;
        assert!(
            (1.8..2.4).contains(&across),
            "a red lamp's forward drop: {across}"
        );
        assert!(
            (1e-3..2e-2).contains(&current),
            "and the current a 330 ohm resistor lets through: {current}"
        );
    }

    // ── time ───────────────────────────────────────────────────────────

    /// An RC charging, run to one time constant at two step sizes.
    fn rc_after_one_constant(steps: usize) -> f64 {
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
                Element::Capacitor {
                    a: 2,
                    b: 0,
                    farads: 1e-6,
                },
            ],
        };
        let mut run = Transient::at_rest(circuit);
        let constant = 1e-3;
        for _ in 0..steps {
            run.step(constant / steps as f64).expect("stepped");
        }
        run.volts_at(2)
    }

    /// **The error halves when the step does**, which is what first order
    /// means and is a far stronger claim than any tolerance would be: a
    /// tolerance says the answer is close, and this says the method is the
    /// one it is supposed to be. Backward Euler is first order, so the
    /// ratio should be about a half; anything that is accidentally right at
    /// one step size fails here.
    #[test]
    fn a_charging_capacitor_converges_at_the_order_backward_euler_has() {
        let exact = 5.0 * (1.0 - (-1.0f64).exp());
        let coarse = (rc_after_one_constant(100) - exact).abs();
        let fine = (rc_after_one_constant(200) - exact).abs();
        assert!(coarse > 0.0 && fine > 0.0);
        assert!(
            fine < 0.6 * coarse,
            "halving the step should roughly halve the error: {coarse} then {fine}, \
             against the exact {exact}"
        );
        assert!(
            fine < 0.01 * exact,
            "and two hundred steps should be within a percent: {fine}"
        );
    }

    #[test]
    fn a_capacitor_given_long_enough_ends_up_at_the_rail() {
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
                Element::Capacitor {
                    a: 2,
                    b: 0,
                    farads: 1e-6,
                },
            ],
        };
        let mut run = Transient::at_rest(circuit);
        for _ in 0..2_000 {
            run.step(1e-5).expect("stepped");
        }
        assert!(
            (run.volts_at(2) - 5.0).abs() < 1e-6,
            "twenty time constants: {}",
            run.volts_at(2)
        );
    }

    /// The inductor's mirror image, and its own closed form: the voltage
    /// across it decays as `V·exp(−t/τ)` with `τ = L/R`.
    #[test]
    fn an_inductor_lets_its_voltage_decay_at_the_rate_l_over_r_says() {
        let run_to = |steps: usize| {
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
                    Element::Inductor {
                        a: 2,
                        b: 0,
                        henries: 1.0,
                    },
                ],
            };
            let mut run = Transient::at_rest(circuit);
            for _ in 0..steps {
                run.step(1e-3 / steps as f64).expect("stepped");
            }
            run.volts_at(2)
        };
        let exact = 5.0 * (-1.0f64).exp();
        let coarse = (run_to(100) - exact).abs();
        let fine = (run_to(200) - exact).abs();
        assert!(
            fine < 0.6 * coarse,
            "first order again: {coarse} then {fine}, against {exact}"
        );
    }

    /// A circuit that has been sitting there does not move when time
    /// passes, which is the one thing a transient must never get wrong: an
    /// integrator with the sign or the memory wrong drifts from a steady
    /// state, and drift is what every wrong answer downstream looks like.
    #[test]
    fn a_settled_circuit_stays_where_it_is() {
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
                    ohms: 1_000.0,
                },
                Element::Capacitor {
                    a: 2,
                    b: 0,
                    farads: 1e-6,
                },
            ],
        };
        let mut run = Transient::settled(circuit).expect("it has a DC answer");
        let started = run.volts_at(2);
        assert!((started - 2.5).abs() < 1e-9, "the divider: {started}");
        for _ in 0..500 {
            run.step(1e-4).expect("stepped");
        }
        assert!(
            (run.volts_at(2) - 2.5).abs() < 1e-9,
            "and fifty milliseconds later it is still there: {}",
            run.volts_at(2)
        );
    }

    /// The gesture stage 5 is for: change what a source insists on between
    /// one step and the next, which is how the firmware reaches the
    /// circuit.
    #[test]
    fn what_a_source_insists_on_can_change_between_steps() {
        let circuit = Circuit {
            nodes: 3,
            elements: vec![
                Element::Source {
                    plus: 1,
                    minus: 0,
                    volts: 0.0,
                },
                Element::Resistor {
                    a: 1,
                    b: 2,
                    ohms: 1_000.0,
                },
                Element::Capacitor {
                    a: 2,
                    b: 0,
                    farads: 1e-6,
                },
            ],
        };
        let mut run = Transient::at_rest(circuit);
        for _ in 0..50 {
            run.step(1e-5).expect("stepped");
        }
        assert!(run.volts_at(2).abs() < 1e-12, "nothing has driven it yet");

        // The pin goes high. Twenty time constants, because five is
        // ninety-nine percent and this asserts a part per million.
        run.drive(0, 3.3);
        for _ in 0..2_000 {
            run.step(1e-5).expect("stepped");
        }
        assert!(
            (run.volts_at(2) - 3.3).abs() < 1e-6,
            "and it charges to the rail: {}",
            run.volts_at(2)
        );

        // And low again.
        run.drive(0, 0.0);
        for _ in 0..2_000 {
            run.step(1e-5).expect("stepped");
        }
        assert!(
            run.volts_at(2).abs() < 1e-6,
            "and back down: {}",
            run.volts_at(2)
        );
    }

    #[test]
    fn a_step_that_is_not_a_length_of_time_is_refused() {
        let mut run = Transient::at_rest(Circuit {
            nodes: 2,
            elements: vec![Element::Resistor {
                a: 1,
                b: 0,
                ohms: 1.0,
            }],
        });
        assert_eq!(run.step(0.0), Err(Trouble::BadStep { seconds: 0.0 }));
        assert_eq!(run.step(-1e-3), Err(Trouble::BadStep { seconds: -1e-3 }));
    }
}
