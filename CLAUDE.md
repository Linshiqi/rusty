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
  -p rusty-edit -p rusty-lsp -p rusty-dbg --no-default-features \
  --target wasm32-unknown-unknown

# Frontend alone, on http://localhost:1425 — much faster to iterate on than a
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
cd crates/rusty-ui && trunk serve

# The whole app
cd crates/rusty-app && cargo tauri dev

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
```

## Layout

| Crate | Does |
|---|---|
| `rusty-core` | Cargo workspace analysis: dependency graph, duplicates, feature unification |
| `rusty-embed` | Chips, boards, project detection, toolchain, memory, flashing, wizard, simulation. `model/` is a directory now, one file per concern, re-exported flat so `rusty_embed::X` still names everything; `simulate/` likewise, with the `.rusty/sim.toml` format in `board_file.rs` beside the planner. Three things that are *not* simulation have their own modules, because `simulate.rs` had grown into the place they lived and every other module was importing "the simulator" to reach them: `tools` (finding a binary — one ladder, one order, for every tool), `install` (fetching QEMU/gdb/gcc, version pins), `net` (proxy policy, and the one `ureq` agent builder) |
| `rusty-ai` | Bring-your-own-LLM providers, the tool registry, the agent loop |
| `rusty-term` | A real terminal: portable-pty (ConPTY) + vt100, rendered by the frontend |
| `rusty-edit` | File tree, syntax highlighting (semantic tokens, not colours), read/write, rustfmt, project search on ripgrep's engine |
| `rusty-dbg` | Debugging: gdb's machine interface parsed, folded into a session state — breakpoints, stepping, stack, variables |
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
code in rusty to exist, which is why `.rusty/parts/*.toml` can add one.

That ceiling is now the *fallback*, not the roof. `qemu/` holds a real GPIO
model — Espressif's stub replaced — and `qemu-release.yml` builds it for four
platforms and publishes it, so `qemu_download` fetches ours first and falls
back to Espressif's. With ours a LED lights because a pin went high and a
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
frontend does not recognise leaves the weaker claim standing.

The pin channel is a chardev of its own (`-chardev socket` + `-global`, since
the machine creates the GPIO device and there is no `-device` to hang it off),
and the backend feeds its lines into **the same** stream the serial console
uses — rule 5 above, one `absorb`. A button press goes both ways when both
exist: `B14=1` on the console for firmware reading rusty's text protocol, and
`14=1` on the pin channel for firmware reading `Input::is_high()`. The
potentiometer stays console-only, because a GPIO carries one bit and there is
no ADC model to put an analog value into.

### 6. Extensibility is data first

See `docs/extensibility.md`. Chips and boards are TOML in three layers
(built-in < user config < `<project>/.rusty/`); simulator parts are TOML in
`<project>/.rusty/parts/`; a chip's register description is the vendor's own
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

## The first run

A freshly installed workbench on a machine with no Rust could do nothing, and
said so only if somebody found the Toolchain panel and worked out which of six
buttons to press first. Every piece needed to fix that already existed — the
probe, the recipes, the archive downloads — and none of it ran unless asked.

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

**The panel's actions live in the left rail, under the panel switchers** —
there is no toolbar row. A full-width strip cost forty pixels of height on
every panel to hold four buttons, and put the thing you press most as far from
the panel it acts on as the window allows. A panel that registers no actions
leaves no gap. Toolbar content is authored for a *column*: dividers are
`h-px w-5`, and nothing in one may be wider than the 46px rail.

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
  pin map's collapsed state and the locale *cache* are localStorage, and that
  is all that is. They all go through `state::local_get` / `local_set` /
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
  The pure halves now live beside the component: `simulate/geometry.rs` (shapes
  and anchors, under tests that pin the real pinmap, rotated anchor points,
  orthogonality and endpoint anchoring) and `simulate/edit.rs` (what rotate,
  mirror, delete, duplicate and undo *do* to the part list). What genuinely
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
- **Five fields copied onto six structs will lose one.** `SimLed`,
  `SimButton`, `SimRgb`, `SimSeven`, `SimDisplay` and `SimPot` each repeated
  `x`/`y`/`routes`/`rot`/`flip`, comments and all, and the file format
  repeated them again in *two* more sets — one for reading, one for writing.
  `flip` was added to the six wire types and to none of the four other
  places, so mirroring a part worked until the project was reopened. There is
  one `Placement` now, and one `file::Place` used in both directions.
  Nested rather than `#[serde(flatten)]` on the wire side, deliberately:
  flatten routes the struct through serde's buffering path, and the frontend
  decodes from a JS value where a buffered number is not reliably the integer
  `rot` needs. The file side does flatten, because TOML is self-describing
  and its keys have to stay where a hand-written file puts them.
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
  instance fails with `os error 10048` on port 1425 — and stopping the task
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
- **Mirror, do not rotate, to face a part at the chip.** Rotating 180° does
  bring a part's stubs to the near edge — and reverses their order, so seven
  wires to a seven-segment cross on the way in. `flip` mirrors: near edge,
  same order. Both transforms mirror the part's *writing* too, so readouts
  and labels carry the inverse (`readable` in `simulate/mod.rs`).
- **Wire bends belong to the sheet, not to the part — KiCad semantics.**
  Dragging a part stretches only the stub-to-first-bend segment; every bend
  the user placed stays put, and the orthogonal pass grows the elbow the
  stretched segment needs. (An earlier fix translated bends with the part;
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
