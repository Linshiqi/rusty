# rusty — embedded Rust workbench

A desktop IDE for embedded Rust, ESP32 first, STM32 next.

It began as "not an editor — it owns the half of the job rust-analyzer does
not". That half is still where the differentiation lives: which chip, which
toolchain, what fits in flash, what is on the serial port, and an assistant that
can call all of it. But a tool you cannot read or change a file in is a
dashboard about work you do somewhere else, so the editor is in scope too:
files, highlighting, and rust-analyzer behind it for completion, diagnostics
and navigation.

That is a deliberate reversal of the original positioning, not drift. Anything
in this file that still reads as "we do not edit code" is stale.

## Commands

```bash
# Everything, the way CI runs it — and it does now: `.github/workflows/ci.yml`
# runs these four on every push and pull request. For a long stretch this line
# was aspirational, the three workflows were all tag-triggered publishing, and
# nothing looked at a push.
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# The frontend links only the model layers, so these must stay green
cargo check -p rusty-core -p rusty-embed -p rusty-ai -p rusty-term \
  -p rusty-edit -p rusty-lsp -p rusty-dbg -p rusty-git --no-default-features \
  --target wasm32-unknown-unknown

# Frontend alone, on http://localhost:17425 — much faster to iterate on than a
# full `tauri dev` rebuild. Anything needing the backend reports that it cannot
# run; the layout, styling and every click that needs no backend are real.
# (They were not, for a while: one boot-time IPC call ran unguarded, the shim
# threw a synchronous TypeError instead of rejecting, and the page rendered
# and then answered nothing. `ipc::backend_available` guards every call that
# runs at mount.) To exercise backend flows here anyway,
# crates/rusty-ui/mock.js stubs the IPC surface: add
#   <link data-trunk rel="copy-file" href="mock.js" /><script src="/mock.js"></script>
# to index.html while debugging and REMOVE IT BEFORE COMMITTING — a test in
# rusty-ui reads index.html and fails while the line is there, because it
# reached four commits before that test existed. It is inert in
# the real app. Three contracts it enforces: responses must carry every
# non-defaulted field (serde rejects, and the error names only the field);
# streaming commands like lsp_start must return a never-resolving promise —
# a resolved stream reads as "server exited" and flips LSP Ready back off;
# and save/open must be stateful like the disk is, or every save-then-reread
# flow (format-on-save) looks broken in the mock while correct in the app.
# A fourth, learned late: **a stub that has not kept up with the model is a
# panel nobody can drive.** `plan_simulation` answered with the *first*
# board's `leds`/`buttons` for two format changes; serde ignored every key
# of it and the sim panel opened on a bare devkit, so nothing in it could be
# exercised here at all — and the chips carried no `gpio`, which draws a
# devkit with rails and no header and makes every wire to a pin a finding.
# It is a divider on a C3 now: two resistors, both rails, a tap on GPIO4,
# and 1.10 V at the middle for anyone to check.
cd crates/rusty-ui && trunk serve

# The whole app
cd crates/rusty-app && cargo tauri dev

# The tools the installer ships beside the app, into crates/rusty-app/bundled/:
# rusty's QEMU (the peripherals), both esp-gdbs, espflash, the LLDB adapter.
# The release workflow runs this before every build; run it once here and
# `cargo tauri dev` uses them too. Not committed — three hundred megabytes of
# binaries belong in a release asset. Rust itself is deliberately not in it;
# the script says why in its header.
scripts/bundle-tools.sh

# Release: push a tag (`git tag v0.2.0 && git push origin v0.2.0`) and
# .github/workflows/release.yml builds installers on Windows (NSIS), macOS
# (universal DMG) and Ubuntu (deb + AppImage), plus rusty-cli for each, and
# publishes a GitHub Release. With the Tauri updater keypair in the repo
# secrets (`cargo tauri signer generate`, then TAURI_SIGNING_PRIVATE_KEY and
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD), it also emits signed updater
# artifacts and latest.json — the feed the in-app updater will poll.

# The simulation pipeline, proven end to end on a real project without the
# window: detect, plan, build, image, boot in Espressif QEMU, count serial
# lines. Needs espflash and qemu-system-* findable (PATH or the data
# directory's tools/).
cargo run -p rusty-embed --example sim_probe -- <project-dir> [seconds]

# The inward half, which sim_probe cannot prove because it never writes:
# a declared sensor, an injected sample, and a controller that answers it in
# the right direction. Then the same with a plant in the middle, which is what
# turns "it responds" into "it settles".
cargo run -p rusty-embed --example loop_probe -- examples/rate-loop
cargo run -p rusty-embed --example flight_probe -- examples/rate-loop

# The workbench without the window
cargo run -p rusty-cli -- check .
cargo run -p rusty-cli -- size target/riscv32imc-unknown-none-elf/release/app
cargo run -p rusty-cli -- size .   # or the project: newest ELF under target/
cargo run -p rusty-cli -- symbol C2286   # an LCSC part as a schematic symbol

# What EasyEDA actually answers for a part, record by record, beside what
# the reader made of it -- how tests/fixtures/easyeda/ was captured
cargo run -p rusty-embed --example lcsc_probe -- C25804 [out.json]
```

## Layout

| Crate | Does |
|---|---|
| `rusty-core` | Cargo workspace analysis: dependency graph, duplicates, feature unification |
| `rusty-embed` | Chips, boards, project detection, toolchain, memory, flashing, wizard, simulation. `model/` is a directory now, one file per concern, re-exported flat so `rusty_embed::X` still names everything; `simulate/` likewise, with the `.rusty/sim.toml` format in `board_file.rs` beside the planner. Three things that are *not* simulation have their own modules, because `simulate.rs` had grown into the place they lived and every other module was importing "the simulator" to reach them: `tools` (finding a binary — one ladder, one order, for every tool), `install` (fetching QEMU/gdb/gcc, version pins), `net` (proxy policy, and the one `ureq` agent builder); `schematic/` is KiCad and EasyEDA — `.kicad_sym` read and written, `.kicad_sch` read and *patched* back (`docs/kicad.md`), an LCSC part fetched — over `model/symbol.rs`, the drawing the frontend renders. And the sheet answers in numbers now: `solve` is modified nodal analysis (DC, a Shockley junction, backward-Euler transient), `circuit` turns a sheet into one and names what the sheet did not say, `live` walks it in step with a running firmware |
| `rusty-ai` | Bring-your-own-LLM providers, the tool registry, the agent loop |
| `rusty-term` | A real terminal: portable-pty (ConPTY) + vt100, rendered by the frontend |
| `rusty-edit` | File tree, syntax highlighting (semantic tokens, not colours), read/write, rustfmt, project search on ripgrep's engine |
| `rusty-dbg` | Debugging, two protocols behind one handle (`any.rs`): `session.rs` is gdb's machine interface, `dap.rs` is the Debug Adapter Protocol for LLDB. Both fold into the same session state — breakpoints, stepping, stack, variables |
| `rusty-git` | The repository's history, Fork-shaped: `graph.rs` lays the log out into lanes and edges (pure, tested — the frontend only turns a lane into an x), `parse.rs` reads `git`'s machine formats, `repo.rs` runs the user's own `git` in the opened project. No libgit2: one binary on PATH is one implementation of the repository format to agree with |
| `rusty-lsp` | rust-analyzer client: stdio JSON-RPC, diagnostics, completion, hover, definition, signature help, code actions, semantic tokens. `client.rs` is the session and the requests; `discover.rs` finds the binary and spawns it, `uri.rs` is the one percent-decoder and drive-letter folder, `convert.rs` turns replies into `model`, `pull.rs` is the diagnostics-pull loop. `positions` is on the wasm side with `model` — the editor converts scalars to UTF-16 at the DOM boundary exactly as the client converts at its own, and it used to do it with its own untested copy |
| `rusty-ipc` | Command-name constants both sides `use`; a test in rusty-app pins each to a real handler |
| `rusty-i18n` | The interface's languages: one TOML catalogue each, a `t!` macro, and the tests that keep them in step. Compiles to wasm — the frontend is the only caller, because backend text crosses the wire as a *name* the frontend translates |
| `rusty-app` | Tauri backend — thin, no analysis lives here |
| `rusty-ui` | Leptos frontend (Trunk + Tailwind, no npm). Four layers: `view` renders and never calls IPC, `controller` is where every cross-layer action begins, `state` holds signals and pure operations on them, `ipc` is transport. `ipc::call` appears in `controller/` and nowhere else — check that with a grep before believing it. **Anything that grows past ~1,000 lines is holding more than one concern**: `controller/`, `view/panels/files/`, `view/settings/` and `view/dock/` are all directories now, one module per thing, and each was one file that had accreted six to fifteen |
| `rusty-cli` | Headless entry point; the CI and bug-report surface |

## The rules that are load-bearing

### 1. Every crate is split by a `backend` feature

