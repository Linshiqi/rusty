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

# A runner has no global git config, and tests that shell out to `git` mean
# it. This is that environment, and it is how a CI-only failure is found in
# a minute rather than in a release.
GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null cargo test --workspace

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
# flow (format-on-save) looks broken in the mock while correct in the app —
# the sheet included: Run saves it and plans again, and a plan that answered
# with the example put a resistor changed to 1k straight back to 220.
# A fourth, learned late: **a stub that has not kept up with the model is a
# panel nobody can drive.** `plan_simulation` answered with the *first*
# board's `leds`/`buttons` for two format changes; serde ignored every key
# of it and the sim panel opened on a bare devkit, so nothing in it could be
# exercised here at all — and the chips carried no `gpio`, which draws a
# devkit with rails and no header and makes every wire to a pin a finding.
# It is a divider on a C3 now: two resistors, both rails, a tap on GPIO4,
# and 1.10 V at the middle for anyone to check. A fifth: **a Rust map
# crosses as a JS `Map`, not an object** — serde_wasm_bindgen writes one — so
# a stub reading a part's `props.signal` reads nothing and plays its own
# default while the frontend is right; read props with `.get()`. Four
# switches: set `mock.norecents` in localStorage to start on the welcome
# screen (a launch that reopens the last project never shows it),
# `__mock.pickFolder` to answer the folder picker instead of cancelling it,
# `__mock.breathe` before Run to have the playground breathe its LED through
# `[rusty:pwm]` reports instead of blinking it, and `__mock.signal` before
# opening the Simulate panel for a board with a signal generator on GPIO3: a
# run then streams its conversions at 500 Hz on the firmware's clock, a
# firmware printing `raw` and an exponential average `y` of it, declares a
# console `gyro`, and answers `sim_signal_set` — every instrument in the
# Signals tab, the sweep included, can be driven here.
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

# Release: bump the version in Cargo.toml's [workspace.package] and in
# crates/rusty-app/tauri.conf.json, add the `## v<version>` section to
# CHANGELOG.md (rusty-app's tests/version_sync.rs holds the three to one
# number), commit, then push a tag (`git tag v0.2.0 && git push origin
# v0.2.0`) and .github/workflows/release.yml builds installers on Windows
# (NSIS), macOS (universal DMG) and Ubuntu (deb + AppImage), plus rusty-cli
# for each, and publishes a GitHub Release. The workflow also stamps the tag
# into tauri.conf.json *and* the workspace manifest as a safety net — the
# repository said 0.6.12 for twelve releases while the installers were
# right, and every development build called itself 0.6.12 and was offered
# its own release as an update. The Tauri updater keypair is in the repo secrets
# (`cargo tauri signer generate`, then TAURI_SIGNING_PRIVATE_KEY and
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD; the public half is `plugins.updater`
# in tauri.conf.json), so every build emits signed updater artifacts and the
# publish job merges them into latest.json — the feed the running app polls
# a moment after launch, with the tag's CHANGELOG section as its `notes`.
# See "Updating itself" below.

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

# The signal chain, the same way: a generator on the sheet played into the
# converter by rusty's emulator — three tones fitted at the firmware's own
# stamps, one of them switched to by a scenario's `play` step mid-run — and
# the firmware's output held to the design's filter of its input, sample for
# sample. Needs a QEMU with the tables and the exact systimer (qemu-v10 or
# later); `--replay <serial.log>` judges a captured log instead.
cargo run -p rusty-embed --example filter_probe -- examples/filter-lab

# The workbench without the window
cargo run -p rusty-cli -- check .
cargo run -p rusty-cli -- size target/riscv32imc-unknown-none-elf/release/app
cargo run -p rusty-cli -- size .   # or the project: newest ELF under target/
cargo run -p rusty-cli -- symbol C2286   # an LCSC part as a schematic symbol

# The firmware run without the window: build, image, boot, watch. Serial to
# stdout, everything else to stderr; exit 0 passed, 1 failed or timed out, 2
# could not run. A scenario is TOML — [[step]] tables of wait-serial,
# write-serial, press/release, delay, expect-pin, set and play (a signal on a
# generator or a sensor's reading, from that moment on).
cargo run -p rusty-cli -- sim <project> --timeout 10 --expect "ready" --vcd pins.vcd
cargo run -p rusty-cli -- sim <project> --scenario scenario.toml

# The assistant's tools for somebody else's assistant: MCP on stdin and
# stdout, what `claude mcp add rusty -- rusty-cli mcp <project>` runs.
cargo run -p rusty-cli -- mcp <project>

# A shell in a pty the way the terminal starts one, and what it printed --
# the check that the built-in shell comes up, pointed at any executable,
# an installed app's included (`--builtin-shell` starts no window).
cargo run -p rusty-term --example pty_probe -- <exe> --builtin-shell

# Where the seconds go when a project is opened -- the two steps that used
# to be awaited before the window could draw anything of the new project.
cargo run -p rusty-core --example open_cost -- <project>

# Every diagnostic the editor is sent for a file, with who said it --
# `rust-analyzer` for its own analysis, `rustc` for the check -- and when.
# The check for "the error is in Output but not in the editor": whether the
# server never sent it, or sent it and something took it away.
cargo run -p rusty-lsp --example diag_probe -- <project> <file> [seconds]

# What painting a file costs: whole, after a one-line edit halfway down, and
# after a block comment opened at the top — the check that a keystroke in a
# long file repaints its own line and not the file (see "Large files").
cargo run -p rusty-edit --features backend --release --example highlight_cost -- <file>

# Which files rusty will draw dimmed in a project, and every file the scan
# read to decide -- the check for a report of a file wrongly dim, or one
# that should be and is not. `rusty_edit::modules` refuses wherever it
# cannot be sure, so "nothing dimmed" is also what a refusal looks like and
# the probe says so.
cargo run -p rusty-edit --features backend --example unlinked_probe -- <project>

# What EasyEDA actually answers for a part, record by record, beside what
# the reader made of it -- how tests/fixtures/easyeda/ was captured
cargo run -p rusty-embed --example lcsc_probe -- C25804 [out.json]
```

## Layout

| Crate | Does |
|---|---|
| `rusty-core` | Cargo workspace analysis: dependency graph, duplicates, feature unification. `disk/` is the build directory measured and judged (`tree.rs` counts one build tree, `judge.rs` decides what is stale and why, `sweep.rs` removes it, `fs.rs` the filesystem it reads) — the Crates panel's Disk section, `rusty-cli disk` / `sweep` and the `disk_report` tool are its three readers |
| `rusty-embed` | Chips, boards, project detection, toolchain, memory, flashing, wizard, simulation. `screen` reads a monochrome OLED's command stream back into pixels, `sensor` runs a part's own conversion backwards. `model/` is a directory now, one file per concern, re-exported flat so `rusty_embed::X` still names everything; `simulate/` likewise, with the `.rusty/sim.toml` format in `board_file.rs` beside the planner (`plan.rs`), the chips QEMU models and where this machine keeps the tools (`machine.rs`), which peripherals an emulator binary carries (`models.rs`), its extra arguments and a free port (`qemu.rs`) and the sheet a project declares (`sheet.rs`). `nets/` is the reading of the wires the same way — `graph.rs` the three partitions, `evaluate.rs` the rules, `analog.rs` dividers and knobs, `bus.rs` what sits on I2C and SPI, `switch.rs` presses and ties. Two helpers are shared rather than copied: `union_find` (every partition of pins is one) and `layers` (the catalogue, the parts and the symbol library all layer definitions the same way). Three things that are *not* simulation have their own modules, because `simulate.rs` had grown into the place they lived and every other module was importing "the simulator" to reach them: `tools` (finding a binary — one ladder, one order, for every tool), `install` (fetching QEMU/gdb/gcc, version pins), `net` (proxy policy, and the one `ureq` agent builder); `schematic/` is KiCad and EasyEDA — `.kicad_sym` read and written, `.kicad_sch` read and *patched* back (`docs/kicad.md`), an LCSC part fetched — over `model/symbol.rs`, the drawing the frontend renders. And the sheet answers in numbers now: `solve` is modified nodal analysis (DC, a Shockley junction, backward-Euler transient), `circuit` turns a sheet into one and names what the sheet did not say, `live` walks it in step with a running firmware. `sensor` is an I2C sensor's registers from the readings a slider sets; `signal` is what a generator produces, as one line of text rendered sample for sample, `dsp` the spectrum, the single tone and the filter designs a firmware runs (each held to its closed form, and exported as `no_std` Rust), `generator` the tables a run plays — through the circuit to a converter, or into a sensor's registers — and `wave` the lines that put a table on the emulator and read its account of playing it (`docs/signals.md`); `simulate/channel.rs` is the pin channel from the host's side and `simulate/headless.rs` a run without the window, both shared by the app, the CLI and the assistant; `schematic/wokwi.rs` reads a Wokwi `diagram.json` |
| `rusty-ai` | Bring-your-own-LLM providers (both dialects authorise, send and read a line through `provider/mod.rs`), the tool registry, the agent loop (`agent.rs`: open a turn, read it, run its tools), and `mcp` — the registry served over the Model Context Protocol |
| `rusty-term` | A real terminal: portable-pty (ConPTY) + vt100, rendered by the frontend; the built-in shell (`builtin.rs`), and `rusty-shell`, the same as a console program of its own for Windows |
| `rusty-edit` | File tree, syntax highlighting (semantic tokens, not colours), read/write, rustfmt, project search on ripgrep's engine |
| `rusty-dbg` | Debugging, two protocols behind one handle (`any.rs`): `session.rs` is gdb's machine interface, `dap.rs` is the Debug Adapter Protocol for LLDB. Both fold into the same session state — breakpoints, stepping, stack, variables |
| `rusty-git` | The repository's history, Fork-shaped: `graph.rs` lays the log out into lanes and edges (pure, tested — the frontend only turns a lane into an x), `parse/` reads `git`'s machine formats (the log, a commit's diff, the refs, the status), `repo/` runs the user's own `git` in the opened project (`run.rs` the one way a `git` is run, `stamp.rs` what moved without asking git). No libgit2: one binary on PATH is one implementation of the repository format to agree with |
| `rusty-lsp` | rust-analyzer client: stdio JSON-RPC, diagnostics, completion, hover, definition, signature help, code actions, semantic tokens, and navigation — references, implementations, type definitions, outlines, workspace symbols, the occurrences of a name, call hierarchies and macro expansion (`client/navigate.rs`, every answer a place with its line). `client/mod.rs` is the session; `documents`, `edits`, `query`, `navigate` and `hints` are the requests, and `transport`, `handshake` and `dispatch` the plumbing under them; `discover.rs` finds the binary and spawns it, `uri.rs` is the one percent-decoder and drive-letter folder, `convert.rs` turns replies into `model`, `pull.rs` is the diagnostics-pull loop. `positions` is on the wasm side with `model` — the editor converts scalars to UTF-16 at the DOM boundary exactly as the client converts at its own, and it used to do it with its own untested copy |
| `rusty-ipc` | Command-name constants both sides `use`; a test in rusty-app pins each to a real handler |
| `rusty-i18n` | The interface's languages: one TOML catalogue each, a `t!` macro, and the tests that keep them in step. Compiles to wasm — the frontend is the only caller, because backend text crosses the wire as a *name* the frontend translates |
| `rusty-app` | Tauri backend — thin, no analysis lives here. The request/response commands are `commands/`, one module per concern and glob re-exported (`#[tauri::command]` puts a hidden macro beside each command, and `generate_handler!` finds it by the command's own path); the long-running, streaming ones have modules of their own (`ai`, `flash`, `simulate`, `lsp`, `terminal`, `debug`). A command that needs the project asks `AppState::require_root`; blocking work goes through `state::blocking` |
| `rusty-ui` | Leptos frontend (Trunk + Tailwind, no npm). Four layers: `view` renders and never calls IPC, `controller` is where every cross-layer action begins, `state` holds signals and pure operations on them, `ipc` is transport. `ipc::call` appears in `controller/` and nowhere else — check that with a grep before believing it. **Anything that grows past ~1,000 lines is holding more than one concern**: `controller/`, `state/`, `command/`, `view/panels/files/`, `view/settings/` and `view/dock/` are all directories now, one module per thing, and each was one file that had accreted six to fifteen. **A component that outgrows its function keeps what its pieces share in a `Copy` struct** — the signals as fields, the commands as methods — and each piece becomes a component that takes one: `Board` for the sheet editor (`view/panels/simulate/board.rs`), `Pane` for the editing surface (`view/panels/files/surface/pane.rs`). Each was one function of thousands of lines whose view captured whatever it needed from the scope. The signal lab is split the same way: `lab` is its arithmetic — what a source plays, the records every instrument reads, a sweep's steps — pure and tested beside `activity` and `calls`, and `view/lab/` draws it |
| `rusty-cli` | Headless entry point; the CI and bug-report surface, `rusty-cli sim` (the simulator without the window) and `rusty-cli mcp`. One function per subcommand, beside its printers (`check.rs`, `hardware.rs`, `disk.rs`, `sim.rs`, `workspace.rs`); `main.rs` is the arguments and the dispatch |

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

Adding an analysis means adding a tool, and a tool added to the registry is
served to other assistants too: `rusty-cli mcp` (`rusty_ai::mcp`) is the
Model Context Protocol over stdio — `initialize`, `ping`, `tools/list`,
`tools/call`, nothing else — so Claude Code or Cursor calls the same
`project_status` the drawer does and gets the same JSON and the same
refusals. A refusal is a result with `isError`, the agent loop's rule; only a
name that is no tool is a protocol error. **The Cargo workspace is loaded by
the first tool that needs it** (`LazyWorkspace`, the context's
`workspace_on_demand`), not before the question: resolving the graph is
`cargo metadata`, unbounded where no lockfile exists, and most questions
need none. It was a workspace or nothing once, and the app only had one
after the Crates panel had loaded it — so the drawer's Cargo tools told a
user with a project open to open a project. The app seeds it with the
panel's and keeps what a tool loaded (`keep_workspace`); the MCP server,
connected for a whole session while the project changes under it, keeps it
while a stamp of the root manifest, the lockfile and every member's manifest
holds, and reads the catalogue and the newest firmware afresh on every call.

**The MCP server also serves `simulate`, and the drawer does not.** It builds
the firmware, boots it in rusty's emulator with the sheet on its pins and
buses, drives the steps it is given and answers with what the firmware
printed, which pins moved and what crossed the buses — the one tool that
answers "does it do what I think" by watching rather than reading. It runs
commands and cargo may fetch, so it declares `runs_commands` and `network`,
and `ToolRegistry::served()` is the only registry that has it: an MCP client
reads `readOnlyHint: false` and asks the user first, and the built-in
assistant, which has no step where anybody says yes, stays on
`ToolRegistry::workbench()`, every tool of which is read-only. The test that
pins that is still there; it now pins the drawer's registry.

**The project's files are tools as well** (`tools/files.rs`: `read_file`,
`search_project`, `list_files`), through `rusty_edit` so the model sees the
project exactly as the Files panel does — confined to the root, `.gitignore`
honoured, dot entries and `target/` absent, nothing written. They exist
because the assistant could name a project's chip and not read the README
beside it, and told a user asking about a chapter of their own book that it
had no way to open the file. Every answer is capped *and says so*
(`truncated`, `total`): a model handed the first half of a file that reads
as whole describes half a file as if it were one. The file the user has open
travels with their question as `Content::Attachment`, so the common case
needs no call at all. `tests/agent_loop.rs` is the proof that the pieces
meet: a loopback server in the OpenAI dialect answers with a `read_file`
call, and the test reads the second request to see the chapter's numbered
text go back as the tool message — the check a desk with no route to a
provider cannot make by asking, which is the desk this was written at.

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
thousands of edges a second on the line the console shares. **The carrier
comes after an `@` when whoever reported it knew** (`5=0.0750@50.0`): a servo
answers to the *width* of the high part, 1.5 ms is its middle whatever the
period, and a fraction alone cannot be read as an angle at all — the panel
drew every LEDC-driven servo three quarters of the way round its travel
until the emulator said how often. rusty's QEMU models the timer, so it
says; firmware narrating its own line cannot, and `Duty::servo_angle` reads
the fraction straight when there is no carrier rather than inventing one.
The two pulse widths the horn's ends answer to are the *part's*
(`min`/`max`, 500 and 2500 µs by default): 1000..2000 is the other ordinary
pair, and reading one as the other is forty degrees at each end.
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

**That one channel now carries six peripherals**, because all six are the
host's view of one board. `A<pin>=<counts>` puts an analog value on a pin and
`adc.read_oneshot()` returns it; `i2c 68:75=68` puts a byte behind an I2C
address, `i2c 3c=+` declares a device with nothing to read and `i2c 3c=-`
takes one off; `spi 0=1a68` is what a chip select answers with. Back the other
way, `[rusty:adc@<us>] <pin>=<counts>` says what the converter handed over,
`[rusty:i2c@<us>] 3c w 00ae` what crossed the bus, `[rusty:spi@<us>] 0 w
aea501` what crossed the wire, `[rusty:pwm@<us>] 5=0.2500@24002.4` how hard a
pin is driven and how often, and `[rusty:rmt@<us>] 8 100000002000000030` the
bytes a strip's codes carried. All are reported per *change* — a driver
polling a sensor converts thousands of times a second — and all go through the
same `absorb`. The buses remember their last report **per verb**: a
`write_read` alternates a write and a read, so one shared slot suppresses
nothing.

**A device with no registers is a display, and every write to it is said.**
The repeat rule is right for a sensor being polled and fatal for a
framebuffer: clearing a screen is the same sixteen zero bytes sixty-four
times over, each landing somewhere else in its memory, so suppressed it
arrives as a screen with one line on it. The declaration decides — an
address the host gave registers is a sensor, one declared bare (`i2c 3c=+`)
is a display. And a write longer than the 32-byte FIFO crosses it in steps,
so `w+` is **more of the message before it**: the control byte that says
what every byte after it means comes once per transaction, and each step
read as a message of its own has its first data byte taken for one.
`rusty_embed::screen` is the reading of that stream — the SSD1306 and SH1106
command sets, the addressing window, the RAM the data lands in — and the
sheet's display part draws it as pixels. **Which controller is behind the
glass is the sheet's to say** (`panel`), not something taken from the
traffic: the SH1106's window sits two columns into its RAM, and a picture
two pixels out is one nobody can check. Until it is named the part shows
what the firmware prints to `[rusty:disp]`, as it always has.

