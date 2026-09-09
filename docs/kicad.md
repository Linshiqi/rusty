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

`(mirror x|y)` is implemented as flip-then-turn and is **unverified**:
neither file had a mirrored instance. One `(mirror y)` symbol at 90° with a
wire on an asymmetric pin would settle it, the same shape of evidence the
rotation needed. The test says so rather than implying otherwise.

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

What is left of the stage: buses, no-connect flags, ERC beyond today's
findings, annotation, footprint fields. This is the part that will try to
become infinite; the boundary is in "What this is not" below.

**4 — Kirchhoff.** Modified nodal analysis: a DC operating point first, then
transient. Either written here — a bounded piece of work for R/C/L/V/I plus
diodes and transistors through Newton–Raphson — or ngspice bundled, which is
what KiCad itself drives and which the installer's tool machinery already
knows how to carry. Writing it is more work and no dependency; bundling it is
less work and a real one. **Decide at stage 4, on how far stage 3 got**, not
now.

**5 — the firmware in the loop.** This is the reason to do any of it.

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
