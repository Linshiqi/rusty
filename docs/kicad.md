# Meeting KiCad

A schematic that can be drawn here, simulated here, and then opened in KiCad
— and one drawn in KiCad, brought here, simulated, and handed back without
the trip having cost anything. `docs/schematic.md` is the model this builds
on; this file is the interoperability, and the design decision the whole of
it hangs on.

## What is already here

Not a plan: measured, in the tree today.

- **`.kicad_sym`, read and written** (`schematic::kicad_sym`, 818 lines).
  Multi-unit symbols, pins with their electrical type and angle, polylines,
  rectangles, circles, three-point arcs, text, stroke widths and fills. The
  writer produces what KiCad 8 writes, so an imported part opens in KiCad's
  own symbol editor and the round trip is a test.
- **An S-expression reader** (`kicad_sym::read` → `Sx`). The schematic file
  is the same syntax with a different vocabulary, so the parser is not part
  of the work.
- **The sheet**: placed symbols with a turn and a mirror, wires, labels,
  power symbols, and `nets` reading them into a union-find.
- **The canvas**, with KiCad's gestures: `R` turns, `X` mirrors, wires are
  orthogonal polylines through the author's bends, a moving part stretches
  only the leg beside its pin, aligned segments merge, and a wire dropped on
  another wire branches.

**Nothing at all** exists for `.kicad_sch` — the schematic file. No UUIDs,
no `lib_symbols`, no junction or no-connect objects, no hierarchical sheets,
no buses.

## The measurement that decides the design