**A pad has a pull, and a key joins two pads.** `Input::new(pin,
Pull::Up)` with `is_low()` is how nearly every button on every board is
read, and the pull lives in IO_MUX — which upstream does not map, so an
input nobody drives read whatever it last read and every such button read
as *held down* from reset. rusty's QEMU answers IO_MUX on both parts —
and on the ESP32 the pads are a table in *pad-name* order (`GPIO0` at 0x44,
`MTDI`/GPIO12 at 0x34), so the device holds the map rather than computing
it, transcribed from the SVD's field order and checked against it by
script; its input-only GPIO34..39 refuse a pull, as the silicon does. And
`sw 4-6=1` **joins** two pads rather than driving either: which way the
level flows is whichever of them the firmware is driving, which is the one
thing a level cannot say and the whole of why a matrix keypad could not be
simulated — during a scan the row is an output for a moment, so driving the
column low instead would be holding down every key in that column. Both end
in one place in the model (`esp32_gpio_settle`: a driver through a closed
switch, then a level the host stated, then the pad's own pull, then what it
was left at) and `[rusty:sw@<us>] 4-6=1` is its own account of the join.
`rusty:Keypad` is the part, sixteen caps from one rule the drawing, the
paint and the press all read; `nets::switch_tie` is the same thing for a
plain switch wired between two GPIOs, and both travel as `sim_switch`
rather than a level.

**A strip's colours arrive as bytes, and the part says what they mean.**
`[rusty:rmt]` carries what the wire carried, because the emulator reads a
one-wire bit off the *shape* of a pulse code (a long high then a short low)
and not off anybody's timings — so WS2812, SK6812 and WS2811 all report the
same way. `strip_colours` is the WS2812 family's order, green first, which
is the one thing about those bytes that is not obvious; `rusty:Strip` is the
part, as long as its value says (`30`, or `WS2812 x30` — a part number alone
is a name, not a count of two thousand eight hundred and twelve).

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

**A part is a declaration, not a `match` arm.** `<project>/.rusty/parts/
<id>.toml` says which addresses a part answers on, what it reads and in
what units, where each reading sits, how many counts a unit is worth, which
register changes that (`[[range]]`), which bits it clears once it has acted
(`[[clear]]`) and what a reset puts back (`[[reset]]`). `partfile.rs` is
the reader, `sensor::Spec` the wire type the sliders are drawn from, and
`SimPlan.parts` how the frontend gets them — the symbol libraries' three
layers, applied to behaviour. **The three parts rusty ships are three such
files** (`data/parts/*.toml`, compiled in and read by the same reader): a
privileged path for the built-ins is how a declared path rots without
anybody noticing, and the MPU-6050's whole register file — identity, two
full-scale selections, seven readings, a reset — turned out to be
expressible, so it is expressed rather than compiled. **What a declaration
cannot express is named rather than approximated**: `Quirk` is arithmetic
in this crate, Bosch's compensation polynomial over the calibration in the
part's own memory is the only one, and a project's file may not name one
because a quirk is code that is not there. A linear stand-in for a BME280
reads plausibly and is wrong by degrees.

**A sensor rusty answers for moves.** `regs` are bytes fixed for the run —
enough for a bus scan and a `WHO_AM_I`, and no use for a reading. A part with
a `model` prop (`mpu6050`, `bmp280`, `bme280`; `rusty_embed::sensor`) starts
from its own registers — identity, the calibration a Bosch part carries,
readings encoded the way its declaration says — and the sliders
under it on the sheet move those readings while the firmware runs, so an
ordinary driver crate reads a tilted board or a warmer room. The encodings
are the datasheets' formulas run backwards, and the tests hold them to the
formulas run forwards — against the shipped files rather than a fixture, so
what is proven is what a user's own part would get — the BMP280's worked
example first. **The part answers
the firmware's writes as well**: the pin channel reads every `[rusty:i2c]` write
on its way past (`PinChannel::answer`) — a range the firmware chose
re-encodes every reading, a reset bit clears itself, a forced measurement
goes back to sleep — because a register file that only stores would leave a
driver polling a bit that never falls, the lesson the write-triggered bits
taught the model itself. Between runs a slider sets where the next run starts,
which is the sheet's; during one it writes the part's registers
(`sim_sensor_set`).

**The emulator waits for the pin channel before it boots** (`wait=on`). With
`wait=off` the guest started at once and rusty's connection caught up when
QEMU's main loop got to it — measured on Windows at about four hundred
milliseconds, by which time a sensor firmware had asked for its `WHO_AM_I`,
found nothing declared and given up, and a blinky's first edge had gone
unreported. The sheet has to be declared before the firmware reaches for it,
and only a guest that has not started is certain not to have. The channel
retries every ten milliseconds until the run hangs it up (`hang_up`), not for
a fixed time, so a waiting emulator is never left waiting on a caller that
stopped trying.