`model` (and `catalog`'s data types) compile to `wasm32` and contain no IO.
Everything that spawns a process, reads a file, or walks a graph sits behind
`backend`. The frontend takes each crate with `default-features = false` and
`use`s the model types **directly** — there is no generated binding layer, so
the wire contract cannot drift.

Adding a field to a model type is free. Adding an `std::process` call to one is
a build break on wasm, which is the point.

### 2. File formats and wire formats are different types

`catalog.rs` parses TOML into its own structs and converts to `model`. The file
format is a public contract with users who write board definitions; `model` is
an internal contract with the frontend. Tying them together means a UI refactor
silently breaking everybody's board files.

### 3. Refuse rather than guess

The recurring failure mode in this domain is a plausible answer that sends
someone down a wrong path for an hour. So:

- No probe-rs target for an STM32 → error telling them to run `probe-rs chip list`,
  not a guessed name with the wrong memory map.
- A chip not in the catalogue → `known: false` **and** a note telling the model
  not to describe it from memory.
- A serial flash for a part with no serial bootloader → refuse and say why.

When a tool cannot answer, it says what is missing in terms the caller can act
on. Silence makes a model invent something.

### 4. The analyses are the assistant's tools

`rusty-ai`'s registry wraps `rusty-core` and `rusty-embed`. The assistant does
not read `.cargo/config.toml` and theorise; it calls `project_status` and gets
the actual mismatch. Tool descriptions are written against the specific failure
they exist to prevent, not as feature summaries — see `tools/embedded.rs`.

Adding an analysis means adding a tool. The same definitions are intended to
back an MCP server later, so third parties get them too.

### 5. The simulator's contract is one serial line

Espressif's QEMU boots the same merged image `espflash` would burn, and
everything the board view knows travels as text on that one serial line:
`[rusty:gpio] 26=1,27=0` (or `[rusty:gpio@1234] …` with the systimer in
microseconds — what the Waves panel and the VCD export time by) and
`[rusty:disp] hello` out of the firmware,
`B14=1` and `P34=128` into it. `protocol.rs` owns the parsing, compiled
unconditionally because the frontend reads the stream as it passes.

The same line carries the Plot panel, which is what a control loop is
developed against rather than a debugger — stopping a flight controller to
read a variable means the craft falls. `[rusty:tel@1234] gyro_x=1.25,pid_p=-0.5`
is a sample on arbitrary named channels; `[rusty:param] kp=2 0..20` announces
a tunable with the range it accepts; `Skp=8.5` sets one and the firmware
answers with the `[rusty:param]` line carrying what it actually **took**, so
a clamp reads as a clamp. No slider is drawn without a range the firmware
gave: a range the tool invented is how somebody sends a gain of 500 to a
motor loop. `examples/pid-tune` is the worked end of it, proven in QEMU —
`Ssetpoint=80` moved the plant, `Ssetpoint=500` came back as 100, and an
unknown name changed nothing and said nothing.

**A boolean channel cannot carry a control loop, so three lines carry
numbers.** `[rusty:pwm] 5=0.75` is how hard a pin is driven rather than
whether it is high — reported per *change*, because timing the `[rusty:gpio]`
edges would be the more honest measurement and is unavailable: 1–20 kHz is
thousands of edges a second on the line the console shares.
`[rusty:sensor] gyro=3 rad/s -35..35` declares a sensor the firmware wants
fed and `Igyro=1.25,-0.5,0.02` feeds it; `A34=2900` puts raw ADC counts on a
pin. Counts and not volts, because rusty does not know anybody's divider and
a claimed 3.7 V that the firmware's arithmetic disagreed with is the
confident wrong answer in miniature.

The inward half is what makes a flight controller simulatable at all. QEMU
models no I2C and no SPI slave, so firmware reading an MPU6500 reads nothing
and the attitude loop — the whole of the thing — could not run at a desk.
The declaration is the tunables' rule pointed the other way: a panel that
offered `gyro` because a drone usually has one, over a range it chose
itself, would one day inject 2000°/s into a loop written for 250.

**A sample travels whole or not at all.** Split across three lines the
firmware can read x from one moment and y from the next, and an attitude
fused from a torn sample drifts in a way that looks exactly like a bad gyro.
`examples/rate-loop` is the worked end, and `loop_probe` is the check that
`sim_probe` cannot be — it writes, and it requires rolling each way to move
the motors the *opposite* way, because a loop that answered every injection
with the same asymmetry would pass every weaker test while being wired
backwards.

**Injection alone is an open loop, so there is a plant.** A rate fed in never
changed because the motors spun, which catches a reversed axis and says
nothing about whether a loop settles. `rusty_embed::plant` is the integrator
between the two: duties in, body rates and an accelerometer out, run by the
panel on a timer and by `flight_probe` headless. Orientation is a quaternion
because angle accumulators get rotation order and gimbal lock wrong, and the
tests pin exactly that. Carrying an orientation is what lets it synthesise
gravity in the body frame, which is what makes a *fusion* filter testable —
the larger half of the reason it exists.

It is a model and says so in its own header: no translation, so it is a drone
on a test gimbal; no aerodynamics past damping; and no claim about anybody's
aircraft. **What transfers is the sign of each axis, the order of the motors,
and whether the loop is stable in shape — not the gains.** `flight_probe`
holds it to that: a gust at the firmware's gains must come back, and the same
gust at the top of a declared range must look visibly worse, because a plant
that showed every tune settling would quietly reassure about all of them.
Measure overshoot and sign changes rather than peak rate — the mixer clamps
each motor to 0..1, and a gust big enough to saturate it makes every gain
command the same thing.

The reading of that protocol lives in *one* function (`controller::absorb`),
because it was per-stream once and the consequence was telemetry that plotted
in the simulator and vanished on hardware.

Debugging rides the same boot: `-s -S` freezes the CPU with the gdbstub on
:1234, and the terminal attaches the matching esp-gdb (`break main` lands in
the user's source with full backtraces — proven against the real blinky
image). Espressif's prebuilt QEMU has the plugin interface compiled OUT
("plugin interface not enabled in this build"), so register-level tracing
needs our own QEMU build one day; until then the gdbstub's watchpoints are
the honest bridge to register truth.

That is a deliberate ceiling, and it is one the *stock* emulator imposes:
Espressif's `esp32_gpio_write` is an empty function, so a pin has no state in
either direction — which is why probing the real register addresses over QMP
reads zero on esp32 and esp32c3 alike. The board therefore shows *what the
firmware says it set*, and the panel says so in as many words. A part needs no
code in rusty to exist, which is why a KiCad `.kicad_sym` under
`.rusty/symbols/` — or an LCSC part number typed into the library panel — can
add one.

That ceiling is now the *fallback*, not the roof. `qemu/` holds real device
models — Espressif's GPIO stub replaced, and beside it the SAR ADC, the I2C
master and `SPI2`, none of which upstream maps at all — and
`qemu-release.yml` builds them for four platforms and publishes them. Four
peripherals in one file (`esp32_gpio.c`, which keeps upstream's name because
it replaces upstream's file) on purpose: all four carry the host's view of
one board and share its socket, so "one channel, one protocol, one reader"
stays true on the emulator's side as well as rusty's, and a new file would
mean a new entry in upstream's build system for each. **The installer ships it**: `scripts/bundle-tools.sh`
unpacks the `qemu-v*` asset into `crates/rusty-app/bundled/`, `bundle.resources`
packages that directory, and the app hands Tauri's resource directory to
`tools::set_bundled_dir` at setup, so a fresh install simulates with ours and
never sees the stock build's notice. The bundle sits *after* the data
directory in the finder's ladder (a copy the user installed on purpose wins)
and only counts when its `PLATFORM` file names this machine — a universal
macOS app carries the arm64 QEMU, and an Intel Mac must fall back rather
than try it. `qemu_download` is the fallback for that case, for the CLI and
for a checkout that never ran the script: ours first, then Espressif's. With ours a LED lights because a pin went high and a
button is read through the register the firmware actually reads; with theirs
everything works exactly as it always did. `qemu/README.md` has the gates each
build passes and why each can fail.

**Which of the two is running is never assumed.** `has_gpio_model` reads the
binary for a marker only rusty's build emits, because a user who dropped a
stock QEMU into the same directory has to get the right answer. The run then
announces `[rusty:pins] emulator`, and the board's caption follows that rather
than asserting — a caption promising register-level truth over a stock build
would send somebody with a dark LED to check their wiring when the bug is a
missing `println!`, and the reverse is just as wrong. An announcement the
frontend does not recognise leaves the weaker claim standing. And the stock
build **says what it costs**, in the dock at every run (`[rusty:pins]
firmware — …its GPIO write handler is empty, so is_set_high()/is_high() read
0…`) and on the Simulate panel (`SimPlan.emulator`, whose `gpio_model` is
`has_gpio_model` of the binary the plan will boot, with an Upgrade button
that runs the same download over the copy that is there — the Toolchain
panel never lists QEMU, only the simulator's plan does).
Found when a user's blinky printed `GPIO2 high :false` for ever under a
QEMU installed before rusty's build existed: `toggle()` reads the output
register back, the stub never stores it, and nothing on screen said the
emulator was the reason. Measured both ways on one image — stuck at `false`
under Espressif's build, alternating under ours.

The pin channel is a chardev of its own (`-chardev socket` + `-global`, since
the machine creates the GPIO device and there is no `-device` to hang it off),
and the backend feeds its lines into **the same** stream the serial console
uses — rule 5 above, one `absorb`. A button press goes both ways when both
exist: `B14=1` on the console for firmware reading rusty's text protocol, and
`14=1` on the pin channel for firmware reading `Input::is_high()`.

**That one channel now carries four peripherals**, because all four are the
host's view of one board. `A<pin>=<counts>` puts an analog value on a pin and
`adc.read_oneshot()` returns it; `i2c 68:75=68` puts a byte behind an I2C
address, `i2c 3c=+` declares a device with nothing to read and `i2c 3c=-`
takes one off; `spi 0=1a68` is what a chip select answers with. Back the other
way, `[rusty:adc@<us>] <pin>=<counts>` says what the converter handed over,
`[rusty:i2c@<us>] 3c w 00ae` what crossed the bus and `[rusty:spi@<us>] 0 w
aea501` what crossed the wire. All are reported per *change* — a driver
polling a sensor converts thousands of times a second — and all go through the
same `absorb`. The buses remember their last report **per verb**: a
`write_read` alternates a write and a read, so one shared slot suppresses
nothing.

The potentiometer's `P34=128` stays console-only, deliberately: what a wiper
converts to depends on what its two ends are wired to, and turning 128 into
counts would assert a rail-to-rail divider nobody stated. `A` already carries
the number the firmware's own ADC would have produced, so it needs no
conversion to reach the model — which is why it is the message that travels
down the channel.

`examples/sense-board` is the worked end of all three: a knob on the ADC and
a sensor on the bus, read with `adc.read_oneshot()` and `i2c.write_read()`
and nothing else. The firmware knows nothing about the simulator; the sheet
says what is on the pin and what the sensor answers, and the numbers in the
Plot panel follow both.

**A part is on the I2C bus because it carries an `addr` prop and is wired to
one**, not because of its kind: a sensor, a display and a breakout imported
from LCSC all reach it the same way (`nets::bus_devices`). `cs` and `miso` do
the same for SPI (`nets::wire_devices`). The wiring is checked rather than
assumed — the emulator's buses do not route through the GPIO matrix, so a
device with no wires would answer there and be dead on the desk, which is the
confident wrong answer in miniature. No address is not an address of zero:
absence refuses.

### 6. Extensibility is data first

See `docs/extensibility.md`. Chips and boards are TOML in three layers
(built-in < user config < `<project>/.rusty/`); the simulator's parts are KiCad
symbol libraries in `<project>/.rusty/symbols/`, or LCSC parts imported by
number into the data directory's `symbols/`; a chip's register description is
the vendor's own
SVD, found in `<project>/.rusty/svd/` or the data directory and fetched on
demand — never bundled, because a vendor file is a hundred thousand lines of
XML nobody wants in a git repository by accident. Code extensions go through MCP. UI contributions are
declarative — extensions never ship markup or styles.

## Modal editing

Vim keys in the editor, off by default, switched on from View. `vim/` is a
pure state machine — `(keys, text, cursor) -> Step`, no DOM — under 45 tests
that name the property a Vim user would notice missing. The editor reads
`Step` and does only the three things a browser forces on it: set `value`,
set the selection, `preventDefault`.

**The precedence is the whole design, and it is what makes it liveable.**
Normal and visual mode own *unmodified* keys; no global binding uses one, so
the overlap is almost nothing. Chords stay with the editor and the globals
except five — `Ctrl+R`, `Ctrl+O/I`, `Ctrl+D/U` — so Ctrl+S, Ctrl+K and
Ctrl+A are untouched. Insert mode claims only Escape, which is why
completion, quick fixes and every learned shortcut keep working the moment
you type. `Step::handled` false is that path; `stop_propagation` on the taken
ones is what stops a `d` in normal mode also reaching the window listener.

- **The block cursor *is* the selection.** Normal mode selects the character
  under the cursor, so styling that selection is the cursor — no second
  element, and no way for the two to disagree about where they are.
  Translucent, because the textarea's glyphs are transparent by design and an
  opaque block hides the character it points at.
- **Indices are Unicode scalars**, converted to UTF-16 at the DOM boundary
  exactly as the LSP client converts at its own. A `中` in the buffer must not
  shift every motion after it.
- **`Span` carries two numbers on purpose.** `e` puts the cursor *on* the
  word's last character and `de` deletes *through* it; a motion returning one
  number gets one of the two wrong. Bare motions use `cursor`, operators use
  `start..end` — and using `end` for both was the first bug, visible as the
  cursor landing one past every word.
- **What it does not know, it names**, in the status line beside the mode. A
  key that vanishes teaches people the editor is broken. `Ctrl+O`'s jump list
  says so today rather than pretending.
- **The switch is `workbench.toml`.** A second window must boot into the same
  mode — landing in the wrong one is not a shrug, it is twenty keystrokes
  doing something else. It loads in `restore`, not in a project-reopen branch:
  put there first, it was written and never read.
- Undo granularity is Vim's, not the editor's. `Step::seal` closes the unit
  at a command boundary, so `ciwfoo<Esc>` undoes in one press instead of one
  keystroke at a time.

## Code folding

**Folding is the one feature that makes the text on screen stop being the text
in the file**, and every other rule here follows from that. The surface is a
transparent `<textarea>` over a highlighted `<pre>` that line up glyph for
glyph, so hiding a region means removing it from the textarea too — and from
that moment the caret, the selection, every squiggle and every card is
positioned in a coordinate system that is not the document's.

- **The arithmetic is in `rusty_edit::fold`, pure and tested.** Which regions
  can collapse (indentation, as VSCode's default is — a file mid-edit does not
  parse), the two line conversions, and `splice`, which turns an edit made
  against the folded screen back into an edit against the document.
- **`view/panels/files/folding.rs` is the only place that reads them off
  state.** `screen` is what the textarea holds; `row_for` is what every
  overlay anchors to. A line inside a collapsed region answers `row_for` with
  the *header* rather than with nothing, so an error in a folded function
  marks the fold instead of disappearing.
- **What made this safe to add to a working editor: with nothing folded,
  `screen` is the draft and `row_for` is the identity.** Every existing call
  site behaves exactly as before, so a site that was missed is a misplaced
  overlay while something is collapsed — never a wrong write.
- **One keystroke path is fold-aware; every wholesale rewrite is not.** Cut,
  paste, undo, comment toggle, completion accept, replace-all, a Vim operator
  and a format all compute a new *document* and hand it to `set_value`. They
  go through `set_buffer`, which expands the folds first. Twelve separate
  splices would be twelve chances to write the wrong bytes to disk; unfolding
  makes the two texts the same text again. Only `on:input` splices, because
  only it carries the screen after the edit.
- Folds are session state, parked per tab like the caret, and **not
  persisted**: restoring yesterday's folds onto a file somebody else has since
  edited collapses the wrong lines.

## Bracket pairs, and what a keystroke asks the server

- **Four rules, pure and tested, in `view/panels/files/pairs.rs`.** An opener
  brings its closer with the caret between; a closer typed against its twin
  steps over it; Enter between `{|}` puts the caret on an indented line with
  the closer below; a closer on a blank line takes its opener's indentation
  (the matching opener, found by depth, not the nearest); Backspace inside
  an empty pair removes both; an opener typed over a selection wraps it.
  Single quotes are left alone — `'a` is a lifetime. A `"` inside an
  unterminated string closes it, and after a word it is a typo being
  corrected, except after `r` and `b`. Every rule returns an `Edit` in
  document bytes that `apply_edit` puts through the same record / echo /
  `set_buffer` path as any other write, so undo, the echo and the folds
  cannot disagree with it. Vim's insert mode passes every key but Escape, so
  the rules hold there too.
- **A prevented key never reaches the input event.** The completion and
  signature triggers lived in `on:input`; a `(` that opened a pair without
  asking for the signature would have taken a feature away by adding one.
  `typed_triggers` is the one place the character behind the caret is
  judged, and both paths call it.
- **The caret is a screen position; edits are document edits.**
  `selectionStart` indexes the folded text. `doc_selection` maps it through
  the fold table and is the identity with nothing folded; Enter, Tab and
  every pair go through it. Before, a fold above the caret put the newline
  at the wrong byte of the draft.
- **The popup owns its keys only while it shows rows.** `visible_items` is
  the one filter, shared by the view, the accept and the key handler.
  `Some` alone was the test, and a popup narrowed to nothing was invisible
  yet still ate Enter and Tab, so a line ending in `v.xyz` could not be
  broken.
- **rust-analyzer's item order is arrival order; its ranking is `sortText`.**
  Taking the first hundred *unsorted* shipped a hundred arbitrary slice
  methods for `v.` and dropped `len`; typing `le` then narrowed the popup to
  nothing — an editor with no completion, reported in exactly those words,
  while every request and reply was correct. `convert::completion_items`
  sorts first and caps at 400.
- **rust-analyzer offers an unimported item only to a client that can
  resolve `additionalTextEdits` lazily.** `enable_imports_on_the_fly` is
  gated on `completionItem.resolveSupport` naming that property — computing
  a `use` line per candidate eagerly is too slow — so a client without it
  gets no `Output` for `Out` in a file that lacks the import, and no
  `Output::new` after, since the path does not resolve. Reported as "still
  no completion", with a hover of `{unknown}` for the variable, which was
  correct. The client declares it; `completion()` keeps the raw reply;
  `resolve_completion(path, index)` asks `completionItem/resolve` for the
  accepted item and the frontend splices the edits above the caret, shifting
  it by what was inserted. `label_detail` carries the ` (use …)` note so the
  row says what accepting it will add.

## Two editor groups

Side by side, VS Code's everyday split, and no more than two. The editor was
written for one group — every component, controller and effect reads
`state.editor` — and the second group did not need a second editor.

- **A group is an `AppState` with `editor` and `find` swapped.** `AppState`
  is a bundle of `Copy` signal handles, so `state.group(Second)` is a copy
  pointing at the second group's signals with everything else shared, and
  `EditorGroup` (`view/panels/files/editor.rs`) provides it as the context of
  its subtree. `AppState::expect()` below it answers with that state, and
  every controller called from there works on that group without knowing
  there are two. What is *not* below — the tree, the finder, a search hit,
  the palette's Back — asks `state.focused()`, which follows the pointer and
  the focus (`layout.focus`).
- **Shared handles stay shared.** `Editor::beside` copies the tree, the
  expanded folders, the text zoom, the Vim switch, the source-view choice and
  the stale list from the first group; only what is open and how it is being
  edited is fresh. Separate copies would be a zoom that took on one side only.
- **One group per file.** Two drafts of one path would overwrite each other
  on save, so `open_file` fronts a path the other group holds and focus
  follows; "Open to the side" and the strip's split button *move* a file
  (`transplant`: draft, caret, history and all). `is_dirty` and `follow`
  look at both groups because of this rule, not in spite of it. VS Code
  would open a second copy; the same file in two groups needs a document
  model shared between them, which this editor does not have yet.
- **"Beside" is the right group, from either side.** The first version sent
  a file to *the other* group: from the right group that moved it left, and
  when it was the right group's last file the right group vanished under
  the click. Files move into the right group and never out of it; the right
  group closes only when its last tab does; the split button and "Open to
  the side" appear only in the left strip, because there is nothing further
  right of the right group. Ctrl+\ acts on the left group whichever has
  focus, for the same reason.
- **The split never shows an empty pane.** `settle_groups` closes a second
  group that lost its last file, and a first group that lost its last file
  takes the second's files — so the layout is never "nothing on the left,
  the work on the right". The split button wants a second tab to leave
  behind for the same reason.
- **Both strips persist** in `workbench.toml` (`ProjectTabs.second`), and the
  split comes back with them; the divider between the groups is
  `Divider::EditorSplit`, in permille like the diff's.
- **The title bar's centre holds the finder's icon and the project's verbs,
  and nothing else.** A command-centre search box was tried there for one
  release and read as a second search field in front of the finder's own,
  and the project's name it carried as a placeholder said nothing anybody
  needed — the status bar names the chip. The icon, Ctrl+P and View ▸ Go to
  file open `view/quick.rs`, whose candidates are the tree the Files panel
  already holds, flattened — no second walk to keep in step with the first.
  Ranking is pure and under tests. Ctrl+P, Ctrl+B (fold the tree) and
  Ctrl+\ (split) are VS Code's chords, so hands that know them need not
  learn ours.

## The Git panel

The repository, with Fork as the reference for what it should look like.
Three views (`state::GitMode`) behind one branch picker — a button showing
the checked-out branch, marked, or the branch the log is filtered to when
that is another one, opening a menu of them all with the filter in force
highlighted (a repository with thirty branches is ordinary and thirty chips
are a paragraph nobody reads; and the checked-out branch used to be repeated
beside the button, which read as clutter): *History* — a graph
of lanes beside the commits, labels on the commits that carry branches and
tags, a commit opened below with its files and each file's patch; *Changes*
— the working tree as staged and unstaged lists, a file's diff, the commit
box; *Stashes*. The branch row's right end carries refresh, fetch, pull, push
and a new branch.

- **Reads are IPC; writes are dock commands.** The log, a commit, the status,
  the stash list and one path's diff answer with model types and touch
  nothing. Commit, stash, checkout, branch, fetch, pull and push run through
  the same runner every `cargo` uses, so the exact `git` line and everything
  it says back are in the dock — a checkout refused on a dirty tree or a
  rejected push is a paragraph there, not a banner nobody can act on. The
  one quiet write is staging: `git add` on a path is instant and reversible,
  and a dock line per click would bury the commands that matter.
- **Changes is two columns, not a stack.** Files and the commit box on the
  left, the diff on the right at full height — Fork's and VS Code's shape.
  Stacked, the diff at a fixed fraction and the commit box at its natural
  height took the space between them, and the file lists — the thing the view
  is for — were three rows tall in a panel two thousand pixels wide.
- **Every boundary somebody would want to move is a `Divider`.** The log
  against the opened commit, the message against the files, the files against
  the patch, the Changes view's two columns: four more variants, each with a
  default, bounds and a storage key beside the sidebar's and the dock's, so
  Reset layout and the boot restore hear about them for free. The message's
  divider sets a *cap* rather than a height — a one-line message never fills
  it — and `Divider::travel` is the one place that says which way each one
  grows; a divider added to the enum and not to that match drags sideways.
  The fifth, where old meets new in a side-by-side diff, is in *permille*
  of the text width, because half is the right default at every pane width
  and pixels cannot say "half": `drag_from` carries how many units a pixel
  of travel is worth (1.0 for the pixel ones), the diff's grip measures the
  grid on grab to set it, and `split::grab` is the one entry both kinds of
  handle go through.
- **A diff is read once, in `rusty_git::diff`, and laid out two ways.** git
  speaks unified text and nothing else; side by side is a reading of it —
  removed and added runs paired index for index, the longer run overhanging,
  `\ No newline at end of file` on the side of the line it is about — and the
  pure reader is under tests that pin Fork's exact shape. The hunk carries
  both readings (`rows` and `lines`), because rebuilding git's order from the
  paired rows puts a note after the additions it preceded. The view only
  decides what a row looks like; the toggle is remembered. The hunk header
  is drawn once per side, as Fork draws it — one header across both columns
  crossed the centre line, and a row crossing it reads as a layout that has
  come apart.
- **The right-click menu is local to the thing under the pointer**: a commit
  offers copy hash, a branch from here, a detached checkout, cherry-pick and
  revert; a commit's file offers open and copy path; a file in the Changes
  view offers Fork's list — stage or unstage, discard (asking first, in
  words that say whether the file goes back to the index or to the last
  commit; an untracked file is deleted, `clean -f` on that one path), stage
  all, stash this file, copy path and full path. Which list it was clicked
  in travels with the target (`GitTarget::Change`), because discard means
  three different things across the two lists. Every write in it is the same
  dock command a button would run, and the panel's own `contextmenu` handler
  swallows the browser's menu everywhere else.
- **A commit asks who you are before git refuses.** `git_identity` reads
  `user.name` and `user.email` as `git config --get` resolves them (exit 1
  is "unset", an answer); when either is missing the commit box shows a
  name-and-email form and disables Commit, and Save runs `git config
  --global` — or local, git's own two offers — as dock commands, then reads
  the identity back. Found on a fresh machine as "Author identity unknown"
  in the dock after the button, which is the wrong place to learn it.
- **Amend fills the box with HEAD's whole message** before anything is typed,
  because `--amend -m` with only a summary would silently cut an essay down
  to its first line; an amend with the box empty is `--no-edit`. A stash is a
  commit, so clicking one opens it below the list through the same `git show
  -m --first-parent` the History pane uses — that *is* the working tree it
  holds — plus its **third parent** when it was saved with
  `--include-untracked`: the untracked files live there, the first-parent
  diff never reaches them, and a stash of one new file opened as "no files
  changed" while `git stash list` plainly held it. `untracked_parent` knows
  that parent by git's own subject (`untracked files on …`), so an octopus
  merge is not mistaken for a stash. Switching views drops the selection,
  since the pane would otherwise describe something no longer listed. And
  the log runs with `--exclude=refs/stash` before `--all`: a stash's parents
  are commits, and one `git stash` put three rows and two lanes that no
  branch owns into the graph — stashes are read in their own view.
- **A commit message is one argument however many lines it has.** The dock's
  line runner splits on whitespace, which would tear a message at its first
  space; `run_args_at_root_then` takes an argument vector and hands it to the
  process as-is, and `shell_word` quotes it *for display only*. Anything
  passing user prose to a command goes that way.
- **`git diff` exits 1 when there is a difference**, which is the answer
  wanted, so `run_allowing` treats the codes a caller names as success and
  `run` is the `&[0]` case. An untracked file's diff is `--no-index` against
  `/dev/null`, a name git resolves itself on every platform, Windows included.
- **Status is porcelain v2 with `-z`**, the one format whose field layout is
  a contract: branch headers, `1`/`2`/`u`/`?` entries, a rename carrying its
  old path as the next NUL-separated token. Parsed once, in `parse::status`,
  under tests pinning the real output.
- **Branch delete is `-d`, never `-D`.** A branch whose work is merged
  nowhere is refused, and that refusal in the dock is the right answer;
  force-deleting is a decision for a terminal, not a button. The new-branch
  field creates from the branch selected in the strip, or from HEAD.
- **A push with no upstream sets one** (`-u origin <head>`), because a bare
  `git push` on a new branch refuses with a hint nobody reads. Stash is
  `push --include-untracked` — "everything I have" is what the button says.
- **The graph is laid out on the backend and only drawn on the frontend.**
  `rusty_git::graph::lay_out` is pure and under tests that name the shapes
  that go wrong — a merge, two tips, a branch bending back into a lane that
  was waiting for it, a lane reused once free. The view turns a lane index
  into an x coordinate and nothing more, so which lane a commit sits in is
  one fact rather than two opinions.
- **One row is one SVG whose lines run past its bottom edge.** Each row knows
  only its outgoing edges (this row's centre to the next row's), so a line is
  drawn by the row it leaves and `overflow: visible` lets it reach the row it
  arrives at. A lane a commit *opens* for a second parent has no line arriving
  from above, and the layout must not emit one — it did, and every merge grew
  a stray tail. The SVG is `relative z-10`, because the next row's hover or
  selection fill is painted after it and covered the part of the line that
  had crossed into that row — the graph looked cut at whichever row the
  pointer was on.
- **Lane colours are fixed hex, the board sheet's exemption applied again**:
  a commit graph is the same colours in every client that draws one, and a
  lane that changed colour with the theme would read as a different branch.
- **`git`, not libgit2.** Every question is one invocation with a machine
  format — `%x1f`/`%x1e` separators for the log, because a subject can carry
  tabs and newlines; `--name-status` and `--numstat` for the files; the patch
  split on `diff --git`. The user's own git, config, credentials and hooks;
  `GIT_PAGER=cat` and `GIT_TERMINAL_PROMPT=0` because a git that waited on
  either would hang the panel. Not a repository is a sentence in the panel,
  not the banner: it is an ordinary thing to open — and the one refusal the
  panel can fix, so it carries an Initialize button that runs `git init`
  through the dock. The panel knows *which* refusal it got from
  `CommandError.kind` (`not-a-repository`), the stable name beside the
  prose, rather than by matching the English.
- **A merge is shown against its first parent** (`-m --first-parent`), as
  Fork does — `git show` on a clean merge prints an empty combined diff,
  which reads as "this merge changed nothing".
- **Clone is the one git command that runs with no project open**, because
  it is how a project arrives: `git_clone` streams like an install step
  (same session slot, same dock), and the frontend opens the checkout on
  exit zero. The directory it creates is `rusty_git::repo_name(url)` — what
  `git clone` itself would pick — and the dialog says so before running.
- **An image is compared as pictures, not as a patch.** `is_image_path` is
  decided by extension in the model crate so both sides agree; `git_blob`
  hands back base64 of `git show <spec>:<path>` (a hash, `HEAD`, or `:0`
  for the index) or the working-tree file, and the frontend builds a
  `data:` URL. Which two specs to compare is the caller's knowledge: a
  commit's first parent against the commit, `HEAD` against the index for a
  staged change, the index against the disk for an unstaged one, and no old
  side at all for a file git has never seen.
- **A commit tears off into its own window** through the same
  query-parameter boot the detached editor uses (`?gitdiff=<target>`,
  `query_param` in `state.rs`); the window reattaches to the backend's
  project and shows `Detail` standalone. Hide folds the pane to a strip and
  is session state.
- **After any write, everything is read back** (`after_git`): history,
  branches, status and stashes, and the tree — so the panel never shows a
  state git has already left. The open files follow through the watcher like
  any other change to the checkout.
- **The history follows the disk.** `.git/` is a dot directory and unwatched,
  so the refresh rides on the working-tree batches a commit, checkout or
  fetch produces; it is a no-op until the panel has been opened once, so a
  project nobody looks at the history of costs no `git log` per save.

## Following the disk

The workbench is never the only thing writing to a checkout. `rusty_edit::watch`
is `notify` behind a quiet window; `controller/watch.rs` decides what to do
with a batch.

- **`target/` is not watched.** One `cargo build` writes tens of thousands of
  files, and a watcher that reported them would spend the build storming the
  frontend. Dot directories go the same way — the tree does not draw them.
- **Batched, and structural changes are told apart from content ones.** One
  Ctrl+S in another editor is up to four notifications on Windows; a `git
  checkout` is one action that takes a second of syscalls. Re-reading one open
  file is free and walking the project is not, so a modify says "this file"
  and a create/remove/rename says "the tree".
- **An unsaved draft is never reloaded — it is marked.** The tab shows a
  warning beside its dirty dot and the reload is skipped. Silently replacing a
  draft with the disk's copy is an editor eating work, and a modal prompt per
  file is unusable after a checkout that touched a dozen. The reload is
  re-checked *after* the round trip too, because typing is synchronous and the
  read is not.

## The build directory

A Rust `target/` only grows: every `cargo update` leaves the old version of
each moved dependency compiled beside the new one, every dropped dependency
leaves its artifacts, every crate's incremental cache outlives its last
change. One machine here had 140 GB under one project's `target/` and a
full disk, and the compiler's report of that was `IO failure on output
stream`. `rusty_core::disk` is the answer; the Crates panel's Disk section,
`rusty-cli disk` / `sweep` and the assistant's `disk_report` tool are its
three consumers, as with every other analysis.

- **Stable cargo records nothing about when an artifact was last used.** A
  build that finds a unit fresh touches none of its files — `-Z mtime-on-use`
  is nightly-only, checked empirically before this was written — so "recently
  used" cannot be read off the filesystem, and cargo-sweep's default mode
  does not work on stable. What can be read is *what the build needs today*:
  the resolved graph from `cargo metadata`. Every unit's dep-info file names
  its source, and a registry source names `<name>-<version>`. Stale means:
  a version the lockfile no longer resolves, a package no longer in the
  graph, an incremental cache idle past the threshold (a week by default,
  adjustable in the section), or an incremental cache beyond a crate's
  newest four — rustc keys the cache on the unit's flags, so every feature
  set, profile override and wrapper leaves one, and this workspace had a
  hundred per crate and 77 GB of them. Nothing else — the same version
  built by another toolchain looks identical and is kept. A whole tree's
  `incremental/` can also be dropped on request; it is a cache and rustc
  rebuilds it. An empty dep-info file is a compile that never finished and
  is left to cargo, which rebuilds the unit regardless.
- **Nothing is deleted on a guess.** A dep-info the scan cannot read, a
  fingerprint record not in the shape it knows, an empty yardstick because
  `cargo metadata` failed: each marks nothing stale and lands in the
  report's `warnings`. Build-script run directories are linked to their
  compiled script through cargo's fingerprint record (`deps[..][3]` is the
  script's fingerprint hash, and the script's own record holds that hash in
  hex); when the record is unreadable only a package gone from the graph
  condemns them.
- **A removal re-scans; the frontend never sends paths.** `sweep` takes a
  policy and a tree, `remove_tree` a path the scan itself lists (a
  `<profile>` or `<triple>/<profile>` tree, or an extra it knows —
  `doc`, `rusty-sim`, `tmp`, `flycheck*`), and both refuse a tree whose
  `.cargo-lock` is held: cargo and rust-analyzer's check both hold it for
  the whole of a build. Whole trees and cargo's caches ask first with the
  size; a sweep does not, because the table is its preview and nothing it
  removes is needed by the build as configured.
- **A cargo command is refused below 2 GiB free** on the volume it would
  write to, with the number, before it starts. `cargo_writes` names the
  verbs that write; `cargo metadata` and friends are not guarded.
- **The auto-sweep is opt-in and lives in `workbench.toml`**
  (`disk_auto_sweep`), read by the backend at the end of every successful
  cargo command from the dock, which then reports what it removed in the
  same output. Off by default: deleting without being asked is not a
  default even when the deletion is safe.
- **rusty does not write the user's cargo config.** A shared build
  directory (`build.target-dir` in `~/.cargo/config.toml`) is the biggest
  saving of all — each dependency compiled once for every project — and the
  Disk section says so, shows the exact snippet with the absolute path, and
  copies it. The simulator, `size` and the tool follow `cargo metadata`'s
  `target_directory`, so a shared directory needs no other change.

## The first run

A freshly installed workbench on a machine with no Rust could do nothing, and
said so only if somebody found the Toolchain panel and worked out which of six
buttons to press first. Every piece needed to fix that already existed — the
probe, the recipes, the archive downloads — and none of it ran unless asked.

**Most of it does not have to be fetched at all now: the installer carries
it.** `scripts/bundle-tools.sh` puts rusty's QEMU, both esp-gdbs, espflash and
the LLDB adapter into `crates/rusty-app/bundled/`, which ships as a Tauri
resource — three hundred megabytes unpacked, and the difference between an
install that is a workbench and one that is a list of things to go and find.

- **The bundle is a fallback, not a preference.** `tools::find` reaches it
  after the data directory, cargo's bin and PATH, so a copy the user
  installed on purpose still wins — "check the environment, skip what is
  there" answered at run time, per tool, rather than by an installer copying
  three hundred megabytes into the data directory and needing elevation to do
  it. The one exception is `bundle_wins`: rusty's QEMU, because a stock
  `qemu-system-riscv32` wears the same name and has none of the peripherals.
- **Rust itself is deliberately not in it** — rustup, cargo, the standard
  library, espup's Xtensa fork. Which toolchain a project needs is decided by
  its `rust-toolchain.toml`, rustup is the only thing that installs them
  correctly, and a copy frozen into an installer goes stale in six weeks. The
  setup screen asks for those, which is the honest shape for a dependency the
  user has to own. The C cross compilers are out too: four hundred megabytes
  each, for a case the bundle could not complete on its own anyway, since
  Xtensa needs espup regardless.
- **Not every desktop gets every tool, and the script says which.** Espressif
  publishes no macOS esp-gdb, so CodeLLDB is that platform's debugger;
  CodeLLDB is *not* bundled on Linux, because it carries a hundred and thirty
  megabytes of host LLDB and the AppImage bundler walks every ELF among the
  app's resources — the failure QEMU's firmware directory already caused
  once. Linux has esp-gdb in the bundle instead.
- **"Not published" and "the download failed" are different answers.** The
  first version treated any `curl` failure as the first, and a network that
  drops TLS connections produced an installer quietly missing two tools,
  blamed on a platform that publishes both. The status code decides now, and
  anything that is not a clean 404 or a clean download stops the build.
- **Each tool is proven by the binary rusty asks for**, not by the archive
  having unpacked. `simulate::find_gdb` looks for `xtensa-esp32-elf-gdb` —
  Espressif builds that family per chip and ships no plain
  `xtensa-esp-elf-gdb` — so a check on the family's own name would pass on an
  archive rusty cannot use. `the_shipped_bundle_answers_for_every_tool_it_
  carries` runs the real ladder over the real directory, and skips aloud on a
  checkout that has not run the script.

- **`rusty_embed::setup::plan` is the one derivation of "what is missing".**
  The Toolchain panel and the setup screen read the same
  `ToolchainReport` through it, so they cannot disagree; it is pure, so the
  ordering rules are tests rather than something discovered on a laptop.
- **Order is not cosmetic.** Without `rustup` nothing else can install, so
  that case collapses to one item and a link. `espup` comes before the Xtensa
  target it provides, because `rustup target add xtensa-…` without it fails
  complaining about an unknown target. Everything blocking comes before
  anything optional, so a queue somebody interrupts halfway has fixed the
  parts that mattered.
- **The queue is sequential and stops at the first failure.** Two `cargo
  install`s at once fight over the package-cache lock — the same collision
  Trunk hits — and a queue that carried on past a failure would end by
  reporting a ready machine that is not.
- **It says where each thing lands, before running anything.** Three homes are
  involved and only one is rusty's: `~/.cargo/bin` is cargo's (redirecting it
  with `--root` puts espflash where flashing cannot find it), rustup's home is
  rustup's, and the data directory is the one the user may move.
- **It only appears when the machine cannot build.** An optional tool missing
  is worth offering, not worth a dialog. A first-run check that shows up on a
  working machine is one people dismiss without reading, and then dismiss the
  time it mattered. Help ▸ "Check my environment…" is the way in when nothing
  interrupted.
- **On an `-msvc` host the linker is checked before anything is offered.**
  Without the C++ build tools every `cargo install` compiles for a minute and
  dies with "linker `link.exe` not found". The `msvc` row finds `link.exe`
  the way rustc does (`cc::windows_registry::find_tool`), so it cannot call
  missing a linker rustc would use; it is not installable — a Visual Studio
  installer is the user's decision — so, like rustup, it collapses the plan
  to one item and a link.
- **`espup install` names its version** (`install::XTENSA_RUST_VERSION`).
  espup's own "latest" asks GitHub's API, which answers 403 once the shared
  address behind a proxy has spent its sixty unauthenticated calls an hour —
  seen on the first run, the one run that has to work. The archives come
  from release downloads, which have no quota. The setup step reads the
  command off espup's tool row, so the pin is spelled once.
- **Downloads resume.** Each attempt is bounded (fifteen minutes), and a
  slow link carries a 420 MB archive only a piece at a time; the first
  version restarted from zero on every route and never finished. `download`
  keeps what arrived, asks for the rest with `Range`, appends only to a
  `206` continuing the same length (a `200` to a range request starts the
  file over — appending it would corrupt the archive), and asks a route that
  was delivering again before moving on. `continuation` is the pure decision,
  under tests.
- **espup's environment does not reach a running rusty.** `espup install`
  puts the Xtensa linker under the `esp` toolchain and then writes the user's
  PATH into the registry on Windows, or `~/export-esp.sh` elsewhere — for
  *new* processes. rusty ran espup from its setup sheet and was already
  running, so the `cargo build` it spawned next died with `linker
  xtensa-esp32-elf-gcc not found`, on a machine the sheet had just called
  ready. `esp_env.rs` reads espup's export file (its own statement of what
  it installed), falls back to the layout under `RUSTUP_HOME`, and
  `process::command` appends what exists to every child's PATH and sets
  `LIBCLANG_PATH` when the user has none. The user's environment is never
  written; `tools::find` consults the same directories so the panel cannot
  report absent what the build would find.

## The gutter, and one line height

The margin carries two things beside each line — a fold chevron and the
breakpoint dot — and for a while a third, a run arrow for a `#[test]`, which
has since moved beside the item as a lens (next section). Adding them cost
two bugs worth writing down, because both read as "the editor is broken"
rather than as a layout mistake.

- **One integral row height, and nothing computes its own.** `row_height(zoom)`
  rounds `LINE_HEIGHT * zoom` to whole pixels, and the two layers plus every
  overlay take it from there. The gutter draws its rows as flex containers and
  the echo draws its as blocks; at a fractional `line-height` the two round
  differently — fifteen thousandths of a pixel each — which is invisible on one
  row and a whole line by row eighty. `LINE_HEIGHT` itself is used by nothing
  else, so a new site cannot reintroduce the fraction.
- **The icons scale with the row.** A fixed 13px chevron is *taller than the
  row* once the editor is zoomed out, and a row that out-grows its line height
  pushes every number below it down. Sized from `row_height` instead.
- **The margin is `justify-end`, so anything that does not fit overflows off
  the left edge** rather than wrapping or scrolling — silently. That is how the
  run arrows were invisible for a while: the width reserved a column for them
  and then the chevron took it. The width counts the columns the file actually
  needs.
- Fold chevron right of the number, hard against the code, as VSCode puts it.
  Each control is its own click target, because one glyph that means two
  things depending on where you hit it is how you set a breakpoint when you
  meant to run a test — the reason the run arrow was never folded into the
  dot, and the reason it is a lens now.

## Test lenses

`▶ Run Test | Debug` beside every `#[test]` and every module holding one,
where VS Code puts it, in place of the margin's run arrow. The user's
complaint was exact: an entry point at the far edge of the margin is not
where anyone looks for it, and the margin had no room to say "Debug".

- **An overlay, never a row.** VS Code inserts a row above the item. This
  editor cannot: the textarea and the echo must stay glyph for glyph (see
  "Code folding"), and a row present in one and absent from the other is a
  caret that drifts. So `lens_anchor` (`view/panels/files/lens.rs`, pure,
  tested) puts the lens on the attribute line above the item — the row VS
  Code's lens occupies — after that line's text, and on the item's own line
  when nothing is above it. Positioned through `row_top`/`col_left` like
  every overlay, skipped when its line is inside a collapsed fold.
- **Run is what the arrow did**: `controller::run_test`, a substring filter
  with `--nocapture`, for the reasons written above that function.
- **Debug builds, asks, then runs.** `debug_test` (rusty-app) runs `cargo
  test --no-run` visibly, then the same with `--message-format=json` to learn
  where each test executable landed, then asks each binary `<exe> <filter>
  --list` and starts the *one* that lists a match under gdb
  (`rusty_dbg::Target::Host`). Which binary holds a test is cargo's private
  knowledge — `src/lib.rs` tests live in the library's, `tests/x.rs` in its
  own, a `#[path]` module anywhere — so the binaries are asked rather than
  the path read. Two matches are refused by name; none is refused because
  `cargo test` with such a filter exits zero having run nothing.
- **A host program is run, not attached to.** `Target::Host` sends
  `-exec-arguments` instead of `-target-select`, is `attached` from the start
  (pushed at once, so the frontend places its breakpoints *before* the first
  resume), turns that first resume into `-exec-run`, forwards the program's
  stdout — raw lines in gdb's pipe, since a native inferior inherits it — as
  `DebugState.output`, and quits gdb when the program exits, because a gdb
  with nothing left to debug is a session that never ends.
- **Refuse where gdb could only pretend.** An `-msvc` host carries its debug
  information in a PDB, which gdb does not read: it would load the binary,
  set breakpoints that never hit and show addresses where lines should be.
  `host_debug::gdb_reads` refuses that before the build with the reason and
  the alternative, and does not switch the project's target itself. The
  chips' gdbs are not a host gdb either; `host_gdb` looks for plain `gdb`.
- **One channel carries the build too.** The build's lines travel as
  `DebugState { output }` snapshots on the session's channel, so the command
  has one streaming argument like `debug_start`, and the frontend appends
  `output` to the dock whatever produced it. The Output tab shows first and
  the Debug tab takes over on attach; a Debug panel saying "starting" over a
  two-minute compile looked hung.

## Languages

`rusty-i18n` is one TOML catalogue per language plus a `t!` macro. English is
the source; every other file is checked against it, so a key added without a
translation fails a test rather than reaching a screen.

- **The setting is `workbench.toml`; `localStorage` is a cache of it.** The
  backend reads the setting and a second window has to agree with the first, so
  it cannot be WebView-local — but it arrives over IPC and the language has to
  be picked *before the first paint*. So `crate::i18n` reads a cache
  synchronously at boot and reconciles with the file a moment later. Losing the
  cache costs one reload and heals, which is the storage rule's actual test.
- **Changing language reloads, and reloading must happen at most once.** The
  first version applied the system language, read the file, and reloaded on
  disagreement — and `set_locale` writes an atomic in wasm memory that the
  reload destroys, so every boot rediscovered the same disagreement. A window
  that never finished loading, with no error anywhere. Writing the cache
  *before* reloading is what terminates it, and is the whole reason the cache
  exists. (VS Code restarts for this too; a half-translated window is worse
  than the language you did not want, because you cannot tell which half is
  stale.)
- **Backend text is translated by its key, not by its English.** A tool's
  purpose comes over the wire as prose; the *name* beside it is the stable
  half, so the frontend looks up `tool.<name>` and falls back to what the
  backend said when there is no entry. Refuse-rather-than-guess applied to
  translation: no entry means no claim. `rusty_i18n::translate` is the Option
  form that makes the fallback possible — `lookup` asserts on a missing key,
  which is right for `t!` and wrong here.
- **One binary can need two wordings.** The Toolchain panel says what a tool
  is; the setup sheet is asking permission to run it and says more. `espup` has
  a different sentence in each, so `setup.purpose.<name>` is tried before
  `tool.<name>`. One wording silently answering for the other is a
  mistranslation nobody would notice.
- **A `Problem` carries a `kind` and its `args` beside the English.** Prose
  with values baked into it cannot be looked up, so the stable name travels
  next to the sentence and the values travel apart from it — the frontend
  refills `problem.<kind>-title` / `-detail`, and the CLI prints the English it
  always did. Each `Problem::new` names its kind as a *literal*, because the
  test that checks coverage reads them off the source; a computed kind is
  unscannable and silently unchecked.
- **A scalar key and a table cannot share a name.** `[dock]` held the tab
  titles and the panels below then needed sections of their own, so the tabs
  moved to `[dock.tab]`; `menu.file` became `menu.bar.file` for the same
  reason. TOML rejects the collision, and the test that parses every catalogue
  is where it surfaces.
- **Four tests carry it.** One asserts every language has exactly English's
  keys. One scans the frontend source for `t!(` and asserts each key exists
  — the macro cannot check that at expansion time, and `lookup`'s debug
  assertion only fires if somebody opens the screen the key is on. (It scans
  for `t!(` and then skips whitespace: rustfmt breaks a call with arguments
  after the paren, and a scan that demanded `t!("` on one line missed exactly
  those.) The third scans `rusty-embed` for every `Problem::new` kind: falling
  back to English is correct for a diagnostic nobody has translated, and
  silently correct is how a gap survives a release. The fourth is the one the
  other three cannot be: it reads `view/` and `controller/` for string
  literals that *read as prose* — three or more words, one of them a word
  only sentences use — because a sentence that never became a key is
  invisible to a key check, and some sixty of them sat in the Chinese window
  that way (a palette footer, a waves header, a flight blocker). A literal it
  flags goes into the catalogue; the short allowlist in the test is for text
  that is meant to stay English, which today is the trunk-only dev banner.
- **Group headings and templated titles are keys too.** The palette's
  headings were `&'static str` literals beside translated rows, and "— needs
  a project" was a `format!` suffix; `panel.needs-project` and
  `palette.show-dock` take the name as an argument. A `&'static str` field on
  a type that reaches the screen is the tell.
- Not translated, deliberately: command lines, tool names, chip ids, target
  triples, and the dock's output. Users retype them, search them, and paste
  them into issues.
- **`t!` returns `String`.** A `&'static str` prop or return type on the path
  to a label has to widen. That is most of the work in a new file and all of
  the compile errors.

## UI conventions

**The rail switches panels and does nothing else.** It once carried the
active panel's actions under the switchers — save, build, flash, run, debug,
the debugger's transport, git's fetch/pull/push — and read as one 46px column
of sixteen icons at one weight, with Run in a different place on every panel
and the transport pushing it down the column when a session began. Four kinds
of button, four homes now. The **project's verbs** (Build, Test, Run/Stop,
Debug, Flash) sit in the title bar's centre with the file finder's icon
(`view/run.rs`), where Xcode and CLion put them: one position on every
panel, in a row the window already spends, and Run switches to the board
itself so nothing is far from anything. Build is `cargo build --release`
and nothing more — it never ran the tests, and until Test arrived the only
way to the suite was one lens at a time. Test is `cargo test` at the
project root through the lens's own path (`test_project`), and it is
refused where it is certain to fail: a root with a chip of its own and no
`firmware_dir` (`EmbeddedProject::root_is_firmware`) builds its tests for
the chip, which has no harness. **The refusal is on the click, in the dock
and the banner — not a disabled button.** Disabled with the reason in its
tooltip, it was refused in silence: the user clicked, nothing happened, and
the previous build's output still in the dock read as "Test ran a build".
Run and Debug are still disabled when blocked, and get away with it because
the Simulate panel lists what is missing; a refusal with no panel behind it
has to say so where the click landed. The **debugger's transport** floats over the working area while a
session is live (`view/transport.rs`) — VS Code's debug toolbar, an overlay so
its arrival moves nothing, and one copy where there were two. A **panel's own
actions** sit at the right of the row that names the panel — the Files
header, Git's branch row, the Crates and Toolchain headings, the board
sheet's corner — as VS Code's view titles carry theirs. **Save** sits at the
right of the file header beside the dirty dot, because it acts on the file.
A full-width toolbar row was tried before the rail and cost forty pixels on
every panel; the title bar is the row that already exists.

Chrome actions are icon buttons with a `title` tooltip — flat like VSCode's,
no ring, no fill; colour lands on the glyph (accent Play, crimson Stop). Text
appears in a control only when it carries state (a zoom %, a grid size).
Dot-entries never show in the file tree. Every dock surface answers a
right-click with its own menu or not at all — the browser's default menu is
always a bug.

**Nothing transient reflows the workspace.** The error banner is an overlay
in the working area's top-right corner, not a row above the panel: as a row
it pushed everything under it down forty pixels on arrival and back up on
dismissal, and a click already in flight landed on whatever had moved under
the pointer. It stays until dismissed or replaced by the next failure — a
success no longer clears it, because every controller call shares one
success path and a background re-probe was dismissing banners before they
were read. The dock keeps a copy regardless.

**Lists of things the shell has are generated from the thing.** The View
menu and the palette iterate `DockTab::ALL` and the panel registry; five of
the nine dock tabs were once spelled out by hand and the other four were
reachable from nowhere but a click on the strip. `Divider::ALL` and
`Divider::default_size` play the same role for Reset layout.

**The dock's strip carries the tabs that have something to say.** Problems,
Output and Terminal (`DockTab::PINNED`) are always there; the other six
appear when something puts them there and go when the user hides them with
the × on the tab. Two doors: `show_dock` — a button, the View menu, the
palette — puts a tab on the strip *and* in front; `reveal_tab` puts it on
the strip and nothing else, because a panel that switched under somebody
reading Output is the banner that reflowed the workspace again. The second
is called from `absorb`, beside the reading of the protocol: telemetry or a
tunable reveals Plot, a sensor declaration reveals Flight, a gpio report
reveals Waves, and a debug session reveals Debug and Registers together. A
`[rusty:pwm]` line reveals nothing, since a servo is a duty too. The strip
is session state (`Layout.dock_tabs`), not persisted: nothing is running at
boot, so the strip starts with what is true at boot, and Reset layout puts
it back. Nine tabs on a window with no project open was nine names for
things that were not happening.

**A page draws what a book puts in it, and reads the rest aloud.** The
Markdown page (`view/markdown/`) renders a chapter's formulas, figures and
raw HTML, each by a rule that names what it will not do. Formulas are
MathML: `$…$` and `$$…$$` go through `pulldown-latex` and the WebView draws
the result — Chromium, WebKit and Gecko all do now, so there is no KaTeX to
ship — and a formula the parser refuses is shown as its source with the
reason in the tooltip, never dropped. Not `latex2mathml`, which shipped for
an afternoon: it read `v_i^2` as a superscript on the subscript, a picture
subtly wrong in the way this project fears most, and knew no `aligned`; the
real book found both within one chapter. A picture in the project is fetched
(`files::BLOB`, base64, the way a picture in a diff is) and resolved against
the page's own path, never above the project root; a remote one stays alt
text with the reason in the tooltip, because fetching it tells its host who
opened the file. Raw HTML is *read*, by a small tag reader
(`markdown/html.rs`) and an allowlist in `element` — a `<figure>` is a
figure, a `<kbd>` a key cap, an `<a>` copies like a Markdown link — rather
than injected: injected markup would run, and an `<a href>` in it would
navigate the workbench away with no back button. html5ever would be a
megabyte of wasm to read `<figure><img><figcaption>`. An unknown element
shows its children; a script or frame is named and not run; an inline tag
that pairs with nothing stays the text it is. An image file opened from the
tree is a picture too (`is_picture`), with the Markdown page's source toggle
for SVG — drawn from the *draft*, so an edit shows the moment the toggle
flips back — and the blob fetch for anything binary, which used to be a
notice that the file was not text. The fetched pictures live in
`editor.images`, shared by both groups; the watcher drops an entry when its
file changes on disk. A Leptos trap met on the way: `#[prop(optional, into)]`
on an `Option<String>` prop *strips the Option* and the setter wants a
`String`; `optional_no_strip` is the one that takes the `Option`.

**The board sheet is dark in both themes, on purpose.** The canvas, the
devkit and the parts are drawn in hard-coded colours (`#101216` and
friends) rather than theme tokens, the way a schematic sheet is the same
colour in every editor; the panel chrome around it follows the theme. It is
the one surface exempt from "every theme block carries the whole palette",
and this sentence is what makes that a decision rather than an omission.

## Where state lives

One rule decides: **if the backend, the CLI, or another window could ever care,
it is a file; if only this WebView cares and losing it costs a shrug, it may be
localStorage; high-volume queryable data picks its own format when the feature
that needs it lands.**

**In the window itself, `AppState` is grouped by concern** — `state.editor.draft`,
`state.debug.session`, `state.find.open`. It was 112 signals in one flat struct,
which is a struct nobody can read and a boundary nothing enforces; the `find_`,
`search_`, `ai_` and `sim_` prefixes half the fields carried were the group's
name written into every field for want of a group to put it in. A new signal
goes in the group it belongs to, or a new group gets added — not on the end.

- The data directory (`config::data_dir()`) holds `boards/` and
  `workbench.toml` — plain TOML, user-readable, checked by tests. Its location
  is configurable: a fixed anchor (`%APPDATA%
usty`) holds `location.toml`
  pointing at the real directory. Relocation copies and switches the pointer;
  the originals stay until the user deletes them. Pointing it at a synced
  folder is the cloud-sync story.
- Secrets stay in the OS credential store, never in the data directory — a
  synced directory must never sync a key.
- Per-project, team-shared things live in the project's `.rusty/`, where they
  are diffed and reviewed: board overlays, the simulated board (`sim.toml`,
  which is what the canvas editor writes) and user-defined parts (`parts/`).
- Theme, divider positions, the editor's text zoom, the interface scale, the
  pin map's collapsed state, the file tree's fold, the Git panel's diff
  layout (one column or side by side) and the locale *cache* are
  localStorage, and that is all that is. They all go through `state::local_get` / `local_set` /
  `local_take` — one door, so the list above is a grep and not a claim.
  **Audit that claim when you add one** — it had already drifted twice. The
  assistant profile failed the rule (a second window boots the same frontend,
  and the backend reads it at request time) and so did the per-project tab
  strip, which additionally kept one key per project ever opened, never
  pruned, keyed on the path *as typed* so another spelling of the same
  directory silently had no tabs. Both are `workbench.toml` now, where
  `recent_projects` already had the same-directory matching and the cap.
  The WebView's storage is not the user's browser — clearing Chrome does not
  touch it — but nor is it carried by relocating the data directory, which is
  the whole cloud-sync story, and it is not backed up, readable or diffable.
  Moving anything out of it needs a read-once-and-delete migration **in the
  same commit**, or the upgrade is the thing that loses the data.

## Testing conventions

- **Assert on *which* problem, not that something failed.** These panels exist
  to name a specific mistake; a test that only checks `problems.len() > 0`
  passes while reporting the wrong thing.
- **Test the property, not the number.** `disabling_defaults_removes_serde`
  survives an upstream crate splitting a dependency out; `assert_eq!(crates, 8)`
  does not.
- **Fixtures are real.** `rusty-core/tests/fixtures/feature-lab` is a genuine
  workspace; `tests/memory.rs` writes a real ELF with `object`'s writer. Mocks
  would not have caught the section-flag classification bugs.
- **A round-trip fixture must differ from the default in every field, or it
  proves nothing about the ones it does not.** `the_board_round_trips_through_
  save_and_load` existed for the whole time `flip` was being dropped on save,
  and passed: the fixture set `flip: false` on every part, so a writer that
  never wrote the field and a reader that hard-coded `false` agreed perfectly.
  Same blind spot in the frontend's `parts_survive_the_round_trip_through_the_
  wire_model`, which set `rot` and left `flip` alone. Both fixtures now set
  every optional field to something that is not its default.
- **Geometry and protocol get tests; views get driven.** The board canvas got
  its arithmetic wrong three times while none of it was reachable from a test.
  The pure halves now live beside the component: `simulate/geometry.rs` (pin
  points on the grid, the devkit's generated symbol against its row points,
  turned and mirrored parts, wire paths, hit-testing, the symbol markup)
  and `simulate/edit.rs` (what placing, wiring, renaming, rotating,
  deleting, duplicating and undoing *do* to the parts and the wires). What genuinely
  needs a browser — a drag, a right-click — is driven through `mock.js` and
  asserted on numbers read back from the DOM, in a *separate* call from the one
  that dispatched the event.
- The built-in catalogue is checked by a `debug_assert!` at load — a typo in
  `data/*.toml` would otherwise surface only as a part mysteriously missing.

## Commit conventions

- **No `Co-Authored-By` trailers.** The user removed every one from history
  (2026-08-12, `git filter-branch`) and asked that none be added again. This
  overrides any default that says otherwise.

## Hard-won specifics

- **Never use PowerShell to transform source files.** It corrupted this repo
  twice: once mangling UTF-8 comments read as ANSI, once flattening a nested
  array so `$pair[0]` indexed a *character* and replaced every `u` with `s`
  across four files. Use the editing tools. If a bulk change is needed, do it
  file by file.
- **And the same trap has a Python spelling: `open(p, "w")`.** On Windows,
  text mode translates every `\n` into `\r\n` on the way out. A script that
  read a file, changed one token and wrote it back turned 34 files CRLF in one
  sitting — the repository is LF, git stores LF, `core.autocrlf` is false, so
  `git diff` then reported 8,000 changed lines of which about 300 were real.
  Nothing is corrupted and nothing fails to build, which is what makes it
  worth writing down: the damage is that the diff becomes unreviewable, and a
  review is the only thing standing between a bulk edit and the two failures
  above. `newline=""` on the write, or read and write bytes. `.gitattributes`
  and `rustfmt.toml` now both pin LF, and `file crates/**/*.rs | grep CRLF`
  is the check.
- **`cargo fmt` is the house style, and CI checks it.** Not because the
  formatting was bad, but because unmanaged drift hides real damage: the
  leftovers of those bulk edits — `warning: None,` at forty columns of
  indent, a `\` line-continuation lost so a user-facing string carried
  twenty-five spaces mid-sentence — sat in the tree looking like formatting
  nobody had got round to. rustfmt normalises the first kind and cannot see
  the second, so the second is worth grepping for on its own:
  `"[^"]*[^ ] {6,}[^ ]`. Where hand alignment genuinely reads better —
  `rusty_ai::model::presets` is a table — say so with `#[rustfmt::skip]` and
  a comment, rather than leaving the file unformatted.
