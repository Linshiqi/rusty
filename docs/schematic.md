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

[board]
chip = "esp32c3"
x = 460                      # where the devkit sits, sheet units
y = 40

[[part]]
ref = "D1"
symbol = "Device:LED"        # library:name, or "lcsc:C2286"
value = "red"                # a lamp's colour; a resistor's 220; anything
x = 168                      # the symbol's anchor, sheet units
y = 96
rot = 90                     # quarter turns clockwise; absent is 0
mirror = true                # absent is false

[[part]]
ref = "V1"
symbol = "rusty:Analog"
value = "4095 = 4.2 V through 100k/27k"
x = 200
y = 120
[part.props]                 # a behaviour's own knobs, as text
max = "4095"

[[wire]]
from = "U1.GPIO2"            # the devkit is U1; its pins are named by GPIO
to = "R1.1"                  # and rail: GND, 3V3, VIN, 5V, EN
[[wire]]
from = "R1.2"
to = "D1.A"                  # a pin by name, or by number where a name repeats
bends = [[300, 96], [300, 200]]
```

A wire joins two pins; its bends are the author's, and a wire without any
routes itself. Nets are derived, never stored. A file with no `version` is
the first board — `[[led]]`, `[[button]]` and friends — and is read by the
reader that always read it, then migrated: a lamp on GPIO *n* becomes a
`Device:LED` wired to `GPIOn` and `GND` (or to `3V3` and `GPIOn` when it was
active-low), a button a `Device:SW_Push` to its GPIO and its rail, the rest
the `rusty` library's parts, with no resistors, because the old board had
none. The plan's notes say so; the editor rewrites the file in version 2
the first time it saves.

## What the simulator reads

`rusty_embed::nets` — pure, compiled for both sides — turns the wires into
what the parts show. The emulator (or the firmware's own narration) sets
GPIO levels; the rails are fixed; everything else follows:

- **A net's level** is the level of the rail or the reported GPIO driving
  it. Ground and a supply on one net is a `Short`; two GPIOs reported at
  different levels on one net is a `Conflict`. A net with no driver floats.
- **A two-terminal passive** joins two nets for the purpose of a DC level: a
  resistor conducts, a capacitor is open. So GPIO → R → LED → GND lights the
  LED; GPIO → C → LED does not.
- **An LED** (a symbol with pins `A` and `K`, or KiCad's `D` with pin 1 the
  cathode) lights when its anode's net is high and its cathode's low, and
  nothing else lights it. Reversed, it stays dark; that is the finding, not
  a failure. **An RGB lens or a digit** lights a channel against its `COM`:
  common anode lights a channel pulled low, common cathode one driven high,
  and no common wired lights nothing.
- **A missing series resistor** — a GPIO in the *wired* net of a lamp's pin,
  nothing between — is `LedWithoutResistor`: the lamp lights here, and on
  the desk it would not for long.
- **A switch** joins its pins while pressed (a four-pin tactile switch has
  its pairs joined always). `button_drives` reads what a press does — the
  GPIO on one side, the rail on the other — and both the sheet and the
  backend's pin channel read it, so the emulator is driven to the level the
  wiring means. A switch that reaches no GPIO, or no rail, is
  `SwitchDrivesNothing`.
- **A rail symbol** (`rusty:GND`, `rusty:Supply`) puts its net at that
  level wherever it is drawn, and **a net label** (`rusty:Label`) joins
  every label carrying the same value into one net. A label with no value
  joins nothing.
- **A buzzer** is on by the lamp's rule — its `+` high and its `-` low —
  and is not asked for a series resistor, because a sounder does not want
  one. **A servo's** horn follows the duty on its signal pin. **A sensor**
  (`rusty:Sensor`) names, in its value, the channel the firmware declared
  with `[rusty:sensor]`; the sheet feeds that channel and refuses to invent
  one the firmware never asked for.
- **A knob, a source, a motor** are *on* whatever GPIO their pin reaches
  through the wires and the resistors (`gpio_of`): the pot's wiper sends
  `P<gpio>=`, the analog source `A<gpio>=`, the motor reads its duty from
  `[rusty:pwm]` on that GPIO and its direction from the levels at `IN1` and
  `IN2`.

Every rule is under a test that names the circuit it checks, and the
findings are `Warning`s with a stable kind: the frontend translates them,
the CLI prints them.

`Evaluation` also carries the nets themselves — which pins the current
joins — so a wire can be asked what it is on. That is the probe in the
properties panel: high, low or floating, and every pin in the net.

## Proving a board without eyes

```bash
cargo run -p rusty-embed --example board_probe -- <project> [seconds]
```

Boots the project in the emulator with the pin channel attached, replays
the sheet's rules over the pins the emulator actually reports, and says
what each part did — a lamp that lit *and went out* is one the firmware is
driving. Then it presses every button on the sheet and requires the pin it
reaches to move. It exits non-zero when a lamp wired to a GPIO never
lights, when a press moves nothing, or when the emulator reports no pins at
all, so a board can be kept working by a machine. `qemu.yml` runs it on
`examples/blink-rust` as gate 7 of every emulator build.

## The editor

`view/panels/simulate/`: `geometry.rs` (pure — pin points after a turn and
a mirror, hit-testing, wire paths, the devkit's generated symbol, the
symbol's SVG markup), `edit.rs` (pure — what placing, wiring, renaming,
rotating, deleting, duplicating and undoing do to the parts and the wires),
`library.rs` (the palette, grouped by library, with the LCSC import box),
`mod.rs` (the canvas and the properties panel).

Parts are drawn as the components they are (`art.rs`): a 5 mm lamp with
its flat and its long anode leg, a resistor with the colour bands of its
value, a tactile switch with a cap, a screen on a carrier board, and a
package for anything rusty has never heard of. **The drawing decides where
the pins are** — a wire lands on the end of a leg, as it does on the bench
— and the symbol's own coordinates are used only for the devkit, whose
pins are its header's rows. Text — pin names, the reference above, the
value below — is placed after the turn, so it always reads upright. Pins
are gold dots that pulse until something reaches them; a wire is pulled
from any pin to any other, in either direction, and lands only within
reach.
Wires are orthogonal polylines through the author's bends, with KiCad's
semantics: moving a part stretches only the leg beside its pin, a
dragged segment pushes its neighbours, aligned segments merge. Lamps glow,
a digit lights segment by segment, a pressed switch sinks its cap, wires
take the colour of their net's level while the firmware runs, and the
rules' findings sit in the sheet's corner.

## What the parts are plugged into

Three of the things this section used to list as missing are here, and they
are the emulator's rather than the sheet's — `qemu/README.md` has the whole
of it. What the sheet contributes is the *declaration*:

- **An analog source reaches the converter.** `A<pin>=<counts>` goes down
  the pin channel as well as the console, so `adc.read_oneshot()` returns
  what the slider is set to. The source's `start` prop is where the slider
  sits and what the run puts on the pin as it begins, so the two agree
  before the first drag.
- **A part with an `addr` prop is on the I2C bus**, and `regs` is what it
  answers: `75=68,3b=010203040506`. Its kind does not decide — a sensor, a
  display and a breakout imported from LCSC all reach the bus the same way.
- **A part with a `cs` prop is on SPI2**, and `miso` is what it answers
  with, read from the start of every transfer.

Both are checked rather than assumed: a part with an address whose `SDA` and
`SCL` reach no GPIO is named and left off the bus, because the emulator does
not route through the GPIO matrix and a device wired to nothing would answer
there and be dead on the desk. `examples/sense-board` is the worked end.

## Not yet

A broader ERC than the findings listed above. EasyEDA's text records beyond
plain labels. A screen that shows what the firmware *drew*: the bus carries
a display's bytes now and is reported, but nothing decodes an SSD1306's
command stream into pixels.

Quantities are half here. A resistor's *value* is read (`nets::ohms`) and
decides one thing: where a pin sits between the rails, which is what
`divider_at` answers and what lets a potentiometer across the rails reach
the converter as real counts. What is still not here is anything measured
in volts or amps — the rails are on and off, because a symbol claiming to
know 3.3 V from 5 V would claim more than it reads, and the current through
an LED depends on a forward voltage the sheet does not carry and would not
be right to guess. A capacitor still never charges: that wants a solver,
which is `docs/kicad.md`'s stage 4.

Four things this section used to list are done: a wire that ends on another
wire, symbols with several units, turning the devkit, and a resistor's value
taking part in the arithmetic.

`.kicad_sch` — reading and writing the schematic file itself, as against the
symbol library — is its own project, and `docs/kicad.md` is its design.