**Which emulator runs is the most capable copy, not the first.** For every
other tool the first copy on the ladder wins, because somebody put it there;
the data directory's QEMU is usually rusty's own download from whenever it
was installed, and one from before the converter and the buses beat the
current build in the bundle — firmware reading a knob hung in its own
`read_oneshot()` with the right emulator one directory away. `find_emulator`
takes the copy carrying the most model markers (`models_carried`, the
ladder breaking ties, so a user's own install beats an equal bundle), and
the plan says what the one it took can do (`Emulator.peripherals`), so an
early build gets the panel's Upgrade as a stock one does. **A marker is
asked only of the binary that can carry it** (`markers_of`): the ESP32's
interrupt matrix is compiled into the Xtensa emulator alone, so its marker
asked of the RISC-V one would call every current C3 build out of date. And
a marker names what a build *does*, never which build it is — the previous
ESP32 marker was the name of a region the next generation removed, and it
left with it. The QEMU workflow's packaging step checks the same list per
binary, so a build missing one never becomes the release rusty pins.

**What the emulator cannot do on a chip is said before the run**
(`SimLimit`, a stable kind beside the English), **and it depends on the
emulator as well as the chip**: `for_chip(chip, outdated)`. The ESP32 has
every peripheral the C3 has, in its own layout, every interrupt source
reaching its handler — a timer's and a software interrupt's as well as a
GPIO edge's, which is what an Embassy application's clock is made of — and
the FPU on from reset — so with rusty's current
build it has no limit, and with an older copy it has one
(`esp32-outdated`), because each of those arrived after that copy was
built; the panel's Upgrade is the fix. The S3 has never been checked,
whatever the build. **Every peripheral that has a layout of its own per
part picks it in `realize`**, exactly as the pin configuration's offset
always did — never a second copy of the model. `SimLimit::explaining` puts
a diagnosis where a run stopped: the emulator's own `[rusty:cpu]
coprocessor 0 is disabled` (`cpu-fpu-off` — something switched the FPU off
and a float followed), and the `divide by zero` an outdated emulator spun
its way to.

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
pure state machine — `(keys, text, cursor) -> Step`, no DOM — under sixty tests
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

- **The block cursor is drawn, because a read-only field has no caret.** It
  has been two other things. A styled one-character selection could not
  stand on an empty line or past a line's end — nothing there to select —
  and then `caret-shape: block` on the textarea's own caret, which worked
  only while normal mode's read-only guard was broken (*A `prop:` name is a
  JavaScript property name*, under Hard-won specifics): the release that
  fixed the guard lost the cursor, since a browser paints no caret in a
  read-only field. Now `surface/cursor.rs` draws a translucent block (`.vim-cursor`) where
  `vim_cursor` says the next key starts — the one function `vim_key` reads,
  so the two cannot disagree — in visual mode too, where the selection alone
  does not say which end moves. It follows the textarea's `selectionchange`,
  the one event every way of moving the caret fires, shows only while the
  textarea has focus, and restarts its blink on a move by swapping between
  two identical animations. Normal mode selects nothing, so the old
  one-character selections are gone from every path (a cut, a paste,
  `Ctrl+D`) — a selection is what copy and cut act on. Translucent, because
  the textarea's glyphs are transparent by design and an opaque block hides
  the character it points at.
- **The textarea's selection is not the cursor, so the cursor is
  remembered.** Every key starts from the cursor, and `vim_key` read it off
  `selection_start` — right in normal mode, where the selection is the
  character under the cursor, and wrong in visual mode, where it runs from
  the anchor through the cursor: with the cursor right of the anchor the
  start *is* the anchor, so every `l` began again there and a selection
  could not grow past two characters, and `Vjj` stopped at two lines. The
  pure machine was right all along — its tests feed `step.cursor` back — and
  the bug lived in the round trip no test made. `Editor.vim_caret` keeps the
  selection Vim set and the cursor it stands for; `remembered_cursor` trusts
  it while the textarea shows exactly that selection in the same file, and
  anything else (a click, a find, a paste, undo) is read off the textarea as
  before. `modal::tests` drives keys through a simulated textarea, which is
  the test that was missing.
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
- **The machine knows nothing about folds, so `vim_key` maps across them.**
  Its scalars index the document; the textarea's selection indexes the
  screen, which is the document less what is collapsed. They went across
  unmapped — exact with nothing folded, and below a fold every key landed a
  line or more from the cursor. `vim_cursor` reads the textarea through
  `folding::doc_byte_at`, `screen_selection` writes back through
  `screen_units_at`, and a cursor Vim moves into a collapsed region opens
  it first, as VSCodeVim's does, rather than standing on a line nobody can
  see. Both conversions are pure and tested against a real fold.
- **`i` and `a` in visual mode begin a text object** (`select_object`), as
  in Vim. The grammar read them as insert and append, so `viw` left visual
  mode at the `i` and typed the `w` into the file. The object comes from the
  same `object::apply` as `diw`, and a test holds `viwd` and `diw` to the
  same buffer.
- **`j` and `k` remember the column** — Vim's `curswant`, `Vim::want_col`.
  A short line clamps the cursor and the next long enough one gets the
  column back; any other motion or command forgets it, and so does a cursor
  that arrives somewhere this machine did not put it (`Vim::placed`), since
  a click or insert-mode typing moved it from outside.

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

## Large files

A file used to stop being coloured, and start being read-only, at 5,000
lines — and a file of 4,000 was already slow, because everything the editor
did per keystroke was the size of the file. Measured on a 24,000-line file
before the change: the whole file re-highlighted after every pause in typing
(a second of syntect and 4.5 MB back over IPC), every line and every gutter
row rebuilt on every keystroke, and the semantic colours asked for whole
(3.5 MB and 600 ms, then parsed on the one thread the window has). Now the
limit is the 2 MB refusal (`document::MAX_BYTES`), and what a keystroke costs
is what it changed.

- **A file is painted once and repainted by the edit**
  (`rusty_edit::highlight::Painting`). A grammar's state at a line is what
  every line above left, so the parser is kept as it stood between every two
  lines — shared through an `Arc`, since most lines leave it unchanged.
  `repaint` starts at the first line the edit touched and stops at the first
  line below it where the parser stands exactly as it stood before: the text
  and the state are both what they were, so the painting is too. One line
  typed on is one line; a block comment opened repaints what it now covers.
  The test that holds it paints edits incrementally and whole and compares.
  `highlight_cost` measures it: 957 ms whole, 0.56 ms for a line.
- **The backend keeps the painting; the editor names which one it holds.**
  `Files` keeps a dozen, each under a number that only counts up
  (`Document.paint`, `Repaint.version`); a request naming another — evicted,
  or moved on by another window — is answered whole, which is correct
  whatever the editor held. The editor sends the lines it shows plain
  (`crate::paint`: a keystroke echoes the lines it wrote as plain text, and a
  letter typed and deleted leaves the text unchanged and the line plain, so
  the backend is told), and an answer that lands after more typing is placed
  line by line where those lines are now rather than dropped — dropped, the
  backend would be a painting ahead of the screen. One ask at a time per
  group; `painted_whole` is the one door for everything that puts a whole
  painting on screen, and drops an ask on its way.
- **Lines are split at `\n` everywhere**, backend and frontend: a text ending
  in a newline has an empty last line, painted and numbered, because the
  textarea has it too. `str::lines` on one side and `split` on the other is
  an off-by-one between the painting and the draft that a patch cannot
  survive.
- **The echo and the gutter draw a window of rows** (`window.rs`): the rows in
  view and forty either side, in steps of 32, between spacers as tall as the
  rows they stand for, so the scrollbar and every overlay's `row_top` are
  unchanged. Rows are `<For>`-keyed on their line and a hash of everything
  drawn on them (`EchoRow`, `GutterRow`), so typing rebuilds the row it
  changed. The textarea still holds the whole text — the caret, the selection
  and the browser's keys need it — so the pre is given the widest line's
  width (`line_px`), or a long line outside the window would scroll the
  textarea inside itself. Find washes, lenses and the new marks draw only in
  the window, and find places its matches in one walk (`match_lines`).
- **A long file's semantic colours are asked for around the lines on screen**
  (`semanticTokens/range`, 400 lines either side, above 3,000 lines), and
  again when a scroll settles outside what the answer covered. The overlay
  finds a line's spans by halving the sorted list; it used to filter every
  token in the file for every line it drew.
- **Everything that still reads the whole file per keystroke is linear and
  cheap, and was not always.** `fold::regions` scanned forward from every line
  (quadratic in a long block); it is one pass with a stack, held to the old
  scan by a test over real files. `tests_in::runnables` returns at once for a
  text without the word `test`. Undo keeps two hundred snapshots *and* 48 MB of
  them (`EditHistory::trim`), since every step is the whole text.
- **The floor is the textarea itself**: a bare textarea holding 388 KB takes
  about 25 ms from `insertText` to the next frame in WebView2, and typing in
  the editor at that size measures 30–45 ms. Going below it means a textarea
  that holds a window of lines, with the selection, IME and native undo
  rebuilt around it — not attempted.
- **Profile a release frontend.** `trunk serve` builds wasm at opt-level 0,
  where dropping a `String` walks its bytes, and the first profile of this
  work blamed code that costs nothing in release. `cargo tauri dev --config
  '{"build":{"beforeDevCommand":{"script":"trunk serve --release","cwd":"../rusty-ui"}}}'`
  serves an optimised one.

## Bracket pairs, and what a keystroke asks the server

- **Where the caret rests is marked, twice** (`surface/washes.rs`, `brackets.rs`):
  the bracket beside it and its pair are outlined, found on the *painted*
  lines, whose tokens already say what a string and a comment are, so a `{`
  in `"{}"` pairs with nothing; and the other places the name under it occurs
  are washed, asked of rust-analyzer 400 ms after the caret stops — later
  than the edit pulse, so the server has the text the position is in. An
  edit clears the washes (`schedule_pulse`), since they are about the text
  before it.

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
  sorts first and caps at 1,000 — it was 400, cut before any filtering,
  and a large scope lost `println!` and `Vec` to the alphabet before `pr`
  was typed.
- **Completion asks on the first letter and keeps asking while the answer
  is incomplete** (`complete::ask_for`, pure and tested) — VS Code's rule,
  and the one rust-analyzer is built for: it marks every answer incomplete,
  because its imports are searched by the word typed. The editor used to ask
  once per word, on the second letter, and never again, so an ask that
  failed or came back empty while the index warmed up meant no completion
  for that word at all — "unreliable, unusable", in the user's words. A
  word already showing is asked about again 40 ms after the typing pauses;
  deleting widens an open popup and never opens one; an ask that fails is
  made once more if it is still the latest, because rust-analyzer cancels a
  request an edit overtakes (`content modified`).
- **An answer is shown only while its word is still the word being typed.**
  Every ask is numbered and anchored to its file, line and word start
  (`state::CompletionAsk`); every dismissal bumps the number. The popup
  closes when the caret leaves the word — an arrow, Home or End, a click,
  Enter, Backspace past the word's start, a character that ends it — and
  an answer landing after that is dropped. It used to be shown wherever it
  landed: `foo` and a quick Enter opened the popup for `foo` on the next
  line, and the next Enter accepted it. The first answer for a word is
  shown the moment it arrives even with later asks for the same word out;
  an older answer never replaces a newer one.
- **The popup ranks what it shows** (`complete::ranked`): names that start
  with the word first, in the server's order, an exact-case start ahead;
  then names the word only fuzzily matches, the first character at a word
  start — `itr` finds `iter`, `hm` finds `HashMap`, `ln` does not find
  `println`. `filterText` stands in for the label when the server sends one.
  The keyboard's row goes back to the first on every narrowing (kept, it
  became another item that Enter accepted), and Up and Down wrap.
- **Snippets are on, and expanded by the editor** (`complete::expand_snippet`,
  pure and tested): a function arrives as `name($0)` and a macro as
  `println!($0)`, and accepting puts the caret between the parentheses and
  asks for the signature. Only the first tabstop is honoured, so the client
  asks rust-analyzer for `callable.snippets: "add_parentheses"` rather than
  its default of arguments to tab through. Escape closes the popup before
  Vim sees it, as the suggest widget's does in VS Code.
- **rust-analyzer offers an unimported item only to a client that can
  resolve `additionalTextEdits` lazily.** `enable_imports_on_the_fly` is
  gated on `completionItem.resolveSupport` naming that property — computing
  a `use` line per candidate eagerly is too slow — so a client without it
  gets no `Output` for `Out` in a file that lacks the import, and no
  `Output::new` after, since the path does not resolve. Reported as "still
  no completion", with a hover of `{unknown}` for the variable, which was
  correct. The client declares it; `completion()` keeps the last four raw
  answers, numbered; `resolve_completion(path, reply, index)` asks
  `completionItem/resolve` for the accepted item of *its own* answer — the
  popup asks on every keystroke, and the newest answer is often not the one
  an item was picked from — and the frontend splices the edits above the
  caret, shifting it by what was inserted. `label_detail` carries the
  ` (use …)` note so the row says what accepting it will add, and it is a
  field of its own only because the client declares `labelDetailsSupport`:
  without it rust-analyzer glues the note onto the label.
- **A rust-analyzer that failed to load the workspace looks exactly like one
  that works.** It still lexes and parses, so the squiggles keep arriving
  while every completion, hover and jump answers nothing — for ever, with
  the status bar green and nothing on screen saying why. The handshake asks
  for `experimental.serverStatusNotification`; that notification and an
  error-level `window/showMessage` both travel as `LspEvent::Health`, the
  status bar turns crimson with the server's own reason in its tooltip, and
  the dock gets the sentence once (once, not per notification: rust-analyzer
  repeats its state on every change). Found on a report of "completion is
  still missing" from another machine, where the project and the server were
  both fine here. `cargo run -p rusty-lsp --example complete_probe --
  <project> <file> [target]` is the headless check that splits the two
  halves: it starts the server the way the app does, asks for `imp`,
  `impl Qu` and `impl core::` every two seconds until they answer, and
  prints what the popup would keep — `["impl", "impl for"]`,
  `["Quaternion"]` — beside how long the server took to get there.
- **A file in no crate's module tree is the *other* way rust-analyzer goes
  silent, and it looks identical.** No `mod` or `pub mod` names the file, so
  it is in no crate: the server parses it, reports its syntax errors, and
  answers nothing for completion, hover or navigation. It says so —
  `unlinked-file` — as a **Hint**, which the Problems panel filters out by
  the rule above it, so the one sentence explaining the silence was the one
  thing on screen nobody could see. Found by `complete_probe`, which
  reproduced "no completion at all" in one run after three rounds of asking
  the user for facts; the lesson under that is the user's own: *trace the
  whole path from the server's answer to the screen* rather than
  interrogating the person in front of it.
  Two things say it now, and neither is prose. `convert::diagnostics`
  promotes that one code to a Warning, so the squiggle and its hover card
  carry rust-analyzer's own `Insert mod …;` fixes. And the name is dimmed —
  in the tree, in its tab and in the header — which is the only one of the
  two that works *before* anybody opens the file. A notice with a sentence
  and a button sat between them for one release; see the dimming rule
  below.
- **The tree cannot ask rust-analyzer, so it reads the `mod` lines itself**
  (`rusty_edit::modules`, pure and tested). `unlinked-file` arrives only for
  a file the client has opened — a `didOpen` per file in the project would
  be hundreds of notifications and hundreds of diagnostic computations — and
  a dim that only appears once you open the file explains nothing. So the
  backend reads every `.rs` file's declarations once per tree read and names
  the files under a crate's `src/` that nothing declares.
  **It is arranged to fail towards "linked", and every rule in it is that
  decision**: a `#[path]` attribute anywhere in the project takes the whole
  answer away, a `mod` in a comment counts as a declaration, every entry
  point cargo knows is a root, and nothing outside a crate's `src/` is
  claimed about at all. Dimming a file the compiler builds is the confident
  wrong answer in miniature; being silent about an exotic project costs
  nothing. rust-analyzer's own verdict wins where the frontend has it.
- **A fix that edits another file used to be dropped, which is why the one
  fix for that state was unreachable.** rust-analyzer's fix for an unlinked
  file edits *only* the parent module, and the client refused any action
  touching a second file. `convert::split_edits` splits a WorkspaceEdit into
  this file's edits and the others'; `CodeActionFix.elsewhere` names the
  others so the row says what accepting it will write, the frontend splices
  its own buffer and `apply_action_elsewhere` writes the rest the way a
  rename does. A target file with an unsaved draft refuses the whole fix by
  name — those edits land on disk, and the next Ctrl+S there would put the
  draft's stale bytes back over them.
- **Quick fixes hang off the hover card, because that is where the pointer
  already is.** Ctrl+. at the caret was the only door, and it needs you to
  know the fix exists before you go looking for it. The card already shows
  the diagnostic; the actions for that position are asked for **only when
  there is one**, arrive after the card rather than with it — a `codeAction`
  resolves every offer it did not come with, and a tooltip that waits for
  that is a tooltip that does not appear — and are dropped unless the card
  they were asked for is still up. `impl core::ops::Mul for Quaternion {}`
  offering *Implement missing members* is the case this was built for.
- **Two askers need numbered answers.** The client kept *one* code-action
  slot per file, and an accepted fix was applied from it by index. That was
  safe while the caret was the only asker; with a hover asking too, a card
  appearing beside an open popup silently renumbered the popup's fixes, and
  the click would have written another position's edits into somebody's
  other file. `CodeActions` carries a `reply` number and the client keeps the
  last four answers — exactly the shape `completion`/`resolve_completion`
  already had, arrived at the same way and for the same reason.
- **A shade of grey is the message; a paragraph beside it is noise.** The
  unlinked file got three surfaces in one release — a promoted diagnostic, a
  full-width amber notice with a sentence and a button, and a dim in the tree
  — and the user's verdict on the middle one was exact: "a pile of warning
  text", where VS Code says the same thing by greying the name. The notice is
  gone. The name is dimmed in the tree, in its tab and in the header, the
  sentence is the tooltip, and the fix is where every other fix now is: on
  the hover card, where rust-analyzer offers `Insert mod …;`, `Insert pub mod
  …;` and `Insert pub(crate) mod …;` — three precise options where the button
  had one. Removing an affordance was safe *because* the hover card had
  arrived; it would not have been the release before.
- **VS Code does not dim an unlinked file's code, and the protocol says why.**
  Read off the wire: `unlinked-file` arrives as severity 4 (a *hint*), over
  the range `0:0–0:2` — the first two characters — and carries **no `tags`
  field**, so there is no `DiagnosticTag.Unnecessary` for VS Code's
  `editorUnnecessaryCode.opacity` to act on. Side-by-side screenshots looked
  as though VS Code greyed the text; what differs is the theme, plus the fact
  that neither editor can *semantically* colour a file it cannot analyse, so
  both fall back to lexical highlighting. rusty dims the buffer anyway
  (`opacity-60` on the echo, `surface/echo.rs`), which is a deliberate step past
  parity: rusty reads the `mod` lines itself and knows before the file is
  opened, where VS Code has only a two-character hint. The card over the
  dimmed text is a sibling, not a child, so it stays at full opacity —
  opacity compounds through a parent, and the one thing that must stay
  readable is the way out.
- **rust-analyzer offers no `Insert mod …;` when the parent module file does
  not exist.** `src/vector.rs` beside a `main.rs` gets three fixes on the
  hover card; `src/math/matrix.rs` with no `src/math/mod.rs` gets none, because
  the fix has nowhere to write. Nothing rusty can do about it — the removed
  banner's button asked for the same actions at the same position and would
  have been just as empty — but it is the case a user hits, and the dim is
  then the whole of what tells them.
- **Two sources for one fact need a rule about which wins, not an `or`.**
  `is_unlinked` was the scan `||` rust-analyzer's `unlinked-file`, so either
  could assert and neither could retract: adding the `mod` line left the file
  dimmed, because the server's diagnostic stays until it re-analyses *that*
  file and nothing was going to make it. The scan wins wherever it has an
  opinion — it has just read the files — and the server answers only where
  the scan refused. **Which means the refusal has to be sayable**, so
  `modules::unlinked` returns `Option<Vec<String>>`: an empty list is "every
  file is declared" and `None` is "ask somebody else". They were the same
  value, and a refusal read as a clean bill of health.
- **Half-composed text from an input method is not input.** A Chinese IME
  puts its own pinyin segmentation in the field while composing — typing
  `flyegg` passes through `f'l` and `f'l'y` — and fires an `input` event for
  each. The wizard sent every one to the backend to be checked as a crate
  name, and every refusal came back as a red banner about a name nobody had
  typed. Anything that *judges* what was typed waits for `compositionend`
  and ignores an `input` whose `isComposing` is set; anything that merely
  echoes it need not. Every other field that judges text as it arrives — a
  branch name, a search — has the same exposure.
- **A name is refused beside the field, by a rule both sides share.**
  `crate_name_problem` is in `rusty_embed::model`, so the wizard can say
  which character is wrong while it is being typed and disable Create, and
  the generator's own `valid_name` calls it rather than keeping a second
  copy — the Git panel's `ref_name_problem`, applied to the one name the
  generator never sees. It was a backend check on every keystroke whose only
  voice was a banner. **And moving the voice to the field was half of it**:
  `choose` still asked for the plan on every change, the plan still refused
  the name, and clearing the field to type a new one put "`` is not a name
  cargo accepts" in the dock once per change under the field saying the
  same in red. No plan is asked for until the name passes; Create stays
  where it is, greyed. When a check moves to the front, find every call
  that still asks the back. The rule itself was wrong for as long as it
  existed — it let a leading digit through and refused a leading `_`, the
  opposite of cargo, measured on a manifest (`the name cannot start with a
  digit`).
- **A new entry is named where it will be.** The box sat above the whole
  tree with the target folder's path beside it in grey, which is a form
  rather than a file being made, and it said `core/src/` while the tree was
  already showing that folder open. It is a row of the folder's own level
  now, indented with its future siblings and built like `RenameBox` beside
  it; `begin_naming` expands a collapsed folder first, because a box drawn
  inside something nobody has opened is a caret in a void.
- **Say what a failure costs, measured, or do not say it.** v0.6.29's note
  about Espressif's cargo refusing `--lockfile-path` said the workspace "did
  not load, so completion, hover and navigation answer nothing there". Run
  with `RUSTY_LSP_LOG=1`, rust-analyzer says what it actually does: `cargo
  metadata failed and returning succeeded result with --no-deps`. It retries
  without the dependency graph and carries on — the project's own code
  resolves perfectly and only the *dependencies* answer nothing, which on an
  embedded project is `esp_hal::` and most of what anybody types. Overstating
  rusty's own breakage is how a tool stops being believed. And rusty cannot
  configure the flag away, which was measured rather than assumed: the esp
  cargo rejects it even with `-Zunstable-options`, rust-analyzer's
  `--print-config-schema` has no setting for it, and pointing `CARGO` at a
  stable cargo changes nothing because rust-analyzer resolves cargo through
  rustup regardless. The way out turned out to be forwards — a newer Xtensa
  toolchain; see "rust-analyzer picks its lockfile flag from a semver
  comparison" below for why, and for how long it took to find.
- **`RUSTY_LSP_LOG=1` is the difference between a theory and the answer.**
  The app reproduced "partly loaded" on an esp project while `complete_probe`
  on the same project looked perfect, and two rounds of reasoning about
  environments got nowhere. The variable was the target hint — `lsp_start`
  detects at the *opened root*, which for the standard embedded layout has
  no chip, so it passes none — and one run with the server's own stderr let
  out named both the cause and the fallback in a single line.
- **Copy, cut and paste with nothing selected act on the whole line**, as
  VS Code's `editor.emptySelectionClipboard` does (`files/clip.rs`, pure and
  tested). Ctrl+C puts the caret's line and a break on the clipboard; Ctrl+X
  takes the line out, break and all, the caret keeping its column in
  characters; Ctrl+V of *that* text — compared with `\r\n` made plain,
  because the Windows clipboard hands `\n` back that way — goes in above the
  caret's line wherever the caret is, rather than into the middle of it.
  Without a selection the browser's copy and cut do nothing, so the keys did
  nothing. **With a selection they act on the selection, in every mode** —
  Vim's normal mode included, whose cursor is drawn rather than selected, so
  a selection there is a double-click or a drag somebody made. It was once
  counted as "nothing selected", back when the one selected character was
  the cursor, and Ctrl+C on a double-clicked word copied its whole line. A
  cut of a selection in Vim's modes is the editor's own, since the read-only
  textarea would copy and delete nothing, and a paste over one replaces it,
  as VS Code's does; a copied line goes in above the caret only when
  nothing is selected. **A double-click selects the word and not the space
  after it** (`pointer::word_span`): Chromium on Windows takes the
  trailing whitespace too, the platform's convention and not VS Code's, so
  a double-clicked word pasted with a space nobody typed.
  **Paste reads the `paste` event, never `navigator.clipboard.readText()`**:
  the read waits on a permission this WebView never answers — measured as a
  hang — while the event carries the text with the key press. A browser
  neither pastes into a read-only field nor tells the page it was asked, so
  Ctrl+V in Vim's modal states makes the textarea writable for that one
  paste (`open_for_paste`) and `paste_into` makes it read-only again the
  moment the text arrives; a timeout covers a paste that never does.
- **Auto-save is not `save_file`, and it is not a format.** Off by default
  (`auto_save` in `workbench.toml`), it writes a second after typing stops
  — VS Code's `files.autoSave: afterDelay`. It cannot reuse Ctrl+S's path:
  `save_file` re-reads the file afterwards and seeds `draft` from it, which
  is right when the user has stopped and is an editor eating work when they
  have not — the round trip takes tens of milliseconds and the keys pressed
  during it would be replaced by the disk's copy. `format_then_save` is
  worse: rustfmt rewrites the line being typed, and mid-expression it cannot
  parse at all, so every second would put a failure in the dock. So
  `autosave_file` writes and moves the *document* forward to exactly the
  bytes written; the dirty dot is `draft != document.text`, so it clears
  itself and lights again on the next key, and the draft is never touched.
  It rides `schedule_pulse` because that is the one hook every edit path
  already goes through — a second list of edit sites would be a list that
  drifts — on a counter of its own, since the highlight pulse fires four
  times as often.

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
- **One file open on both sides is one document in two views**
  (`controller/views.rs`), as VS Code's split is. It was one group per file
  for a release — `open_file` fronted a path the other group held, and the
  split button *moved* a file — because two drafts of one path overwrite
  each other on save. The document is not one object now either: every
  component reads its own group's signals, so each group keeps a copy of the
  file's draft, painting and disk text, and **the copies are kept the same
  by carrying every change across the moment it happens**, from the group it
  happened in. An edit rides `schedule_pulse`, the hook every edit path
  already goes through (`share_edit`); a repaint that lands carries its
  lines (`share_repaint`); a save's re-read, a reload from the disk and an
  auto-save carry the document (`share_document`). Only the group the edit
  was made in repaints and syncs rust-analyzer, so the backend keeps one
  painting and the server hears once. **The undo history is shared rather
  than copied** (`Editor::histories`, one pair of stacks per path for both
  groups): an undo in either view undoes the last edit made in either, and
  a history per group would let one view's undo put back a text from before
  the other's edits.
- **What stays each view's own is where it is looking**: the caret, the
  selection, the folds, the scroll, Vim's mode, the popups. A file opened on
  the other side copies the view there (`open_view`) instead of reading the
  disk, which would put the disk's text beside an unsaved draft. Closing one
  view of a dirty file asks nothing, since the other still holds it; the
  last view to close is the one that asks, gives the file back to
  rust-analyzer and drops its history. `follow` reads a changed file once
  however many views it has.
- **The other view's textarea is written when it is used, not per
  keystroke** (`Editor::lagging`). An edit carried across moves that view's
  folds with the text (`Folded::follow`) and puts the new text in its echo
  at once — everything anybody sees — and leaves the textarea, whose text is
  transparent, behind. **Rewriting a textarea's whole value lays the whole
  file out again**: measured at 120 ms, twice per keystroke, with a
  24,000-line file open on both sides, where the view being typed in lays
  out only what changed; written per keystroke, typing there took 200–280
  ms a key, and lagging, 36–45 ms, which is what one view costs.
  `catch_up` writes it when the view is next used — a press or focus
  anywhere in the group (`EditorGroup`), which a jump into it and a menu's
  key both cause before they touch the selection — and moves its selection
  from what the textarea holds to what it should, by character
  (`paint::follow_byte`), so a caret after an edit earlier on its own line
  moves along the line. Reading its caret maps the same way without writing
  (`selection_now`); a textarea that has the keyboard is written at once,
  since the next key lands in it; the `prop:value` binding leaves a lagging
  textarea alone, and showing another document clears the flag.
- **"Beside" is the right group, from either side.** The first version sent
  a file to *the other* group: from the right group that moved it left, and
  when it was the right group's last file the right group vanished under
  the click. "Open to the side" and the split button open the file on the
  right as well — a second view, the left keeps its own — and from the right
  group they front it there; the right group closes only when its last tab
  does; both appear only in the left strip, because there is nothing further
  right of the right group. Ctrl+\ acts on the left group whichever has
  focus, for the same reason. The split button needs only a file in front:
  it wanted a second tab to leave behind when splitting meant moving.
- **The split never shows an empty pane.** `settle_groups` closes a second
  group that lost its last file, and a first group that lost its last file
  takes the second's files — so the layout is never "nothing on the left,
  the work on the right". "Move to new window" closes the file on both
  sides, because a detached window is one more editor of it.
- **A group's textarea is found by its group, never by an id.** Both were
  `id="editor-area"`, and a lookup by id answers with the first in the
  document whichever group asked: the right group parked its tabs with the
  left group's caret and recorded the left's position in the history, and
  the Edit menu's undo, cut, copy, paste and rename acted on the left file
  while the right one had focus. `controller::editor_area(group)` queries
  `textarea[data-editor=<group>]`; nothing may look the editor up by id.
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
- **The finder is also where navigation lands.** VS Code's prefixes: `:` a
  line (`42`, `42:7`; Ctrl+G), `@` the outline of the file in front
  (Ctrl+Shift+O) and `#` the workspace's symbols (Ctrl+T), asked of
  rust-analyzer — `workspace.symbol.search.kind` is `all_symbols`, since its
  default finds structs and not functions. A symbol answer is kept with its
  ask (`SymbolAnswer`), so a slow reply to `#gp` is never shown under
  `#gpio`. References (Shift+F12), implementations (Ctrl+F12) and type
  definitions list in the same overlay under a heading naming what they are
  (`PlaceList`), sorted by file and line; one implementation or type
  definition is a jump, as a definition is, and no references is an empty
  list saying so rather than a key that seems to do nothing. Every place
  arrives with its line (`rusty_lsp::Place`), because a row of `lib.rs:41`
  is a riddle. `controller::go_to` is the one jump — the panel, the file,
  Back, the reveal — and Ctrl+click's definition goes through it too.
- **Who calls a function is a tree, in the dock** (`DockTab::Calls`,
  `view/dock/calls.rs`) — VS Code's call hierarchy, which is a tree in a
  side view there, and a list in the finder could not be: each level is
  asked of rust-analyzer when its row opens (`callHierarchy/incomingCalls`
  and `outgoingCalls`), handing back the item the server sent exactly as it
  sent it (`CallItem.item`, opaque JSON — the server puts what it needs to
  answer the next level in `data`). The tree is a flat list of rows with
  depths (`crate::calls`, pure and tested), and an answer finds its row by
  number, so opening a row above one that is still asking does not put the
  answer in the wrong place; a tree started afresh drops answers meant for
  the old one. A click goes where the call is made — in the caller, which
  is the row for calls in and the row above for calls out — a double-click
  to the function, and a count lists every call in the finder. A function
  under itself says *recursive*. No default key: VS Code's Shift+Alt+H is an
  Alt-letter chord, which this binding system leaves to menu mnemonics and
  AltGr.
- **A macro's expansion is a document with no file**
  (`rust-analyzer/expandMacro`, VS Code's "Expand macro recursively"),
  opened beside the code, read-only and painted as Rust by the backend
  (`Files::virtual_document`). Its path is `expansion:/<file>:<line>/<macro>`
  (`rusty_edit::expansion_path`): the tab reads the macro's name, two
  expansions of one macro are told apart by where they were called, and the
  same call expanded again is the same tab. `is_expansion` is how everything
  that would touch the disk leaves it alone — no reveal, no relative path,
  no window of its own, not remembered with the strip, and never announced
  to rust-analyzer, which hears only about `.rs` paths anyway. Nothing to
  expand says so in the dock, as a rename that found nothing does.
- **Ctrl+Tab is the focused group's files by recent use** (`view/switcher.rs`,
  VS Code's editor history in a group). Held, Tab walks down the list and
  Shift+Tab back up, and letting go opens the pick; a tap opens the file
  before this one, and the list waits 150 ms before it draws, so flipping
  between two files never flashes it. The order is `RecentEditors`, touched
  at the two doors a file comes on screen through (`show_document`,
  `front_parked`) and *reconciled with the strip when read*: a closed, moved
  or renamed tab drops out, and a restored tab nobody has fronted follows
  in strip order — nothing else keeps it in step, because a list of every
  place a tab changes is a list that drifts. **Its listener is the one in
  the capture phase.** Every other binding bubbles, and a Tab that reaches
  the focused element first is an indent, an accepted completion or a tab
  sent to the shell — and the terminal and Vim stop the events they take, so
  a bubbling listener would never hear Ctrl+Tab from either. It matches the
  binding ids, not the keys, so Settings can rebind the pair; letting go is
  the `keyup` that leaves no modifier down, so a chord rebound to a bare
  function key is a tap; and the window losing focus with Ctrl still down
  puts the list away, because that `keyup` will never come. From the
  palette or the View menu there is no key to let go of, so the action is
  the tap.

## What the editor draws beside the text

Indent guides, sticky scroll, inlay hints, a minimap and more than one
cursor — VS Code's, each but the cursors switchable in Settings ▸ Editor
(`EditorView`, `[editor]` in `workbench.toml`, written only when a switch is
off) — and a name held under Ctrl drawn as the link it is.

- **Inlay hints are drawn in the line, as VS Code draws them**:
  `let total: f32 = both();`, `sample(sensor: &gyro)`. They were drawn at
  the end of the line for one release, on the argument that a textarea
  cannot make room inside a line — true, and the user's verdict on the
  result was that it looked wrong. **The echo makes the room; the textarea
  stops being where anything is drawn.** The hint is a span in the echo's
  flow (`highlight::decorate`), so the rest of the line moves over; the
  textarea still holds the file, laid out without the hints, so from a
  line's first hint on the two layers disagree about where a character is.
  Everything that read the textarea's layout reads `hints::HintedLine`
  instead — one walk of a line's characters and hints, `pen_after` for tabs,
  answering where the caret at a column stands, where a character starts,
  where a column begins, and which column a point is on. The caret and the
  selection are drawn (`selection.rs`; `caret-transparent` and `::selection`
  hide the textarea's own), a press is placed rather than left to the
  browser (`pointer.rs`, below), every overlay measures through it — find,
  occurrences, brackets, the Vim block, the completion popup, the hover
  card, the lens — and the widest line counts its hints, or the textarea
  would scroll inside itself. **The textarea's value is still the screen
  text, and every offset read off it still means what it meant**: the
  alternative, putting the hints' text into the textarea so the browser's
  layout included it, would have made forty reads of `selectionStart`
  wrong by the hints above the caret, and a missed one is a wrong write.
- **A hint belongs to one side of its column.** A type after a name, a
  chain's type, a closing brace's block belong to the code before them; a
  parameter's name to the argument after it. The caret at that column
  stands on the side of the code the hint is not about — before `: f32`,
  after `sensor: ` — so what is typed there lands beside the code the hint
  describes, and `crate::inlay::follow` moves hints through every edit the
  same way (`echo_edit`), and moves an answer that lands after more typing
  from the text it was asked about to the one on screen. Dropped until the
  next answer, a hint takes its width with it and the rest of the line
  slides under the caret and back. Parameter names are asked for again —
  rust-analyzer's default, which the end-of-line version had turned off.
- **A press is placed here, not by the browser** (`pointer.rs`), because the
  browser places it by the textarea's layout: after a hint it landed as many
  characters along as the hint is wide. `preventDefault`, then the focus and
  the selection by hand; a drag followed from the window, scrolling while
  the pointer is outside the view; a double-click takes a word (a run of
  word characters, of spaces, or one character of anything else), a
  triple-click the line and its break, and a drag after either grows by
  words or lines; Shift extends from the selection's anchor; a right-click
  outside the selection moves the caret first. Points are read from the
  text column's rectangle and not `offsetX`, because the textarea moves
  while an input method composes.
- **An input method reads the textarea's own caret**, which a hint before
  it on its line leaves short of the drawn one, so the textarea is shifted
  by exactly that (`caret_shift`, a `translateX`) from `compositionstart` to
  `compositionend` and the candidate window stands where the caret is.
  Measured: the shift was the hint's width to the hundredth of a pixel, and
  gone once the text was committed.
- **Hints arrive whole only once the server has settled.** rust-analyzer
  answers `inlayHint` with nothing while it loads, and its
  `workspace/inlayHint/refresh` — which reaches only a client declaring
  `refreshSupport`, and is passed on as `LspEvent::Refresh` — can come
  while it is still loading, so what is asked then is empty too; a
  restarted server answered every refresh with no hints and then said
  nothing more, the hints appearing only once somebody typed. The client
  sends `Refresh` itself on the turn to `quiescent` as well, once per
  settling; the editor asks again 300 ms after the last.
- **Ctrl over a name draws it as a link** — underlined, the link colour,
  the hand — when the server says it has a definition (`has_definition`;
  VS Code's behaviour, and the user's request). Asked when the pointer
  moves with Ctrl held and when Ctrl goes down with the pointer still;
  gone when it comes up, the pointer leaves or the window loses focus. The
  echo draws it (`decorate`'s `link`), so the text changes colour under the
  underline rather than an overlay covering it.
- **Sticky scroll is the fold regions read the other way** (`sticky.rs`,
  pure and tested): the headers of the regions holding the line under the
  stuck rows, outermost first, at most five, drawn in the gutter's own style.
  **The search for that line only ever adds rows.** A stuck row covers the
  line it sits on, which changes which regions hold the first line still
  showing, which changes the count: at a block's closing brace one row fewer
  uncovers a line of the block and one more covers the brace, and the first
  version took turns between the two — nothing stuck at all at the offset
  where it mattered. `stuck` grows from the first row's headers until the
  line under them adds none, a fixed point rather than an oscillation.
- **The minimap is a canvas coloured by the theme.** Two pixels a row and
  one a character, scrolled against the editor with VS Code's arithmetic
  (`minimap::frame`, tested). A token class's colour is read off a hidden span
  carrying the echo's own classes, so a theme change recolours it with
  nothing to keep in step. A drag listens on the window and gives the
  listeners back on cleanup.
- **Indent guides** come from the indentation (`guides::indent_levels`; a
  blank line takes the deeper of its neighbours, or every empty line in a
  block would break its guide), and the bright one is the block the caret is
  in. The levels are in `row_hash`, so a row redraws when its guides change
  and not otherwise.
- **The textarea's selection is the first cursor; the others are state**
  (`editor.cursors`; `crate::cursors` is the arithmetic, pure and tested,
  `multi.rs` the keys, and `selection.rs` draws them all alike). A browser keeps one selection, so with a second
  cursor every key that edits or moves is taken in `keydown` and applied at
  every cursor in one pass: one document, one undo step, through the same
  record / echo / `set_buffer` path as any other write. The second cursor
  opens the folds, because every cursor is a document position and a
  textarea holding the folded screen would need each one mapped across them
  for every key. **An input method composes at one place**, and the textarea
  cannot be written mid-composition, so a composition starting drops the
  other cursors rather than type the composed text at one of them. A paste
  with one line per cursor gives each its line in document order, as VS
  Code's does. Off in Vim mode, whose keys are Vim's.

## The Git panel

The repository, with Fork as the reference for what it should look like.
Down the left, every ref (`view/panels/git/sidebar.rs`): the local branches,
each with how far it is ahead of and behind its upstream and whether that
upstream is gone; the remotes' branches, grouped by remote; the tags. A
click goes to the commit, a double-click checks a branch out, the funnel on
hover shows that branch's history alone, and a right-click offers the rest.
Beside it three views (`state::GitMode`): *History* — a graph of lanes
beside the commits, labels on the commits that carry branches and tags, a
commit opened below with its files and each file's patch; *Changes* — the
working tree as staged and unstaged lists, a file's diff, the commit box;
*Stashes*. One row above everything: what is checked out and how it stands
against its upstream (a click goes to it), the filter in force, a search
over the log that dims what does not match and steps through what does, and
refresh, fetch, pull and push — pull and push carrying the counts they
would move — and a new branch.

It was a picker: a menu of branch names whose rows *filtered the log*, with
checkout and delete appearing only once a filter was chosen, so seeing a
branch and switching to it were one gesture and everything else took two.
The view is a directory, one module per region, where it was one file of
1,750 lines.

- **A `git` process is the unit of cost, and it is about 58 ms on Windows**
  before git does any work. The panel was slow because it spent them
  freely: every save anywhere re-read everything — nine processes — and a
  click on a commit was four (`rev-parse` to ask whether this was a
  repository, then `--name-status`, `--numstat` and the patch as three
  `git show`s). A click is one `git show --raw --numstat -p` now, taken
  apart by `parse::diff_parts`; the refs are one `for-each-ref`, whose
  `%(upstream:track)` gives ahead, behind and gone without a `rev-list
  --count` per branch; and "is this a repository" is asked only after
  something has failed (`inside_work_tree`), never before every read.
- **Read only what moved** (`rusty_git::GitStamp`). The stamp is the sizes
  and modification times of what git itself rewrites — `HEAD` and its
  reflog, the refs tree and `packed-refs`, the index, the stash — read with
  no `git` at all, and `stale_since` says which reads a difference
  invalidates: HEAD or a ref moving means history, refs and status; the
  index alone, the status; the stash, the stash list. A save re-reads the
  status and asks the stamp about the rest, and the panel asks the stamp
  every 2.5 s while it is showing — which is how a commit made in a
  terminal, invisible to the watcher, arrives. Where the git directory is
  comes from one `rev-parse --git-dir --git-common-dir` per root, cached, so
  a worktree's stamp reads the refs it shares. **Each part is folded to 53
  bits**, because the stamp crosses the wire as JSON numbers: a full 64-bit
  FNV is not a safe integer in JavaScript, `serde_wasm_bindgen` refused
  every stamp, and the probe failed silently on every call while every Rust
  test passed. Found by committing in a terminal beside the running app and
  watching the log not move.
- **Nothing is set that did not change, and no read runs twice at once.**
  Every answer goes through `set_if_changed` — an unchanged history set
  again rebuilt every row — and `state::ReadGate` keeps one of each read in
  flight with at most one more asked for, so a burst of saves is two reads,
  not ten. A history answer for a filter or a length no longer asked for is
  dropped and asked again.
- **Background reads take no locks** (`GIT_OPTIONAL_LOCKS=0`). `git status`
  refreshes the index as a side effect and takes `index.lock` to do it, so a
  status every few seconds would sooner or later collide with the user's own
  `git commit` in a terminal: `Unable to create '…/index.lock': File exists`.
- **The log draws a screen of rows, not the log.** Every row is 26 px, so
  the rows in view are division (`crate::gitlog::window`, pure and tested):
  a spacer as tall as the whole log keeps the scrollbar honest, and only the
  rows in view and a dozen each side exist — the ones above because a row
  draws the lines that *leave* it. Rows are keyed on what they draw
  (`gitlog::row_key`), and a row's selection and search match are its own
  memos, so a click repaints two rows. A thousand commits were a thousand
  SVGs, all rebuilt whenever the selection moved.
- **The opened commit stays on screen while the next one is read**, dimmed:
  it used to be cleared first, and the pane collapsed and grew back on every
  click. The last 32 opened are kept by hash, which names one content for
  ever — never under `stash@{n}`, which names a different stash after every
  push. The frame, the file list and the patch are three closures with three
  keys, so picking a file redraws the patch alone, and a patch past 1,500
  rows draws that many and offers the rest. The arrows walk the log and the
  commit opens once they stop (140 ms), since holding a key passes rows
  faster than a commit can be read.
- **Switching to the panel is not opening it.** The panel is rebuilt
  whenever it is switched to, and it read everything again and dropped the
  opened commit each time; a root it has shown is only probed now
  (`open_git_panel`), and a new root starts clean.
- **Decorations are read in full** (`--decorate=full`) and a label's kind
  comes from its namespace. In the short form a local `feature/x` cannot be
  told from a remote's `origin/x`, and was drawn as a remote.
- **A remote branch checked out becomes a local branch tracking it**
  (`checkout_args`: the local branch of that name when it exists, `--track`
  otherwise). Checking out `origin/feature` by its own name detaches HEAD,
  which is never what a double-click on it meant.
- **What rewrites history or reaches other people asks first; what git
  refuses safely does not.** Deleting a branch on its remote (`push <remote>
  --delete`), rebasing the current branch, deleting a tag — whose pushed
  copy stays, and the question says so — and aborting a stopped operation go
  through `ipc::confirm`. Merge, `branch -d` and push do not: git's own
  refusal in the dock is the right answer to a mistake there.
- **A name git would refuse is refused while it is typed.** One field serves
  a new branch — from HEAD, or from the branch, tag or commit whose menu
  opened it — a rename and a new tag, and `ref_name_problem`
  (`check-ref-format`'s rules, pure and tested) says under it what is wrong
  and disables OK, where the dock used to say `is not a valid branch name`
  after the command.
- **A merge, rebase, cherry-pick or revert that stops says so above
  everything** (`Status.operation`, read off `MERGE_HEAD`, `rebase-merge/`
  and their kind in the git directory — no process): how many conflicts are
  open, Continue (disabled while any are) and Abort. Continue is `commit
  --no-edit` for a merge and `--continue` for the rest, and dock commands
  run `git` with `GIT_EDITOR=true`, because a `--continue` that opened an
  editor nobody can see would hold the dock for ever.
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
- **The right-click menu is local to the thing under the pointer**: a branch
  offers checkout, merge into and rebase onto the branch checked out (named
  in the item, since "current" says nothing about which that is), a branch
  from it, rename, push, delete — or delete on the remote — its history
  alone and its name; a tag, a detached checkout, a branch from it, push,
  delete and its name; a commit, its hash long and short, a branch or a tag
  on it, a detached checkout, cherry-pick and revert; a commit's file, open
  and copy path; a file in the Changes view, Fork's list — stage or
  unstage, discard (asking first, in words that say whether the file goes
  back to the index or to the last commit; an untracked file is deleted,
  `clean -f` on that one path), stage all, stash this file, copy path and
  full path. Which list it was clicked in travels with the target
  (`GitTarget::Change`), because discard means three different things
  across the two lists. Every write in it is the same dock command a button
  would run, and the panel's own `contextmenu` handler swallows the
  browser's menu everywhere else.
- **Remotes are read from the config, not inferred from their branches.**
  The sidebar used to group `refs/remotes/` by remote name, so a remote
  nothing had been fetched from did not exist on screen — and neither did
  anywhere to add one: a repository made with `git init` could not be
  connected to GitHub from the panel at all, and Push ran `push -u origin
  main` into git's "'origin' does not appear to be a git repository".
  `repo::remotes` is one `git config -z --get-regexp` over `url` and
  `pushurl` (exit 1 is "none"; a name may hold dots, so it is what lies
  between `remote.` and the suffix), read only when the config moves — the
  stamp's `config` part — since remotes change far less than anything else
  here. The section is always drawn, with `+` on its heading; a remote with
  no branches says so; a right-click fetches, changes the URL, renames,
  copies the URL or removes it (asked first). **Push with no remote opens
  the remote form and pushes once it is added**, told the remote's name
  directly, because the list is still being read again when git's `remote
  add` returns. Names follow a branch's rules plus "not taken"; a URL is
  refused only for what git would misread (empty, a line break, a leading
  `-`) — not parsed, because git takes `https://`, `git@host:path` and a
  plain directory alike. Every remote command puts `--` before what was
  typed.
- **A failed `git` is reported by its `error:` and `fatal:` lines, not its
  first line.** git writes warnings first, and on a machine with
  `core.autocrlf=true` staging any file with LF endings begins with
  `warning: in the working copy of '.gitignore', LF will be replaced by
  CRLF…` — so a stage that failed on something real was reported as that
  warning, and the reason three lines down was dropped. `failure_detail` is
  the rule, with git's own stderr from the reproduction as its test.
- **Staging is `add --ignore-errors`, so one path cannot stage nothing.**
  The reported case was "Stage all" in a workspace generated before the
  wizard removed esp-generate's `git init`: `firmware/` is a repository of
  its own with no commits, git refuses it (`does not have a commit checked
  out`) and, without the flag, stages none of the others either. The rest
  go in now and the failure still names the path. `Status` marks such a
  directory (`StatusEntry.nested`, `repo::is_empty_repository` — the one
  rule the wizard also uses): its row says *empty repository*, "Stage all"
  leaves it out, and its menu offers *Include in this repository…*, which
  moves its `.git` to the recycle bin after checking again that it has no
  commits. The move used to stop rust-analyzer and start it again, because
  the server held the directories it watched open and the recycle bin said
  "Some operations were aborted"; the client watches the disk now (*The
  client watches, so the server does not*, under Hard-won specifics), and
  the same move succeeds with the server running.
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
  force-deleting is a decision for a terminal, not a button.
- **A push with no upstream sets one** (`-u origin <head>`), because a bare
  `git push` on a new branch refuses with a hint nobody reads. Stash is
  `push --include-untracked` — "everything I have" is what the button says.
- **The graph is laid out on the backend and only drawn on the frontend.**
  `rusty_git::graph::lay_out` is pure and under tests that name the shapes
  that go wrong — a merge, two tips, a branch bending back into a lane that
  was waiting for it, a lane reused once free. The view turns a lane index
  into an x coordinate and nothing more, so which lane a commit sits in is
  one fact rather than two opinions. **A commit takes the leftmost lane
  waiting for it.** A first parent another lane already waited for joined
  that lane whichever side it was on, so a main line jogged over into the
  lane of a branch merged into it, at the commit the branch grew from; only
  a lane to the left is joined now, and one to the right converges into the
  commit — the main line straight, the branch bent back in, as Fork and
  `git log --graph` draw it.
- **One row is one SVG whose lines run past its bottom edge.** Each row knows
  only its outgoing edges (this row's centre to the next row's), so a line is
  drawn by the row it leaves and `overflow: visible` lets it reach the row it
  arrives at. A lane a commit *opens* for a second parent has no line arriving
  from above, and the layout must not emit one — it did, and every merge grew
  a stray tail. The SVG is `relative z-10`, because the next row's hover or
  selection fill is painted after it and covered the part of the line that
  had crossed into that row — the graph looked cut at whichever row the
  pointer was on. **A line is Fork's shape, not a slant** (`gitlog::edge_line`,
  pure and tested): straight down a lane, a quarter circle where it joins a
  commit on the lane to its left — out of a merge at the top, into the
  commit a branch grew from at the bottom — and an S where a lane only
  shifts, with no commit to turn at. Which end has a commit depends on the
  next row, so each row is handed the next row's lane and keyed on it
  (`gitlog::row_key`); a turn is coloured as the lane it runs down, so a
  branch's curve into the commit it grew from is the branch's colour.
- **Lane colours are fixed hex, the board sheet's exemption applied again**:
  a commit graph is the same colours in every client that draws one, and a
  lane that changed colour with the theme would read as a different branch.
  **A branch's label is its line's colour** (`label_view`): a fixed colour
  per kind — rust for the checked-out branch, green for any other — said
  "branch" and nothing about which line it sat on. Filled, with dark words,
  which read on every lane colour in both themes; the checked-out one
  carries a tick; a remote's is a tint with an edge in the same colour, a
  copy of a branch rather than one; a tag keeps its own colour, because it
  is not a line.
- **`git`, not libgit2.** Every question is one invocation with a machine
  format — `%x1f`/`%x1e` separators for the log and the commit, because a
  subject can carry tabs and newlines; `%1f` fields for `for-each-ref`;
  `--raw` and `--numstat` for the files, in the same `git show` as the
  patch, which is split on `diff --git` in one pass. The user's own git,
  config, credentials and hooks;
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
  `query_param` in `state/window.rs`); the window reattaches to the backend's
  project and shows `Detail` standalone. Hide folds the pane to a strip and
  is session state.
- **After any write, everything is read back** (`after_git`): history,
  refs, status and stashes — each compared with what is drawn before it is
  set — a new stamp as the baseline, and the tree, so the panel never shows
  a state git has already left. The open files follow through the watcher
  like any other change to the checkout.
- **The history follows the disk, including the part the watcher cannot
  see.** `.git/` is a dot directory and unwatched: a working-tree batch
  re-reads the status and asks the stamp about the rest, and the stamp is
  asked on a timer while the panel shows, which is what catches a commit,
  checkout or fetch made in a terminal. Both are no-ops until the panel has
  been opened once, so a project nobody looks at the history of costs no
  `git` per save.

## The tree's own verbs

Drag-and-drop and the right-click menu do to an entry what VS Code's
explorer does: move, cut, copy, paste, rename, delete, copy the path, reveal
it in the file manager. `rusty_edit::entries` is the backend — confined to
the root like every other write, every answer the relative path it produced
— and `controller::follow_move` is why a tab survives its file moving.

- **HTML5 drag and drop needs `dragDropEnabled: false` on the window.** With
  the default, WebView2 routes drops to Tauri's own file-drop handler and
  the page's `dragover`/`drop` never fire on Windows; the tree's drop
  worked in a browser and did nothing in the app. Nothing in rusty used the
  native drop events, so nothing was lost.
- **A row stops the drag events, valid target or not.** Otherwise a drag
  over an invalid row — a folder over itself — bubbled to the sheet and was
  offered the root. `drop_target_for` is the pure rule (a file's row stands
  for its folder; a folder never accepts itself or anything below it; the
  folder an entry is already in is not a move) and the highlight follows it,
  so the target is never a guess.
- **Paths that move take their state with them, before the watcher hears.**
  `follow_move` retargets the tabs, the parked editors, the document on
  screen, the expanded folders, the source-view choices and the stale list
  in both groups, and re-announces moved `.rs` buffers to rust-analyzer
  under the new name. `retarget` checks the separator, not the prefix —
  `src2/a.rs` is not under `src`. The watcher's batch, arriving later,
  finds the old paths gone and nothing open under them.
- **Move refuses an existing name; copy takes a free one.** An editor that
  silently replaces a file eats work, so a move or rename onto a taken name
  is `Error::Exists`; a copy pasted where its name is taken is ` copy`,
  ` copy 2`… before the extension, because pasting beside the original is
  the ordinary case. A directory into itself is `IntoItself`; a rename with
  a separator in it is `BadName` — a rename is a name, a path is a move to
  somewhere the tree did not show.
- **Delete is the recycle bin, never `remove_dir_all`** (`trash`), asked
  first through `ipc::confirm` with the platform's own word for the bin.
  **On a thread of its own**: the recycle bin is a COM call, `trash`
  initialises COM in apartment mode, and on a pooled runtime thread that
  something else had already initialised the other way it *panics* —
  `Call to CoInitializeEx failed. HRESULT(0x80010106)` — so whether a
  Delete worked depended on which worker it landed on. A fresh thread has
  no such history, and a panic there becomes an error.
  Every tab under the entry closes without a second question: the file is
  gone and a draft of it has nowhere to be saved.
- **Reveal is Explorer's `/select,<path>` as one argument** — a space after
  the comma makes Explorer open the home folder — `open -R` on macOS, and
  the containing folder through `xdg-open` elsewhere, where no file manager
  takes a selection portably.

## Two crates from the wizard

`WizardLayout::Workspace` makes the layout the rest of this file keeps
describing — host-testable crates as members, the bare-metal crate
excluded — instead of one crate. `wizard::scaffold_workspace` writes it
*around* the generator's output: the generator runs inside the project
directory and is asked for a crate named `firmware`, whatever the project
is called, and the root manifest, `core/`, the README and the
`<name>-core` dependency line are rusty's, written after the generator
succeeded and never over a file that exists. The name the generator never
sees is checked as a crate name first (`valid_name`), because it becomes
`<name>-core` and cargo would refuse it minutes later. `firmware` needs no
`[workspace]` of its own: the root's `exclude` is what tells cargo it is
not a member, and rusty then finds it exactly as it finds cf-drone-rs's.

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
  hundred per crate and 77 GB of them. **A cache is named after the crate
  rustc compiled, and a crate is a target, not a package**: an example, an
  integration test, a bench, a bin named apart from its package and every
  `build.rs` (`build_script_build`, one name for all of them) each have
  caches of their own. Judged against package names, the scan called all
  their live caches "package gone" — 1.2 GB of this checkout's, for a sweep
  to delete — so the yardstick counts every local target under the name it
  compiles as, and a name several targets share keeps each one's four.
  Nothing else — the same version built by another toolchain looks
  identical and is kept. A whole tree's `incremental/` can also be dropped
  on request; it is a cache and rustc rebuilds it. An empty dep-info file is
  a compile that never finished and is left to cargo, which rebuilds the
  unit regardless.
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
- **A shared build directory is judged by age alone.** Other projects
  build into it, and one project's graph cannot speak for them: judged
  against it, their dependencies read as versions and packages gone and
  their crates' caches as dropped, and the sweep — the auto-sweep after
  every successful build included — removed them. So a build directory
  outside the workspace (`shared`, decided against the graph's own
  workspace root, so a `target-dir` of the project's own inside it is still
  its alone) has only incremental caches idle past the threshold judged
  stale, and the report says why. Superseded caches are not judged there
  either: another project's crate of the same name leaves its caches under
  the same name. **And a sweep keeps what its preview kept**: `SweepPolicy`
  carries `keep_variants`, which it once did not, so `rusty-cli sweep
  --keep-variants 2` listed one set and removed another.

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
  interrupted, and it opens the Environment page (*Building, flashing and the
  environment*, below) rather than this sheet: the page says everything the
  sheet does and what is installed besides.
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
  command off espup's tool row, so the pin is spelled once. **And the pin
  alone was not enough**: espup 0.17 checks a named version against
  esp-rs/rust-build's release list — through the same API — before it
  downloads a byte, and stopped at `GitHub API returned status code: 403`
  on a machine behind a proxy. `--skip-version-parse` skips the check, and
  the same install then completed a minute later. `install::ESPUP_INSTALL_ARGS`
  carries both, and the Xtensa problem's fix text is joined from it, where
  it was a second spelling of the command.
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

## Building, flashing and the environment

The three things an embedded workbench does before anything else, and the
three that were still shaped like a debug view: Build was a button whose
whole result was a scrollback, Flash was a button that opened a dock tab — a
device list, a mode toggle, a command, a second button — and the Toolchain
panel was a grid of readouts over a list of binaries. The references are the
tools people arrive from: PlatformIO's and Arduino's one-click Upload and
their board-and-port box, Xcode's destination picker and activity view,
`flutter doctor` and the ESP-IDF extension's doctor.

- **Flash is one click and builds first** (`controller::device_action`):
  `cargo build --release`, then the plan against the image that build just
  made, then `espflash flash --monitor`. It used to flash whatever had last
  been built — a board that disagrees with the code on screen for a reason
  nothing names. The command still reaches the dock before it runs, and the
  picker shows it before that; the rule the old Devices tab kept, kept
  without the tab.
- **The device is chosen once, in the title bar** (`view/device.rs`), and
  kept. With nothing chosen, the one board plugged in is the answer
  (`only_candidate`, pure and tested): ports that look like boards when the
  chip has a serial bootloader — a C3 on native USB is a port *and* a probe,
  and counting both would ask every time about one board — and probes when
  it has none. Anything else opens the picker, and the verb that opened it
  waits there (`Device::pending`) and runs when a row is picked, rather than
  asking for the click again. A port whose boards cannot carry the project's
  chip says so on its row, and the plan's warning asks before the write.
- **A monitor holding the port is let go first** (`AfterStop::Device`), as
  PlatformIO's Upload does. The flash starts from the old session's own
  exit (`note_exit`), not from the stop's reply: that is the only moment the
  port is certainly free, and the stop's reply can land after the build has
  begun — which is why it clears `session_running` only while no activity
  has replaced the one it stopped.
- **Monitor attaches without writing, and needs no build.** The planner's
  ELF is optional for a serial monitor (a board flashed last week is exactly
  the one somebody wants to watch) and passed when it exists, for defmt and
  panics. A probe's monitor is `probe-rs attach`; it was `run`, which
  rewrote the flash of a board somebody had asked only to watch.
- **The status bar's first item is the activity view** (`crate::activity`,
  pure and tested; `view/activity.rs`): what runs, with the crate being
  compiled and a clock, and then the verdict until the next run — `Build
  succeeded · 12.4 s · Flash 85.3 KB · RAM 20.1 KB / 320 KB (6%) · 2
  warnings`. The counts are cargo's own summary lines, kept **per unit**: a
  crate that fails says its warnings twice (`generated 1 warning`, then
  `…; 1 warning emitted`), and summing lines reported two for one — found by
  driving a failing build, not by reading cargo. A flash that goes on
  monitoring reports *flashed* at espflash's `Flashing has completed!` and
  carries on as a monitor of the same port; a run somebody stopped has no
  verdict; a command's success says nothing, since a git write's success is
  the panel moving. The image's size comes from the memory report, which a
  build now refreshes — `target/` is not watched, so nothing else would.
- **The Environment page answers before it lists** (`view/panels/
  toolchain.rs`): a verdict with the one button that fixes it — *Install what
  is missing*, which installs exactly the rows it marked, not the sheet's
  whole plan with the optional tools in it — then each tool in the group of
  work it serves, drawn in the Settings page's shapes, its state and its
  Install on the right. What is needed is read off the report's own fields
  as the setup plan reads them, plus what the backend's blocking problems
  name: counted from problems alone, the page said "one missing" above two
  rows marked missing. The Xtensa toolchain appears only when the chip needs
  it; "absent — not needed here" at headline size was the old page's most
  confusing line. Install progress is `Setup::busy`, shared with the
  first-run sheet, so an install begun on either is drawn on both.
- **The keys are the ones people bring**: Ctrl+Shift+B builds and F5,
  Ctrl+F5 and Shift+F5 debug, run and stop, as in VS Code — F5 on a paused
  debug session resumes it — and Ctrl+U flashes and Ctrl+Shift+M monitors,
  as in Arduino. Not PlatformIO's Ctrl+Alt letters: on a layout with AltGr
  they type characters, which is why this binding system leaves Alt letters
  alone. Every verb is also a menu row and a palette entry — the same
  `Action`.
- **The debugger's keys are VS Code's too** — F6 pauses, F10, F11 and
  Shift+F11 step over, into and out — **and for as long as the transport
  existed its tooltips named them while nothing was bound to any.** "Step
  over (F10)" was written into the catalogue, so a user pressed F10, got
  nothing, and asked whether Vim had eaten it; Vim passes every F-key and
  the terminal sends none, so the key reached the window's listener and
  matched no binding. A tooltip takes its key from the bindings now
  (`palette::with_chord`, the title bar's helper moved to where the
  bindings are), never from the catalogue, so a rebound key is the key
  shown and an unbound one is not shown at all. The steps are live when
  `DebugState::stopped` says — attached, at rest, not exited — for the
  button, the key (`controller::debug_verb`) and the Project menu's row
  alike; the buttons used to be live whenever the target was not running,
  which included the second or two gdb spends attaching. F10 is a system
  key on Windows, and WebView2 still gives it to the page: the host's
  `AcceleratorKeyPressed` runs first, wry does not subscribe to it, and F10
  is none of WebView2's browser keys.

## The playground

Wokwi's new project, without the folder, the generator or the account: one
click from the welcome screen, File ▸ Playground or the palette opens code
beside a board that runs it. `rusty_embed::playground` writes one project
per chip into `<data dir>/playground/<chip>/`; the window's half is
`controller/playground.rs`.

- **The templates are the proven projects, not new ones**
  (`data/playground/<chip>/*.in`, compiled in): the C3's is
  `examples/blink-rust`'s shape and lockfile, which gate 7 boots, and the
  ESP32's is `qemu/esp32-probe`'s, which gate 16 boots. Each board is
  checked by a test against the sheet's own rules *and* required to solve —
  the red LED carries its `vf` — because a template the rules disagreed
  with would open every new user's first minute on a warning, and one the
  solver refused on a refusal in the inspector. A ground on a devkit that
  has two is spelled by its row (`U1.13`), as the editor itself writes it.
  The lockfiles are in cargo's own order, so the first build leaves them
  alone. The release profile is made for editing, not shipping — no LTO,
  incremental — so a change runs in about two seconds after the first
  build, measured through `rusty-cli sim` on both chips. The `.in` on every
  name keeps any tool walking the repository from taking a template for a
  project of its own.
- **Written once, never over a file that is there**: what somebody wrote in
  the playground yesterday is theirs today. *Restore example* rewrites the
  templates and then opens the playground again from the disk, so no draft
  of the old code is left on screen to be saved back over the example.
  *Keep as project…* copies everything but `target/` into an empty folder
  (refused when the folder has anything in it, or is inside the playground)
  and opens that.
- **Out of the recents list, and marked by the app, not by detection.**
  `open_playground` goes through `open_at`, which is `open_project` less the
  recents write — the list is the projects somebody works on, and the
  playground has its own doors. `EmbeddedProject.playground` names the chip;
  detection cannot know it (it is a fact about where rusty keeps its data),
  so `detected_at` sets it, which is why the playground's folder opened
  through File ▸ Open is laid out as one too.
- **Code beside the board is a layout anybody may have**
  (`Layout.board_beside`, `Divider::Board`, anchored to the right like the
  assistant). The editor strip's board button, View and the palette toggle
  it; entering a playground turns it on, leaving one turns it off, and
  between two ordinary projects it stays as it was set. A kept playground
  keeps it, since it is the same work carrying on. With the board in view,
  Run no longer switches to the Simulate panel.
- **Beside the editor the sheet is the whole pane** (`Simulate { compact }`):
  the parts library opens from the corner's `+` and closes on the part it
  adds, as Wokwi's picker does, and the inspector floats over the sheet
  while something is selected. Both stay mounted and are hidden by class
  rather than rebuilt, because the inspector's closure captures half the
  editor and a conditional around it would have to be rebuilt per change.
  Import, export, tidy and the grid dial are the panel's; the pane's own
  two are *open the whole editor* and *hide*.
- **What runs is what is on screen — the code and the board.** Build, Run,
  Test and Flash write every unsaved draft first (`save_all_then`: both
  groups, the parked tabs, one write per path), moving each document
  forward to the bytes written as auto-save does, so a key pressed during
  the round trip is not replaced by the disk's copy. Run also writes the
  board editor's unsaved sheet (`save_sheet_then`), because the pin
  channel's polarities, the buses and the knobs are read off the file. The
  editor on screen offers its sheet under a number (`offer_unsaved_sheet`)
  and withdraws only that number: when the plan reloads, the editor that
  replaces it can register before the old one's cleanup runs.
- **Run while a simulation runs restarts it**, Wokwi's loop — change the
  code, run it again. It is the flash-waits-for-the-monitor mechanism
  generalised: `AfterStop::Simulate` is set, the run is stopped, and the
  new run starts from the old one's own exit (`note_exit`), the one moment
  the old session's end cannot clear the new one's running flag. While a
  simulation runs the title bar's Debug becomes Restart in place, so
  nothing beside it moves, and Ctrl+Shift+F5 is VS Code's restart.

## Updating itself

The running app asks the release feed a moment after launch, and when a
newer version exists a sheet shows the release's own notes with *Download
and install*, *Later* and *Skip this version*. `crates/rusty-app/src/update.rs`
drives `tauri-plugin-updater`; nothing in the WebView calls the plugin, so
it needs no capability.

- **Four commands, not one, because the shape is a hundred-megabyte download
  that ends by restarting the app.** `check_update` answers with an
  `UpdateStatus` and parks the plugin's `Update` in `AppState`;
  `download_update` streams progress on a channel and verifies the signature
  before a byte is kept; `apply_update` installs and restarts; `skip_update`
  writes `skipped_update` to `workbench.toml`. Download and restart are two
  gestures on purpose — an update that restarted the workbench the moment
  its download happened to finish would take an unsaved edit with it — and
  the verified bytes wait in `AppState` (`UpdateStage::Ready`), so the
  restart is still on offer from Settings ▸ Updates after the sheet is put
  away. A Tauri command cannot be cancelled from the WebView, so the fetch
  runs as a task whose abort handle the state holds: the slot rule again.
- **The launch check fails silently; the manual one says what it found either
  way.** No network is the normal state of a bench, and a red banner at every
  launch teaches people to dismiss banners; but Help ▸ *Check for updates…*
  opens the sheet whatever the answer, because a menu item that sometimes
  does nothing is one people stop trusting. The manual check also ignores a
  skipped version — skipping is about not being *interrupted*.
- **On Windows `install` never returns.** The plugin hands the NSIS installer
  to the shell with `/P /R` (passive, restart the app) and calls
  `std::process::exit(0)` itself, after `cleanup_before_exit`. On macOS and
  Linux it replaces the bundle and returns, and `app.restart()` is what picks
  the new one up. So `apply_update`'s answer never arrives on Windows, and
  the frontend treats that as normal.
- **Restart asks first while something runs that would not stop by itself**
  (`controller::apply_update`, `running_work`): an install, a build, the
  tests, a flash or a dock command, named in the question. The restart ends
  the process and what it started goes with it, and an install cut off
  halfway is how a toolchain loses its compiler — a restart during an espup
  run left a user's `stable` with every component removed and their `esp`
  with no `rustc.exe` (the next file, `rustc_driver.dll`, was loaded by
  something and could not be deleted, which is where the removal stopped),
  so every cargo command failed until both were reinstalled. A monitor or a
  simulation is only stopped, as its own Stop would.
- **The whole flow is testable without a release.** Debug builds honour
  `RUSTY_UPDATE_FEED=<url>` as the endpoint (the plugin allows plain http in
  debug builds only, with a warning), and a fake installer signed with the
  real private key verifies against the pubkey in the config — the
  scratchpad's `feed/serve_feed.py` serves both. Never take that test as far
  as *Restart now* against a fake artifact: see the previous point.
- **The feed's `notes` are the release body**, written to `release-notes.md`
  by the publish job and read by both the feed merge and the release step,
  so the sheet and the release page cannot say two different things. The
  sheet draws them with the Markdown page, so they are written for the
  person reading it and for nobody else.

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
  caret that drifts. So `lens_line` (`view/panels/files/lens.rs`, pure,
  tested) puts the lens on the attribute line above the item — the row VS
  Code's lens occupies — and on the item's own line when nothing is above
  it, after everything drawn there, hints included (`hints::line_right`).
  Positioned through `row_top` like every overlay, skipped when its line is
  inside a collapsed fold.
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
  **It skips a line that calls `t!`, asked of the macro and not of the
  characters**: `format!(` ends in `t!(` as well, and while the skip was a
  substring every sentence built with `format!` went unread — ten of them,
  found the day the rule was mended.
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
Debug, Flash, Monitor, and the device they go to) sit in the title bar's
centre with the file finder's icon (`view/run.rs`), where Xcode and CLion
put them: one position on every panel, in a row the window already spends,
and Run switches to the board itself so nothing is far from anything. Three
groups in the order the work happens — build and test, run and debug on the
simulator, flash and watch the board — and every tooltip carries the key
that does the same, read off the bindings so a rebound one is what is shown.
Build is `cargo build --release`
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
header, Git's branch row, the Crates and Environment headings, the board
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

**The status bar is one line, and text a server wrote is clipped.** Every
item is `shrink-0` and `whitespace-nowrap` (`Status`, `BuiltFor`,
`PinStatus`), except the language server's, which is `Status`'s `clip`:
rust-analyzer's progress is its own words, a crate name per piece of work
and for `Roots Scanned` a whole path — measured at startup as `Roots Scanned
92% 13/14: C:\Users\…\lib/rustlib/src/rust/library\std` — which pushed the
bar's other items off the edge. It is cut with an ellipsis at 22rem, is the
first thing to give way when the bar is short of room (the interface zoom
makes a 960 px window narrower than that), and carries the whole line in its
tooltip. The chip item was wrapping onto two lines at the same width before
it was given the same rule.

**Lists of things the shell has are generated from the thing.** The View
menu and the palette iterate `DockTab::ALL` and the panel registry; five of
the nine dock tabs were once spelled out by hand and the other four were
reachable from nowhere but a click on the strip. `Divider::ALL` and
`Divider::default_size` play the same role for Reset layout.

**A menu is read at a glance, so what it holds is folded into flyouts.**
The View menu grew to thirty-seven rows, and the user's word for it was
"too long". It is the two finders, five flyouts — Go to, Appearance,
Layout, Panels, Panel below — and the Vim switch, which stays on the top
level because it was in a submenu once and "I cannot turn Vim on" was the
report. `view_menu` is a function of the shortcut lookup alone, so
`the_view_menu_folds_without_losing_a_row` holds the folded menu to every
action the flat one reached. **Which flyout of a level is open belongs to
the level** (`menu.rs`, `Rows`) and follows the pointer on a delay — the
hover card's generation rule: 80 ms before the first opens, 200 ms to
switch, 300 ms to close on a plain row. Opened and shut by each row's own
enter and leave, the trip that matters broke: from a row down and across
into its flyout the pointer crosses the row below, which shut the flyout
it was heading for. The flyout sits raised by its own padding so its first
row is level with its parent, and the parent stays lit while it is open.
Driven in the browser with events dispatched at chosen gaps, which is the
only way to hold a timing to account.

**The dock's strip carries the tabs that have something to say.** Problems,
Output and Terminal (`DockTab::PINNED`) are always there; the others
appear when something puts them there and go when the user hides them with
the × on the tab. Two doors: `show_dock` — a button, the View menu, the
palette — puts a tab on the strip *and* in front; `reveal_tab` puts it on
the strip and nothing else, because a panel that switched under somebody
reading Output is the banner that reflowed the workspace again. The second
is called from `absorb`, beside the reading of the protocol: telemetry or a
tunable reveals Plot, a sensor declaration reveals Flight, a gpio report
reveals Waves, and a debug session reveals Debug and Registers together. A
`[rusty:pwm]` line reveals nothing, since a servo is a duty too. Calls
comes through the first door, from *Show call hierarchy*. The strip
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
`String`; `optional_no_strip` is the one that takes the `Option`. A fenced
code block is highlighted by the backend (`files::HIGHLIGHT_SNIPPET`,
`highlight::snippet`), which resolves the fence's language the way a
Markdown renderer does — extension first, then name without regard to
case, syntect's `find_syntax_by_token` — and paints with the editor's own
`class_of` map, so a fence and the file it was copied from are the same
colours. The runs are cached in `editor.snippets` by a hash of language
and text, shared by both groups: one request per distinct block however
often the page re-renders, and never stale because the key is the content.
An answer still streaming renders its blocks plain (`Markdown`'s `live`),
since a block re-rendered on every delta would ask once per delta for a
text about to change. No language, or one no grammar answers to, stays as
written; so does everything under the trunk-only preview.

**Settings are macOS System Settings' shapes, and prose is one line under
the group.** `view/settings/shell.rs` has the three: a page is a title over
`Group`s, a `Group` is a rounded box of `Row`s divided by hairlines with an
optional small title above and an optional one-line `footer` below, and a
`Row` is a label (with an optional second line) on the left and its control
on the right — `stacked` when the control is a URL or a path. Controls are
`Segmented`/`Segment` for a handful of exclusive choices, `Switch` for a
boolean, `TextField` for a value a machine reads. The first version put a
paragraph under every field and a summary under every sidebar entry, and the
user's verdict was exact: cluttered, and written to be admired rather than
read. Labels are one to three words; a footer is one sentence stating a
fact ("Changing this restarts the terminal"), never a rationale; the
rationale lives in the code comment. Same for the assistant drawer: an
empty transcript is one line and the composer — the paragraph about the
tools, the four openers and the tool-name chips were read once and then in
the way of every conversation after, and the tools are listed in Settings.
The drawer's width is `Divider::Assistant`, anchored to the right so
dragging left grows it, with the same default, bounds and storage key
plumbing as every other divider; it was a fixed 400px for a release.

**The open file goes with a question**, as VS Code sends the active editor:
a chip above the composer names it, its × drops it for that question, and
it travels as `Content::Attachment { path, text }` — its own block, so the
transcript draws the chip and only the providers render it as prose
(`Content::prose`, the one place the framing is spelled). The text is the
focused group's *draft*, unsaved edits included, cut at
`ATTACHMENT_CAP` on a character boundary and marked as cut.

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
  which is what the canvas editor writes) and user-defined parts
  (`parts/`, one TOML per part — see *A part is a declaration* below).
- Theme, divider positions, the editor's text zoom, the Markdown page's
  zoom (its own factor: prose and a listing are read at different sizes),
  the interface scale, the
  file tree's fold, the Git panel's diff layout (one column or side by side)
  and the locale *cache* are localStorage, and that is all that is. (The pin
  map's collapsed state was on this list while the map floated over the
  editor's corner; it is a status-bar popover now, closed unless clicked,
  and `PinStatus` takes the old key out once.) They all go through `state::local_get` / `local_set` /
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
- **The client watches, so the server does not.** Left to watch the disk for
  itself, rust-analyzer holds handles on the workspace's directories, and on
  Windows a directory with an open handle anywhere below it cannot be
  renamed or moved. Measured with `diag_probe` and a rename loop: `src`
  refused, access denied, on every attempt while the server ran, and
  `src/math` — nothing below it — renamed freely; the tree's rename and move
  met exactly that, and so did moving an empty `.git` to the recycle bin. A
  client declaring `workspace.didChangeWatchedFiles.dynamicRegistration` is
  asked to watch instead (`files.watcher: "client"`, rust-analyzer's
  default, said explicitly), and rusty already has a watcher: `watch_project`
  tells the server what each batch amounts to (`rusty_lsp::watched`) — the
  difference between two listings of the files it reads on a structural
  change, so a renamed directory arrives as every file in it deleted and
  created, which one event for the directory would not be. Measured again
  after: `src` renames every time, and an excluded, linked `firmware/`'s
  empty `.git` goes to the recycle bin with the server running — which is
  why *Include in this repository…* no longer stops and restarts it.
  `LspClient::did_change_watched_files` sends nothing until the server has
  registered, since a server that never registers is watching for itself.
- **An open document is the client's to keep, so a closed one must be
  given back.** rust-analyzer answers from the text it was sent for any
  document the client opened and ignores the disk for it; `didClose` was
  never sent, so a file closed in the editor stayed, to the server, the
  text it had when it was open — whatever a `git checkout` did to it
  afterwards — and every file ever opened stayed in its memory. A closed tab
  and a moved file's old name close now (`lsp_closed_doc`), the client drops
  the file's pulled analysis with it, and the puller skips a file closed
  while its pull was on the way.
- **`procMacro.enable: false` is not a lighter mode — it is poison.** It
  takes the built-in derives down with it, sysroot trait resolution collapses,
  and any open file containing an `impl` with `&self` gets *no diagnostics at
  all*, silently. Leave proc macros on. Probed live with `--example probe`,
  which injects an in-buffer error and asserts it is still present at the
  end of a 45s watch, on a host project and on a real Xtensa `build-std`
  project.
- **rust-analyzer's own analysis is not the compiler, so the check is on.**
  It was off for a month (`checkOnSave: false`) on a wrong reading of one
  symptom — squiggles that appeared for a few seconds and vanished were
  blamed on `build-std` — beside a claim that native diagnostics cover
  "type errors, unresolved names". Measured with `examples/diag_probe`:
  rust-analyzer says nothing about `pub v: Vector3d` with no such type, an
  unused import or a borrow error; only rustc does. The user found it as
  "errors only show in Output after a build". **The vanishing was this
  client**: with pull negotiated, rust-analyzer *pushes* only the check's
  results and answers *pulls* with its own — confirmed in its `main_loop.rs`
  — and the client sent each to the frontend as it arrived, so an empty pull
  replaced a rustc error three seconds after it came. `Shared` keeps `pulled`
  and `pushed` per file and always sends `pull::merged`, which also drops
  exact repeats: a `core` shared by the host workspace and the firmware's is
  checked twice and reported twice. An empty push clears only the check's
  part. **And the check has to be started.** rust-analyzer runs it on a save
  and at no other time, and a workspace reload clears its results — measured
  as the errors arriving, being wiped at 58 s by the build-data reload, and
  not coming back — so the client sends `rust-analyzer/runFlycheck` each time
  `experimental/serverStatus` turns `quiescent`. A build-std firmware project
  watched for four minutes with the check on produced no storm and no wipe.
  The Files tree colours what the check finds (`tree::problem_mark`): a file
  with errors red with the count, warnings amber, a folder in the colour of
  the worst thing inside it — and never for a hint.
- **A flattened `"cargo.buildScripts.enable"` key beside a `"cargo"` object is
  silently ignored** in rust-analyzer's initializationOptions. The first
  attempt at the fix above failed while looking applied, because the sibling
  `procMacro` object *did* take effect. Nest keys in their object.
- **Nothing goes on the critical path of a project switch that the new
  project's first paint does not need.** Opening a project is one awaited
  command, and until it answers the window is still showing the last
  project — so every millisecond inside it is a millisecond of a workbench
  that looks frozen. Two things were in there that nothing on screen reads:
  - **The Cargo analysis.** `Workspace::load` resolves the whole dependency
    graph: measured at 113 ms on a small embedded project, 812 ms on this
    workspace, and unbounded on one whose lockfile does not exist yet —
    which is exactly the project somebody has just generated, and exactly
    the case reported. Only the Crates and Features panels read it, and
    neither is on screen when a project opens. `AppState::workspace` loads
    it on the first ask and keeps it, without holding the state lock across
    the load (that would queue every other command behind it: the stall
    moved rather than removed) and parking nothing if the project changed
    underneath.
  - **The outgoing language server's funeral.** `LspClient::drop` asks
    rust-analyzer to shut down and waits for the process, and `set_lsp` ran
    it inline. Measured at 540 ms against this workspace — the server
    answers `shutdown` in a moment and then outlives `EXIT_GRACE`, so the
    poll runs out and kills it. `set_lsp` hands the outgoing client to the
    blocking pool now; nothing waits on a corpse.

  `cargo run -p rusty-core --example open_cost -- <project>` is the
  measurement, so the next person to add a step to open can see what it
  costs rather than guess.
- **A generator that runs `git init` leaves a repository inside the
  workspace.** esp-generate initialises one in the crate it writes, which is
  right when that crate *is* the project. Under `WizardLayout::Workspace` it
  is a nested repository with no commits, and git refuses to add one: the
  brand-new project's first `git add` stopped with `'firmware/' does not
  have a commit checked out`, and the Changes list showed `firmware/` as a
  single untracked entry rather than the files under it — the same fact seen
  from the panel. `wizard::unnest_repository` removes it, and only ever one
  with no refs and no `packed-refs`: a `.git` holding work is somebody's
  history, never a scaffold's to delete.
- **Two tabs that read as the same word are two tabs nobody can tell apart.**
  The strip showed a bare file name, and the standard workspace has three
  `Cargo.toml`s, two `lib.rs`es and two `main.rs`es — eleven tabs with six
  labels between them, reported as "I cannot find the file the tab is
  showing". `tab_hints` (pure, tested, beside the strip) is VS Code's rule:
  the name alone while it is unique, otherwise the shortest suffix of the
  directories above it that separates the whole group, grown one segment at
  a time. `core/src/lib.rs` against `firmware/src/lib.rs` needs two; a `bin`
  against a `src` needs one. A file at the project root keeps a bare name
  even when it shares one — having nothing above it *is* the distinguishing
  mark, and inventing a word for it would be a label the path does not
  contain.
- **A remembered tab is not a file the user asked for.** `restore_tabs` puts
  the strip back from `workbench.toml`, keyed on the project *directory* —
  and a directory can hold a different project than it did last week, which
  is exactly what the wizard does when somebody generates over a path they
  used before. Reopening through `open_file` banners, so a freshly generated
  project greeted its author with a red *could not read
  firmware/src/bin/main.rs* about a file nobody had asked for. `reopen_file`
  is the quiet door: gone means off the strip. The comment above
  `restore_tabs` had claimed this behaviour for as long as the function had
  existed; the code went through the loud path the whole time.
  **That fixed the active file and left the other ten.** They sat on the
  strip as names, and the first click on one banner’d about a file from a
  layout that no longer exists. `project_tabs` drops every tab whose file is
  gone as it reads the strip — one `exists` per tab against a root the
  backend already has, where the frontend would need a round trip each.
- **rust-analyzer picks its lockfile flag from a semver comparison, and
  `1.95.0-nightly` loses it.** rust-analyzer hands `cargo metadata` a copy
  of the lockfile and chooses the spelling from the toolchain's version
  (`project-model/src/cargo_config_file.rs`): `--lockfile-path` for
  `[1.82, 1.95)`, `-Zlockfile-path` plus `CARGO_RESOLVER_LOCKFILE_PATH` for
  `[1.95, 1.97)`, the variable alone after. cargo dropped the flag *in*
  1.95 — and a nightly is a pre-release, which semver sorts **below** the
  release it is becoming. So a cargo calling itself `1.95.0-nightly` is
  asked for the one spelling it has just lost; `cargo metadata` fails,
  rust-analyzer retries with `--no-deps`, and every dependency answers
  nothing. Espressif's Xtensa fork sits on exactly that version, so it is the
  normal state of an esp project, not a bad week on nightly. The same
  `cargo metadata` given `-Zlockfile-path` and the variable instead answers
  with the whole graph on the user's own esp toolchain — the cargo is fine,
  the number is the bug. **The remedy is an upgrade**: Xtensa Rust 1.97.0.0
  (rusty's own `XTENSA_RUST_VERSION`) calls itself `1.97.0-nightly` and is
  asked the way it understands. `model::cargo_loses_dependencies` is the rule,
  `toolchain::report` raises `xtensa-analyzer-blind` with that command, and
  `convert::explain_health` says the same beside the server's own words.
  **This passage was wrong three times, and each wrong version was
  "measured"** — `espup update` "cannot help", then "pin rust-analyzer 1.90,
  it gives zero warnings". The zero was warnings on rust-analyzer's stderr,
  which 1.90 simply does not log; `esp_hal::` completion under 1.90 was
  still empty, and 1.90's binary carries the flag. **Measure the symptom the
  user reported, not a proxy for it**, and read the source of the decision
  before building around it: a `CARGO` shim was written, tested and deleted
  in one sitting, because with a sysroot in hand rust-analyzer runs rustup's
  proxy (`Sysroot::tool` → `prefer_proxy`) and never reads `CARGO` at all.
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
- **A GUI-subsystem executable cannot be a pseudoconsole's program.** A
  release build of the app is one on Windows (`windows_subsystem`, so no
  console window opens behind it), and the built-in shell was that same
  executable re-entered with `--builtin-shell`. portable-pty starts every
  child with `STARTF_USESTDHANDLES` and invalid handles; a console
  program's startup swaps them for the pseudoconsole's, a window program's
  does not, and `CONIN$`, `CONOUT$` and `AllocConsole` do not reach it
  either — each measured with a window-subsystem probe in a pty. So the
  installed app's terminal read end-of-input at once and said "The shell
  exited with status 0." over an empty screen, every time, while the
  debug build — a console program — worked every time, which is how it
  read as "sometimes". The installer carries `rusty-shell.exe`, a console
  program that is nothing but the shell (`rusty-term`'s bin, built by the
  release workflow into `bundled/`); `terminal::builtin_argv` runs it where
  it is found, re-enters the app only when the app's own PE header says
  console, and otherwise runs the system shell rather than one that exits
  at once. `cargo run -p rusty-term --example pty_probe -- <exe>
  --builtin-shell` is the check, and it can be pointed at an installed app.
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
- **And a CPU line is the OR of every source mapped to it — upstream's
  ESP32 matrix drove it from whichever source changed last.** The same
  ESP-IDF assumption again: one source per line, so the last change *is*
  the line's state. esp-hal maps sources onto a line by priority, a TIMG
  alarm and `FROM_CPU0` share one, and a timer handler that raises the
  software interrupt to wake a task and then clears its own source lowered
  the line under the switch — which was never taken, so nothing set the
  next alarm and an Embassy clock stopped at its first tick. Beside it, the
  ESP32's timer group enables a level interrupt through the timer's own
  `LEVEL_INT_EN`; `INT_ENA` does nothing for it there and esp-hal never
  writes it, and upstream gated the line on `INT_ENA`. Both are
  `patches.py`'s now, and `qemu-v7` shipped with neither: a GPIO edge
  reached its handler and was announced as "every interrupt".
- **A proof that passes whichever way the model is written proves
  nothing.** The first version of the clock probe had the timer clear
  itself and *then* raise the switch — and under that order the
  last-change rule is right, because the line's last change is the
  switch. Measured on an emulator rebuilt with the old rule: 347 alarms
  and 347 switches, as healthy as the fixed matrix's count. The order that
  tells them apart is the one Embassy uses (raise, then clear), which on
  the same rebuilt emulator stops at `timer 1 switch 0`. Before trusting a
  gate, make it fail.
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
  **And the original ESP32's *are* 0..4** (`RSTART` 0, `WRITE` 1, `READ` 2,
  `STOP` 3, `END` 4) — the same trap from the other side, and the reason
  the numbers are per part now (`i2c_op_*`, set in `realize`). Both sets
  come from each part's SVD, whose `COMD` register enumerates `OPCODE`: the
  register map, not a driver. Found in one run this time, because the
  `default:` arm that once hid the C3's mistake now says `?op0`.
- **A model that drops the registers it has no opinion about breaks the
  part whose layout differs.** Every peripheral here shadows its whole
  window, so a driver's read-modify-write gets back what it wrote — every
  one but RMT, and on the C3 that never showed, because its memory size
  sits in the same register as its start bit, which the model kept. On the
  ESP32 `MEM_SIZE` is in `CHnCONF0`, the driver read it back to learn how
  many blocks it owned, got zero, and every transmission failed with the
  RAM untouched. When a model is extended to a second part, audit what it
  *drops*, not only what it handles.
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
- **The ESP32's FPU is on from reset, and upstream's emulator left it
  off.** This paragraph has been wrong twice, and the second time it was
  "measured". For months it said Espressif's QEMU "dies on the first FPU
  instruction": `Fatal error: divide by zero` appears **nowhere in QEMU's
  source** — it is libgcrypt's `_gcry_fatal_error`, reached long after the
  fact. Then, with `-d int` and the gdbstub (`qemu/float-probe`), it said
  the *application* had to switch its FPU on: `CPENABLE` reads 0 at the
  float, `lsi f8, a1, 40` takes a coprocessor-disabled exception, the next
  one is `EXC_DOUBLE` at `save_context + 138`, which is `rur.fcr` — **the
  handler saves the floating-point registers, so the handler faults too** —
  and one `wsr.cpenable` in the application fixed it. All of that was true
  and the conclusion was still wrong. **A GPIO interrupt on an ESP32 found
  it**: with the status words answered the handler still never ran, and
  read back from the guest, the context save was writing CPU registers
  through the GPIO window — the *interrupt* entry saves the floating-point
  registers too (`float-save-restore` is one of esp-hal's default
  features), so on this emulator every esp-hal ESP32 application that took
  any interrupt faulted, not only the ones that multiply. And esp-hal's
  interrupts work on real boards. Disassembled, all three of the ROM, the
  second-stage bootloader and the application: not one write to CPENABLE,
  one read. So the silicon comes out of reset with the FPU usable — the
  ISA calls the reset value undefined, and ESP-IDF clears it on purpose at
  start-up for its lazy coprocessor switch — while QEMU's system emulation
  sets it only in user mode. `patches.py` brings the CPU out of reset with
  CP0 on (`target/xtensa/cpu.c`, cores with an FPU only), and the float
  probe does **nothing** to CPENABLE and runs to its last line, as it does
  on a board. `[rusty:cpu] coprocessor 0 is disabled …` stays
  (`target/xtensa/exc_helper.c`) for the case that is real: something
  switched CP0 off — xtensa-lx-rt does inside every interrupt when
  `float-save-restore` is off — and a float followed. **Four claims, three
  of them "measured", and the one that held was the one a second, unrelated
  symptom was made to agree with.** Measure the symptom the user will meet,
  and when a fix works, ask what else the broken state would have broken.
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
  later, when an update will not verify. The plugin arrived ten releases
  after the config did (v0.6.23), and in between `plugins.updater.endpoints`
  pointed at the retired public mirror while nothing read it — the check in
  use went to the GitHub API by a constant of its own. `tests/updater_config.rs`
  pins the endpoint to `REPO_RELEASES` now, because a wrong one is an app
  that can never find an update and never says why.
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
  — through one `Drag::Wire` and one `edit::connect` in `pointerup`,
  because two gestures that agreed about what wiring means only in prose
  would drift. `pin_under` works in sheet units so the reach does not
  shrink with the zoom, and answers nothing when nothing is in reach — a
  wire that landed on a pin forty pixels from the pointer would be a
  connection nobody made. A pin to itself and a pair already joined are
  refused as wires that mean nothing.
- **And a wire is drawn click by click, the way every schematic editor
  draws one.** A click on a pin (a press that travels less than
  `CLICK_SLOP`) starts a `Drawing`; each click on the sheet fixes a
  corner; a click on a pin finishes it, and on a wire makes the T; the
  route follows the pointer the whole way — fixed corners solid, the live
  leg dashed — rather than appearing once the wire exists. Reported by the
  user as the one way wiring works everywhere else, and it was: pressing
  and holding across the sheet was the only gesture there was. Space turns
  the live leg the other way round (KiCad's posture), Backspace takes back
  one click and then the drawing, Escape or a right-click abandons it, and
  a drag let go on bare sheet becomes the first corner rather than a wire
  dropped on the floor. **A `Drawing` is not a `Drag`, because it outlives
  every press**: the middle button still pans mid-wire and the wire
  survives it. **While one is live the sheet is one click target** — an
  overlay under the corner controls — so a part or a wire's grab handle
  cannot take a press meant as a corner. **What is drawn is what is made**:
  the corners are stored already square, stub out of the pin included, so
  `orthogonalize` has nothing to add between two of them and the wire draws
  the preview's polyline point for point (`connect_drawn`, and a test that
  holds the two equal). With nothing laid between two pins the click-click
  wire is the routed wire a drag makes, and the preview says so.
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
- **A tab is not a character wide, and every overlay measures one the way
  the text draws it.** Both layers set `tab-size` (`TAB_SIZE`), and a tab
  goes to the next stop of four spaces — and past it to the one after when
  it is nearer than half a space, which is Blink's `Font::TabWidth`.
  The overlays' measure summed each character's own advance and measured
  `\t` as one, so on a tab-indented line a find match, the completion popup,
  the Vim cursor and hover's hit-testing all sat up to three columns left
  of the text. `caret::pen_after` is the one rule, pure and tested, and
  `hints::HintedLine` walks every line with it — hints included, since a
  tab after a hint goes to the stop counted from past the hint; measured in
  the app, the Vim cursor on `\t\tfoo` sits within a tenth of a pixel of
  the glyph it covers.
- **Programmatic `.value` writes destroy the textarea's native undo stack.**
  The editor writes value on every echo, completion accept and format, so
  Ctrl+Z was silently dead. The editor keeps its own snapshot history
  (`EditHistory`), one per file and shared by both groups
  (`Editor::histories`); the caret after undo is recomputed from
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
- **A key the editor takes and a window binding also names is handled twice,
  unless the editor stops it.** Ctrl+/ toggled the comment in the textarea,
  bubbled to the window, matched `editor.comment`, and that binding sends
  Ctrl+/ to the textarea — which toggled it back. On the caret's line the key
  did nothing; over a selection only the first line lost its marker, since
  the second toggle ran with the caret moved to the selection's start. A
  handler for a chord that is also a binding calls `stop_propagation`; F2
  does too.
- **The backend names a grammar as syntect does — `Rust`, `TOML`, `C++`.**
  The comment marker matched `"rust"`, which is what `mock.js` calls the
  language and nothing the app sends, so Ctrl+/ found no marker in the app
  while working in the browser — the second fault under the one above. Compare
  language names without regard to case, and put the real names in the test.
- **A `prop:` name is a JavaScript property name, and those are
  case-sensitive.** The same guard, moved back onto the textarea, was still
  `prop:readonly` — which sets an expando called `readonly` that nothing
  reads; the DOM's property is `readOnly`. `t.readOnly` was `false` in Vim's
  normal mode for as long as the guard had existed, so an IME could still
  type there. Found only because the clipboard work needed the textarea to
  refuse a paste and read the property back. It is the boolean attribute now
  (`readonly=move || …`), which a browser reflects into the property itself.
  `prop:checked` and `prop:value` are fine: their DOM names are lowercase.
  Check any other `prop:` against the property's real spelling. **And a
  guard that starts working changes what the element draws**: a read-only
  textarea gets no caret, so Vim's block cursor — the caret, styled — went
  with the fix, and three releases passed before anybody said so. When a
  broken guard is mended, look at the thing it guards as well as the thing
  it keeps out.
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
- **A test that asserts a command *failed* has asserted nothing.**
  `a_conflicted_merge_is_named_until_it_is_aborted` ran `git merge` without
  the identity every other call in that file sets, and checked
  `!status.success()`. A runner has no `user.email`, so git refused before
  merging at all — "Committer identity unknown", exit 128, no `MERGE_HEAD`
  — and that assertion passed, leaving the next line to fail with nothing
  explaining why. It was red on every runner for weeks and green on any
  desk with a global git config. Assert on the *specific* failure: git exits
  1 for a conflict and 128 for a refusal, and the two mean opposite things.
  **`GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null cargo test
  --workspace` is the runner's environment**, and reproduces this class in
  one line rather than one release.
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
  **The same goes for installing a component**: the language server's
  "not installed" answer offered `rustup component add rust-analyzer`, its
  own copy of the recipe without `--toolchain stable`, and run in an esp
  project rustup aimed it at `esp`, which cannot take a component — the
  button did nothing, twice. It is `toolchain::install_command` now, the
  one the Environment page shows.
- **An esp toolchain without a cargo of its own borrows stable's.** espup's
  1.95 toolchain shipped no `cargo.exe`, so rustup ran the stable
  toolchain's through its fallback (`~/.rustup/fallback`) — and with stable
  broken, every cargo command in every esp project died with `unable to
  hard link fallback exe … (os error 3)`, which names neither toolchain.
  1.97.0.0 carries its own cargo. When an esp build fails before compiling
  anything, check `rustc +stable --version` as well as `+esp`.
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
- **The two debuggers end, print and select by one set of rules.** A
  session that ends without an `exited` — gdb gone, or an adapter's
  `terminated` — has not exited 0, and says so in `error`; the DAP path
  invented the 0 long after gdb's had stopped. A printed line travels in
  exactly one state (`session::publish` takes `output` with the state it
  sends, under the state's lock): an adapter's `output` event used to stay
  behind and go out again with every later state. The selected frame moves
  once the frame has been asked about, and every stop puts it back on the
  innermost (`DebugState::halted`), since that is whose variables a stop
  asks for. And a session lets go of its process: `stop` kills *and waits*,
  once, and `Drop` calls it — a failed start and a stream that ended both
  used to leave the adapter running or gdb a zombie.
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
  It caught `strip_colours` at 1.98 on exactly the same lint, one release
  after this paragraph was written, which is what "or update" is for.
- **A job that `needs:` a matrix waits on every leg of it, including the
  legs it never uses.** `qemu.yml`'s thirteen gates run against the Linux
  build and nothing else; `msys2/setup-msys2` then began exiting 1 on the
  Windows leg, and all thirteen were *skipped* — the models went unproven
  for a reason that had nothing to do with them, twice, with a perfectly
  good binary sitting in an artifact. `if: always() && needs.build.result
  != 'cancelled'` is the fix, and the artifact download is the honest
  failure when it is the Linux leg that broke. Publishing keeps the strict
  `needs:`, because a release missing one platform's asset is worse than no
  release. The other half of that morning: **`update: true` is a full
  `pacman -Syuu` before a single package is installed**, and an action's
  cache rides the Actions cache service, which is being migrated. Both are
  ways for a Windows build to fail for reasons that are not the build's;
  neither is needed when the install list is explicit and a header check
  follows it.
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
  makes its tab disappear". `open_file` already knew the lazy case — and
  `open_view`, which copies the other side's view of a file, answers false
  for it so its caller reads the disk — and the strip's own click handler
  was the one caller that did not.
  When two functions read one state, grep for every reader before changing
  what the state means.
- **The working area's scroller is one DOM element for every document that
  passes through it.** The `Editor` closure re-runs on a document change and
  Leptos rebuilds the view in place, so the page's `overflow-y-auto` div (and
  the code surface's) keeps its `scrollTop` across the switch — measured:
  the same element, still at 900, with the next chapter in it. Every switch
  therefore landed the new file at the old file's offset, and coming back
  to a half-read chapter meant scrolling to find the place. A tab parks its
  viewport beside its caret (`ParkedEditor.viewport`, read off the scroller
  tagged `data-scroller=<group>` at park time); fronting sets
  `Editor.viewport`, which the view that owns the scroller consumes one tick
  after mounting — caret placed without scrolling first, then the offset —
  and a fresh document gets a `(0, 0)` restore so it opens at the top. A
  `reveal` for the same path clears the pending viewport: a jump into a
  parked file lands on the target, not where the tab was left.
- **A field nobody reads is a reply that never arrives.** Reasoning models
  in the OpenAI dialect stream their thinking as `reasoning_content` (or
  `reasoning`) beside an empty `content`, and can spend the whole
  `max_tokens` there: the user saw a question with nothing under it and
  `4096 out` on the meter, because the field was unread, the stream ended on
  `length`, and the loop pushed no message for a turn with no text. Thinking
  is `ChatEvent::ThinkingDelta` and `Content::Thinking` now — shown folded,
  kept in the history, skipped by both providers on the way back (DeepSeek
  rejects its own reasoning as input; Anthropic would want it signed) — and
  `StopReason::MaxTokens` sets `ai.cut_short`, which the drawer says under
  the answer with the number, because the returned history cannot carry a
  fact about text that was never produced.
- **One output budget for every provider is possible only because a
  provider that caps lower says so.** `DEFAULT_MAX_TOKENS` is 200 000, far
  above any model's cap, so a reasoning model is never cut off by a default.
  Anthropic, OpenAI and DeepSeek all refuse a larger `max_tokens` with a 400
  that names the cap (`> 64000, which is the maximum allowed number of
  output tokens`, `supports at most 16384 completion tokens`, `the valid
  range of max_tokens is [1, 8192]`); `output_cap_in` reads the largest
  number below the ask out of a refusal about output, the loop asks again at
  that number, and `AppState::output_caps` keeps it for the session so the
  refusal happens once per provider, not once per question. It reads only
  the provider's words — never this crate's `answered 400` framing, whose
  status code would read as a cap of 400 — and nothing under 256. The
  setting itself is left alone: what the user chose and what the provider
  takes are two facts, and the drawer's cut-off note uses the count the
  provider reported rather than either.
- **pulldown-latex 0.8 escapes the entity it writes for a control space.**
  `\ `, `~` and `\nobreakspace` come out as `<mtext>&amp;nbsp;</mtext>`, so
  the page showed the six characters `&nbsp;` between the components of
  every tuple in a chapter that spaces them with `\ ` — 379 lines of it in
  one book. `math_html` repairs that one element exactly (`&#160;`), and
  only that one: a literal `&nbsp;` typed inside `\text{}` is not the whole
  of an `<mtext>` and stays as typed. Measured by printing what the
  converter emits for each spacing command (`\,` `\;` `\:` `\!` `\quad` are
  `<mspace>` and fine; `<` and `&` from `\&` come out raw inside `<mo>`
  and survive only because the HTML tokenizer forgives them). When a
  renderer shows markup as text, print the string it was handed before
  blaming the browser.
- **Two round trips fired together answer in either order.** The settings
  page called `store_key` and then `refresh_key_state` back to back; the
  check overtook the write, "not saved" arrived and stayed, and the next
  request used the key perfectly well — a flag contradicting the store it
  describes. A re-read that depends on a write goes in the write's success
  callback, which is where `store_key` runs it now. The same shape waits
  wherever a view calls two controllers in a row.
- **A controller that clears a signal and later compares against it
  compares against nothing.** `check_update` set the last answer to `None`
  so the sheet would show the new one, and its completion closure then read
  that same signal to decide whether the verified download was for the same
  version — it never was, and a manual re-check put a downloaded update
  back to "Download and install". Read what a decision needs into a local
  *before* the reset, and hand the local to the closure. Found by driving
  the sheet over CDP; the stage is on the dialog as `data-stage` now, so
  a driven test asserts on the state and not on which buttons it guesses
  the state produced.

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
proof of the import. **A Wokwi `diagram.json` comes across too**
(`schematic::wokwi`, through the same import button as a KiCad file): the
parts rusty has a counterpart for, with their attributes — an LED's colour, a
resistor's value, an MPU-6050's readings — and their wiring on the devkit
rows for the same GPIOs, a board's `TX` being the chip's own console pin.
What has no counterpart is named in the notes with the connections it took
with it, and a pin the project's chip does not have is said rather than moved
to one it does.

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
  **A power symbol from a KiCad file is a rail too** — anything whose
  reference is KiCad's `#PWR`, a ground when its name is one of KiCad's
  (`GND`, `GNDA`, `GNDD`, `GNDS`, `GNDREF`, `GNDPWR`, `Earth`; `is_ground`)
  and a supply otherwise. They were parts nobody knew, so a lamp drawn to an
  imported `power:GND` stayed dark. `PWR_FLAG` is `#FLG` and stays a part:
  it tells KiCad's checker a net is driven, and read as a supply it would
  short the ground it is nearly always drawn on.
  **This paragraph was true of every reader but one.** `button_drives` looked
  for the rail among the devkit's own rows only, so a button wired to a
  `rusty:GND` drove nothing: the sheet said "pressing it changes nothing",
  the press never reached the emulator, and firmware reading its pull-up
  kept its LED lit whatever the user pressed — a user's own board, and the
  most natural way to draw a button. `drivers` and `divider_at` had always
  read power symbols; a reader that finds rails for itself has to be held to
  the same list, and `a_switch_to_a_power_symbol_drives_its_gpio` holds this
  one.
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
- **A running board is not a drawing** (`live`, from Run to the run's end,
  build included). Nothing moves, nothing is rewired, nothing is selected
  and no inspector opens: a switch is pressed, a knob turned, a slider
  slid, a drag on the sheet pans, and the keys, the menu and the corner's
  controls keep only what views. It was an editor for the whole run — wires
  that dragged, an inspector over the board at every click — which the user
  named as exactly that. **The wires let the pointer through while it
  runs**, because the wire layer is drawn over the parts: a wire routed
  across a switch's cap took every press meant for the switch, so which
  wire the pointer is near is asked of the geometry (`wire_under`) instead.
  What the inspector used to say while it ran is said by the reading line
  at the sheet's foot, for whatever the pointer is over: `D1 · 1.97 V ·
  6.03 mA`, or a net's level and voltage — the probe, without a panel over
  the board. **Beside the editor the inspector floats on the side away from
  the part it describes** (`inspector_left`), and is only as tall as what it
  says; it stood over the right of a pane whose parts were all on the
  right.

## Laying the sheet out

`view/panels/simulate/layout.rs` is pure and tested, and it exists because
a board that is *correct* can still be unreadable. Every rule in it is one
way a reader loses the thread:

- **Nothing is planted on top of anything.** `free_spot` searches outward
  in rings on the grid from where the part was asked for, so a part dropped
  into a crowd lands beside it rather than on it — and an import, whose
  coordinates are another editor's canvas, is laid out on arrival rather
  than piled into one square inch.
- **A wire goes round what is in the way.** `route` is the candidate set a
  schematic uses — two Ls and the Zs on the lanes between and beyond the
  ends — scored so a crossing costs far more than a corner and a corner a
  little more than length. It returns bends, so the author can still drag
  every one of them.
- **Including its own part.** A wire's own two bodies were left out of the
  obstacles altogether, on the argument that a wire always starts inside
  one — so a pin on the far side of its own part routed straight back
  across it, which on a keypad wired to a header on its right is four
  lines drawn over its own keys. They are obstacles now, as the *body*
  (`part_box`) rather than the drawing: the stub already stands a whole row
  pitch beyond the body, and growing the box to meet it would make the
  first segment of every wire a crossing.
- **Four wires out of one edge take four lanes.** Two wires down the same
  lane are drawn as one line, and a reader cannot see where either goes —
  which is worse than the crossing that avoiding it costs, and is most of
  what makes a correct board look like a mess. So a route is scored
  against the wires already laid down as well: a pixel of *shared* lane
  costs far more than a crossing, and a crossing is worth a detour of
  about a hundred pixels and no more. The freedom that pays for it is how
  far the wire runs straight out of its pin before it turns — one row
  pitch is the schematic default and gives every wire off one edge the
  same turning line, so the longer stubs are searched too. Only when the
  short one leaves a fault, because the search is the square of that list
  and most wires are the only wire in their corner.
- **A dot marks a join and never a crossing.** `junctions` reads the drawn
  paths, not the net model, because what a reader needs marked is what is
  drawn: two ends at a pin is a dot (the pin is a conductor too — KiCad's
  three-things rule) and so is an end landing on another wire's line, which
  a branch makes and which the pin-only rule drew nothing for. **And so is a
  fork that is nobody's end.** A branch is a wire to the trunk's pin laid
  along the trunk (`branch_route`), so where it leaves the trunk is one of
  its *bends*, and a rule that read only ends drew that T with no dot —
  reported with a red box round it, beside a crossing that looked the same.
  A bend is a fork when the lines through it leave in three directions or
  more (`arms_at`, a bit per direction): the corner two wires share turns
  and gets none, and a crossing is nobody's bend. Three unrouted wires out
  of one pin share its lane and part where the first turns off, which is a
  fork too. **All of it between wires of one net only** — wires whose drawn
  ends meet, directly or through each other — because a line of another net
  over the same point is a drawing fault, and a dot there would make it a
  connection the board does not have.
- **The ghost under the pointer is the route, computed by the code that
  will make it.** `edit::connection` is `connect` less the push, so the
  preview cannot promise a shape the connection does not deliver — a
  straight diagonal that became an orthogonal route somewhere else on
  release was a preview of nothing. With no pin in reach it is the elbow
  out of the pin, which is the shape a schematic wire has whatever it ends
  on. A new wire is routed by `route_beside`, against the parts *and*
  every wire already there; `reroute` over a one-wire slice was what it
  used to be, which could not see the rest of the board and put a hand-drawn
  wire exactly on top of one that was there.

**The layout is measured against what is *drawn*, not the body.**
`part_box` is the drawing's bounds; the reference sits a row above it and
the value a row below, so parts laid out a comfortable gap apart had their
labels sitting on each other — six overlapping pairs on the first arranged
sheet, measured in the browser off the rendered SVG. `drawn_box` grows the
box by a row, and the same measurement then says zero.

**`arrange` is a command, never a rule that runs.** A board somebody laid
out by hand is theirs; tidying it unasked would move their work out from
under them. It places every part beside the pin it reaches — sorted by that
pin's y, in a column each side of the devkit, which is what leaves almost
nothing to cross — and then re-routes every wire. The devkit does not move:
it is what everything else is placed against. A part *dropped on* a wire is
the one exception (`reroute_broken`), and even then only the wires whose
path now crosses a part are touched, and only if the new route is better.

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
- **Every ground on the sheet is the one ground** (`circuit::of`). A
  devkit's GND pins are one piece of copper — the C3's has one on each
  side — and a `rusty:GND` drawn twice is two symbols for one net; the
  rules have always read each of them as low. The bridge took one for
  ground and left anything on another floating, so the solver called
  "floating" or "contradiction" a sheet the rules drew lit — found by the
  playground, whose LED went to the other GND. The rest are joined to the
  first with a `Short`. And a devkit row is found by its *number* first
  (`kit_net`): a name two rows share keys only one of them in
  `solid_nets`, so asked first it answered for both rows.
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
- **A lamp is as bright as its current, and a PWM pin is read over a
  period.** The glow was on or off, so a resistor changed the reading and
  not the lamp, and a lamp the firmware was breathing through LEDC sat dark:
  its pin had a duty and no level, and the rules read only levels.
  `rusty_embed::period` reads the sheet — the rules and the solver both — at
  each *moment* of one PWM period in which a different set of pins is high,
  and weights each reading by how long its moment lasts. The pins are
  ranked by duty, so `n` pins are `n + 1` moments; that assumes they rise
  together, as one LEDC timer's channels do, and a lamp on one pin does not
  depend on the assumption at all. **Everything the board says goes through
  it, PWM or not**: with nothing on PWM it is one moment and answers exactly
  what `nets::evaluate` and `operating_point` answer, which a test holds, so
  a lamp on a PWM pin and one on an ordinary pin are never two code paths.
  **The split between `period` and `weights` is the cost.** Which pins, and
  in what order, decide what is read; the duties decide only the weights.
  A breathing lamp is a hundred re-weightings a second, not a hundred
  solves, and the wire colours and the findings are memos of their own for
  the same reason. A pin no wire reaches is never ranked, or a quad's four
  motors would cost five readings of a sheet they are not on.
- **What a meter shows over a period is three averages, not two and a
  product** (`period::Measured`): a lamp lit half the time dissipates half
  its power, and its average volts times its average amps is a quarter. A
  reading that differs between moments is `steady: false` and says
  *average* on screen — a lamp under PWM never sits at its average voltage,
  and `0.59 V` beside a lit LED with nothing saying why is a number somebody
  would go and check. A net under PWM is drawn green in dashes and read as
  `PWM, high 30% of the time`, never as floating.
- **The glow is the current to the power 1/2.2** (`simulate/glow.rs`). A
  pixel's value is its light to that power, so drawn this way the light
  leaving the screen is in proportion to the lamp's and the eye does the
  rest: a duty ramped in a straight line brightens fast and then barely,
  here as on the desk. Full is 10 mA, where `vf` is quoted. Where the solver
  refuses, the rules' lit share stands in at full current — what every lamp
  was drawn as before there were numbers. An RGB lens mixes its channels'
  shares through the eight colours it was always drawn in, and a digit's
  segments dim the same way. The lamp's light is two reactive attributes
  over a dark body, so a duty changing a hundred times a second redraws an
  opacity and not the part.
- **A pin is in `sim.gpio` or in `sim.pwm`, never both, and the newer
  report decides** (`controller::session::forget`). A level left from before
  LEDC took a pin lit at full a lamp the firmware was dimming, and a duty
  left after GPIO took the pin back would hold the lamp at its last
  brightness whatever the pin did next. One edge is the emulator's: a pad
  given back to GPIO reports its level *before* the channel's idle line, so
  the idle level stands until the pin next moves.

## Signals: a generator on the bench, and the lab to judge a filter with

A filter is designed against signals and proven against them, and
`docs/signals.md` is the design: the emulator plays tables (`qemu/
esp32_gpio.c`, `[rusty:wave@]`), `rusty_embed::signal` says what a generator
produces, `dsp` measures and filters, `generator` renders what a run plays,
`wave` puts it on the pin channel; in the window, `crate::lab` is the pure
half, `view/lab/` the four instruments of the Signals tab and
`controller/lab.rs` the sweep and the console feed.

- **A signal is played by the emulator against its virtual clock, never
  pushed by the host.** The host's pace is a millisecond with jitter — a third
  of a radian of phase at 50 Hz, enough to make a working notch look broken —
  which is the whole reason the device model changed. Tables are
  double-buffered and `on` keeps the phase, so a tone given a new amplitude
  does not start over. Playing one is a capability of its own
  (`Emulator.waves`, marker `[rusty:wave@`), not a peripheral marker: a build
  without tables is current for everything else, and a sheet that plays
  anything on one carries `signals-outdated` before the run rather than a
  signal quietly pushed at the host's pace instead.
- **A generator's table is the sheet's circuit stepped, one loop dropped
  first.** An RC charged by a tone is mid-swing when its loop comes round; a
  table rendered from rest puts a seam there. Only the pins a generator
  *reaches* get a table — one step with each generator a volt away against
  one without, a step and not a DC solve so an AC-coupled pin counts — and a
  reached pin that never moves gets one sample: a table on every pin with a
  `fullscale` held a knob on another pin still. It renders with the
  firmware's pins at rest and does not follow a GPIO the firmware later moves
  on the generator's own net; that is stated in `docs/signals.md`, which
  claimed otherwise before the code existed.
- **Every choice a run makes is a function in `generator`** — the rate (20 kHz
  unless `rate`), the loop (one to ten seconds, exact in fractions, the seam
  and repeated noise said), the seed (the reference and the property's key,
  FNV, mixed with `seed`) — so the lab renders exactly what the emulator was
  handed, and the Time view's "played" lane lies on the converter's reports.
- **A sensor's reading can be a signal too** (`signal.<reading>`, played as the
  part's whole register block at `signal.rate`, 1 kHz), and a range the
  firmware chooses re-renders it with the phase kept. A reading with no
  signal stays where its slider is. A channel declared on the console
  (`[rusty:sensor]`) has no table to play and is fed from the lab at fifty
  samples a second inside its declared range, **with the host's pace said in
  amber above the fields**.
- **The sweep is timed by the firmware's clock.** Each step waits for the
  emulator's own report that the table switched, settles five periods,
  listens ten, and measures both records by `tone` with each phase carried to
  one instant — two records need not start on the same sample, and a phase
  compared across two instants is off by `2πf` times the gap. What goes in is
  the converter's record by default, in counts as the firmware's filter reads
  them, so the gain is the filter's and not the converter's scale besides; a
  step's frequency is rounded to three figures, since `f=100.00000000000004`
  in the step's signal was the logarithm's rounding. Stopped, finished or
  failed, the source plays what it played before, unless a newer sweep has it.
- **Every instrument reads a record** (`crate::lab::record`): a telemetry
  channel's rate is its stamps', a converter's reports get their left-out
  repeats back by holding each value on the commonest interval between them,
  and a played table is taken at the question's rate with straight lines
  between samples, as the emulator reads a pin between them.
- **Proven twice.** `qemu.yml`'s signal gate boots `qemu/wave-probe` on the
  packaged Linux build: every conversion is the table's value at that instant
  to the count, the firmware's own systimer times the tone at the table's
  frequency, and two thousand burst reads of a sensor's block are never torn.
  Its first CI run stopped at `apt-get` with exit 100 before a gate ran — a
  mirror mid-sync — so the install is retried now, with apt's own retries
  inside each try. The lab itself was driven through the mock end to end: a
  twelve-step sweep of the mock's exponential average read a first-order
  low-pass at ten hertz, beside a second-order design's curve. And
  `filter_probe` runs the whole chain with nothing mocked (`examples/
  filter-lab`): a generator through rusty's host code into the emulator,
  firmware running the Design view's exported low-pass, `raw` carrying
  three tones at the sheet's counts — one of them switched to by a `play`
  step — and `y` the design's filter of `raw`, sample for sample.
- **A line a sample is more than the console carries at a kilohertz.**
  Printing one telemetry line costs the emulated core about a millisecond
  and a half, so filter-lab written for a thousand samples a second fell
  behind its own deadlines and ran at 643 — and a filter's coefficients are
  right at one rate only, so every response it produced was another
  filter's. The gate caught it by checking the rate off the firmware's own
  stamps before measuring anything; the example samples 250 a second and
  prints whole counts. Firmware that filters faster than that has to
  decimate what it prints, and the lab measures whatever rate the stamps
  say.
- **The firmware's clock was slow, and the gate that would have seen it
  was loose.** Upstream's systimer counted the whole ticks between two
  readings and threw the fraction left over away at every one, so firmware
  polling it — every busy wait, every esp-hal delay — ran slow by however
  often it looked: blinky's 500 ms wait lasted 502 ms of virtual time on a
  runner and 530 on a slow container, and a 50 Hz tone the emulator played
  against the virtual clock reached the firmware at 50.25 Hz by its own.
  The signal gate timed its tone within half a percent and passed; the
  filter gate fitted the hum at the firmware's stamps and found a third of
  it, then a seventh. `qemu/patches.py` counts both ends of each interval
  from the clock's zero so the fraction carries, leaves
  `[rusty:systimer-exact]` in the binary, and `has_wave_model` asks for it
  beside the tables — a signal is in the firmware's time only when the
  firmware's clock keeps that time. `qemu.yml` holds blinky's own systimer
  stamps to the emulator's (slope within 1e-4), and the signal gate's
  tolerance is a twentieth of a percent. Reproduced and proven in the
  Docker container before a runner saw it: slope 0.977 before, 0.999999
  after. **A tolerance wider than the effect it guards is not a gate.**
  And two checks that had passed on the drift fell over without it: the
  signal gate's sensor rate, measured between its first and last reading,
  had a first transaction a few milliseconds late cancelled by a clock
  running slow — both now fit a slope and leave the start out — and the
  board probe counted an edge already on its way when `stop` landed as the
  emulator running on. **A check that passes because two errors cancel is
  waiting for one of them to be fixed.**
- **And the first explanation was wrong, confidently.** Between the two
  runs this file said the runner held the emulator up and the firmware
  caught up in bursts — true of QEMU without `-icount` (the virtual clock
  keeps the host's, stalls included), so the gate fits its tones at the
  firmware's stamps and says how evenly they fell, and a held-up host
  still bunches samples a filter written for even ones reads wrong. But the
  run that followed had every sample exactly 4 ms apart and a worse fit.
  The hypothesis had explained the numbers without predicting anything;
  the probe printing how even the samples were is what retired it. **Make
  the diagnosis print the fact that would refute it.**
- **The live-circuit probe counted every conversion twice and took lines
  from the circuit it was testing.** It read the firmware's line *and* the
  emulator's report of the same conversion — equal neighbours that cut every
  climb into pairs — and fetched the report with a `try_recv` off the pin
  channel that `Live::absorb` then never saw, a drive edge among them. It
  passed on qemu-v9's release run and failed on the next push with a clean
  climb read as a climb of two. One witness now, and every pin line reaches
  the circuit: **two accounts of one event are not two readings**.

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