- **Five fields copied onto six structs will lose one.** The first board's
  `SimLed`, `SimButton`, `SimRgb`, `SimSeven`, `SimDisplay` and `SimPot` each
  repeated `x`/`y`/`routes`/`rot`/`flip`, comments and all, and the file
  format repeated them again in *two* more sets — one for reading, one for
  writing. `flip` was added to the six wire types and to none of the four
  other places, so mirroring a part worked until the project was reopened.
  The six types went with the first board — the sheet has one `Instance`
  and one `Wire` — but the rule they taught stays in `board_file`: one
  `Part` and one `WireRecord`, each used to read *and* to write, so a field
  added on one side cannot be forgotten on the other. The file keeps its
  keys flat, where a hand-written file puts them.
- **TOML scoping**: in `data/boards.toml`, every scalar key must precede the
  first `[[board.usb]]` or `[board.pins]` header. A `flash_baud` after the usb
  block is parsed as a usb field. `deny_unknown_fields` catches it.
- `object` 0.40 wraps ELF section flags in newtypes; read `sh_flags`/`sh_type`
  off `SectionFlags::Elf` rather than trusting `SectionKind`, because these
  linker scripts invent section names (`.rwtext`, `.rodata_wifi`) that no
  heuristic classifies correctly.
- **rustup's `rust-analyzer` proxy dispatches by the project's pinned
  toolchain.** An ESP project pins `esp`, which has no rust-analyzer component,
  so spawning the bare name fails with `unknown binary 'rust-analyzer' in
  toolchain 'esp'` — precisely for the projects this workbench serves. Resolve
  `rustup which --toolchain stable rust-analyzer` first; the stable binary
  analyses any toolchain's project and reads the pinned sysroot itself.
