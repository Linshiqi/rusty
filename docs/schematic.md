# The schematic model

The board editor's second generation: parts are schematic symbols with real
pins, wires join pins into nets, and the simulator reads the nets rather than
"which GPIO is this lamp on". KiCad's schematic editor is the reference for
how a symbol looks and how a wire behaves; the part library is KiCad's own
format plus whatever LCSC (嘉立创) sells, imported by part number.

## Why a rewrite rather than more parts

The first generation had one abstraction — a part *is* the pin it sits on —
and every part was drawn by hand in the view. That answered "does GPIO2 go
high" and nothing an electronics person asks: which way round is the LED,
where is the resistor, what is this node connected to. A capacitor has no
place in that model at all. The abstract parts are not kept: they are
migrated where a symbol can stand in for them, and the file says so.

## Symbols

A **symbol** is the vendor-independent drawing of a part with its pins —
`Device:R`, `Device:C`, `Device:LED`, `Device:SW_Push` — in a coordinate
system of its own: KiCad's, millimetres with y *up* and the origin at the
symbol's anchor. The renderer flips y once, at the edge, rather than every
importer remembering to. A symbol carries:

- `pins`: number, name, electrical type (passive, input, output, power in,
  bidirectional…), the connection point, the length and the angle — KiCad's
  convention, pointing from the connection point *into* the body, 0 meaning
  the body lies to the right. The pin number is what a wire attaches to and
  what a net remembers; a pin is found by number and then by name, so
  `D1.K` and `D1.2` both land.
- `graphics`: polylines, rectangles, circles, three-point arcs and text,
  with stroke width and fill — enough for every symbol in KiCad's `Device`
  library and for what EasyEDA emits.
- `reference`: the prefix a placed instance is numbered with — `R`, `C`,
  `D`, `SW`, or `LED` as LCSC's library has it.

`rusty_embed::Symbol` is the wire model, wasm-safe, in `model/symbol.rs`.
Two importers produce it, both in `rusty_embed::schematic`:

- **`.kicad_sym`** (`kicad_sym`): the S-expression library format of KiCad
  6, 7 and 8, read and written. The built-in library is `data/symbols/
  Device.kicad_sym`, compiled in; a project may add its own under
  `.rusty/symbols/`, one file per library name. A file is read, never
  trusted: unknown nodes are skipped, a derived symbol (`extends`) is
  skipped whole, a pin without a number is refused with the symbol's name.
  The writer produces what KiCad 8 writes, so the round trip is a test and
  an imported part opens in KiCad's own editor.
- **EasyEDA** (`easyeda`): the JSON the EasyEDA/LCSC component service
  answers for a part number (`api/products/C2286/components`), whose
  `dataStr.shape` is a list of `~`-separated records — `P` pins, `R`
  rectangles, `PL`/`PG` polylines and polygons, `E` ellipses, `A` arcs,
  `PT` SVG paths, `T` text — in units of 10 mil with y down and the anchor
  at `dataStr.head.x/y`. The service is asked through `net`'s proxy ladder
  on both of its hosts (`easyeda.com`, then `lceda.cn`); the answer is read
  into a symbol named by its part number in the `lcsc` library and kept in
  the data directory's `symbols/lcsc.kicad_sym`. A part that cannot be
  fetched or has no pins is refused in words — no placeholder rectangle
  stands in for it — and a record the reader does not know is listed in
  `warnings`, never dropped in silence.

  What the captured answers taught (`tests/fixtures/easyeda/`, a resistor,
  a capacitor, an LED and a tactile switch): the pin's line is written from
  either end, so the body end is whichever end is not the connection dot;
  the rotation field's 0 is a pin sticking out to the right of its body,
  KiCad's 180; `Value` is the electrical value and `name` the manufacturer
  part. `cargo run -p rusty-embed --example lcsc_probe -- C25804 out.json`
  captures another answer and prints every record beside what was made of
  it, and `rusty-cli symbol C25804` is the headless proof that a machine
  reaches the service.

The library is three layers, later ones winning by `library:name`: the
built-in file, the data directory's `symbols/`, then the project's
`.rusty/symbols/`. `schematic::load(project)` reads them in that order and
names in `warnings` every file it could not read.

## The board file, version 2

`.rusty/sim.toml`:

```toml
version = 2
chip = "esp32c3"

[[part]]
ref = "D1"
symbol = "Device:LED"        # library:name, or "lcsc:C2286"
value = "red"
at = [120, 80]               # sheet units
rot = 0
mirror = false

[[part]]
ref = "R1"
symbol = "Device:R"
value = "220"
at = [80, 80]

[[wire]]
from = "U1.GPIO2"            # the devkit is U1; its pins are named by GPIO
to = "R1.1"
[[wire]]
from = "R1.2"
to = "D1.A"                  # a pin by number or by name
[[wire]]
from = "D1.K"
to = "U1.GND"
```

A wire joins two pins. Nets are derived, never stored: union-find over the
wires, with the devkit's `GND` and `3V3` pins naming the rails. A version-1
file is read by the old loader and migrated: a lamp on GPIO *n* becomes an
LED whose anode is wired to GPIO *n* and whose cathode goes to GND, with a
note that the resistor is missing, because that is exactly what the old
board claimed and the new rules will now say what is wrong with it.

## What the simulator reads

The emulator (or the firmware's own narration) sets GPIO levels. The rules
turn nets into what the parts show:

- **A net's level** is the level of the GPIO or rail driving it; two GPIOs
  driving one net is reported, not resolved.
- **A two-terminal passive** joins two nets for the purpose of a DC level: a
  resistor conducts, a capacitor is open. So GPIO → R → LED → GND lights the
  LED; GPIO → C → LED does not.
- **An LED** lights when its anode's net is high and its cathode's net is
  low, and nothing else lights it. Reversed, it stays dark; that is the
  finding, not a failure.
- **A missing series resistor** — an LED between a GPIO and GND with no
  resistor in the path — is a warning on the sheet, in the words a person
  would use: the LED lights, and on the desk it would not for long.
- **A button** joins its two nets while pressed; the pin it drives is read
  through the same rule, so a pull-up input reads low while it is pressed
  and high otherwise, exactly as `Input::is_high()` sees it.

Everything above is pure, in `rusty_embed::schematic::rules`, under tests
that name the circuit they check.

## Rendering

A symbol's graphics become an SVG group; pins are drawn as KiCad draws them
— a line of the pin's length ending in a circle at the connection point,
number beside the line, name inside the body. Wires are orthogonal
polylines between pin ends, as the first generation drew them. Rotation
and mirroring are transforms on the group; the reference and value are
counter-rotated so they read upright.

## Order of work

1. ✔ `symbol` model and `kicad_sym` importer, with KiCad's own `R`, `C`,
   `LED` and `SW_Push` as fixtures.
2. ✔ `easyeda` importer against captured answers for a resistor, a
   capacitor, an LED and a switch; the fetch through `net`; `rusty-cli
   symbol C2286` to prove it headless.
3. Nets and rules, pure and tested.
4. The file format and the migration.
5. The editor: symbol rendering, pin-to-pin wiring, the library panel with
   "import from LCSC…".
