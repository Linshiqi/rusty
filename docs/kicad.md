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

**0 — the decision, and the geometry that follows from it.** The decision is
above. What it needs in code is small: KiCad-space pin positions for a placed
instance, in `rusty-embed` where both sides can reach them (pure arithmetic
over `Symbol` — `local` and `orient`, which the frontend already has for its
own space). Nothing about the sheet, the canvas or the file format changes.

**1 — the reader.** `.kicad_sch` into a `Sheet`, plus the `Sx` tree it came
from. The work is the vocabulary, not the syntax: `lib_symbols` (the file
carries a copy of every symbol it uses, which is what makes an imported
board self-contained), `symbol` instances, `wire`/`junction`/`label`, and
KiCad's connectivity rules. Read, never trusted — a node the reader does not
know is kept in the tree and skipped, exactly as `kicad_sym` already does.

**2 — the writer.** Patch the tree for a file that came from KiCad; write
from scratch for a sheet that did not. A round-trip test that reads a real
file, changes nothing, writes it, and asserts the bytes are unchanged is the
gate this stage lives or dies by.

**3 — the editor catches up.** Buses, no-connect flags, ERC beyond today's
findings, annotation, footprint fields. This is the stage that will try to
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