- **Pushed diagnostics die at rust-analyzer's workspace switch; pull them.**
  When build data arrives, r-a switches workspaces and never recomputes pushed
  diagnostics for files already open — they are wiped and stay gone, on every
  project shape. Editors do not see this because they speak LSP 3.17 pull:
  the server sends `workspace/diagnostic/refresh` after the switch and the
  client re-requests. `rusty-lsp` declares the pull capability, re-pulls on
  refresh/didOpen/didChange with busy-retry, and treats a pushed empty set
  for an open file as a poke to re-pull, not as truth.
- **`procMacro.enable: false` is not a lighter mode — it is poison.** It
  takes the built-in derives down with it, sysroot trait resolution collapses,
  and any open file containing an `impl` with `&self` gets *no diagnostics at
  all*, silently. Leave proc macros on; the only thing rusty disables is
  flycheck (`checkOnSave: false`), because `cargo check` under `build-std`
  emits messages for packages `cargo metadata` never listed and r-a drowns.
  Probed live with `--example probe`, which injects an in-buffer error and
  asserts it is still present at the end of a 45s watch, on a host project
  and on a real Xtensa `build-std` project.
- **A flattened `"cargo.buildScripts.enable"` key beside a `"cargo"` object is
  silently ignored** in rust-analyzer's initializationOptions. The first
  attempt at the fix above failed while looking applied, because the sibling
  `procMacro` object *did* take effect. Nest keys in their object.
- **A workspace that excludes its firmware gets no IDE services there.**
  The standard embedded layout is host-testable crates as members and the
  bare-metal crate `exclude`d, so `cargo test` at the root does not try to
  build `no_std` for the host. rust-analyzer loads *one* workspace from the
  root, so every file under the excluded directory comes back "not included in
  any crates" — no completion, no diagnostics, no navigation, in exactly the
  half of the repository this workbench is for. The client reads
  `workspace.exclude` and names those manifests in `linkedProjects`. Read
  rather than guessed: linking every `Cargo.toml` under the root would pull in
  fixtures and vendored copies. And `toml::Table`, not `toml::Value` — in toml
  1.x `Value`'s `FromStr` parses a single *value*, so a manifest fails at its
  first table header with an error that reads as a broken `Cargo.toml`.
- **The build follows the chip, the tree follows the user.**
  `project::firmware_root` is the directory cargo, espflash and the emulator
  run in: the opened project for anything ordinary, and the single excluded
  firmware crate when the root has no chip of its own. Identity for every
  normal project, so nothing about a normal build changed. `state.root()`
  stays the opened directory — the file tree, the editor, the language server
  and the per-project tab strip all belong to the whole repository, and
  `project.root` is what the title bar names it by. Only the *build* moves,
  and `chip_source` says the chip came from a subdirectory.
  **Exactly one candidate, or none.** Two excluded firmware crates is a
  question with no right answer, and answering it anyway means flashing one
  board with the other's binary — so that case stays at the root and the
  problem names both. One candidate is `Info`, not `Blocking`: everything
  that needs the chip finds it, and a red badge on a working project is
  crying wolf.