KiCad's wires are geometric segments and connectivity is read off the
coordinates: endpoints that coincide, junction objects where three or more
meet, labels that name a net. rusty's are `Wire { from: PinRef, to: PinRef,
bends }` — pin to pin, with the geometry a consequence.

The obvious plan is to adopt KiCad's model. **The obvious plan is wrong, and
one measurement says so.** rusty draws parts as the components they are and
*the drawing decides where the pins are* — a wire lands on the end of a leg,
where it does on the bench. Compare where a `Device:LED`'s pins sit in the
symbol's own coordinates with where its legs actually end on the canvas:

```
pin K:  symbol (-24, 0)    canvas (-4, 18)
pin A:  symbol ( 24, 0)    canvas ( 4, 24)
```

and, for the devkit, whose symbol is generated from the same rows the art
draws:

```
kit pin 1:  symbol (10, 16)    canvas (10, 16)
```

The two spaces differ, they differ **per pin rather than by any transform**,
and they differ most for exactly the library parts a KiCad file is made of.
A five-millimetre lamp with real legs is not the KiCad symbol of a diode
moved sideways; it is a different drawing that happens to have the same
pins.

So a wire endpoint in rusty's space has no general translation into KiCad's.
**Except one: a point that is on a pin.** That translates exactly, because a
pin is the same pin in both spaces.

## The design

**Two coordinate spaces, and the pin is the bridge.**

- rusty's sheet keeps its model. Wires stay pin to pin, the drawing stays
  the physical one, and `.rusty/sim.toml` needs no new version. The thing
  that makes this simulator worth using — a board that looks like a board —
  is not traded away for a file format.
- **Import** derives nets from KiCad's geometry by KiCad's own rules, then
  states them as pin-to-pin wires. Connectivity survives exactly; the
  author's chosen wire routing does not, because in this space it cannot.
- **Export patches the original file.** This is the half that makes the
  round trip honest, and the technique is already in this repository:
  `migrate.rs` rewrites `Cargo.toml` with word-bounded textual
  substitutions and never a parse-and-reserialise, precisely so that
  comments, ordering and version spellings survive byte for byte. The same
  rule here: keep the S-expression tree the file was read as, patch the
  nodes that changed, write the rest back untouched. **Everything rusty does
  not model survives** — hierarchical sheets, buses, text boxes, images,
  footprint fields, the lot.

The consequence worth stating plainly, because it is what a user will
notice: **a wire nobody touched keeps its original geometry; a wire rusty
changed is re-routed.** Import a two-hundred-part board, move one resistor,
export — one wire is rewritten and the other four hundred are the bytes that
came in. A parse-and-reserialise writer would have rewritten all of them, and
the first round trip would have cost the user their layout.

A file rusty *creates* from a sheet drawn here has no original to patch, so
it is written from scratch and routed by rusty: orthogonal, on KiCad's grid,
from KiCad pin positions. That is a starting point in KiCad, not a
reproduction of the canvas.

### What crosses, and what does not

| KiCad | rusty | Crossing |
|---|---|---|
| `symbol` instance + `lib_id` | `Instance` + `symbol` | Both ways |
| `property` Reference / Value | `reference` / `value` | Both ways |
| `at`, mirror | `x`/`y`, `rot`, `mirror` | Both ways, in each space |
| `wire` segments + `junction` | `Wire` pin-to-pin | Nets both ways; geometry in only |
| `label`, `global_label` | `rusty:Label` | Both ways |
| power symbols | `rusty:GND` / `rusty:Supply` | Both ways |
| `no_connect` | — | Preserved, not modelled |
| `bus`, `bus_entry` | — | Preserved, not modelled |
| hierarchical `sheet` | — | Preserved, not modelled; only the root sheet is read |
| `text`, graphics | — | Preserved, not modelled |
| UUIDs | — | Preserved; never regenerated for a part that already had one |

"Preserved, not modelled" is the whole point of patching. A thing rusty
cannot draw is a thing rusty must not delete.

## The stages

**0 — the decision, and the geometry that follows from it.** *Done*
(`schematic::place`). KiCad-space pin positions for a placed instance: pure
arithmetic over `Symbol`, and nothing about the sheet, the canvas or the
file format changes.

The library is y-up and the sheet is y-down, so a pin flips before anything
else happens to it — not in doubt, and confirmed by which side of its
connection point a ground symbol is drawn. **Which way the angle turns was
in doubt, and it is one boolean that silently reverses a diode when it is
wrong.** It is `R(-a)`, and what settled it was a lamp: a `Device:LED`
turned 90° between a supply and a ground, and *what KiCad drew* — the
cathode bar on the ground side. Two things could not settle it and one of
them looked as though it had:

- **Pin coordinates cannot.** Every rotated part on a real 33-symbol board
  was a one-pin power flag or a point-symmetric resistor. For those, both
  candidate signs put pins on the same two points and differ only in which
  pin is which — exactly the case that matters, and exactly the one the
  geometry is silent about.
- **An autoplaced field is not a witness.** `Device:LED`'s `Reference` sits
  at `(0, 2.54)` in the library and KiCad wrote the instance's on the `+x`
  side, which `R(+90)` predicts and `R(-90)` does not; a resistor on a
  second board agreed. Both were wrong. KiCad places field text where it
  reads well rather than carrying it through the symbol's transform, and the
  tell was a third instance at 270° whose field implied the opposite sign
  from the 90° ones on the same sheet. A witness that contradicts itself is
  not a witness.

**The mirror was the second boolean, and it was wrong.** It acts on the
screen, *after* the turn — and the first implementation had it before,
which is indistinguishable at 0° and 180° and wrong at every quarter turn.
The witness is three `Simulation_SPICE:NPN` transistors, asymmetric in both
axes, which is what a two-pin part can never be: `(at … 90) (mirror x)` is
drawn with the base up, the collector left and the emitter right, and
mirroring first answers the opposite for all three. A reader with that
backwards would exchange a transistor's collector and emitter on every
quarter-turned part, silently.

`(mirror y)` was never seen in any file, and there is a reason: a left–right
flip is the same orientation as `mirror x` plus 180°, so KiCad need not
write it — asked for one it stored `(at … 180)` with no mirror node at all.
It follows the rule the other one established rather than a measurement of
its own, and the code says so.

**1 — the reader.** *Done* (`schematic::kicad_sch`). `.kicad_sch` into a
`Sheet`. The syntax was free — `kicad_sym`'s tokeniser moved to
`schematic::sexpr` and both vocabularies read it — and the work was the
vocabulary plus the one thing that is genuinely different: **connectivity is
geometric**. A segment joins its own two ends; two segments ending at one
point are one node because the point is the key; a junction joins every
segment through it, which is what makes two crossing wires with a junction
one net and two without it two nets; a pin joins any wire it lies on,
*including in the middle*, because KiCad connects there and a reader that
matched only endpoints would silently drop every part somebody wired by
running a line across its pins. Labels of one name are one net. The answer
is then stated as rusty's pin-to-pin wires, a star per net.

An anchor crosses by one scale (`MM_PX`, which now lives in `model` so the
canvas and the writer cannot spell it differently); pins do not cross at
all, which is the whole finding this project is built on.

**2 — the writer.** *Done* (`schematic::kicad_out`). Two shapes and one
rule. A sheet that came from KiCad is written by **patching the bytes it
came in as** — spans found in one string-aware pass, never a reserialised
tree, because a tree cannot give the formatting back (KiCad writes tabs,
puts small nodes inline and long ones one per line, and writes `0` where a
parser only knows `0.0`). A sheet drawn in rusty has no original and is
written whole.

The gate the stage lives by is `a_file_nobody_changed_comes_back_as_the_
bytes_that_went_in`, and it passes. Moving, turning, renaming, revaluing,
adding and deleting a part are patched in place: the part keeps its uuid,
its footprint field and everything else in its node, and **every wire keeps
its own bytes**, which is the assertion beside it. What a patch will not do
is pretend — rewiring changes a netlist, and which of KiCad's segments
belonged to which net is not a question the geometry answers once the nets
have moved, so a wiring change regenerates every `wire` and `junction` and
says so in `Written::notes`. Everything else in the file still keeps its
bytes.

**3 — the editor catches up.** *Begun.* The first part of it was not on the
list and had to be: **the plumbing**, because stages 1 and 2 had no caller
and a reader nobody can reach is a reader nobody has. `schematic::import`
and `schematic::export` are the project-level pair, `sim_import_kicad` and
`sim_export_kicad` the commands, and two buttons sit beside Save in the
sheet's corner. Import re-reads the file's own symbols into the project's
`.rusty/symbols/`, one file per KiCad library, because a schematic carries
the only copy of some of them and without that the sheet would draw once
and come back as a row of unknown boxes. Export re-reads whatever is at the
path and patches it, so the promise holds against what is on disk *now*
rather than against what was read an hour ago.

**And an imported board can be run.** A KiCad schematic has a module of its
own where rusty's sheet has a devkit, and rusty drives pins through `U1`'s
rows — so an imported board used to draw, check, and do nothing when run.
`nets::bind_to_kit` reads the module's pin *names* (`GPIO5`, `IO5`,
`GPIO05` — what the author wrote, not a guess) and joins each to the row of
the same number, skipping any the chip does not have. Exactly one candidate
or none: two parts that both look like the microcontroller is a question
with no right answer, so that case binds nothing and names both, the way
`firmware_root` refuses two excluded firmware crates. Four GPIO-named pins
is the bar — fewer is a connector, and a header labelled `IO0` should not
become the chip.

Those joins are rusty's and not the file's, and the writer knows it: a wire
touching `U1` is neither written out nor counted as a change, so binding an
imported board and exporting it again is still the identity. That is its own
test, because without it every export of an imported board would rewrite the
whole layout — the one thing the patching writer exists to prevent.

**No-connect flags and two more findings** are in. `Sheet.no_connect` is a
list of pins the author has answered for; a right-click on a pin toggles it
and the pin then wears KiCad's cross instead of pulsing. It crosses both
ways — KiCad's `(no_connect (at x y))` is a mark on a *point* and this is a
mark on a *pin*, matched by position on the way in and written at the pin's
position on the way out, and a set that changed rewrites those nodes and
nothing else.

The findings it makes possible:

- **`PinReachesNothing`** — a pin nobody joined, on a part somebody was
  joining. Three things keep it usable rather than noise: it waits until the
  part has at least one wire (a symbol just dropped on the sheet has every
  pin loose and does not want six findings), it stands down for a
  no-connect, and it stands down where something more specific already names
  the part. A switch with one side loose is `SwitchDrivesNothing`, which is
  the same fault said better.
- **`OutputsFighting`** — two pins that both drive, on one net. Not
  `Conflict`, which is two GPIOs the firmware has driven apart while it
  runs: this one is true of the drawing, before anything is built.

What is left of the stage — **and is deliberately not being done** — is
buses, annotation and footprint fields. A bus has no consumer here: the
simulator does not need one and neither do the rules, so drawing one would
be feature parity rather than gesture compatibility, and the file already
keeps its own. Annotation is KiCad's job and an imported file arrives
annotated. Footprints are the board's, and rusty does not lay out boards.
This is the part of the stage that would try to become infinite; the
boundary is in "What this is not" below, and this paragraph is where it is
applied.

**4 — Kirchhoff.** *Decided: written here, not ngspice.* The comparison,
because the decision is not obvious and the losing option loses on one
criterion rather than on all of them.

Two facts frame it. First, **most of what a rusty sheet can hold has no
SPICE meaning at all**: of sixteen `Behaviour`s, eight are circuit elements
(lamp, resistor, capacitor, switch, RGB, digit, pot, rail) and eight are
behavioural by construction — a `Display` shows a channel, a `Sensor` feeds
one the firmware declared, a `Motor` is a duty and a direction, a `Servo` is
an angle, a `Buzzer` is on or off, a `Label` is a name, an `Analog` *is* ADC
counts. Second, **a KiCad schematic already carries ngspice annotations** —
`Sim.Device "NPN"`, `Sim.Type "GUMMELPOON"`, `Sim.Pins "1=C 2=B 3=E"` — for
the plain reason that KiCad's simulator is ngspice.

| | written here | ngspice bundled |
|---|---|---|
| Stepped by QEMU (stage 5) | in-process `step(dt, inputs) -> outputs` | built to be handed a netlist and a duration; global state, one simulation per process, and lockstep down a pipe at microsecond granularity |
| Device models | R/C/L/V/I and a diode, then it stops | thirty years of them, and the ones KiCad's annotations name |
| Provable | against closed form — a divider's ratio, `τ = RC`, a diode against Shockley | "it ran and said something" |
| Installer | nothing | three more platform binaries, a version pin, a licence review, and one more thing that can fail to download |
| Convergence | **the real risk.** SPICE's forty years are not in the MNA formulation, which is a page of linear algebra; they are in gmin stepping, source stepping, adaptive timesteps, LTE control and limiting for exponential devices | solved |

**What decides it is stage 5, not stage 4.** For "simulate a circuit"
ngspice wins on models and it is not close. For "the firmware drives the
circuit" the solver has to be steppable from inside this process at the
firmware's own timescale, and ngspice's shape is wrong for that. Three
things support the same answer: being provable against closed form is worth
more in this repository than in most; the coverage gap is smaller than it
looks, because nothing rusty draws is a semiconductor and transistors arrive
only with an imported board; and the convergence risk is one this project's
own rule disarms — **a solver that refuses to converge is in keeping, and
one that quietly returns a wrong operating point is not.**

**What would change it**, written down so it can be noticed rather than
argued about later: if importing KiCad boards with real semiconductors
becomes the main use — if people start depending on those `Sim.*`
properties — then writing this becomes reimplementing Gummel-Poon and
ngspice wins. The reader can already see those properties, so the signal is
countable.

The scope, in order, each step provable before the next:

1. DC operating point: MNA with a dense LU, since these circuits have tens
   of nodes. Resistors, voltage and current sources, and a closed switch as
   a zero-volt source rather than a zero-ohm resistor.
2. The gates: a divider's ratio, resistors in parallel, a Thévenin
   equivalent — closed-form answers a test can assert exactly.
3. Newton–Raphson and a Shockley diode, with gmin stepping, and *not
   converging* as a reported outcome.
4. Transient: backward Euler first — unconditionally stable, where the
   trapezoidal rule rings — and capacitors and inductors with it.
5. The bridge from `nets`, which is nearly free: the union-find already
   answers which pins are one node, and MNA wants exactly that incidence.

**And the answers are on the panel.** `circuit::operating_point` is the
sheet-level door — bridge, solve, and one error type over both halves — and
the editor holds it in a memo beside the rules': `eval` says on and off,
`solved` says numbers, over the same parts, the same wires, the same held
switches and the same levels the firmware has reported. The probe on a wire
gains what its net is *at*, and a selected part gains what is across it,
what is through it and what it is dissipating, under the value that decides
all three.

Three things that shape it more than the arithmetic does:

- **A refusal goes where the question was asked.** `solved` is an `Err`
  far more often than it is an answer, and that is the design: a lamp with
  no `vf` is an ordinary state of a sheet somebody is still drawing. So the
  reason takes the number's place in the probe, and sits beside the part it
  is about when it is about one — `Unsolved::part` is that test, and a
  reason shown on all thirty parts would say "something is wrong here"
  twenty-nine times over.
- **A refusal speaks the sheet's language, not the solver's.**
  `Trouble::Floating` names a node number, which is an index into an array
  the bridge built and renumbered; `operating_point` turns it into the pins
  on that node, because that is what somebody can point at.
- **The reading is what a meter shows.** `view/panels/simulate/readout.rs`
  is engineering notation, pure and tested: `4.08 mA`, not
  `4.0799e-3 A`, and exact zero is `0 V` rather than a prefix chosen from
  `log10(0)`.

**All five are done**, and 4 and 5 were taken in the other order on
purpose: until something built a circuit, `solve` was a library nothing
called, and a modelling gap is easier to find without another layer on top
of it. `Transient` is the shape stage 5 needs — `step(seconds)`, read the
voltages, `drive` a source, step again — and the accuracy claim is a
convergence-order test rather than a tolerance: halving the step halves the
error, which says the method is the one it is meant to be instead of saying
one answer happened to be close.

**5 — the firmware in the loop.** *The coupling is done* (`live`), and it
is the reason for every stage before it.

**It is event-driven, and that is what made it tractable.** The difficulty
was never electrical. QEMU runs at roughly wall-clock and a transient steps
in microseconds, so advancing the circuit a microsecond per microsecond of
guest time would be a million solves a second, nearly all of them computing
that nothing had changed. The circuit only has to be advanced when something
*asks*: a pin the firmware drove — reported with the systimer's own
microseconds, which the pin channel has carried all along as
`[rusty:gpio@1234]` — or a converter it read. Between those it is only
relaxing.

Which is where backward Euler earns its place a second time. Over a long
quiet gap one coarse step is not merely stable: it lands *on* the steady
state, because the method is implicit and the steady state is its fixed
point. A coarse step across a quiet stretch is not an approximation that
degrades, it is the answer — and that is its own test.

Two things the coupling has to get right, and both are tests:

- **The edge lands where the firmware put it.** `drove` advances to the
  reported instant *before* applying the level, which is the difference
  between a pulse width the panel can be believed about and one it rounded
  to the last step.
- **Charge survives a rebuild.** A pin nobody has reported is not a source —
  rusty does not claim to know the voltage of a pin it has heard nothing
  about — so the first report of a pin adds an element and the circuit is
  rebuilt. Starting the new one from rest would discharge every capacitor on
  the sheet at the exact moment the firmware first touched a pin, so the
  voltages are carried across by node.

`Live::absorb` is where the two halves meet, and the same function is used
by the app and by the gate that proves it — one reading of the coupling, for
the reason the serial protocol has one `absorb`. The backend's pin-channel
reader calls it on every line and sends back what the converter should read;
a circuit that stops having an answer stops answering and says so once,
because a failure repeated at the emulator's rate is a log nobody can read.

`qemu/live-probe` is the firmware that proves it: it drives one pin and
reads another, with a resistor and a capacitor between them on the sheet.
**The assertion is the shape of the numbers.** A host echoing the pin level
would step from nothing to full scale in one conversion; a host solving the
board makes the reading climb through the time constant somebody drew — and
then arrive, because something that only ramped would be a host sending a
ramp of its own. `cargo run -p rusty-embed --example live_probe --
qemu/live-probe` is that gate.

## What this is not

**Not a replacement for eeschema.** KiCad's schematic editor is on the order
of two hundred thousand lines and thirty years old. Feature parity is a
multi-year project and it would be fought on KiCad's own ground, against a
tool the user already has and can open the same file in. The goal for "the
experience is KiCad's" is **gesture compatibility** — the keys, the grid, how
a wire behaves, what a right-click offers — and not feature parity. Where
rusty cannot do something, the file is already open in KiCad; say so and get
out of the way.

## Why this is worth doing at all

KiCad and ngspice can simulate a circuit and have no firmware. rusty runs
the firmware — really runs it, in QEMU, on the chip's own instruction set,
with a channel carrying its pins, its converter, its buses. **Neither half
alone answers the question an embedded engineer actually has**, which is
whether *this* firmware works on *this* circuit.

Joining them is the differentiated thing and nobody else is placed to do it.
It is also the hard part, and the difficulty is not electrical: it is time.
QEMU runs roughly at wall-clock and a transient analysis steps in
microseconds, so the two need a synchronisation scheme — the emulator's
systimer is already on the pin channel (`[rusty:gpio@1234]`), which is where
that scheme would start.

That is stage 5, and stages 0 through 4 are worth doing on their own terms.
But it is what they are for.