- **esp-hal 1.x ships the vendor's pin table as generated Rust, and a lock
  can hold two versions of it.** The pin map read `esp-metadata`'s
  `devices/<chip>.toml`, which esp-hal 1.0 replaced with
  `esp-metadata-generated`'s `src/_generated_<chip>.rs` — a `for_each_gpio!`
  macro, one `(2, GPIO2(_2 => FSPIQ) (_2 => FSPIQ) ([Input] [Output]))` per
  pin, `([Input] [])` for a pin with no driver, analog functions in
  `for_each_analog_function!` — so every current project got "could not find
  esp-hal's description" and its named pins painted red as "not on this
  part", a claim nothing there could make. `generated_pins` reads that shape
  (0.1.0 spelled the inner macro `_for_each_inner`, later versions
  `_for_each_inner_gpio`; both are read). And `locked_version` took the
  *first* `esp-metadata-generated` in the lock, which was a 0.1.0 a
  transitive dependency pinned beside esp-hal's 0.4.0: `dependency_version`
  reads the version off esp-hal's own dependency list — cargo writes
  `"esp-metadata-generated 0.4.0"` there exactly when two exist — and every
  candidate is tried, newest first. A project whose table still cannot be
  read lists its pins as *unverified*, neutrally; "not on this part" is
  said only against a table that was read.
- **Hint-severity diagnostics are not problems.** The Problems panel says it
  lists what would stop the project building; a `#[cfg]` branch being off is
  the normal state of every crate that supports more than one chip. They were
  in the list while the count beside the tab already excluded them, so the
  two disagreed. Hints stay in the editor, where `diag-hint` dims the span.
- **rust-analyzer's `check.allTargets` default buries no_std projects.** It
  builds tests and benches, which need a test harness `no_std` does not have,
  so every real diagnostic drowns in "can't find crate for `test`". The client
  sets it false and passes `cargo.target` from chip detection.
- **LSP positions are UTF-16 code units unless negotiated otherwise.** One CJK
  comment shifts every column after it. The client offers utf-8 (rust-analyzer
  takes it), converts to Unicode-scalar columns at the boundary, and the
  integration test keeps a 中文 comment above the assertions so ASCII-only
  arithmetic cannot pass.
- **rust-analyzer's WorkspaceEdit URIs come back with a lowercase drive
  letter** (`file:///e:/…`) where this client builds `file:///E:/…`. A strict
  string compare judged every code action "multi-file" and dropped it — no
  quick fix ever appeared, silently. Compare through `same_file_uri`, which
  folds only the drive letter.
- **`ParameterInformation.label` offsets stay UTF-16 even after negotiating
  utf-8.** The negotiated encoding covers *document* positions; offsets into
  strings the server sent (signature labels) are UTF-16 by spec, always. Two
  conversion paths, one request.
- **ConPTY will not start the shell until the terminal answers `ESC [ 6 n`.**
  Its first act is to ask where the cursor is, and it blocks on the reply. The
  symptom is total: the pty yields exactly four bytes and then silence for
  ever, so the terminal is a blank rectangle with no error anywhere. `vt100`
  parses but never replies — it has no callback for it — so `pty.rs` scans the
  stream itself and answers DSR and Device Attributes.
- **A ConPTY child's console still cooks input.** Being inside a pty does
  not make a process raw: conhost line-buffers and echoes for whoever reads
  stdin, so the built-in shell saw every command twice and arrows never
  arrived as VT bytes. A shell child must clear echo/line/processed input
  and set `ENABLE_VIRTUAL_TERMINAL_INPUT` itself (termios raw on Unix) —
  and flip processed input back on around child commands, or Ctrl+C stops
  interrupting them.
- **A debug run needs a different build, not just different QEMU flags.**
  A release build has no code on many lines, so gdb moves a breakpoint to
  the next line that does and the margin ends up marking a line execution
  never reaches. Dropping `--release` is not enough either: esp-generate's
  template sets `[profile.dev] opt-level = "s"`, so the dev profile is
  optimised too. Debug runs pass `--config profile.dev.opt-level=0`, which
  leaves the user's manifest alone and shows in the dock. Measured on the
  demo project: 284 KB against release's 85 KB — 7% of a 4 MB flash, and
  the breakpoint lands on the line that was clicked.
- **A terminal session must own its slot by identity, and close before it
  reopens.** Two races produced the same symptom — a blank terminal after
  switching shells — and each alone was enough. First, the frontend cleared
  the screen signal *before* awaiting the close, so the reopen effect started
  a new session and the in-flight close then killed *it*. Second, a finished
  session's cleanup called `set_terminal(None)`, whose contract is "kill
  whatever it replaces": the outgoing session killed its own successor.
  `release_terminal` now clears only on `Arc::ptr_eq`, `close_terminal`
  awaits the close before clearing, and a test with two real pty sessions
  pins the ordering (it fails against either old behaviour).
- **On Windows a pty read never reports end-of-file.** The master keeps the
  pseudoconsole open however dead the child is, so exit has to be detected by
  polling `Child::try_wait` on its own thread. Inferring it from the reader
  works on Unix and hangs here.
- **An internally-tagged enum cannot have a newtype variant wrapping a string.**
  `#[serde(tag = "type")] enum Content { Text(String) }` compiles, and then
  fails at *runtime* with "cannot serialize tagged newtype variant" — there is
  nowhere inside a bare string to put the discriminant. For `Content` that
  meant every assistant answer failing at the IPC boundary, nowhere near the
  declaration. Use a struct variant (`Text { text: String }`). Any type that
  crosses the wire deserves a round-trip test; `rusty-ai/src/model.rs` has one.
- `serde_json` sends `i64` as a plain JSON number, so model deltas are `i32` —
  a 64-bit integer would have generated a TypeScript `bigint` that never matched
  the wire.
- **`RUSTUP_TOOLCHAIN` leaks from `cargo tauri dev` into every spawned
  cargo.** The rustup shim sets it for *rusty's own* build; rustup lets it
  outrank the project's rust-toolchain.toml, so a spawned `cargo build`
  compiles an esp-pinned Xtensa project with stable and dies with "can't
  find crate for `core`". `process::spawn` strips the variable.
- **The board view draws a module for the one module rusty knows, and a chip
  for everything else.** `kit_rows` was the classic 30-pin ESP32 devkit for
  every part, so a C3 board showed GPIO36/39/34/35 — pins it does not have,
  and a wire could be dropped on one. Header order is a property of the
  *board*, so it cannot be derived; the pin set is a property of the *die*, so
  it can. ESP32 keeps its real header; everything else is drawn in numeric
  order from `Chip::gpio`, transcribed into the catalogue from esp-hal's own
  device description rather than typed from a datasheet. An empty list draws
  rails only. The row count is now a value, not a constant, so `row_point`,
  `row_under` and the wire router all take it — a drawing and a hit-test that
  disagree about which side a row is on is the bug this shape prevents.
- **A capability only some parts support belongs in the catalogue as an
  optional field whose absence refuses.** The chip switch shipped keyed off
  nothing but the target triple, so it happily offered esp32 → stm32f103 and
  produced `espflash --chip stm32f103` and an `esp-hal` feature that does not
  exist — a complete-looking plan that cannot build, from a `Chip` the code
  never asked the right question about. The right question is data: `hal`
  names the crate a project selects the part through and asserts the chip id
  *is* its feature name. Espressif entries carry it, the STM32 entries
  deliberately do not, and a part added tomorrow gets no migration until
  somebody states how a project names it. A `match` on chip id in code is the
  version of this that silently does the wrong thing for the next part.
- **Switching a project's chip is mechanical except for pins, and the split is
  the whole design.** Four things bind a project to a part — the target triple
  and `--chip` in `.cargo/config.toml`, `build-std` (mandatory on Xtensa, a
  flag stable cargo refuses everywhere else), the channel in
  `rust-toolchain.toml`, and the chip feature on every `esp-*` dependency —
  and all four are rewriteable. `GPIO26` on a part with no GPIO26 is not:
  only the author knows what it should become. So `migrate.rs` changes the
  four, states in the plan that it changed nothing else, and lets the compiler
  name every site. Measured on the demo project: esp32 → esp32c3 built the
  entire dependency tree for RISC-V and stopped on exactly four errors, all of
  them pins. The edits are **word-bounded textual substitutions**, never a
  parse-and-reserialise — `esp32` in `features` and in `--chip esp32` move
  while `xtensa-esp32-none-elf` and `esp32c3` do not, and comments, ordering
  and version specs survive byte-for-byte.
- **A read that degrades to `default()` in front of a read-modify-write is a
  data-loss bug, not a fallback.** `workbench()` returned an empty state for a
  `workbench.toml` that failed to parse, and every writer — recents, tabs,
  keybinds, proxy — reads, changes and writes the whole file back. One
  unparseable file, and the next save wrote that emptiness over it: the recent
  projects list vanished between two launches with nothing in the logs. "Not
  there yet" and "there and unreadable" have to be different answers. A file
  that does not parse is now moved to `.broken` and named on stderr, so
  nothing is lost, the next save creates rather than clobbers, and the app
  starts clean instead of needing somebody to edit TOML by hand.
- **One global flag cleared from a shared error path belongs to nobody.**
  `track` wraps every controller call, and its failure branch cleared
  `session_running` — so one unrelated error told a *running* simulation it
  had ended, the Stop button vanished, and QEMU kept going with nothing in the
  window able to reach it. Only the call that started the session may say it
  ended: `track_session`. The same reasoning says a debug run's Stop must stop
  what the Debug button started, since that button is what booted QEMU.
- **A debugger reading a different build than the one running answers every
  question, fluently, about the wrong binary.** Debug runs build unoptimised
  (`--config profile.dev.opt-level=0`) into `target/<triple>/debug/`, but the
  frontend took gdb's ELF path from its cached `plan_simulation` result — and
  that call passes `debug: false`, so gdb read the *release* ELF while QEMU
  booted the unoptimised image. Symptoms, none of which point at the cause: the
  breakpoint is reported six lines below where it was set (the release line
  table), it never hits (the address means nothing in the running image), and
  the Debug panel sits on "Running" for ever. Only the run that built the image
  may say what the debugger reads; `run_simulation` records it (`state::Attach`)
  and `debug_start` has no `elf` parameter to get wrong. When something has one
  right answer and two computations of it, delete a computation — a test can
  only catch the drift after someone reintroduces it.
- **espflash `save-image` does not create parent directories** — a missing
  `target/rusty-sim/` fails as `os error 3`, which reads like a broken tool
  rather than a missing mkdir. `simulate::prepare` runs first.
- **Three ways a tunable firmware works in QEMU and is deaf on the real
  part**, all found by flashing `examples/pid-tune` to a C3 and finding that
  output arrived while every set vanished. First, `Uart::new` leaves every
  pin **unconnected** — QEMU's UART model bypasses the GPIO matrix, so a
  driver without `.with_rx(GPIO20)` reads perfectly in the simulator and
  reads nothing on silicon, with no error anywhere. Second, `esp-println`'s
  default `auto` backend decides **at runtime**: it reads the USB-Serial-JTAG
  start-of-frame flag and prints over native USB when a host is there, UART0
  otherwise — so on a C3 the console is whichever socket the cable is in, and
  firmware that reads only one of them talks back on some cables and not
  others. Read both. Third, tunables announced only at boot are invisible to
  every panel that connects later, which is nearly all of them; re-announce
  on a timer. `tune_probe` is the check: it opens the port both ways and says
  which of these is happening.
- **Pure autoscale draws a settled loop as static.** The Plot panel scaled
  each channel to its own min…max, so a controller holding 88.0 ± 0.5 — a 1%
  ripple — filled the full height with alternating full-scale noise, and four
  channels of it made the panel a wall of stripes. "Has it settled?" is the
  only question a tuning plot is read for, so the scale has a floor at 5% of
  the channel's own magnitude (`band` in `plot.rs`, with tests), a constant is
  centred rather than parked on the axis, and the legend carries `±swing` so
  the *height* never has to be trusted on its own. Found by connecting the
  real IDE to the real board; every earlier check had read the numbers rather
  than looked at the picture.
- **A streaming call cannot report success, so its optimistic state must be
  given back on failure.** `open_link` set `link_port` before calling, because
  `serial_link` never resolves while the link is up — and when the port was
  refused, the panel kept showing "Disconnect" with live sliders over a port
  it did not have. The error banner was correct and the panel contradicted it.
  Any claim staked before an await needs an explicit release in the error arm.
- **A re-announcement is not an answer.** `tune_probe`'s first version
  watched for any `[rusty:param]` line after a write and reported the
  periodic one as confirmation — it printed "clamped from 80" about a board
  that had heard nothing at all. Only a *change* from the pre-write value is
  evidence, and a set to the value already held proves nothing either way and
  now says so.
- **Two independent accounts of the same thing do not start together.** The
  GPIO model's proof compares the emulator's register-level view of GPIO0
  against the firmware's own `println!` about it — genuinely independent, so
  agreement is evidence. The first version demanded they agree element by
  element from zero, and rejected a model that was working: `Output::new(pin,
  Level::Low, …)` drives the pin *before* the loop that prints, so the
  emulator holds one transition the firmware never announced. The check
  rejected the model for being **more truthful than the firmware**, which is
  the whole reason it exists. Compare the sequences *aligned* — everything one
  source announced appears in the other, in order — and print the lead, since
  "the emulator saw N events first" is itself the finding.
- **`espflash monitor` cannot be typed into by a program that spawned it.**
  Its input comes from crossterm's `poll`/`read` — *console* events, not
  stdin — so a monitor rusty launched with piped stdio is one-way however
  correctly you write to its pipe, and the failure is silent: the write
  succeeds, the board never hears it. That is why `serial::open` exists and
  why the Plot panel's tunables are gated on `link_port` rather than on
  "a session is running": a slider that silently does nothing reads as
  firmware ignoring the change. The trade is explicit — rusty's own link is
  plain text, and defmt decoding stays espflash's.
- **esp-hal dispatches interrupts by reading the matrix's per-source status
  registers; ESP-IDF does not, so upstream's QEMU never modelled them.**
  `INTERRUPT_CORE0_INTR_STATUS_0/1` answered zero, so the peripheral raised
  its line, the matrix carried it, the CPU took the interrupt, and esp-hal's
  handler found nothing pending and returned — from the firmware's side
  identical to a line that was never raised. ESP-IDF gives each source its
  own CPU line and dispatches on the line number, which is why it works
  there and why the hole survived. Two rounds of CI said only "nothing
  happened" before a witness in the model itself named the half that was
  working.
- **`qemu_set_irq` on an unconnected line returns without doing anything.**
  A device whose `sysbus_init_irq` line no machine ever connected reports
  interrupts into nothing, silently. The model says `unconnected` on its own
  channel now, because that is the only place that can tell the difference.
- **A missing peripheral is a hang inside the user's own call, not a wrong
  answer.** With nothing mapped at the SAR ADC's registers,
  `adc.read_oneshot()` polls a done bit nothing can set; with nothing at the
  I2C master's, a driver's first transaction waits on an interrupt nothing
  can raise. Both look like the firmware hanging in `read`. So every probe
  here **bounds its own wait** and prints what it found — a bounded poll for
  the ADC, esp-hal's `SoftwareTimeout` for the bus. A witness that reports a
  hang as silence is no witness, and both of these were measured on the
  stock build before either model was written.
- **Every register name in `qemu/esp32_gpio.c` is prefixed `RUSTY_`.**
  Upstream has its own `hw/i2c/esp32_i2c.h` and `hw/ssi/esp32c3_spi.h`, whose
  `REG32(I2C_CTR, 0x04)` expands to the same enumerator, and
  `hw/xtensa/esp32.c` includes both those headers and this one — so an
  unprefixed name is a redeclaration error in a file neither of us wrote.
- **An I2C `RSTART` command word is all zeros, and so is an unused command
  slot.** The peripheral's command list encodes a start as opcode 0 with no
  byte count and no ack bits, so the two are the same thirty-two bits — and
  the silicon needs no way to tell them apart, because it stops at the
  `STOP` or `END` a driver always ends with. A model that read a zero word
  as "the end of the list" executed *nothing*: every transaction completed
  with no acknowledgement, every address on the bus looked absent, and not
  one byte was reported. It reads exactly like a bus with nothing on it,
  which is the hardest kind of wrong answer to tell from a right one.
- **The I2C command op codes are not consecutive and are not the order a
  driver's enum lists them.** The hardware's are `RESTART` 6, `WRITE` 1,
  `READ` 3, `STOP` 2, `END` 4 (esp-idf's `hal/esp32c3/i2c_ll.h`); reading
  them off esp-hal's Rust `Command` enum gives 0..4 and is wrong for three of
  the five. What that produced is the thing worth remembering: a `Start` fell
  through to the default case and addressed nobody, the `Write` after it had
  no device and returned before it could even report a NACK, so every
  transaction completed having done nothing and **not one byte was reported**
  — a bus that read as empty, indistinguishable from a model that was never
  asked. Three rounds of CI said "the firmware did not find them" and nothing
  else. What broke it open was the *firmware* dumping the four command words
  and the status register; decoding them took a minute. When a peripheral
  model appears to do nothing, have the guest read its registers back rather
  than reasoning about the driver.
- **An assertion on a change-suppressed report needs an event that keeps
  changing.** The buses say the same transaction only the first time — a
  driver polling a sensor would otherwise put twenty kilobytes a second down
  the channel — so a firmware that writes to a display *once* is reported in
  exactly one moment, and if nobody is on the channel in that moment nobody
  ever hears it again. Gate 10 asserted on such a write and failed with the
  bus provably working; gate 9 passed only because its write happens after the
  host declares the device. Both probes write every time round now, which also
  makes each write a change from the transfer beside it.
- **Connect the pin channel before the guest boots, not after.** The device
  reports only to a channel somebody is on, so everything the firmware does in
  its first tenth of a second is lost to a gate that waits for a serial line
  and connects afterwards — which is how gate 10 asserted on a write it had
  provably already missed. QEMU opens the socket during machine init, before
  the first instruction, so the only thing to wait for is the process.
- **A step a model does not recognise must say so, not be skipped.** The
  `default:` arm that quietly did nothing is what made the above invisible
  for three rounds. It reports `?op<N>` on the channel now.
- **A write-triggered bit must come back clear.** `CTR.TRANS_START`,
  `CTR.CONF_UPGATE`, `CTR.FSM_RST`, `SPI_CMD.USR`, `SPI_CMD.UPDATE` and
  `SCL_SP_CONF.SCL_RST_SLV_EN` are all `WT` or `R/W/SC` in the register maps:
  the guest sets one, the hardware acts and clears it, and the driver reads it
  back to find out that it has. A model that *stores* them is a driver polling
  a bit that can never fall — `Spi::update()` and `ClearBusFuture` both wait
  on exactly that, and esp-hal runs the second after every NACK, which is once
  per address of a bus scan. Read the map's access column, not just the bit
  position.
- **Renaming a script means changing the line that runs it.** `interrupts.py`
  became `patches.py`; the workflow step's name and its comment were updated
  and the `run:` line was not, so every platform stopped at `can't open file`
  with exit 2, before a single file was compiled. Grep for the old name, not
  for the old description.
- **An ESP32 cannot be simulated once it does floating point.** Espressif's
  QEMU dies — `Fatal error: divide by zero`, taking the emulator with it, so
  the guest's buffered console output is lost too and the log ends mid-boot —
  on the **first FPU instruction** an `-M esp32` application executes.
  Bisected down from a minimal firmware: integer `println!`s tick over
  indefinitely; adding one `black_box(1.25) * black_box(3.0)` ends it before
  the next line prints. LEDC, UART0 with `with_rx`/`with_tx`, and GPIO were
  each cleared first, so it is not a peripheral. `-M esp32c3` runs the same
  code. That is why every example here is a C3 — `pid-tune` is float PID and
  has always worked — and why anything float-heavy for an ESP32 board needs a
  C3 build to be watchable at all. The symptom to recognise: two lines of
  output and then silence, with `esp32_i2c: slave mode not implemented`
  alongside as unrelated machine-init noise.
- Child processes get `CREATE_NO_WINDOW` on Windows; the toolchain panel probes
  six tools on open and would otherwise flash six console windows.
- **`NO_COLOR=1` breaks Trunk.** It maps the variable onto its `--no-color`
  flag, which takes `true`/`false`, and dies with `invalid value '1'`. Set
  `NO_COLOR=true` or unset it before `trunk serve` / `cargo tauri dev`.
- **Any cargo command run while `trunk serve` is live can break its build.**
  Both want the package-cache lock; Trunk's `cargo build` loses and reports
  `bad status returned from cargo artifacts request`, exit 101. It looks like a
  compile error in the frontend and is not — the next rebuild after the lock
  frees succeeds on identical source. Check the serve log's timestamps before
  believing a build failure the browser console reports.
- **A tool version that comes from whatever is on PATH is not a version.**
  `style/input.css` opens with `@import "tailwindcss"` — v4 syntax — and
  Trunk 0.21's *default* tailwind is still 3.3.5, which cannot parse it. This
  machine happened to have a v4 binary on PATH, which Trunk prefers over
  downloading, so every local build passed and the first CI build failed on
  all three runners with a bare `exit status: 1` from a tailwind nobody had
  chosen. Pinned in `Trunk.toml`'s `[tools]`. Reproduce a CI-only build
  failure by taking the tool off PATH — Trunk then downloads what the pin
  says, which is exactly the runner's situation.
- **Trunk only ships assets it was told about.** A bare `<script src="x.js">`
  leaves `x.js` out of `dist/`, and the dev server answers the request with
  `index.html`, so the failure is `Unexpected token '<'` rather than a 404.
  Anything extra needs `<link data-trunk rel="copy-file" href="x.js" />`.
- **`withGlobalTauri: true` is required.** Tauri v2 defaults it to *false*, so
  `window.__TAURI__` does not exist and every IPC call dies — inside the real
  app, not just in a browser. The frontend binds to that global directly rather
  than through `@tauri-apps/api`, which would mean npm.
- **`data-tauri-drag-region` needs `core:window:allow-start-dragging`** in
  `capabilities/`. It is *not* in `core:default`, and without it the attribute
  is present, the injected handler runs, and the window simply does not move —
  no error anywhere in the frontend, because the denial happens on the Rust
  side of the IPC. `allow-internal-toggle-maximize` is the matching permission
  for double-clicking the title bar.
- **`window.confirm` does not exist in the app, and nothing says so.**
  `tauri-plugin-dialog` overwrites `alert` and `confirm` with shims at
  injection time; the `confirm` shim returns a *Promise* (so `web_sys`'s
  `confirm_with_message` reads it as `false`, every time) and invokes
  `plugin:dialog|confirm`, a command that no longer exists in 2.7 — it
  rejects with "not allowed. Command not found" whatever the capability
  grants. The Git panel's discard and the dirty tab's close both did nothing
  in the app for a release while passing every check under `trunk serve`,
  where a browser's real `confirm` answers. `ipc::confirm` is the one door:
  the plugin's own `dialog.confirm` (needs `dialog:allow-message`, since
  `allow-confirm` is now an alias for it) in the app, `window.confirm` in a
  browser, and `mock.js` stubs `dialog.confirm` so the mock exercises the
  same path. A flow whose only proof is a browser has not been proven for
  the app — dialogs, clipboard, focus and anything else the WebView hosts
  differently are the places to test in the app itself (the CDP recipe is in
  the session memory).
- **`createUpdaterArtifacts` needs `plugins.updater` to exist, and fails
  *after* the app has built.** Adding the signing secrets flips the release
  workflow onto `--config '{"bundle":{"createUpdaterArtifacts":true}}'`, and
  the bundler then reads `plugins.updater` for the public key. With no such
  section it stops with "plugins > updater doesn't exist" — after "Built
  application at …", so the log looks like a successful build that failed at
  the end. The section carries the public half of whatever is in
  `TAURI_SIGNING_PRIVATE_KEY`; a mismatched pair builds fine and only fails
  later, when an update will not verify. Note that the *plugin* is not a
  dependency: the config alone is what the bundler wants, and
  `update::check` still only checks and links.
- **`tauri.conf.json` rejects unknown fields**, so a `"//comment"` key fails the
  build with "unknown configuration field" and a misleading suggestion to update
  your Tauri crates. Explain the config here instead.
- **One `cargo tauri dev` at a time, and stop the old one first.** A second
  instance fails with `os error 10048` on port 17425 — and stopping the task
  kills `trunk serve` but *not* the app window, which is a detached child. Kill
  `rusty-app.exe` too, or the next run inherits a stale window.
- **Never run `trunk build` while `trunk serve` is running.** They share `dist/`
  and its staging directory, and the collision surfaces as two unrelated-looking
  failures: the browser blocks the stylesheet for an `integrity` mismatch
  (index.html from one build, CSS from the other), and `tauri dev` dies with
  `error writing JS loader file to stage dir / os error 3`. `cargo tauri dev`
  already runs `trunk serve` for you — to rebuild, touch a source file and let
  it do it.
- **Every theme block carries the whole palette, or it is not a theme.** A
  token defined in one block and missing from another leaks across theme
  choices: the system-dark media block once lacked the `--term` syntax set
  (light ink on dark ground), and `[data-theme="light"]` lacked it the other
  way (dark-theme pastels on white). Both read as "the code is unreadable",
  far from the stylesheet. When adding a token, add it to all four blocks in
  `input.css`.
- **A part that hides a wire reads as a broken wire.** The board canvas draws
  the grid under everything and the wires *over* everything, in two SVG layers
  with the parts between: a 140px display parked on a net used to swallow its
  middle and look like a disconnection. Both SVG layers are
  `pointer-events: none`; only a wire's own grab handles opt back in with
  `pointer-events: stroke`, so a top layer spanning the sheet still lets
  presses through to parts and to the pan gesture.
- **A lamp has a polarity, and so does a button, and the sheet says which.**
  Every lamp and button was drawn active-high: lit when the pin was high,
  pressed drove the pin high. Most devkits' onboard LEDs are active-low
  (anode on 3V3, the GPIO sinks) and most buttons are to ground with a
  pull-up (`Pull::Up` + `is_low()`), so the board showed the opposite of the
  desk, and a pull-up button *released* in the emulator when the user
  pressed it. `active_low` travels on the four wire types and in
  `sim.toml`; the canvas reads a level through it (`lit`), the properties
  panel has the checkbox, a new button starts active-low and says so in the
  file, and the backend's pin channel turns "pressed" into the level the
  board file means (`pin_level`). The console message `B<pin>=1` keeps
  meaning *pressed*, because that is what the text-protocol firmware reads.
- **Mirror, do not rotate, to face a part at the chip.** Rotating 180° does
  bring a part's pins to the near edge — and reverses their order, so seven
  wires to a seven-segment cross on the way in. `mirror` mirrors: near edge,
  same order. Neither transform touches the writing: pin names, numbers,
  the reference and the value are placed in sheet coordinates *after* the
  turn (`pin_labels`), so nothing is ever drawn mirrored.
- **Each part on the sheet is its own keyed view.** The parts were one
  closure rebuilding every part's DOM on every change to the list — on
  every pointer-move frame of a drag, that is — and a hover on one pin cost
  a re-render of thirty. `<For>` keyed by index, and every field a view
  reads comes through its own memo (`this`, `symbol`, `place`, `value`,
  `bbox`, `pin_dots`, `labels`), so a drag frame touches one part's
  `transform`. The body follows the symbol and nothing else; the face
  follows the symbol's behaviour and the rules' reading through closures
  inside it.
- **Both ends of a wire are a place to start it.** Any pin drags to any
  other — a part's to the devkit's, the devkit's to a part's, part to part
  — through one `Drag::Wire { from }` and one `edit::connect` in
  `pointerup`, because two gestures that agreed about what wiring means
  only in prose would drift. `pin_under` works in sheet units so the reach
  does not shrink with the zoom, and answers nothing when nothing is in
  reach — a wire that landed on a pin forty pixels from the pointer would
  be a connection nobody made. A pin to itself and a pair already joined
  are refused as wires that mean nothing.
- **Selection is a set, and the left button on empty sheet draws it.** A
  plain drag on the background is the rubber band (`Drag::Box`,
  `parts_in_box`, touching rather than enclosed); panning is the middle
  button or Ctrl/Alt with the left, as in every map. Shift-click adds or
  removes one, Ctrl+A takes all. `marked` is the set and `selected` stays
  the one the inspector describes; the inspector shows a count and the
  group's verbs when the set is more than one. A group drag snapshots where
  every other member stood at the press (`group_start`) and `edit::translate`
  moves each by the grabbed part's displacement *from the start*, so a
  snapped frame cannot accumulate into drift; the legs of every wire
  touching the group slide along their own axis exactly as a single
  part's do. Any removal clears the set, because every index above the
  removed part has shifted.
- **Wire bends belong to the sheet, not to the part — KiCad semantics.**
  Dragging a part stretches only the leg from its pin to the nearest bend,
  at whichever end of the wire the part is (`wire_legs`, `follow_bend`);
  every bend the user placed stays put, and the orthogonal pass grows the
  elbow the stretched leg needs. (An earlier fix translated bends with the part;
  that read a rendering artefact as a semantics bug and inverted the
  behaviour every schematic editor has taught.)
- **Leptos flushes to the DOM in a microtask.** Clicking an element and reading
  the DOM back in the *same* synchronous block always shows the pre-update
  state. When driving the UI from a browser tool, put the click and the
  assertion in separate calls — otherwise every interaction looks broken, which
  cost an hour of chasing a reactivity bug that did not exist.
- **A bare `>` in a `view!` attribute value ends the tag.** `disabled=move || a
  > b.get()` compiles the attribute as `move || a` and reports a type mismatch
  on a line that looks fine. Any comparison in an attribute has to be bound
  above the macro — same fix as the `match` case below, same class of error
  message pointing nowhere near the cause.
- **An overlay textarea scrolls itself and never tells you.** The editor is a
  `<pre>` echo under a transparent `<textarea>`; any instant where the
  textarea's content outgrows its box (the keystroke that adds a line, before
  the echo re-renders; a line wider than its column) makes the browser scroll
  the textarea *internally*, and that offset stays forever — caret drifting
  off its glyph, a column or a row at a time. The fix is structural: size the
  overlay's column to the content (`w-max` row), mirror any internal scroll
  out to the shared scroller and pin it back to zero, and follow the caret
  explicitly after every edit.
- **Programmatic `.value` writes destroy the textarea's native undo stack.**
  The editor writes value on every echo, completion accept and format, so
  Ctrl+Z was silently dead. The editor keeps its own snapshot history
  (`EditHistory`), parked per tab; the caret after undo is recomputed from
  where the two texts diverge rather than stored.
- **Setting a selection before a mounted textarea has its value snaps to
  EOF.** The reveal effect fires on a freshly opened file before the value
  lands; consume it one `set_timeout(0)` later, and focus with
  `preventScroll: true` — the browser's own focus scroll arrives async and
  overwrites a deliberate `set_scroll_top`.
- **rust-analyzer must only ever hear about `.rs` files.** A didOpen for
  `.git/info/exclude` produced a syntax-error per line — sixty-eight problems
  from a file that was never code. Gate didOpen/didChange/didSave on the
  extension, not on "it is open in the editor".
- Leptos's `view!` cannot parse a bare `match` or `if` as an attribute value.
  Compute it into a binding above the macro rather than wrapping it in braces —
  it reads better and the error when you forget is about close tags, which
  points nowhere near the cause.
- A future built from `&SomeStruct { .. }` inline borrows a temporary that dies
  at the end of the statement. Bind the struct, then move it into an `async`
  block.
- **An attribute on the wrong element compiles.** Leptos spreads an unknown
  attribute onto a component's root, so when the `files.rs` split left the
  editor's `prop:readonly` — the guard that keeps an IME from typing into
  Vim's normal mode — on the context menu's Paste row, it became a `readonly`
  on a button and guarded nothing. Twelve lines of comment explained a guard
  that was not there. When an attribute exists to enforce something, grep for
  it on the element it belongs to.
- **An effect that reads the state its own request produces is a loop.** The
  Registers tab re-read the selected peripheral on every `debug.session`
  change; the read's answer arrives *as* a session change. Key such an effect
  on the facts that should trigger it — here the stop address and the
  peripheral — and compare with the previous run, as `memory.rs` does for the
  ELF path.
- **Tracked reads outside a reactive owner warn on every boot.** Controllers
  and event handlers have no owner to subscribe; `has_project()` from one
  printed Leptos's "outside a reactive tracking context" four times per
  launch. `has_project_now()` / `active_path_now()` are the untracked forms
  for that side; the tracked ones are for views and effects.
- **A test that tolerates an absent tool has to say so in code, not in a
  comment.** `is_some_and` on an `Option` is `false` for `None`, so a test
  written to *pass* on a machine with no gdb was the one that failed there —
  on every CI runner, for weeks, while the comment above it explained the
  opposite. Spell the tolerance out (`is_none_or`) and skip with an
  `eprintln!` naming the missing tool, the way `tests/analyzer.rs` does for
  rust-analyzer. Better still, inject the tool: the simulator's plan takes a
  `Machine` and the test hands it a directory with a fake gdb in it.
- **`\<` and `\>` are word boundaries to the regex crate, not escaped
  brackets.** A hand-written literal escaper that backslashed every ASCII
  punctuation mark turned `Vec<u8>` into a pattern that can match nothing,
  and every literal search for a generic, `->` or `=>` silently found nothing
  — while replace, which used `regex::escape`, found them. Use
  `fixed_strings` on the matcher builder for literal mode, and never a second
  escaper beside the one the crate provides.
- **Anything that takes a byte prefix of two strings backs up to a char
  boundary.** `fold::splice` computed the common prefix of the old and new
  screen text byte by byte and then sliced there; two CJK characters that
  share their first two bytes — 中 and 世, the ordinary case in one block —
  put the slice inside a character and panicked the window. `positions`
  already had the boundary walk; the arithmetic is the same and so is the
  code now.
- **A thread that owns the `Arc` it waits on lives for ever.** rust-analyzer's
  pull loop held an `Arc<Shared>` whose `poke` field held the sender the
  loop was blocked receiving on; dropping the client killed the server but
  the loop, the `Shared` and every open document's text stayed. The loop
  holds a `Weak` now and the client sends `shutdown`/`exit` before `kill`.
- **rustup answers for the directory it is run from.** `rustup target list
  --installed` in rusty's own cwd reported the default toolchain's targets
  for a project pinned to `nightly-…`, so the Toolchain panel said "target
  not installed" and its fix installed into the wrong toolchain. Every
  rustup probe runs with `current_dir(project.root)`, through
  `process::command`, which is also the one place `RUSTUP_TOOLCHAIN` is
  stripped and `CREATE_NO_WINDOW` set. rusty-core cannot depend on it, so
  `workspace.rs` carries its own three-line `quiet()` for `cargo metadata`
  and `rustc -vV` — guppy's builder gave the GUI a console window per open.
- **`Channel::send` failing means the WebView is gone, nothing less.** A JS
  side that drops its handler tells Rust nothing, so every "stop when the
  user leaves the panel" loop keyed on `send().is_err()` ran until the
  window closed: one more file watcher per project switch, an `ai_ask` that
  could not be cancelled. Long-lived commands own a slot in `AppState`
  instead (`watch`, `asking`), and the loop ends because the slot was
  replaced. `stream.rs` is the one reader loop, and says this above it.
- **`workbench.toml` is written through `config::update`, and only that.**
  Every writer is a read-modify-write of the whole file, and two of them —
  the tab strip on every tab switch, the recents on every open — interleaved
  and lost one. `update` holds a process-wide lock across the read and the
  write; the temporary file carries the process id, because two windows
  sharing one `workbench.toml.tmp` produced a file that was neither's. The
  file has its own private structs (`config::file`), so the wire types can
  be renamed for the frontend without dropping a key from everybody's file.
- **A child's stderr is read or it is `null()`; it is never `piped()` and
  forgotten.** gdb's was piped and unread, and a gdb with a Python warning
  per startup filled the pipe and blocked on it with every MI answer still
  to come. And gdb writes `exit-code` and every non-ASCII byte in *octal* —
  `"012"` is ten, `\346\227\245` is 日 — so both are decoded as octal, into
  bytes, not chars.
- **The pty's exit poll wakes the renderer once, on exit.** It used to send
  a wake every 250 ms as a way to notice the consumer had gone, and every
  wake was rendered and pushed over IPC — four unchanged frames a second
  from an idle terminal. Liveness is read off `Arc::strong_count` instead.
  And a DSR query is answered *after* the bytes before it in the same write
  have reached the emulator, or a program that prints and asks gets the
  cursor from before its own output.
- **"It paints once and then ignores every click" is a wasm trap, and the
  console is the only place it says so.** No panic banner, no CPU spinning,
  and right-click falls through to the WebView's own menu because every
  Leptos handler is dead. Two different faults produced exactly that screen
  in one afternoon: a stack overflow (`RuntimeError: memory access out of
  bounds`, fixed by `.cargo/config.toml` — read it, it explains the size) and
  a panic reading a disposed signal, which in wasm aborts the module. Read
  the console rather than guessing: start the app with
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`,
  connect to `http://127.0.0.1:9333/json/list` over CDP (suppress the
  `Origin` header or the socket is refused 403), enable `Runtime`/`Log`, and
  reload — the first exception names the component, and every one after it is
  that one's corpse.
- **A reader loop that peeks and does not consume must consume *something*
  on every path, or it is a freeze waiting for the right file.** The
  Markdown block reader broke on an `End` it was not waiting for — the
  closer of an HTML block, a construct it did not list — and looped there
  for ever; the page view runs that reader on every render, on the only
  thread there is, so opening a chapter with a `<figure>` froze the whole
  window with no error anywhere. Two defences now: every construct
  `pulldown-cmark` can emit for the enabled options is handled, *and* the
  loop eats an event it made no progress on. The second is the one that
  holds when the first drifts. `RUSTY_MD_CORPUS=<dir>` runs the reader and
  syntect over a real book under a time bound; without it the test skips and
  says so.
- **A `set_timeout` closure outlives the component that made it.** The
  editor's hover grace period reads `hover_gen`/`hover_cell`/`on_card` 300ms
  later; close the tab or switch projects inside that window and those
  signals are disposed, reading one panics, and the panic takes the whole
  window down. Anything deferred past a frame uses `try_get_untracked` and
  returns on `None`. The signals that belong to `AppState` are fine — it is
  the component-local ones that die.
- **Completion inside a macro's arguments works; an empty answer there is
  the server still loading, or a name that does not resolve.** An earlier
  note here said `assert!(…, e.)` answers with nothing and blamed macro
  expansion. Measured again (2026-09-07) against a warm rust-analyzer on
  `examples/pid-tune`: `println!("…", output.)` answers with the ninety
  methods of `f32`, and the popup shows them — rust-analyzer completes with
  a placeholder identifier at the caret, so the incomplete `output.)` is no
  obstacle. The two things that *do* produce an empty answer: a receiver
  whose type is `{unknown}` (an `Output` never imported — the auto-import
  completion is the fix for that), and a server that has said `Ready` but
  is still indexing, which is why the status bar now shows its progress
  (`workDoneProgress` on, `$/progress` folded into `LspEvent::Progress`)
  instead of "rust-analyzer" in green over a minute of empty replies. gdb reads
  DWARF; Rust's default Windows target emits a PDB, so on `-msvc` gdb loads
  the binary, sets breakpoints that never hit and shows addresses where lines
  should be. LLDB reads PDB — it resolves a Rust test symbol to its source
  line on that exact target — and LLDB's machine interface is DAP, not MI. So
  `rusty-dbg` has two backends and `any.rs` picks between them; `gdb_reads`
  answers "is gdb usable here" and `host_adapters` answers "what instead".
- **Drive a debug adapter over its socket, never over its stdin.** Both
  `lldb-dap` and `codelldb` hand the debuggee their own stdout, so the test's
  `running 1 test` lands inside a DAP frame and every frame after it is
  garbage. Over `--port` the two separate: the socket carries the protocol and
  the adapter's stdout carries only the program's output, which is what the
  dock shows. Measured on this machine, not feared.
- **The adapter is a download, like QEMU and the debuggers.** CodeLLDB
  bundles its own LLDB, so one archive is the whole dependency — which
  matters because the `lldb-dap` LLVM ships for Windows does not work and a
  VS Code extension is not something rusty may require. Its `.vsix` is a zip
  that bsdtar reads, but its payload sits under `extension/`, so it unpacks
  into `tools/codelldb/` of its own rather than over `tools/`: the adapter
  has to keep the `lldb/` beside it that it loads its debugger from.
  `tools::find` cannot see it — that ladder looks for `<family>/bin/<exe>` —
  so `host_adapters` knows the one shape, and `tool_bin_dirs` does not reach
  two levels down, which is why nothing new lands on a child's PATH.
- **"The adapter exists" and "the adapter answers" are different facts.**
  LLVM 19.1.0's Windows `lldb-dap` starts, stays alive, and answers nothing —
  not stdio, not a socket. So discovery returns a *list*, the caller tries
  each, and the connect timeout is short because a working adapter listens at
  once. `cargo run -p rusty-dbg --example dap_probe -- <project> <exe> <file>
  <line> [args…]` is the check that a machine can debug at all.
- **A debug adapter takes breakpoints only between `initialized` and
  `configurationDone`.** After that the program is running and a breakpoint
  placed then is one a short test has already run past. The standing list
  therefore travels *with* the launch request rather than being placed on
  attach the way the gdb path does it, and the panel's first resume is a
  no-op rather than an error about a program that is already going.
- **The build follows the chip; the tests follow the user.** `run_command`
  runs in `firmware_root`, which for the standard layout is the *excluded*
  bare-metal crate — and `cargo test` there fails with "can't find crate for
  `test`", because a `no_std` target has no test harness, which is the whole
  reason that crate is excluded. Host commands pass `at_project_root`, so
  they run where the testable members are. The Run Test lens, the title
  bar's Test and `debug_test` all do — and the backend says which layout it
  found as a value, `firmware_dir`, beside the prose in `chip_source`, so
  the frontend can refuse Test on a root that is its own firmware without
  matching English.
- **CI's clippy is today's stable; the machine's is whenever `rustup update`
  last ran.** v0.3.1 was tagged with clippy green here on 1.97 and failed on
  the runners' 1.98 — `chunks_exact(5)` where `as_chunks::<5>()` now exists,
  and three `use leptos::prelude::*` lines that `use super::*` already
  supplied. Neither is wrong code; both are `-D warnings`. Before a tag, run
  the four gates on the runner's toolchain — `rustup toolchain install
  <latest> --profile minimal -c clippy -c rustfmt` and `cargo +<latest>
  clippy --workspace --all-targets -- -D warnings` — or update. The job log
  needs a GitHub login, so the test job now also names what failed in
  annotations and the step summary, which the run page shows to anyone.
- **A bare `tar` on a Windows PATH is often Git's GNU tar, and GNU tar reads
  `E:/…` as a remote host.** `where tar` on this machine answers `C:\Program
  Files\Git\usr\bin\tar.exe` before `C:\Windows\System32\tar.exe`, and the
  QEMU upgrade's unpack died with `tar: Cannot connect to E: resolve failed`
  — GNU tar's colon rule, which no slash style escapes. Every archive rusty
  unpacks goes through the absolute System32 `tar.exe` (bsdtar 3.8 with
  liblzma, so `.tar.xz` and zip both read) on Windows; the dock shows that
  path so nobody reads the line as a plain `tar`. And with `-U`: the QEMU
  upgrade unpacks over the install that is there, and bsdtar's hard links
  inside the archive fail on an existing file (`Can't create …
  esp32s3_rev0_rom.bin: File exists`, exit 1) — every binary replaced, the
  install reported as failed.
- **And the tar that reads a zip is not on every runner.** Windows' System32
  tar and macOS's own `tar` are bsdtar, which reads `.zip`, `.tar.gz` and
  `.tar.xz` alike; the `tar` on a Linux runner is GNU tar, which cannot read
  a zip at all. espflash publishes a zip for *every* platform, so
  `bundle-tools.sh` unpacked it perfectly on the two desktops it was written
  on and stopped the Linux release build with `This does not look like a tar
  archive` — after the app itself had built, the same late shape the updater
  config once failed in. `unpack` reads the format off the file (a zip
  begins `PK`) rather than off a name the download does not carry, and
  reaches for `unzip` only where the tar is GNU's. Both branches are proven
  against the real archives, because reasoning about which `tar` a platform
  has is precisely what produced the bug.
- **linuxdeploy deploys the dependencies of every ELF among the app's
  resources, and esp-gdb ships five it cannot resolve.** Espressif puts
  `-3.8` … `-3.12` python-linked gdbs in each archive beside a `-no-python`
  build; the plain name rusty asks for is a 428 KB launcher that reads
  `python3 -V` and execs the match, falling back to `-no-python`. An
  ubuntu-22.04 runner has no `libpython*.so.1.0` at all, so the AppImage
  ended with `Could not find dependency: libpython3.8.so.1.0` — after the
  deb beside it had built, the third late-failing bundle step in one
  release. `bundle-tools.sh` drops the five on Linux only: Windows keeps
  them, because nothing walks its resources and they are what gives gdb its
  Rust pretty-printers. **The container is the way to settle this, not
  reading.** `docker run ubuntu:22.04` with the workflow's own apt list
  reproduces the failure exactly and proves the fix in four minutes, where
  three rounds of reasoning cost three release builds — and it proves the
  parts reading cannot reach: that QEMU and both gdbs still *run* out of the
  AppDir once linuxdeploy has rewritten their rpaths, under the names
  `find_gdb` asks for, with the `[rusty:gpio@` marker intact so
  `has_gpio_model` still answers correctly. A harness that hides `apt` behind
  `>/dev/null` under `set -e` reports all of this as an empty file; two runs
  went that way before the output was let out.
- **KiCad's schematic space and rusty's canvas are two spaces, and only a
  pin crosses between them.** `docs/kicad.md` is the design; the fact that
  decides it is measured: a `Device:LED`'s pins are at `(-24, 0)` and
  `(24, 0)` in symbol coordinates and at `(-4, 18)` and `(4, 24)` on the
  canvas, because rusty draws parts as the components they are and *the
  drawing decides where the pins are*. The two differ **per pin**, not by
  any transform, and most for exactly the library parts a KiCad file is made
  of. So a free wire point has no translation and a point on a pin has an
  exact one — which is why the sheet keeps its pin-to-pin wires and the
  crossing is `schematic::place`, not a new wire model.
- **An autoplaced field is not a witness to a symbol's transform.** Which
  way KiCad's placement angle turns is one boolean that silently reverses a
  diode, and it is `R(-a)`. `Device:LED`'s `Reference` sits at `(0, 2.54)`
  in the library and KiCad writes the instance's on the `+x` side, which
  `R(+90)` predicts and `R(-90)` does not; a resistor on a second board
  agreed; both were wrong, because KiCad places field text where it reads
  well rather than carrying it through the transform. The tell was a third
  instance at 270° whose field implied the opposite sign from the 90° ones
  on the same sheet — **a witness that contradicts itself is not a
  witness**. Pin coordinates cannot settle it either: every rotated part on
  a real 33-symbol board was a one-pin power flag or a point-symmetric
  resistor, and for those both signs give the same two points and differ
  only in which pin is which. What settled it was a drawn diode: a lamp
  turned 90° between a supply and a ground, with its cathode bar on the
  ground side.
- **And a mirror acts on the screen, after the turn** — the same class of
  boolean, and the first implementation had it the other way. Before or
  after is indistinguishable at 0° and 180° and wrong at every quarter
  turn, so a two-pin part cannot witness it either: it took three
  transistors, asymmetric in *both* axes. `(at … 90) (mirror x)` is drawn
  with the base up, the collector left and the emitter right; mirroring
  first answers the opposite for all three, which would exchange a
  transistor's collector and emitter on every quarter-turned part and say
  nothing. `(mirror y)` never appears in a file at all — a left–right flip
  is `mirror x` plus 180°, and KiCad stores it that way.
  **The lesson under both: when a transform is invisible in the geometry,
  draw the asymmetric case and look at it.** Two files and two rounds of
  inference from field positions produced one right answer and one wrong
  one; a screenshot of a diode and one of three transistors produced both.
- **A file format writer patches bytes; it does not reserialise a tree.**
  KiCad writes tabs, puts small nodes inline and long ones one per line, and
  writes `0` where a parser only knows `0.0`, so a tree cannot give back
  what came in and a reserialising writer would rewrite a two-hundred-part
  board on its first save. `kicad_out` finds top-level spans in one
  string-aware pass and replaces only what changed — the same technique
  `migrate.rs` uses on `Cargo.toml`, for the same reason. The gate is a test
  that reads a real file, changes nothing, writes it and asserts the bytes;
  the one beside it moves a part and asserts every *wire* still holds its
  own bytes, because moving is not rewiring.
- **A Windows verbatim path (`\\?\E:\…`) cannot be handed to a tool that
  appends to it.** Tauri's `resource_dir()` comes back canonicalised under
  `cargo tauri dev`, and QEMU joins `-L <dir>` to `esp32c3-rom.bin` with a
  forward slash, which the verbatim prefix forbids: `ROM code binary not
  found`, for a file exactly where the bundle put it, on the first run
  after the bundle shipped. `tools::plain` strips the prefix at
  `set_bundled_dir`, so every path derived from the bundle is plain.
  Anything else that turns a canonicalised path into a command-line
  argument wants the same.
- **A script committed from this Windows checkout is mode 644, and
  `core.fileMode=false` hides that here.** macOS and Linux runners then
  refuse to execute it — `Permission denied`, exit 126 — which is how the
  first v0.6.12 release built only the Windows installer. Mark scripts
  executable in the index (`git update-index --chmod=+x scripts/x.sh`)
  and have workflows run them through `bash scripts/x.sh` regardless.
- **The AppImage bundler deploys the dependencies of every ELF among the
  app's resources.** QEMU's `share/qemu` carries firmware for every
  machine it models, some of it ELF for other architectures
  (`openbios-sparc32`), and linuxdeploy walking the bundled tree ended the
  Linux release with `failed to run linuxdeploy` while the deb beside it
  had built. `scripts/bundle-tools.sh` keeps only the `esp32*` ROMs, and the
  bundle step sets `NO_STRIP` so linuxdeploy does not rewrite the QEMU
  binaries either. Anything else shipped as a resource on Linux has to
  pass the same walk.
- **A file on PATH called `rust-analyzer` is usually rustup's proxy, not
  rust-analyzer.** The proxy exists on every machine with rustup whether or
  not the component does; with the component missing it starts, prints an
  error and exits. `find_rust_analyzer` used to return it because it was a
  file, and the end-to-end test — written to *skip* when there is no
  rust-analyzer — spawned it and failed on every runner. A candidate is a
  rust-analyzer when `--version` says so. The same trap waits for every tool
  rustup proxies (`rustfmt`, `cargo-clippy`, `rust-gdb`): existence is not
  installation. The Toolchain panel's probe follows the same rule now
  (`component_binary`): it did not, and a setup sheet declared a machine
  ready above a status bar saying rust-analyzer was missing — one side had
  asked `--version`, the other had only found a file.
- **`Path::join` inserts the host's separator, so a Windows path built with
  it is Windows-only.** `shell_choices` is pure over its probes and takes
  `windows: bool` precisely so both lists are tested on every OS — and on the
  Linux runner its Git Bash candidate came out `C:\Program Files/Git\usr\…`,
  failing the test that asserts the spelling a Windows user sees. A path
  that must read as Windows is spelled as text; and `Path::ends_with`
  compares components, so a test matching a Windows suffix on Linux compares
  the string. `Path::strip_prefix` is the same trap from the other side:
  `rusty-dbg`'s tests feed records from real Windows gdb sessions, and on
  Linux the fullname was one component no root could be a prefix of, so
  `relative` relativises textually after bringing both sides to `/`. And
  `cfg!(windows)` is the wrong question for whether a drive letter's case
  folds: a path with a drive letter is a Windows path whatever host reads
  it, so `rusty-lsp`'s `same_path_text` folds the letter everywhere and the
  rest only on Windows. Three crates, one class of bug, found one per push
  because `cargo test` stops at the first failing binary — the CI passes
  `--no-fail-fast` now so a run names them all.
- **"Listed and not parked" has one meaning: a restored tab nobody has
  clicked yet.** `restore_tabs` puts the whole strip back and reads only the
  active file, so every other tab is a name with no body until it is
  clicked; `activate_tab` read that same state as a corrupt strip entry and
  *dropped* it — so after every restart the first click on any restored tab
  closed it instead of opening it, and the user reported "clicking a file
  makes its tab disappear". `open_file` and `transplant` already knew the
  lazy case; the strip's own click handler was the one caller that did not.
  When two functions read one state, grep for every reader before changing
  what the state means.

## The sheet

The board editor's parts are schematic symbols — KiCad's drawing of a part
with real pins, wired pin to pin — and `docs/schematic.md` is the design.
`model::symbol` is the drawing (wasm-safe, what the frontend renders),
`model::sheet` the board (placed symbols and wires), `nets` the reading of
the wires (unconditional, because both sides read it), `schematic::kicad_sym`
KiCad's `.kicad_sym` read and written, `schematic::easyeda` an LCSC part
number fetched from EasyEDA's component service and read into the same type,
`simulate::board_file` the file in both its formats, and
`view/panels/simulate/` the editor. `rusty-cli symbol C2286` is the headless
proof of the import.

- **One `Symbol` for every source.** KiCad's coordinates — millimetres, y
  up, origin at the anchor — and KiCad's pin convention: `at` is the
  connection point and `angle` points from it *into* the body, 0 meaning
  the body lies to the right. Every importer converts at its own edge; the
  frontend converts once, in `local` and `orient`, with `MM_PX` chosen so
  that KiCad's 100 mil pin pitch is one kit row pitch — a symbol's pins then
  sit on the same grid as the devkit's header, which is what lets a snapped
  wire meet both ends. A pin is found by number and then by name, so
  `D1.K` and `D1.2` both land.
- **The devkit is a part like any other: `U1`, whose symbol is generated
  from its rows.** `nets::kit_rows` names the header (`GPIO2`, `GND`,
  `3V3`; the ESP32 devkit's `RX` is `GPIO3` by name) and the frontend's
  `kit_symbol` puts one pin on each row, so wiring, hit-testing and the
  file need no second code path for the chip. A wire spells a pin by its
  name when the name is unique in its symbol and by its number otherwise
  (`pin_key`): `U1.GPIO2`, `D1.K`, and `U1.9` only where `GND` repeats.
- **Polarity is wiring.** There is no `active_low` any more. A lamp lights
  when its anode's net is high and its cathode's low; on 3V3 with the GPIO
  sinking the cathode, it lights when the pin is low. A switch to ground
  drives its GPIO low while pressed, one to 3V3 high — `nets::button_drives`
  says which, and the backend reads the *same* function at run start to
  set the pin channel's polarity, so the emulator and the sheet cannot
  disagree. A resistor conducts, a capacitor does not, and both partitions
  matter: the *wired* nets say whether a GPIO sits straight on a lamp (the
  missing-resistor finding), the *conducting* nets say what level reaches
  it.
- **There are three partitions, and the third one is what a short means.**
  `wired`, then `solid` — labels and closed switches, every join with no
  resistance in it — then `conducting`, which adds the resistors. A short is
  two rails in one *solid* node; two rails through a resistor are a voltage
  divider, the commonest analog circuit there is. Joining resistor ends into
  one node made every divider report `ground and a supply share a net`, and
  it was measured before it was fixed: a plain two-resistor divider with its
  midpoint on GPIO4 produced exactly that finding. A net holding both rails
  through resistance now has no level rather than a wrong one — `None`, not
  one of the two it sits between.
- **A resistor's value decides exactly one thing, and `nets::divider_at` is
  it**: where a pin sits between the rails, 0.0 at ground and 1.0 at the
  supply. `ohms` reads `220`, `4k7`, `10K`, `1M` and refuses anything that
  is not a resistance — it lives in `nets` beside the arithmetic rather than
  beside the colour bands it was written for, because a resistor drawn as
  10k and computed as nothing is two answers to one question. One resistor
  deep on purpose: a pin on a rail, a pull-up, a pull-down, and two
  resistors with the midpoint tapped, with parallel paths added as
  conductances because that is exact. Deeper, or a value `ohms` cannot read,
  is `None`. **A path that exists and cannot be valued is not the same as no
  path** — the first version conflated them and put an unvalued divider's
  midpoint flat on ground, so any unreadable resistor reaching a rail now
  refuses the whole answer.
- **The potentiometer reaches the converter, and only where the sheet
  committed.** `P<pin>=<0..255>` was console-only because what a wiper
  converts to depends on what its ends are wired to. With both ends *on*
  rails there is nothing left to assume, so `pot_span` answers with the
  GPIO and the two fractions and the knob becomes ADC counts through an
  ordinary `adc.read_oneshot()`; the text line still goes out beside it. An
  end behind a resistor is refused rather than read — it forms a divider
  with the pot's own track, whose resistance is not on the sheet. The knob's
  zero is pin `1`'s end, and the backend sends the opening counts at run
  start from the same `start` prop the slider reads, so the panel and the
  converter cannot disagree before the first drag.
- **A T-junction needs no junction.** Three wires at one pin have always
  been one net, so a branch dropped on the middle of a wire is a wire to
  *either* of that wire's ends — no model change, no file change, no rule
  change. `wire_under` is the hit test (`pin_under`'s rule: nothing when
  nothing is in reach, and a pin always beats a wire), and `branch_route`
  lays the bends along the trunk from the drop to the nearer end so the two
  draw as a T instead of as a second wire taking its own route. Those bends
  are sheet coordinates like every other bend, so moving the trunk later
  slides the tail off it — the same thing that happens to any hand-bent
  wire whose neighbour moves, and the same repair.
- **The devkit turns like any other part, and that cost almost nothing**
  precisely because it is one: `orient` already carried its header, its
  hit-testing and its wires, so opening it up was `kit_rot`/`kit_mirror` on
  the sheet, the two `is_kit()` refusals in `edit`, and the board's own art
  taking the transform the symbols already take. Two things did not come
  free. Its anchor is the **top-left corner** of a board three hundred
  pixels tall, so a turn about the anchor swings it a board's length away —
  `turned_anchor` keeps the box's middle where it was, and only for the
  devkit, because a KiCad symbol's anchor is already its middle and
  correcting those would move parts in files people have saved. And the row
  names are placed *after* the turn, like every other label, because text is
  never turned.
- **The rules run in one memo** (`eval`, from parts, wires, the firmware's
  levels and the held switches), and every part's face reads its answer:
  `is_lit` for a lamp, `is_pin_lit` per channel for an RGB lens or a digit
  against its `COM`, `level` for a motor's direction pins, `gpio_of` for
  what a knob or a source is on. Findings — a lamp with no series resistor,
  rails shorted, GPIOs fighting, a switch that reaches nothing, a wire to a
  pin that is not there — are `Warning`s with a stable kind, translated by
  the frontend and printed in English by the CLI.
- **A rail is a rail wherever it is drawn, and a name is a wire.**
  `rusty:GND` and `rusty:Supply` put their net at a level without a wire
  running back to the devkit, and two `rusty:Label` parts carrying the same
  value are one net — the two things a schematic uses instead of drawing a
  wire across the whole sheet. Both live in `nets`, so the rules, the probe
  and the backend's button polarity all read them the same way. A label with
  no name joins nothing: an empty tag is one somebody has not written on.
- **`Evaluation` carries the nets themselves**, not only the levels, so a
  pin can be asked what it is joined to. That is the probe: selecting a wire
  says high, low or floating, and lists every pin in its net. The union-find
  was computing it already and throwing it away.
- **A sensor is a part.** `rusty:Sensor`'s value names the channel the
  firmware declared with `[rusty:sensor]`; the sliders under it on the sheet
  feed that channel through the same `sim_sensor` the Flight panel uses, and
  a channel the firmware never declared gets no slider at all — the
  tunables' rule, for the same reason. A buzzer is read by the lamp's rule
  and exempted from the missing-resistor finding, because asking a sounder
  for a series resistor teaches the wrong thing.
- **The board is checked by a machine, twice.** `examples/board_probe.rs`
  boots a project with the pin channel attached, replays the sheet's rules
  over the pins the *emulator* reports, and says what each part did — then
  presses every button and requires the pin it reaches to move. It exits
  non-zero when a lamp wired to a GPIO never lights, which is how a rewired
  sheet and a broken rule both look; `qemu.yml`'s gate 7 runs it on
  `examples/blink-rust` with `RUSTY_CONFIG_DIR` pointed at the workspace so
  the tool ladder finds the emulator that job just built. Pressing drives
  the *released* level first: the emulator models no pull resistor, so a
  pull-up button's press is only an edge if something put the pin high
  beforehand — the first version reported "the button does nothing" about a
  model that was working.
- **The library is three layers, later ones winning by `library:name`**:
  the built-in `Device.kicad_sym` (R, C, LED, SW_Push in KiCad's own shapes)
  and `rusty.kicad_sym` (the parts with a behaviour of their own: pot,
  analog source, display, RGB lens, digit, motor, ground, supply, net
  label, buzzer, servo, sensor — `behaviour_of` keys on
  the id for these and on the reference prefix and pin names for the rest),
  the data directory's `symbols/` — where `lcsc.kicad_sym` holds every
  imported part, one file KiCad itself can open — and the project's
  `.rusty/symbols/`. A file that does not parse is named in the plan's
  notes and skipped; a cache file that no longer parses is moved to
  `.broken` before the import writes, because a read that degrades to
  empty in front of a read-modify-write is how a library of imports
  vanishes. A part whose symbol no library has is *kept*, drawn as a
  labelled box, and named in the notes — deleting somebody's part because
  a library file went missing is a loss, not a repair.
- **`.rusty/sim.toml` is read in two formats and written in one.** Version
  2 is `[[part]]` and `[[wire]]`; a file with no `version` is the first
  board — `[[led]]`, `[[button]]`, each *being* its GPIO — read by the
  reader that always read it and migrated into the circuit it claimed: a
  lamp on GPIO 2 becomes a `Device:LED` wired to `GPIO2` and `GND` with no
  resistor, because the old board had none, and the rules then say so. The
  migration is realised the first time the editor saves and never silently
  before. `Instance.props` is the open bag for a behaviour's knobs (an
  analog source's `max`), so a part added tomorrow carries its settings
  without a format change.
- **Read, never trusted.** A node the S-expression reader does not know is
  skipped; a pin without a number is refused with the symbol's name; a
  derived symbol (`extends`) is skipped whole rather than half-read.
  EasyEDA records the reader does not know go into `warnings` rather than
  nowhere, and a part with no pins is refused — nothing could wire to it.
- **EasyEDA's units are 10 mil, y down, and the pin line is written from
  either end.** LCSC's own library writes `M 40 0 h -10` starting at the
  connection point for one part and `M 20 20 h 10` ending at it for the
  next; the body end is whichever end is not the dot, and the line's
  direction is the angle. The rotation field stands in only when there is
  no line, and its 0 means a pin sticking out to the *right* of its body —
  KiCad's 180. `Value` (`10kΩ`, `100nF`) is the value and `name` is the
  manufacturer's part number; a symbol without `Value` shows its name. All
  of this was read off captured answers (`tests/fixtures/easyeda/`, four
  parts every first circuit has), not the format's documentation — the
  first reader, written from the documentation, would have put the
  capacitor's pins on backwards. `examples/lcsc_probe.rs` captures another
  and prints every record beside what was made of it.
- **The fetch goes through `net`'s ladder**, both hosts (`easyeda.com`,
  then `lceda.cn`), and the failure names the last route tried. It reached
  the service from a machine where `curl` and Python could not.
- **The parts are drawn as the components on the desk, and the drawing
  decides where the pins are** (`view/panels/simulate/art.rs`). A 5 mm lamp
  with its flat and its long anode leg, a resistor with the colour bands of
  its own value, a tactile switch whose cap sinks, a screen on a carrier
  board — beside a devkit drawn as a photograph of one, a KiCad line
  drawing read as two pictures of two different things. So a wire lands on
  the end of a leg, where it does on the bench: `art::layout` answers where
  every lead ends, cheaply and with no markup, and every geometric question
  goes through it; `art::markup` draws the same shapes from the same
  constants, so the leg a wire lands on and the leg that is drawn cannot
  drift apart. A symbol's own coordinates are used for one part only — the
  devkit, whose pins are its header's rows.
- **What the firmware is doing is painted over the drawing, never baked
  into it.** `Layout::lens` is a lamp's dome or a button's cap and
  `Layout::face` a screen, both in the part's own frame; the view paints
  them, so the markup is a pure function of the symbol and its value and
  can be memoised. A lit lamp is its own colour with a drop shadow, a dark
  one that colour dimmed — a red LED is red on the desk with the power off.
- **A part rusty has never heard of is a package**: pins down the two long
  sides, numbered as a DIP is, with its name on the body. An import whose
  reference prefix says what it is — `LED`, `R`, `SW` — gets that part's
  drawing instead, which is what makes importing from LCSC worth doing.
- **Text is never turned.** The body is one `<g>` rotated and mirrored as
  the part is; pin names, the reference and the value are placed in sheet
  coordinates *after* the turn (`pin_labels`), beside the lead rather than
  beyond its tip, where the wire goes. Numbers are not drawn at all: a real
  part has none printed on its legs. `every_part_can_be_drawn_to_a_sheet_
  for_looking_at` writes every part into one SVG when `RUSTY_ART_SVG` names
  a file — the check a unit test cannot be, since whether a drawing looks
  like the thing needs eyes.
- **A part's group takes the pointer; the SVG around it does not.** The
  parts layer is `pointer-events: none` and each part, pin dot and wire
  grab handle opts back in, so a press on empty sheet still reaches the
  canvas for the rubber band and the pan, and a press on a symbol's body
  selects it. A switch is pressed rather than dragged while a session
  runs; the cap sinks, the rules see it conducting, and the GPIO it
  reaches is driven through the same `B<pin>=1` the old buttons sent, so
  firmware written for the text protocol hears it too.

## Numbers on the sheet

The rules say on and off; `solve` says volts and amps. Modified nodal
analysis, written here rather than ngspice bundled — `docs/kicad.md` argues
that decision and names what would reverse it. `circuit` turns a sheet into
a `Circuit`, `live` walks one in step with a running firmware.

**The gates are closed forms, and that is the whole reason for writing it.**
A divider's ratio, resistors in parallel, a Thévenin equivalent asserted
across three loads rather than one, Shockley evaluated on the answer. An
integration with somebody else's engine can only be tested by "it ran and
said something"; this can be checked against arithmetic a person does by
hand, and every bug below was caught that way and by nothing else.

- **Voltages settling is not the circuit being solved.** With limiting
  active, successive Newton guesses can stop moving while the junction's own
  equation is out by two orders of magnitude — each round linearises at the
  same clamped point and hands back the same answer, which is a fixed point
  of the *limited* map and not a solution. The current residual is checked
  as well: what the curve has at the answer against what the straight line
  through the linearisation point claimed. Six tests passed before the one
  that asserted the physics failed.
- **Junction limiting is about the step, not the voltage.** SPICE's `pnjlim`
  compares against the *previous* voltage, so once the guesses settle no
  limiting applies and the linearisation is at the real operating point.
  Clamping the new voltage alone maps every guess past the knee to one
  point — the iteration sits still on a wrong answer, and an operating point
  that genuinely lies above the critical voltage can never be reached at
  all, because the clamp never switches off.
- **An energy-storing element is in `through` at DC and not during a step.**
  An inductor is a short with an equation of its own at DC and a conductance
  with a source in a transient, so its new current comes from the companion
  model (`i = i_before + h·v/L`) and not from the solved source currents.
  Reading `through` in both cases leaves it at zero for ever and the voltage
  across it never decays: a circuit that looks open.
- **The accuracy gate is a convergence order, not a tolerance.** Run an RC
  to one time constant at a hundred steps and at two hundred: the error has
  to halve. A tolerance says an answer was close; this says the method is
  first-order backward Euler, and something accidentally right at one step
  size fails it.
- **Backward Euler, not the trapezoidal rule.** A schematic here is switched
  hard — a GPIO goes from nothing to the rail in one step — and the
  trapezoidal rule answers a step edge with an oscillation that is
  arithmetic rather than circuit. It also earns its place twice: across a
  long quiet gap one coarse step is not merely stable, it lands *on* the
  steady state, which is what makes the event-driven coupling in `live`
  honest rather than merely fast.
- **A node nothing is attached to is not in the circuit, and that is not
  floating.** Every devkit row is a solid net whether or not anything is
  wired to it, so the first bridge handed the solver twenty untouched nodes
  and was told the first one floats. Untouched nodes are pruned. The
  distinction it draws is the useful one: the node past a capacitor is
  *absent* at DC, and a node wired to something that cannot reach ground is
  a finding about a drawing somebody got wrong.
- **What the sheet did not say is refused by name, with the property that
  would say it.** A resistor whose value is not a resistance, a rail called
  `VCC` — which names a net without saying what it is at — a lamp with no
  `vf`, a capacitor whose value is a part number. `docs/schematic.md` has
  said all along that an LED's forward voltage is not on the sheet and would
  not be right to guess, so the lamp asks for it.
- **The converter's full scale is not the rail.** An ESP32's SAR reads about
  1.1 V at its default attenuation and about 3.1 V at 11 dB, so counts
  computed against the supply are out by a factor of three and look entirely
  plausible. `fullscale` is read off a part on the measured net and without
  it `counts_at` answers nothing — the same rule that made `A<pin>=<counts>`
  carry counts rather than volts in the first place, arrived at from the
  other side. The *resolution* has a default and the voltage does not:
  twelve bits is a fact about the chip, and the attenuation is a fact about
  how the firmware configured it.
- **Event-driven is not enough on its own, and only a real run says so.**
  The emulator reports a conversion when the value *changed*, and the value
  only changes when the host sends one — so after a pin moves, nothing is
  said, nothing advances, and the circuit sits at the instant of the edge
  until the next one. The firmware's reading then steps from nothing to full
  scale in a single conversion: a host echoing a pin level in a circuit's
  clothes. Every headless test missed it, because a test that feeds a dense
  stream of lines never stops advancing time. So `Live::advance_by` fills the
  silence on the caller's clock while `settling()` is true, and the guest's
  own timestamps re-sync it whenever one arrives.
- **A gate's resolution is chosen against the firmware, not the physics.**
  The same run then read `0, 4095, 739` — right physics, no resolution: the
  probe reads once a millisecond and the sheet's time constant was one
  millisecond, which is a curve with two points on it and is not
  distinguishable from an echo. Ten times slower is ten points per constant.
  `qemu/live-probe`'s sheet says so in its own header.
- **A pin nobody has reported is not a source.** rusty will not claim to
  know the voltage of a pin it has heard nothing about, so the first report
  of one adds an element and rebuilds the circuit — and the voltages are
  carried across by node, because starting the new one from rest would
  discharge every capacitor on the sheet at the exact moment the firmware
  first touched a pin.
- **The panel reads the solver through one door**, `circuit::operating_point`
  — bridge, solve, and one error type over both halves — held in a memo
  beside the rules' (`solved` next to `eval`, same parts, same wires, same
  held switches, same reported levels). The probe on a wire gains what its
  net is *at* and a selected part gains what is across it, through it and
  dissipated in it. On a divider the two memos say different true things
  about one net: `eval` says nothing is driving it, because a net holding
  both rails through resistance has no level, and `solved` says 1.10 V.
- **A refusal goes where the question was asked, and beside the part it is
  about.** `solved` is an `Err` far more often than an answer — a lamp with
  no `vf` is an ordinary state of a sheet somebody is drawing — so the
  reason takes the number's place in the probe, and `Unsolved::part` decides
  which part's inspector shows it. A reason repeated on all thirty parts
  says "something is wrong here" twenty-nine times over.
- **`Trouble::Floating` names a node number, which is not a thing on a
  sheet.** It is an index into an array `circuit` built and then renumbered.
  `operating_point` is where both halves are in hand, so that is where it
  becomes `Unsolved::Floating { pins }` — what somebody can actually point
  at. It is also the refusal a person is most likely to hit, by moving a
  part off its rail.
- **A sign in prose that contradicts a sign in a test is how a reading gets
  built backwards.** `Solution::through` is documented as the passive
  convention — positive from `plus` to `minus` *through* the element, so a
  source that is supplying reads negative — and said the opposite for a
  while, with the test beside it asserting the truth. `Solution::across`
  and `amps_through` were written against the prose and would have shipped
  every current negated. `turning_a_part_round_turns_its_reading_round` is
  the test that holds it now.
- **One spelling of Shockley.** `Element::current_at` is the curve, and the
  closed-form gate's own helper calls it rather than repeating the formula
  — a gate written against its own arithmetic can only prove the solver
  agrees with the gate.
- **A reading is what a meter shows.** `view/panels/simulate/readout.rs` is
  engineering notation, pure and tested: `4.08 mA`, not `4.0799e-3 A`;
  three significant figures, so a column lines up and the widest a part's
  reading gets is bounded; exact zero is `0 V`, because a prefix chosen
  from `log10(0)` reads as an instrument fault; and anything not finite is
  a dash rather than `inf V` beside a resistor somebody would then go and
  check.

## Meeting C

Embedded Rust does not live alone: vendor SDKs are C, `esp-idf-sys` wraps a C
framework, and teams migrate by putting a Rust module inside C firmware. So
the workbench knows about C without becoming a C IDE.

- **Detection reports, never guesses.** `project::detect` fills `c_interop`
  with what it found — `cc`, `bindgen`, `esp-idf-sys`, a `staticlib`
  crate-type, C sources in the project — and each claim carries the file
  that proves it.
- **Scaffolding refuses before it writes.** `scaffold::c_interop` writes both
  directions (Rust calls C, C calls Rust) and stops on the first path that
  exists: half a scaffold over somebody's code cannot be undone by an error
  message. It does not edit `Cargo.toml` either — `cargo add cc --build` is
  the official path, visible in the dock like every other command.
- **`.h` is C here.** syntect gives the extension to Objective-C, whose
  grammar colours a firmware header wrongly in ways that read as a broken
  highlighter.
- **C/C++ project types are out of scope.** That is ESP-IDF's and
  STM32CubeIDE's job, and doing it badly would cost the thing this workbench
  is actually good at.

## After every feature: review before moving on

Not a release ritual — a step in finishing the feature.

1. **Dead code.** Did this leave an unused `pub fn`, a struct field nothing
   reads, a helper duplicated in two modules? `cargo clippy --all-targets`
   catches private ones; public API needs a grep for call sites.
2. **Did the seams hold?** Does the new code respect the `backend` split, the
   file/wire separation, and refuse-rather-than-guess? If it needed an
   exception, that is a design signal, not a licence.
3. **Is anything now stale?** A pivot leaves debris: config pointing at deleted
   directories, a CLI that predates the domain, docs describing the old
   positioning.
4. **Should the architecture change?** Cheap now, expensive later — the chip
   catalogue became data before boards were referenced everywhere; the AI tool
   context was generalised before a second consumer existed.
5. **Update this file** if a convention was established or a trap was found.
