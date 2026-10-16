# rusty — embedded Rust workbench

A desktop IDE for embedded Rust — Espressif, STM32, RP2040/RP2350, nRF52 and
WCH CH32 parts, and the C and C++ beside the Rust in Cargo, PlatformIO and
CMake projects. ESP32 came first and is still where the simulator is
deepest; a part is data now (rule 6), not a code path.

It began as "not an editor — it owns the half of the job rust-analyzer does
not". That half is still where the differentiation lives: which chip, which
toolchain, what fits in flash, what is on the serial port, and an assistant that
can call all of it. But a tool you cannot read or change a file in is a
dashboard about work you do somewhere else, so the editor is in scope too:
files, highlighting, and rust-analyzer behind it for completion, diagnostics
and navigation.

That is a deliberate reversal of the original positioning, not drift. Anything
in this file that still reads as "we do not edit code" is stale.

**The window is `crates/rusty-desk`, native Slint in one process**: the
libraries are linked in and the window calls them as `rusty-cli` does, with
blocking work on threads and the answers brought back to the UI thread.
`docs/slint.md` is its design and the order the work was done in.

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

# The tools the installers ship beside the app, into target/bundled/: rusty's
# QEMU (the peripherals), both esp-gdbs, espflash, the LLDB adapter. The
# release workflow runs this before every build; run it once here and the
# window finds them (`bundled_dir` in rusty-desk's main.rs). Not committed —
# three hundred megabytes of binaries belong in a release asset. Rust itself
# is deliberately not in it; the script says why in its header.
scripts/bundle-tools.sh

# Release: bump the version in Cargo.toml's [workspace.package], add the
# `## v<version>` section to CHANGELOG.md, commit, then push a tag
# (`git tag v0.7.0 && git push origin v0.7.0`). .github/workflows/release.yml
# builds the window and rusty-cli on three runners and packages them itself:
# Windows `rusty-<v>-windows-x86_64-setup.exe` (crates/rusty-setup,
# the payload appended by `rusty-release pack`), macOS a universal
# `rusty.app` in a DMG and as `.app.tar.gz`, Linux an AppImage and a .deb.
# The workflow stamps the tag into the workspace manifest as a safety net —
# the repository said 0.6.12 for twelve releases while the installers were
# right. The updater key is in the repo secrets (RUSTY_UPDATE_KEY, the
# minisign secret key file or base64 of it, and RUSTY_UPDATE_KEY_PASSWORD;
# the public half is `rusty_embed::update::PUBLIC_KEY`): `rusty-release
# sign` writes each updater artifact's `.minisig`, `rusty-release verify`
# checks it with the app's own check, and the publish job merges them into
# rusty-update.json with the tag's CHANGELOG section as its `notes`. A tag
# with a hyphen (`v0.7.0-rc.1`) is published as a pre-release, which the
# feed never points at. See "Updating itself" below.
rusty-release sign <file> --version <v> [--key <file>]
rusty-release verify <file> --version <v>

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
# The project's own build, as the window's Build runs it: cargo, `pio run`,
# or CMake's configure-once-then-build — whichever file is at the root.
cargo run -p rusty-cli -- build .
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

# What a question to the assistant costs before anybody spends it: the
# prompt and every tool definition, what each tool answers about a
# project, the open file as it would be attached, and two short questions
# round by round with how much of each request repeats the one before --
# the share a provider's prefix cache supplies. Estimated tokens.
cargo run -p rusty-ai --example token_budget -- <project> [file-to-attach]

# A shell in a pty the way the terminal starts one, and what it printed --
# the check that a shell comes up in a pseudoconsole at all. With no
# arguments it runs target/debug/rusty-shell.exe, the built-in shell as the
# console program of its own (`cargo build -p rusty-term` first); any argv
# probes that program instead, an installed copy's included.
cargo run -p rusty-term --example pty_probe -- [<exe> <args>…]

# Where the seconds go when a project is opened -- the two steps that used
# to be awaited before the window could draw anything of the new project.
cargo run -p rusty-core --example open_cost -- <project>

# Every diagnostic the editor is sent for a file, with who said it --
# `rust-analyzer` for its own analysis, `rustc` for the check -- and when.
# The check for "the error is in Output but not in the editor": whether the
# server never sent it, or sent it and something took it away.
cargo run -p rusty-lsp --example diag_probe -- <project> <file> [seconds]

# What one save sets the server doing: open, settle, save the way the app
# does (didSave and the watcher's didChangeWatchedFiles), and print every
# check it starts and every runFlycheck of the client's own. Measured on
# examples/rate-loop: one save, one check, none of ours -- and a save
# landing while a check is still compiling a crate restarts it.
cargo run -p rusty-lsp --example save_probe -- <project> <file> [seconds]

# A mistake typed into a line and taken back, and every diagnostics publish
# for the file between: whether a squiggle left on screen is still in the
# server's answer or only in the window's. `--save` saves the mistake, so
# the check reports it too, and leaves the mend unsaved for a while.
cargo run -p rusty-lsp --example fix_probe -- <project> <file> <line> <from> <to> [--save]

# What painting a file costs: whole, after a one-line edit halfway down, and
# after a block comment opened at the top — the check that a keystroke in a
# long file repaints its own line and not the file (see "Large files").
cargo run -p rusty-edit --release --example highlight_cost -- <file>

# Which files rusty will draw dimmed in a project, and every file the scan
# read to decide -- the check for a report of a file wrongly dim, or one
# that should be and is not. `rusty_edit::modules` refuses wherever it
# cannot be sure, so "nothing dimmed" is also what a refusal looks like and
# the probe says so.
cargo run -p rusty-edit --example unlinked_probe -- <project>

# What EasyEDA actually answers for a part, record by record, beside what
# the reader made of it -- how tests/fixtures/easyeda/ was captured
cargo run -p rusty-embed --example lcsc_probe -- C25804 [out.json]

# A CH32 firmware in rusty's own emulator of the part, with no window:
# every pin it drove, every PWM duty, every line it printed, stamped with the
# part's time, and how long the host took. The fixtures are ch32-hal's PWM
# example, the V003's built with nightly and the X035's with stable; the
# chip is the V003 unless named. `rusty-cli sim` runs a CH32 project the same
# way the window does, build included.
cargo run --release -p rusty-mcu --example mcu_probe -- crates/rusty-mcu/tests/fixtures/ch32v003-pwm/pwm.elf 2000
cargo run --release -p rusty-mcu --example mcu_probe -- crates/rusty-mcu/tests/fixtures/ch32x035-pwm/pwm.elf 2000 ch32x035f8u6

# The CH32X035's remap tables, generated from ch32-metapac rather than typed
python crates/rusty-mcu/scripts/x035_pins.py \
  ~/.cargo/registry/src/*/ch32-metapac-0.1.1/src/chips/metadata_0036.rs \
  crates/rusty-mcu/src/ch32/x035_pins.rs

# The window: open a project and a file in it.
# With RUSTY_PROBE it types, scrolls, opens the docks, the palette and the
# settings, photographs each state into the directory (Window::take_snapshot)
# and prints each key's and scroll step's time to the frame that shows it.
# Then it waits up to a minute for the language server to report a
# problem, and photographs the Problems panel and the hover card — on a
# Cargo project that is rust-analyzer, on a CMake one clangd — then
# completes a field, opens a call's signature, goes to a definition and
# back, lists references and fixes, and (with RUSTY_PROBE_SAVES) renames.
# It exits through File > Exit, so what it typed is kept as a draft (hot
# exit): it refuses to run without a data directory of its own, and a
# second run on the same one shows the draft back. RUSTY_PROBE_SAVES also
# saves badly formatted Rust over the file and says whether rustfmt ran --
# point it at a copy. The header of src/probe.rs lists the other modes;
# RUSTY_PROBE_KEYS rebinds Save and uses it, and RUSTY_PROBE_WINDOWS=<other
# project> opens a second rusty with Ctrl+Shift+N, which probes itself into
# <dir>/second and leaves.
cargo run -p rusty-desk --release -- <project> <file-in-it>
RUSTY_PROBE=<dir> RUSTY_CONFIG_DIR=<dir>/data cargo run -p rusty-desk --release -- <project> <file-in-it>
# A Markdown chapter's page — its formulas, figures and raw HTML — in both
# themes, then image files beside it opened as pictures, an SVG's picture
# over its source and edited, the book's symbol table, and a page it writes
# beside the chapter clicked, pointed at and zoomed through the touch area
# over it. On a copy of the book: it edits an SVG and keeps the draft.
RUSTY_PROBE=<dir> RUSTY_CONFIG_DIR=<dir>/data RUSTY_PROBE_BOOK=1 cargo run -p rusty-desk --release -- <book-copy> <chapter.md>

# Drawing from code, the worked end: an f32 vector type, an example that
# draws two vectors and their cross product, and a test that draws and
# asserts it. In rusty, ▶ Run above `main`; from a shell, the lines rusty
# reads. (Its own `[workspace]`, as every example here has.)
cd examples/draw-vectors && cargo run --example cross
```

## Layout

| Crate | Does |
|---|---|
| `rusty-core` | Cargo workspace analysis: dependency graph, duplicates, feature unification. `disk/` is the build directory measured and judged (`tree.rs` counts one build tree, `judge.rs` decides what is stale and why, `sweep.rs` removes it, `fs.rs` the filesystem it reads) — the Crates page's build-directory section, `rusty-cli disk` / `sweep` and the `disk_report` tool are its three readers |
| `rusty-embed` | Chips, boards, project detection, toolchain, memory, flashing, wizard, simulation. `screen` reads a monochrome OLED's command stream back into pixels, `sensor` runs a part's own conversion backwards. `model/` is a directory now, one file per concern, re-exported flat so `rusty_embed::X` still names everything; `simulate/` likewise, with the `.rusty/sim.toml` format in `board_file.rs` beside the planner (`plan.rs`), the chips QEMU models and where this machine keeps the tools (`machine.rs`), which peripherals an emulator binary carries (`models.rs`), its extra arguments and a free port (`qemu.rs`) and the sheet a project declares (`sheet.rs`). `nets/` is the reading of the wires the same way — `graph.rs` the three partitions, `evaluate.rs` the rules, `analog.rs` dividers and knobs, `bus.rs` what sits on I2C and SPI, `switch.rs` presses and ties. Two helpers are shared rather than copied: `union_find` (every partition of pins is one) and `layers` (the catalogue, the parts and the symbol library all layer definitions the same way). Three things that are *not* simulation have their own modules, because `simulate.rs` had grown into the place they lived and every other module was importing "the simulator" to reach them: `tools` (finding a binary — one ladder, one order, for every tool), `install` (fetching QEMU/gdb/gcc, version pins), `net` (proxy policy, and the one `ureq` agent builder); `schematic/` is KiCad and EasyEDA — `.kicad_sym` read and written, `.kicad_sch` read and *patched* back (`docs/kicad.md`), an LCSC part fetched — over `model/symbol.rs`, the drawing the window renders. And the sheet answers in numbers now: `solve` is modified nodal analysis (DC, a Shockley junction, backward-Euler transient), `circuit` turns a sheet into one and names what the sheet did not say, `live` walks it in step with a running firmware. `sensor` is an I2C sensor's registers from the readings a slider sets; `signal` is what a generator produces, as one line of text rendered sample for sample, `dsp` the spectrum, the single tone and the filter designs a firmware runs (each held to its closed form, and exported as `no_std` Rust), `generator` the tables a run plays — through the circuit to a converter, or into a sensor's registers — and `wave` the lines that put a table on the emulator and read its account of playing it (`docs/signals.md`); `simulate/channel.rs` is the pin channel from the host's side and `simulate/headless.rs` a run without the window, both shared by the app, the CLI and the assistant; `schematic/wokwi.rs` reads a Wokwi `diagram.json`. `spatial` is the math toolbox's arithmetic — vectors, Hamilton quaternions, Z-Y-X Euler angles, matrices, the two frames flight code is written in, and `check`, which names the convention a wrong attitude crossed — with `spatial::sheet` its worksheet language (lexer, parser, evaluator, and every operation's working as steps), `spatial::instrument` what an attitude indicator reads off an attitude, and `sheet_file` its `.rusty/math.toml`. `draw` reads what a program drew with `rusty-draw` into sketches and measures the angles between what it drew. `image` turns an ELF's loaded segments into the UF2 blocks or the flat binary a USB bootloader takes. What the window's panels draw is computed here too, pure and tested: `board/` is the sheet drawn and edited (`art`, `geometry`, `edit`, `layout`, `glow`, `readout`, `picture`), `lab/` the signal lab, `scene/` the 3-D view the Math page and the Draw tab share, `plot`, `waves`, `activity` and `buildlog` what Plot, Waves, the status bar's activity and Problems read |
| `rusty-ai` | Bring-your-own-LLM providers (both dialects authorise, send and read a line through `provider/mod.rs`), the tool registry, the agent loop (`agent.rs`: open a turn, read it, run its tools), and `mcp` — the registry served over the Model Context Protocol |
| `rusty-term` | A real terminal: portable-pty (ConPTY) + vt100, drawn by the window; the built-in shell (`builtin.rs`), and `rusty-shell`, the same as a console program of its own for Windows |
| `rusty-edit` | File tree, syntax highlighting (semantic tokens, not colours), read/write, rustfmt, project search on ripgrep's engine |
| `rusty-dbg` | Debugging, two protocols behind one handle (`any.rs`): `session.rs` is gdb's machine interface, `dap.rs` is the Debug Adapter Protocol for LLDB. Both fold into the same session state — breakpoints, stepping, stack, variables |
| `rusty-git` | The repository's history, Fork-shaped: `graph.rs` lays the log out into lanes and edges (pure, tested — the window only turns a lane into an x), `log.rs` is what drawing it needs (which rows, the shape of each line, what a search matches) and `plan.rs` what a write decides first (where a push goes, how a branch is checked out, what a name field refuses), `parse/` reads `git`'s machine formats (the log, a commit's diff, the refs, the status), `repo/` runs the user's own `git` in the opened project (`run.rs` the one way a `git` is run, `stamp.rs` what moved without asking git). No libgit2: one binary on PATH is one implementation of the repository format to agree with |
| `rusty-lsp` | The language-server client — rust-analyzer for a Cargo project, clangd for a PlatformIO or CMake one (`ServerKind`): stdio JSON-RPC, diagnostics, completion, hover, definition, signature help, code actions, semantic tokens, and navigation — references, implementations, type definitions, outlines, workspace symbols, the occurrences of a name, call hierarchies and macro expansion (`client/navigate.rs`, every answer a place with its line). `client/mod.rs` is the session; `documents`, `edits`, `query`, `navigate` and `hints` are the requests, and `transport`, `handshake` and `dispatch` the plumbing under them; `discover.rs` finds the binary and spawns it, `uri.rs` is the one percent-decoder and drive-letter folder, `convert.rs` turns replies into `model`, `pull.rs` is the diagnostics-pull loop. `positions` is the one set of encoding arithmetic — bytes, the negotiated protocol units and scalars — for document positions and for the UTF-16 offsets into a signature's label alike |
| `rusty-i18n` | The interface's languages: one TOML catalogue each, a `t!` macro, and the tests that keep them in step. The window calls `t!` in Rust and `@tr` in `.slint`; a library's text that is to be translated travels as a stable *name* beside its English, which the window looks up |
| `rusty-cli` | Headless entry point; the CI and bug-report surface, `rusty-cli sim` (the simulator without the window) and `rusty-cli mcp`. One function per subcommand, beside its printers (`check.rs`, `hardware.rs`, `disk.rs`, `sim.rs`, `workspace.rs`); `main.rs` is the arguments and the dispatch |
| `rusty-mcu` | Microcontrollers rusty emulates itself, instruction by instruction — WCH's CH32V003 and CH32X035, which nobody's QEMU models. Pure: `cpu` is a QingKe hart, the V2A's RV32EC or the V4C's RV32IMAC (`Core`), with Zicsr, the address vector table and the hardware stack; `elf` loads a flasher's view of an image; `ch32/` is one machine for both parts — the clock tree, GPIO with AFIO's remaps and EXTI, the timers, SysTick, the PFIC, the USARTs and the debug registers SDI print writes through — with what differs between them as data in `ch32/part.rs`; its output is `Event`s stamped with the part's own time. rusty-embed's `simulate::mcu` is the host: it paces the machine to the wall clock and puts the events on the pin channel. See *WCH's parts* |
| `rusty-draw` | The one crate here that other people's code depends on: drawing from Rust code into the Draw tab. `Scene::new("…").vector("a", a)` prints one `[rusty:draw]` line per shape; `SceneOn` writes them to any `fmt::Write` without `std`. No dependencies and a version of its own; `rusty_embed::draw` is its reader. See *Drawing from code* |
| `rusty-desk` | **The window, in Slint** — `docs/slint.md` is the design and the order of the work. One process: a part calls the libraries as `rusty-cli` does, blocking work goes through `work::spawn` and comes back on the UI thread. `ui/*.slint` draws and `src/` decides: the `Shell` global is what the window shows and asks (`app/mod.rs` fills it, one `sync_*` per region), `Code` the editor's rows (`editor/`: Rust measures and places every character, Slint draws). The words are `rusty-i18n`'s catalogue — `t!` in Rust, `@tr("key")` in `.slint`, which `build.rs` bundles as gettext from the TOML — and `catalogue.rs` holds both to it. `RUSTY_PROBE=<dir> RUSTY_CONFIG_DIR=<dir>/data rusty <project> <file>` drives the window and photographs it. Drafts outlive the window (`editor/backup.rs`, VS Code's hot exit): every backup write and removal goes through one thread in the order asked, or a backup could land after the save that made it stale. **Rust, C and C++ are rows of one table** (`language.rs`: extensions, comment, server, formatter — a test holds it to `ServerKind::serves`), and `lsp.rs` runs the servers the table names: open, change, close and save go through one ordered thread, a question carries its draft, and an event from a server since replaced is dropped by its generation. A Slint `Path` is fitted into its box less its stroke unless `fit: preserve`, which is why the squiggle once drew two thirds of its width. `actions.rs` is every action and its default key, and the `Keymap` of the keys in force over them; `instances.rs` is New Window and the other rusty windows on the machine |
| `rusty-setup` | The Windows installer: one executable carrying the app and its tools (`payload`: one tar of the install, xz-compressed whole and appended, found by a footer — 75 MB where per-file deflate made 148), installing per user in `%LOCALAPPDATA%\Programs\rusty` with an uninstall entry and shortcuts of its own (`shell`). Double-clicked it is a window in Slint (`ui.rs`, `ui/setup.slint`, software renderer only): where it goes and a desktop shortcut, then the progress with the release's highlights going by, then Open — in the system's language, Chinese or English; `RUSTY_SETUP_SHOTS=<dir>` with `--features shots` photographs every page. The updater runs it as `--quiet --run --wait <pid>`, with no window, and — cut down to its own part — the uninstaller. `files` stages the payload beside the install and moves it into place, and writes down what it put there (`install.txt`), so an update removes what it no longer carries and the uninstaller removes exactly that |
| `rusty-release` | What the release workflow runs between the build and the upload: `pack` appends the install directory to the installer, `sign` writes a minisign `.minisig` with the updater key, `verify` checks it with `rusty_embed::update::verify` before anything is published |

## The rules that are load-bearing

### 1. Every crate keeps its data apart from its work

Each library crate has a `model`: the types its answers are made of, with no
IO in them — what the window, the CLI and the assistant's tools all read, so
the three cannot describe one thing three ways. Everything that spawns a
process, reads a file or walks a graph is in the modules beside it. One
process links it all; there is no second copy of any type to keep in step.

(Until v0.7.0 each crate was also split by a `backend` feature, so the
WebView frontend could compile `model` to wasm32. That frontend is gone and
the split with it.)

### 2. File formats and wire formats are different types

`catalog.rs` parses TOML into its own structs and converts to `model`. The file
format is a public contract with users who write board definitions; `model` is
an internal contract with the window. Tying them together means a UI refactor
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
`project_status` the assistant panel does and gets the same JSON and the same
refusals. A refusal is a result with `isError`, the agent loop's rule; only a
name that is no tool is a protocol error. **The Cargo workspace is loaded by
the first tool that needs it** (`LazyWorkspace`, the context's
`workspace_on_demand`), not before the question: resolving the graph is
`cargo metadata`, unbounded where no lockfile exists, and most questions
need none. It was a workspace or nothing once, and the app only had one
after the Crates page had loaded it — so the assistant's Cargo tools told a
user with a project open to open a project. The window seeds it with the
Crates page's and keeps what a tool loaded (`App::crates_keep_workspace`); the MCP server,
connected for a whole session while the project changes under it, keeps it
while a stamp of the root manifest, the lockfile and every member's manifest
holds, and reads the catalogue and the newest firmware afresh on every call.

**The MCP server also serves `simulate`, and the assistant panel does not.** It builds
the firmware, boots it in rusty's emulator with the sheet on its pins and
buses, drives the steps it is given and answers with what the firmware
printed, which pins moved and what crossed the buses — the one tool that
answers "does it do what I think" by watching rather than reading. It runs
commands and cargo may fetch, so it declares `runs_commands` and `network`,
and `ToolRegistry::served()` is the only registry that has it: an MCP client
reads `readOnlyHint: false` and asks the user first, and the built-in
assistant, which has no step where anybody says yes, stays on
`ToolRegistry::workbench()`, every tool of which is read-only. The test that
pins that is still there; it now pins the assistant panel's registry.

**A description that lists things is generated from them.** `simulate`'s
"supported on" named three parts while five ran; it is the catalogue's
parts with an `emulation` now (`simulated_parts`). The prompt and the
project and toolchain descriptions said "Espressif" and "Cargo.toml" for a
workbench that opens PlatformIO and CMake projects for five vendors. And a
Cargo tool asked about a project with no `Cargo.toml` says that, and which
tools still answer — it reported a `cargo metadata` failure, which a model
read as a broken project.

**The project's files are tools as well** (`tools/files.rs`: `read_file`,
`search_project`, `list_files`), through `rusty_edit` so the model sees the
project exactly as the project's tree does — confined to the root, `.gitignore`
honoured, dot entries and `target/` absent, nothing written. **A build
directory is absent whatever the ignore files say** (`rusty_edit::hidden`,
by the `CACHEDIR.TAG` cargo writes into every one): an example with no
`.gitignore` of its own answered `list_files` with 708 files of `target/`,
seven thousand tokens of object files on every round after. They exist
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

**So is the math sheet** (`tools/math.rs`: `math_sheet`), because rotation
arithmetic is where a model is most fluently wrong — a quaternion multiplied
out from memory crosses a convention without a word and reads as a
plausible number. The model writes rows in the Math page's language and
gets back what the panel shows, from the same evaluator: every value, the
working with the numbers in, and every refusal by the panel's own name for
it, worded in English for the model where the panel words it in the
reader's language. Without rows it works out the project's own sheet, in
the sheet's own frame; rows a model writes take that frame too unless the
call names one, so the answer and the user's panel draw the same aircraft,
and a sheet that does not read is refused — never read as an empty one, and
never guessed a frame for. **An attitude comes back with what an attitude
indicator shows** (`spatial::instrument`, the reading the panel's horizon
draws): the nose up or down, the bank, the heading. A model reading a
positive pitch as the nose going up is the mistake the tool exists for, and
with Z up it is the nose going *down*. **The window sends what a running
simulation says with the question** (`App::live_now`, the snapshot the
Math page's live rows read): the assistant's tools hold no telemetry and
run no plant, so without it a sheet the user watches live was worked out
with nothing in its live rows. Frozen for the answer, since
an estimate compared with a truth from another moment is a wrong
comparison. An MCP client has no window; it passes `telemetry` — a
`simulate` run's `[rusty:tel]` line, say.

**A question costs every round of it, and every round sends everything.**
The agent loop sends the system prompt, all thirteen definitions and the
whole conversation so far each time it goes back to the model, so a
question that calls one tool is two requests, and the history is paid for
again in each. Reported as "a simple question uses a huge number of
tokens", and measured before anything changed (`token_budget`, above): two
short questions about `examples/rate-loop`, one tool call each, 41,302
input tokens. Now 18,456, 65% of it a repeat of the request before — what a
prefix cache supplies at a tenth of the price. What did it:

- **Earlier questions go back without their bulk** (`agent::wire_history`).
  A file attached to an earlier question becomes a line naming it, and a
  tool answer longer than 1,500 characters a line naming the call, for the
  model to make again if it needs it. The transcript keeps every word; only
  the wire is shortened, and **the same way in every round**, because a
  prefix that changes between two requests is a prefix no cache can serve.
- **The open file goes whole only when it is short** (`Content::attach`,
  8,000 bytes): otherwise the selection, or the cursor's line and sixty
  either side, with which lines they are, so the model knows to read the
  rest. It was the first sixty kilobytes, on every round of every question.
- **Anthropic caches only where it is told** — three breakpoints: the
  system prompt (which holds the tools before it), the end of the
  conversation before the question, the last block. The OpenAI dialect's
  servers cache prefixes by themselves and need only one that does not
  move, which is the rule above.
- **The prompt and the descriptions say each thing once.** A description
  is what the tool is for; the prompt is only what holds across tools.
- **The meter is the question's** (`Spend`, `app/assistant.rs`): every round summed, the
  cached share beside it. It was the last round's report — and from
  Anthropic the half that arrives last, which carries no prompt, so a
  question that cost thirty thousand tokens read `0 in`. Each provider now
  reports once per round, before `Done`, because some servers repeat a
  running total on every chunk.

### 5. The simulator's contract is one serial line

Espressif's QEMU boots the same merged image `espflash` would burn, and
everything the board view knows travels as text on that one serial line:
`[rusty:gpio] 26=1,27=0` (or `[rusty:gpio@1234] …` with the systimer in
microseconds — what the Waves panel and the VCD export time by) and
`[rusty:disp] hello` out of the firmware,
`B14=1` and `P34=128` into it. `protocol.rs` owns the parsing, and the
window, `rusty-cli sim` and the assistant's `simulate` all read the stream
through it.

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

The reading of that protocol lives in *one* function (`App::absorb`, behind
`App::heard`, the one door every line any run prints comes through), because
it was per-stream once and the consequence was telemetry that plotted in the
simulator and vanished on hardware.

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
mean a new entry in upstream's build system for each. **The installer ships
it**: `scripts/bundle-tools.sh` unpacks the `qemu-v*` asset into
`target/bundled/`, the release packages that directory beside the
executable, and the window finds it there (`bundled_dir` in
`crates/rusty-desk/src/main.rs`) and hands it to `tools::set_bundled_dir` as
it starts, so a fresh install simulates with ours and never sees the stock
build's notice. The bundle sits *after* the data
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
window does not recognise leaves the weaker claim standing. And the stock
build **says what it costs**, in the dock at every run (`[rusty:pins]
firmware — …its GPIO write handler is empty, so is_set_high()/is_high() read
0…`, `desk.sim-stock-qemu`), and in the plan (`SimPlan.emulator`, whose
`gpio_model` is `has_gpio_model` of the binary the plan will boot). The
upgrade is the same download run over the copy that is there; the
Environment page never lists QEMU, only the simulator's plan does, and the
Slint window does not yet offer that upgrade as a button beside the
plan's verdict, which is where it belongs.
Found when a user's blinky printed `GPIO2 high :false` for ever under a
QEMU installed before rusty's build existed: `toggle()` reads the output
register back, the stub never stores it, and nothing on screen said the
emulator was the reason. Measured both ways on one image — stuck at `false`
under Espressif's build, alternating under ours.

The pin channel is a chardev of its own (`-chardev socket` + `-global`, since
the machine creates the GPIO device and there is no `-device` to hang it off),
and rusty feeds its lines into **the same** stream the serial console
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
plain switch wired between two GPIOs, and both travel as a join
(`App::sim_press`) rather than a level.

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
the reader, `sensor::Spec` the type the sliders are drawn from, and
`SimPlan.parts` how the window gets them — the symbol libraries' three
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
(`App::sim_sensor_set`).

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
early build is called out of date as a stock one is. **A marker is
asked only of the binary that can carry it** (`markers_of`): the ESP32's
interrupt matrix is compiled into the Xtensa emulator alone, so its marker
asked of the RISC-V one would call every current C3 build out of date. And
a marker names what a build *does*, never which build it is — the previous
ESP32 marker was the name of a region the next generation removed, and it
left with it. The QEMU workflow's packaging step checks the same list per
binary, so a build missing one never becomes the release rusty pins.

**What the emulator cannot do on a chip is in the plan, before the run**
(`SimLimit`, a stable kind beside the English — which the Slint window does
not show yet, and must, beside the run it is about), **and it depends on the
emulator as well as the chip**: `for_emulation(emulation, outdated)`, the
catalogue's `limit` and, on an older copy, its `limit_outdated`. The ESP32 has
every peripheral the C3 has, in its own layout, every interrupt source
reaching its handler — a timer's and a software interrupt's as well as a
GPIO edge's, which is what an Embassy application's clock is made of — and
the FPU on from reset — so with rusty's current
build it has no limit, and with an older copy it has one
(`esp32-outdated`), because each of those arrived after that copy was
built; the upgrade is the fix. The S3 has never been checked,
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

**A part is data, and so is a vendor.** Everything rusty does differently
per part — how a project is started (`generator`), which emulator runs it
and what that cannot do (`emulator`), how its pins are named (`ports`),
the devkit drawn around them (`header`, `kit`), its debugger, its cross C
compiler, where its SVD is published — is a key of its `[[chip]]` entry,
and a vendor is a `[[vendor]]` table (its HAL, and the crates detection
reads). It was about seventy-five `match`es on chip ids, prefixes and a
closed `Vendor` enum, and the wrong answers they gave were the kind this
file is about: every STM32 sent to esp-generate, every non-Espressif part
handed Espressif's RISC-V gdb and C compiler. A key's absence refuses
(`docs/extensibility.md` has the table), `chip::tests::what_a_part_names_
exists` holds every key to something that exists, and a project's
`.rusty/chips/` can add a vendor and its parts with no code. What stays
code is what is code: a flasher's command line (`Flasher`), a toolchain
kind, an emulator's peripherals.

## Modal editing

Vim keys in the editor, off by default, switched on from View, the palette
or Settings. `rusty_edit::vim` is a pure state machine — `(keys, text,
cursor) -> Step` — under sixty tests that name the property a Vim user would
notice missing. `rusty-desk/src/editor/modal.rs` feeds it the keys and
carries out what it says: the new text, where the cursor goes, what is
selected, and the few things only the editor can do (save, find, scroll,
undo).

**The precedence is the whole design, and it is what makes it liveable.**
Normal and visual mode own *unmodified* keys; no global binding uses one, so
the overlap is almost nothing. Chords stay with the editor and the globals
except five — `Ctrl+R`, `Ctrl+O/I`, `Ctrl+D/U` — so Ctrl+S, Ctrl+K and
Ctrl+A are untouched. Insert mode claims only Escape, which is why
completion, quick fixes and every learned shortcut keep working the moment
you type. The window feeds the machine after the completion popup and
before the editor's own keys; `Step::handled` false is the path on to them,
exactly as with Vim off.

- **Normal and visual mode draw a block over the character the cursor is
  on** (`block-caret` in `ui/editor.slint`, as wide as that character:
  `block_width`) — on an empty line and past a line's end too, where a
  one-character selection standing in for a cursor had nothing to select —
  and in visual mode as well, where the selection alone does not say which
  end moves. What an input method commits in normal mode is not text.
- **The selection is not the cursor, so the cursor is remembered.** In
  visual mode the selection runs from the anchor through the character the
  cursor is on, and its head is one past it: read off the selection, every
  `l` began again from the wrong end, a selection could not grow past two
  characters and `Vjj` stopped at two lines. The pure machine was right all
  along — its tests feed `step.cursor` back — and the bug lived in the
  round trip no test made. `modal.rs` keeps the cursor the machine last set
  beside the selection it set for it (`Placed`), and trusts it while the
  document still holds exactly that selection; anything else — a click, a
  find, an undo — moved it.
- **Indices are Unicode scalars**, converted to the editor's bytes at the
  boundary (`byte_of`) exactly as the LSP client converts at its own. A `中`
  in the buffer must not shift every motion after it.
- **`Span` carries two numbers on purpose.** `e` puts the cursor *on* the
  word's last character and `de` deletes *through* it; a motion returning one
  number gets one of the two wrong. Bare motions use `cursor`, operators use
  `start..end` — and using `end` for both was the first bug, visible as the
  cursor landing one past every word.
- **What it does not know, it names**, in the status line beside the mode. A
  key that vanishes teaches people the editor is broken. `Ctrl+O` and
  `Ctrl+I` walk the editor's own history (*Back and forward*, under *Two
  editor groups*), the one Alt+Left walks, and say so at either end.
- **The switch is `workbench.toml`** (`vim`), read as the window starts and
  written the moment it changes. Landing in the wrong mode is not a shrug, it
  is twenty keystrokes doing something else.
- Undo granularity is Vim's, not the editor's. `Step::seal` closes the unit
  at a command boundary — the history holds one step from a command into
  insert mode until Escape — so `ciwfoo<Esc>` undoes in one press instead of
  one keystroke at a time.
- **The machine knows nothing about folds, and need not.** Its scalars index
  the document, which is all the editor's text ever is — a fold only decides
  which rows are drawn (*Code folding*) — and a cursor Vim moves onto a
  hidden line opens what hides it, as VSCodeVim's does, rather than standing
  on a line nobody can see.
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
- **`cw` on a one-letter word changes that word.** It ran on to the next
  word's end, as `ce` does from there, and `let a =` lost its `=` — found by
  the window's probe, in the shared machine, so a test holds it.

## Code folding

**Folding is the one feature that makes the rows on screen stop being the
lines of the file**, and every rule here follows from that.

- **The arithmetic is in `rusty_edit::fold`, pure and tested.** Which regions
  can collapse (indentation, as VS Code's default is — a file mid-edit does
  not parse) and the line conversions.
- **The text is always the document; the folds only decide the rows.** The
  folds live in the document (`editor/document.rs`), and `Document::row_of`
  and `line_at_row` are the one crossing between a line and the row it is
  drawn on — for a click, the caret, Up and Down, the hover card, the popups
  and the margin. Nothing is ever written against a folded screen, so no
  edit has to be mapped back to the document, and no fold can put bytes in
  the wrong place.
- **A hidden line is drawn on its header's row**, not on none, so an error in
  a folded function marks the fold instead of disappearing.
- **The folds are carried across every edit** (`document::carry`): one below
  an edit moves with it, one whose hidden lines the edit reached opens, one
  being typed on stays. A cursor put on a hidden line opens what hides it
  (`Document::unfold_over`).
- Folds are session state, kept with the document like its cursors, and **not
  persisted**: restoring yesterday's folds onto a file somebody else has since
  edited collapses the wrong lines.

## Large files

A file used to stop being coloured, and start being read-only, at 5,000
lines, and a 24,000-line file cost a second of syntect after every pause in
typing. Now the limit is the 2 MB refusal (`rusty_edit::text_file::MAX_BYTES`),
and what a keystroke costs is what it changed. Measured by the window's probe
on a 24,706-line, 1 MB file with Chinese comments and tabs: a key to the
frame that shows it 14.9 ms, a scroll step 16.0 ms — the next frame at 60 Hz
(`docs/slint.md`).

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
- **The painting is kept by number, and the editor names which one it
  holds.** `rusty_edit::Files` keeps a dozen, each under a number that only
  counts up (`Document.paint`, `Repaint.version`); a request naming another
  — evicted, or moved on — is answered whole, which is correct whatever the
  editor held. A repaint runs off the UI thread, one on its way per document
  (`Surface::paint`); an edit puts plain lines where it changed until the
  repaint lands, and a repaint that lands on a text since edited is placed
  only where it still holds (`Document::painted_from`), with another asked
  for the new text.
- **Lines are split at `\n` everywhere**: a text ending in a newline has an
  empty last line, painted and numbered. `str::lines` on one side and `split`
  on the other is an off-by-one between the painting and the text that a
  repaint cannot survive.
- **The rows are a `Model` over the document** (`editor/rows.rs`), so the
  view asks for the rows on screen and an edit names the lines it touched
  (`Change`): opening a big file builds forty rows, not twenty-four thousand.
  **Rust measures and places, Slint draws**: a row reaches the view as
  pieces — a run of one token's colour and the x it starts at
  (`editor/metrics.rs`) — so the view does no text layout, and the caret, a
  click and every overlay use the arithmetic that placed the text. ASCII is
  one width, measured once over a run of 64; anything else is measured the
  first time it is seen, over a run of 16 — measured once, a `中`'s rounding
  was eight pixels by the end of a line.
- **A long file's semantic colours are asked for around the lines on screen**
  (`semanticTokens/range`, 400 lines either side, above 3,000 lines —
  `editor/analysis.rs`), with the inlay hints, a moment after typing stops
  and when the server says it settled. Whole, a 24,000-line file's colours
  were megabytes and most of a second.
- **Everything that still reads the whole file per keystroke is linear and
  cheap, and was not always.** `fold::regions` scanned forward from every line
  (quadratic in a long block); it is one pass with a stack, held to the old
  scan by a test over real files. `tests_in::runnables` returns at once for a
  text without the word `test`. Undo keeps two hundred steps *and* 48 MB of
  them (`editor/history.rs`), whichever comes first.
- **The analysis is another process; it costs answers, not keystrokes.**
  rust-analyzer loading a project takes seconds to answer completion,
  colours and hints, and none of that waits on the UI thread: the client
  blocks on its pipe on threads of its own (`lsp.rs`), and every answer
  comes back through `work`. What costs keystrokes is a machine compiling
  with every core busy, which is slower for everything on it.

## Bracket pairs, and what a keystroke asks the server

- **Four rules, pure and tested, in `editor/pairs.rs`.** An opener brings its
  closer with the caret between; a closer typed against its twin steps over
  it; Enter between `{|}` puts the caret on an indented line with the closer
  below; a closer on a blank line takes its opener's indentation (the
  matching opener, found by depth, not the nearest); Backspace inside an
  empty pair removes both; an opener typed over a selection wraps it. Single
  quotes are left alone — `'a` is a lifetime. A `"` inside an unterminated
  string closes it, and after a word it is a typo being corrected, except
  after `r` and `b`. Every rule returns an `Edit` in document bytes, applied
  through `Document::edit` like every other write, so undo, the painting and
  the folds cannot disagree with it. Vim's insert mode passes every key but
  Escape, so the rules hold there too.
- **A pair is judged by the file's brackets, counted in code only**
  (`code_brackets`: comments, strings, raw strings and character literals
  skipped, a lifetime told from a character by the quote that closes one).
  Reported as the editor "often putting in the wrong `}`", two rules had
  been blind. An opener is not paired while the file holds a closer with
  no opener — a `{` retyped after deleting one, or added to a line whose
  block and `}` are already below, left the extra `}` — and a closer steps
  over its twin only when this line opened it, which is what an auto-closed
  pair always is: VS Code overtypes only the closers it inserted, and this
  is that rule without a record of them. Stepped over any `}`, the brace
  typed to close a block that was missing one vanished, and rustfmt said
  `found <eof>`. The blank-line indent finds its opener the same way, so a
  `"{"` above no longer sends a `}` under the wrong block.
- **A character a pair rule took still asks the server.** Whatever was
  typed — through a pair or not — reaches `after_typing`
  (`editor/completion.rs`), the one place the completion and signature
  triggers are judged, so a `(` that opened a pair still asks for the
  signature. A rule that took a key away from that path would take a feature
  away by adding one.
- **Where the caret rests is marked, twice** (`editor/resting.rs`), VS
  Code's `matchBrackets` and `occurrencesHighlight`. The bracket beside it
  and its partner are boxed, found on the *painted* lines
  (`editor/brackets.rs`, pure and tested against the painter itself), whose
  tokens already say what a string and a comment are, so a `{` in `"{}"`
  or `'{'` pairs with nothing — a format string's placeholder is painted as
  a constant, and counts as neither — and a block comment's brackets are
  passed over however many lines it runs; a line typed on since its
  painting is read by the pair rules' own lexer (`pairs::code_brackets`)
  until its repaint lands, and the repaint matches again. The bracket after
  the caret first, then the one before, as VS Code takes them; a partner is
  found by nesting over every kind, so `{ ( }` boxes its braces and leaves
  the parenthesis with nothing rather than sending it to some later `)`;
  and it is looked for no further than 3,000 lines either way. The other
  places the name under the caret occurs are washed, asked of the server
  (`documentHighlight`) 400 ms after the caret stops — after the change it
  rests in was sent, and after the 300 ms the colours wait. An edit takes
  the washes away at once, from every row still showing one where its line
  now is (`Resting::take_after`); a move off them takes them away and a
  move within them keeps them; every ask is numbered (`editor/asks.rs`) and
  an answer is drawn only while it is the last and the text is the version
  it was asked about. Both are grey from the theme, so neither reads as a
  selection or a find, and both cover the characters' own extent
  (`inlays::ink`), never a hint at their edge.
- **The popup owns its keys only while it shows rows** (`popup_visible`,
  `editor/completion.rs`). A popup narrowed to nothing that still ate Enter
  and Tab was an invisible popup through which a line ending in `v.xyz` could
  not be broken.
- **rust-analyzer's item order is arrival order; its ranking is `sortText`.**
  Taking the first hundred *unsorted* shipped a hundred arbitrary slice
  methods for `v.` and dropped `len`; typing `le` then narrowed the popup to
  nothing — an editor with no completion, reported in exactly those words,
  while every request and reply was correct. `convert::completion_items`
  sorts first and caps at 1,000 — it was 400, cut before any filtering,
  and a large scope lost `println!` and `Vec` to the alphabet before `pr`
  was typed.
- **Completion asks on the first letter and keeps asking while the answer
  is incomplete** (`rusty_lsp::complete::ask_for`, pure and tested) — VS
  Code's rule, and the one rust-analyzer is built for: it marks every answer
  incomplete, because its imports are searched by the word typed. Asking
  once per word, on the second letter, and never again meant an ask that
  failed or came back empty while the index warmed up left no completion for
  that word at all — "unreliable, unusable", in the user's words. A word
  already showing is asked about again 40 ms after the typing pauses;
  deleting widens an open popup and never opens one; an ask that fails is
  made once more if it is still the latest, because rust-analyzer cancels a
  request an edit overtakes (`content modified`).
- **An answer is shown only while its word is still the word being typed.**
  Every ask is numbered and anchored to its line and word start
  (`editor/completion.rs`); every dismissal bumps the number. The popup
  closes when the caret leaves the word — an arrow, Home or End, a click,
  Enter, Backspace past the word's start, a character that ends it — and
  an answer landing after that is dropped. Shown wherever it landed, `foo`
  and a quick Enter opened the popup for `foo` on the next line, and the
  next Enter accepted it. The first answer for a word is shown the moment it
  arrives even with later asks for the same word out; an older answer never
  replaces a newer one.
- **The popup ranks what it shows** (`rusty_lsp::complete::ranked`): names
  that start with the word first, in the server's order, an exact-case start
  ahead; then names the word only fuzzily matches, the first character at a
  word start — `itr` finds `iter`, `hm` finds `HashMap`, `ln` does not find
  `println`. `filterText` stands in for the label when the server sends one.
  The keyboard's row goes back to the first on every narrowing (kept, it
  became another item that Enter accepted), and Up and Down wrap.
- **Snippets are on, and expanded by the editor**
  (`rusty_lsp::complete::expand_snippet`, pure and tested): a function
  arrives as `name($0)` and a macro as `println!($0)`, and accepting puts the
  caret between the parentheses and asks for the signature. Only the first
  tabstop is honoured, so the client asks rust-analyzer for
  `callable.snippets: "add_parentheses"` rather than its default of
  arguments to tab through. Escape closes the popup before Vim sees it, as
  the suggest widget's does in VS Code.
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
  an item was picked from — and the editor splices the edits above the
  caret, shifting it by what was inserted (`apply_resolved`). `label_detail`
  carries the ` (use …)` note so the row says what accepting it will add,
  and it is a field of its own only because the client declares
  `labelDetailsSupport`: without it rust-analyzer glues the note onto the
  label.
- **A rust-analyzer that failed to load the workspace looks exactly like one
  that works.** It still lexes and parses, so the squiggles keep arriving
  while every completion, hover and jump answers nothing — for ever, with
  nothing on screen saying why. The handshake asks for
  `experimental.serverStatusNotification`; that notification and an
  error-level `window/showMessage` both travel as `LspEvent::Health`, the
  status bar's server item shows the trouble with the server's own words in
  its tooltip, and a notification says the sentence once (`app/servers.rs`;
  once, not per notification: rust-analyzer repeats its state on every
  change). Found on a report of "completion is still missing" from another
  machine, where the project and the server were both fine here. `cargo run
  -p rusty-lsp --example complete_probe -- <project> <file> [target]` is the
  headless check that splits the two halves: it starts the server the way
  the window does, asks for `imp`, `impl Qu` and `impl core::` every two
  seconds until they answer, and prints what the popup would keep —
  `["impl", "impl for"]`, `["Quaternion"]` — beside how long the server took
  to get there.
- **A file in no crate's module tree is the *other* way rust-analyzer goes
  silent, and it looks identical.** No `mod` or `pub mod` names the file, so
  it is in no crate: the server parses it, reports its syntax errors, and
  answers nothing for completion, hover or navigation. It says so —
  `unlinked-file` — as a **Hint**, which Problems filters out by the rule
  that hints are not problems, so the one sentence explaining the silence was
  the one thing on screen nobody could see. Found by `complete_probe`, which
  reproduced "no completion at all" in one run after three rounds of asking
  the user for facts; the lesson under that is the user's own: *trace the
  whole path from the server's answer to the screen* rather than
  interrogating the person in front of it. `convert::diagnostics` promotes
  that one code to a Warning, so the squiggle and its card say it, and
  Ctrl+. offers rust-analyzer's own `Insert mod …;` fixes.
- **The file's name is to be dimmed, before anybody opens it, and a shade
  of grey is the whole message.** `unlinked-file` arrives only for a file the
  client has opened — a `didOpen` per file in the project would be hundreds
  of notifications and hundreds of diagnostic computations — so
  `rusty_edit::modules` reads every `.rs` file's `mod` lines itself and names
  the files under a crate's `src/` that nothing declares (`unlinked_probe`
  prints its answer). The Slint window does not draw the dim yet; when it
  does, it is the name greyed in the tree, its tab and the header, the
  sentence as the tooltip, and the fix where every other fix is. A
  full-width notice with a sentence and a button was tried once, and the
  user's verdict was exact: "a pile of warning text", where VS Code says the
  same thing by greying the name. VS Code does not dim the *code* — read off
  the wire, `unlinked-file` is severity 4 over `0:0–0:2` with **no `tags`**,
  so there is no `DiagnosticTag.Unnecessary` to act on — and rusty, which
  knows before the file is opened, may.
- **The scan is arranged to fail towards "linked", and every rule in it is
  that decision**: a `#[path]` attribute anywhere in the project takes the
  whole answer away, a `mod` in a comment counts as a declaration, every
  entry point cargo knows is a root, and nothing outside a crate's `src/` is
  claimed about at all. Dimming a file the compiler builds is the confident
  wrong answer in miniature; being silent about an exotic project costs
  nothing.
- **Two sources for one fact need a rule about which wins, not an `or`.**
  The scan `||` rust-analyzer's `unlinked-file` could assert and never
  retract: adding the `mod` line left the file dimmed, because the server's
  diagnostic stays until it re-analyses *that* file and nothing was going to
  make it. The scan wins wherever it has an opinion — it has just read the
  files — and the server answers only where the scan refused. **Which means
  the refusal has to be sayable**, so `modules::unlinked` and `modules::scan`
  return `Option<Vec<String>>`: an empty list is "every file is declared" and
  `None` is "ask somebody else". They were the same value, and a refusal
  read as a clean bill of health.
- **rust-analyzer offers no `Insert mod …;` when the parent module file does
  not exist.** `src/vector.rs` beside a `main.rs` gets three fixes;
  `src/math/matrix.rs` with no `src/math/mod.rs` gets none, because the fix
  has nowhere to write. Nothing rusty can do about it, but it is the case a
  user hits, and the dim is then the whole of what tells them.
- **A fix that edits another file used to be dropped, which is why the one
  fix for that state was unreachable.** rust-analyzer's fix for an unlinked
  file edits *only* the parent module, and the client refused any action
  touching a second file. `convert::split_edits` splits a WorkspaceEdit into
  this file's edits and the others'; `CodeActionFix.elsewhere` names the
  others so the row says what accepting it will write, the editor splices
  its own document and `apply_action_elsewhere` writes the rest the way a
  rename does (`app/navigate.rs`). A target file with an unsaved draft
  refuses the whole fix by name — those edits land on disk, and the next
  Ctrl+S there would put the draft's stale bytes back over them.
- **A problem's fixes are on the hover card, under it** (`editor/hover.rs`),
  as well as Ctrl+. in the picker (`Mode::Fixes`), which needs somebody to
  know a fix exists before going looking for it — the card is where the
  pointer already is. Asked for **only when the card shows a problem**, and
  after the card rather than with it — a `codeAction` resolves every offer
  it did not come with, and a tooltip that waits for that is a tooltip that
  does not appear — and dropped unless the card it was asked for is still
  up over the same text (the card's numbered asks, and the version). Only
  the quick fixes are listed (`fixes_for_card`: kind `quickfix`, or none); a
  refactoring stays Ctrl+.'s. A click applies one by the server's own index
  into its numbered answer, through the picker's path
  (`App::apply_fix_from`), another file's edits included. **The rows are
  inside the card's touch area**: a touch area beside them loses the hover
  to theirs, which put the card away under the click. `impl core::ops::Mul
  for Quaternion {}` offering *Implement missing members* is the case it is
  for; the probe's (`RUSTY_PROBE_MARKS`) is a struct literal missing two
  fields and *Fill struct fields*.
- **A key puts the card away, and what the check found follows the
  edits.** The card is about the text as it was when it was asked for, and
  nothing closed it but the pointer: a `;` typed for a `,` and mended under
  a resting pointer left `expected COMMA` up over a line with nothing wrong
  in it — reported in those words. Any key puts it away, as VS Code's does
  (`editor/hover.rs`), and an answer landing after the pointer moved on is
  dropped. Under the card was the second half: rust-analyzer's own `expected
  COMMA` went the moment the change reached it, measured with `fix_probe`,
  but the check runs on a save and at no other time, so the saved `;`'s
  `struct fields are separated by ,` stayed under the `,` that mended it, in
  the editor and in Problems, until the next save. The client now carries
  both sources across every change it sends (`Shared::follow_edit`,
  `rusty_lsp::follow`): moved with the text before them, and gone where the
  edit took away the text they were about — VS Code's decorations, which
  never grow when typed at the edge. Inlay hints and semantic colours follow
  by the same arithmetic (`rusty_lsp::follow::hints`, `follow::semantic`), so
  a line cannot move one way for a hint and another for a squiggle.
- **Two askers need numbered answers.** The client kept *one* code-action
  slot per file, and an accepted fix was applied from it by index. That was
  safe while the caret was the only asker; with a hover asking too, a card
  appearing beside an open list silently renumbered its fixes, and the click
  would have written another position's edits into somebody's other file.
  `CodeActions` carries a `reply` number and the client keeps the last four
  answers — exactly the shape `completion`/`resolve_completion` already had,
  arrived at the same way and for the same reason.
- **Half-composed text from an input method is not input.** A Chinese IME
  shows its own pinyin segmentation while composing — typing `flyegg` passes
  through `f'l` and `f'l'y` — and the wizard once judged every one of those
  as a crate name, and put a red refusal of a name nobody had typed on
  screen. Anything that *judges* what was typed judges committed text;
  anything that merely echoes it need not. The Slint window has not yet been
  tried with a real Chinese IME (`docs/slint.md`), and every field that
  judges as it arrives — a crate name, a branch name, a search — is the
  first place to look.
- **A name is refused beside the field, by one rule.** `crate_name_problem`
  is in `rusty_embed::model`, so the wizard can say which character is wrong
  while it is being typed and disable Create, and the generator's own
  `valid_name` calls it rather than keeping a second copy — the Git panel's
  `ref_name_problem`, applied to the one name the generator never sees. It
  was a check on every keystroke whose only voice was a banner. **And moving
  the voice to the field was half of it**: the plan was still asked for on
  every change, still refused the name, and clearing the field to type a new
  one put "`` is not a name cargo accepts" in the dock once per change under
  the field saying the same in red. No plan is asked for until the name
  passes (`app/wizard.rs`). When a check moves to the field, find every call
  that still asks elsewhere. The rule itself was wrong for as long as it
  existed — it let a leading digit through and refused a leading `_`, the
  opposite of cargo, measured on a manifest (`the name cannot start with a
  digit`).
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
  The window reproduced "partly loaded" on an esp project while
  `complete_probe` on the same project looked perfect, and two rounds of
  reasoning about environments got nowhere. The variable was the target
  hint — the server was started with the chip detected at the *opened
  root*, which for the standard embedded layout has none, so it passed none
  (`lsp::start` passes none there on purpose now: the firmware crate is
  linked as a project of its own, and its own config names its target) — and one
  run with the server's own stderr let out named both the cause and the
  fallback in a single line.
- **Copy, cut and paste with nothing selected act on the whole line**, as
  VS Code's `editor.emptySelectionClipboard` does (`editor/clip.rs`, pure and
  tested). Ctrl+C puts the caret's line and a break on the clipboard; Ctrl+X
  takes the line out, break and all, the caret keeping its column in
  characters; Ctrl+V of *that* text — compared with `\r\n` made plain,
  because the Windows clipboard hands `\n` back that way — goes in above the
  caret's line wherever the caret is, rather than into the middle of it.
  **With a selection they act on the selection, in every mode**, Vim's normal
  mode included, whose cursor is drawn rather than selected, so a selection
  there is a double-click or a drag somebody made; a copied line goes in
  above the caret only when nothing is selected.
- **Tab over a selection moves its lines** (`editor/indent.rs`, pure and
  tested), VS Code's rule. A selection that reaches more than one line, or
  holds the whole of one, is indented a level; Shift+Tab takes a level off
  every line the selection touches, or off the caret's line. A caret still
  types four spaces, and a selection inside one line is typed over. Typing
  four spaces over *any* selection replaced two lines picked out to be
  pushed in with the indentation meant for them. A level is the next stop,
  not four more columns; a line the selection only reaches the first column
  of is not in it; an empty line is given nothing to carry; a line indented
  with tabs moves by a tab, so a Makefile's recipe stays one. One `Edit`,
  with the selection still holding what it held.
- **Ctrl+S writes what is on screen, and never reads back** (`app/saving.rs`).
  Reading the file back into the editor after writing it was slow enough on
  a machine busy with `cargo check` to type through: the keys pressed in
  between went with the re-read, and a save that lost them with the dirty
  dot still lit was reported as "sometimes it will not save". **The format
  before it has a budget and a condition**: a second (`FORMAT_BUDGET`, VS
  Code's rule for its own format-on-save), after which the save goes ahead
  unformatted; and the formatter's answer is used only while the document is
  still the text it was sent, since an answer about a text typed on since
  would put the old text back. Rust goes through rustfmt; C and C++ through
  clang-format only where the project carries a `.clang-format` — a style
  imposed on vendor code is a diff nobody asked for. **A file that does not
  parse is saved without a word** (`rusty_edit::format`, rust-analyzer's
  reading of rustfmt's exit: 1, or 101 with the parser's `error`): its
  syntax error is already a squiggle, and saying so again in Output was a
  line per Ctrl+S — sixteen of them in the report. A failure anybody can act
  on — no rustfmt, a bad `rustfmt.toml` — is still said. Auto-save is not in
  this window; if it comes back, it is not a format: rustfmt cannot parse a
  line mid-expression and would rewrite the line being typed.

## Two editor groups

Side by side, VS Code's everyday split, and no more than two (Ctrl+\).

- **The keys are in one group, and that group is the editor.** `tabs` and
  `active` are always the group with the keys and the other waits beside
  them, so every path that opens, picks or closes a tab works on one group
  without knowing there are two, and giving the keys to the other group
  swaps the two (`App::swap_groups`). The code view is one component, moved
  to the side that has the keys; the other side draws its file from a second
  rows model over its own document (`editor/shade.rs`, the `Shade` global),
  and a press there swaps the groups and is made again in the code view.
- **One file open on both sides is one document in two views**, as VS Code's
  split is: an edit, a painting or the server's answer draws both, and the
  history is the document's, so an undo in either view undoes the last edit
  made in either. Two drafts of one path overwrite each other on save, and a
  history per view would let one view's undo put back a text from before the
  other's edits. Closing one view of a file with changes asks nothing while
  the other holds it; the last view to close is the one that asks.
- **Each document keeps where its view was scrolled**, so a tab comes back
  where it was left, whichever group it comes back in. **And never above
  the top, which Slint's `ListView` will put it** (1.18.1,
  `model/repeater.rs`): a model reset keeps the rows' layout state, and a
  new model with fewer rows than the first one on screen clamps the offset
  to its last row but not the first row's place, so the rows are laid out
  from far down the view and the offset is set positive to match. A short
  file shown after a long one scrolled down sat twenty rows below the top,
  and the offset, kept with the document, carried the gap into a window of
  its own. Every offset read from a view, kept or put back goes through
  `shade::within`, and a view the list moved above the top is put back at
  it (`view_changed`, `Shade.scrolled`), from where the list lays out
  right.
- **The split never shows an empty pane**: a group left with no tabs ends
  the split.
- **Both strips persist** in `workbench.toml` (`ProjectTabs.second`), and the
  split comes back with them (`app/strip.rs`).
- **The divider is dragged, and where it was left is kept** with the docks'
  sizes (`WindowLayout.split`, the left group's share): a 7 px hold on the
  1 px line with a resize cursor, each group at least VS Code's 220 px while
  there is room for two, and a double-click puts it back at half. The share
  is the window's, not a project's, as the docks' sizes are.
- **A tab is named by its group as well as its id.** One file can be a tab in
  both groups under one id, so whatever closes or acts on a tab says which
  group (`App::close_tab_in`, `tab_in`): the other strip's × once closed the
  keys group's tab of the same file.
- **A strip's tabs sit in a lane that ends where its buttons begin**
  (`TabStrip`, `ui/pages.slint`), VS Code's shape: the split button drawn
  over the strip covered the last tab's name. More tabs than fit scroll the
  lane sideways under the wheel, either way of turning it, and the tab in
  front is brought into view whenever it changes or the lane is resized —
  from a timer a millisecond on, because a new tab's place is not laid out
  while the tab is being made, and asked for it then is a binding loop.
- **A tab's right-click menu is VS Code's** (`app/tab_menu.rs`): Close,
  Close others, Close to the right, Close saved, Close all; Copy path, Copy
  relative path (the system's separator, as VS Code's `auto`); Reveal in
  File Explorer (Finder, or the containing folder, named as each system
  names it); Split right — the file in the group to its right, a new one
  when the window is not split. The paths, Reveal and the new group are the
  tree's (`app/tree.rs`, `crate::reveal`), words and all, so a tab and a row
  of the tree do them one way. Each row is an action, named and keyed as
  the palette names and keys it, and the palette's way in acts on the tab
  in front. What would do nothing is left out rather than offered: nothing
  to close beside it, no file on disk behind a page or an expansion, no
  group to the right of the right one. Every close goes through the one
  close path, so a file with changes asks, and keeping one ends the batch.
  A row for another verb is one more id in `ROWS` and one more arm in
  `tab_verb`.
- **The picker is where navigation lands** (`picker.rs`, `app/navigate.rs`):
  the palette (Ctrl+Shift+P), the finder (Ctrl+P), a line (Ctrl+G opens it
  with `:` typed — `42`, `42:7`), references (Shift+F12), implementations
  (Ctrl+F12) and type definitions under a heading naming what they are,
  sorted by file and line (`navigate::Places`); one implementation or one
  type is a jump, as a definition is, and nothing found says so rather than
  being a key that seems to do nothing (`App::show_places`). Every place
  arrives with its line (`rusty_lsp::Place`), because a row of `lib.rs:41`
  is a riddle. `App::jump` is the one jump, and the place it lands is a
  place of its own for Alt+Left however near (*Back and forward*, below).
  Ctrl+P, Ctrl+G and Ctrl+\ are VS Code's chords, so hands
  that know them need not learn ours; type definitions, the call hierarchy
  and a macro's expansion have no key, as in VS Code — whose Shift+Alt+H
  for the hierarchy is an Alt-letter chord, left here to AltGr.
- **Back and forward are VS Code's navigation history, not a list of
  jumps** (`crate::trail`, pure and tested; `app/navigate.rs` feeds and walks
  it). Alt+Left walked only what `App::jump` had recorded, so after picking
  a tab or clicking around it did nothing — reported in those words. Now
  every place the caret settles is told to the trail: another file is a new
  place (a tab picked, a file from the tree, the finder, a search result, a
  problem), a jump is one however near it lands, and a move of ten lines or
  more in one file is — a click, Ctrl+Home/End, Go to Line, a find, each
  measured from the last settling, so the arrow keys never make one.
  Anything else replaces the current place, which is why Back from the
  newest place goes to where the caret *was*, not where it first arrived;
  the same line twice is one place; a place recorded after Back drops what
  was ahead. **The caret settles once a turn** (`App::caret_moved`, a 0 ms
  timer): a file opened at a line shows its document's caret and then the
  line, and recorded at both, the first was a place nobody went to. **A
  walk is not a move**: the trail ignores the caret until it is somewhere
  other than where the step left it, so a file still being read does not
  end the walk, and the arrival is the place walked to. Places follow the
  edits (`Surface::on_lines`, every edit's `Change`, on screen or off); a
  place in a closed file reopens it, and a macro's expansion closed since
  is passed over and forgotten. From a page Back goes to the file that was
  in front. The mouse's side buttons are Slint 1.18's `PointerEventButton.
  back` and `.forward`, taken where the code is drawn (both groups and the
  margin): every other touch area in the window takes the press as its
  own, and Slint has no hook before them short of the unstable winit
  accessor. **And Alt+Left never reached Back from the editor at all**:
  the keys land in the `TextInput` that holds what an input method
  composes, a key the editor rejects goes on to that `TextInput`, and it
  moves its own (empty) cursor on any arrow, Alt or not, and accepts — so
  the root `FocusScope` with the bindings never heard it. What the editor
  does not take is handed to the bindings from the editor's own key handler
  now (`Shell::invoke_key`), before the `TextInput` sees it; found by the
  probe dispatching Alt+Left as the platform does, not by reading the code.
- **Ctrl over a name draws it as a link** (`editor/link.rs`) —
  underlined, in the link colour (`Theme.link`, VS Code's
  `editorLink.activeForeground`), the hand — where the server has a
  definition for it: the definition itself is asked, as VS Code asks, and
  nothing is drawn for an empty answer. Asked when the pointer moves with
  Ctrl held and when Ctrl goes down with the pointer still, once the pointer
  has rested on a name 60 ms, numbered so a late answer for another word is
  dropped; gone when Ctrl comes up (the root `FocusScope` hears that from
  any panel), the pointer leaves the name, the editor loses the keys or a
  key goes down — a modifier on its own puts neither the link nor the hover
  card away, as in VS Code. The rows draw it: the name's characters are
  pieces of their own in the link's colour (`rows::kinds`), not an overlay
  over the text, and the line under them is a mark (kind 11) placed by
  `inlays::ink` like every other. **Ctrl+click follows the link drawn**
  (`Surface::link_under`, `App::follow_link`) rather than asking again
  about the point the click puts the caret at, so what is underlined is
  what the click does. **Slint chooses the pointer's shape only on a pointer
  event**, so a hand that comes or goes with the pointer still is made by
  dispatching the window's last move again (`pointer_again`). The row's
  "left" arrives after the next row's move, so it counts only for the row
  the pointer is on (`link_left`). **The `TextInput` at the caret has no
  width while nothing is composed** (`ui/editor.slint`, `sink`): a
  `TextInput` takes the pointer over it, and its few pixels (more than the
  two it was given: an empty one measures wider) sat exactly where a click
  had just put the caret — so Ctrl over the name just
  clicked, a Ctrl+click there, the hover card and a second click of a
  double-click at the caret all went to it and not to the code. The probe
  aimed three pixels past the caret and got nothing at all until then.
- **The editor's right-click menu is VS Code's** (`App::editor_menu`):
  where the name leads, then rename and the fixes, then the palette, each
  row an action named and keyed as the palette names and keys it, and the
  language's rows only where a server reads the file. The caret moves to
  the click first unless the click is inside a selection.
- **A macro's expansion is a document with no file** (`app/expansion.rs`,
  rust-analyzer's `expandMacro`), read-only, painted as Rust and opened
  beside the code — in the right group with the keys left in the code, as
  VS Code's `ViewColumn.Beside` keeps focus; from the right group, in front
  there. Its path is `rusty_edit::expansion_path`,
  `expansion:/<file>:<line>/<macro>`, so its tab reads as the macro with the
  call's place beside it, two calls of one macro are two tabs, and the same
  call expanded again is the same tab. `is_expansion` is how everything
  that would touch the disk or a server leaves it alone: `Servers::for_path`
  refuses it — the one door every message to a server goes through — the
  strip in `workbench.toml` leaves it out, no draft of it is kept, Ctrl+S,
  F2 and a breakpoint pass it by, and a jump back to one since closed goes
  nowhere.
- **Who calls a function is a tree, in the bottom dock's Calls panel**
  (`app/calls.rs`, `ui/calls.slint`) — VS Code's call hierarchy. Each level
  is asked of the server when its row opens, handing back the item the
  server sent exactly as it sent it (`CallItem.item`: the server puts what
  it needs to answer the next level in `data`). The tree is a flat list of
  rows with depths (`crate::calls`, pure and tested), and an answer finds
  its row by number, so opening a row above one still asking cannot put the
  answer in the wrong place; a tree started afresh, or turned the other
  way, drops answers meant for the old one. A click goes where the call is
  made — in the caller, the row itself for calls in and the row above for
  calls out — a double-click to the function, and a count lists every call
  from that place in the picker. A function under itself says *recursive*.
  In or out is a pair of icons on the dock's row, and the next hierarchy
  reads the way the last was turned.
- **Not in this window yet, though the client answers them**
  (`rusty_lsp::client::navigate`): the finder's `@` (the file's outline) and
  `#` (the workspace's symbols). What was learned building them stays true:
  `workspace.symbol.search.kind` is `all_symbols`, since rust-analyzer's
  default finds structs and not functions, and a symbol answer is kept with
  its ask so a slow reply to `#gp` is never shown under `#gpio`.

## A tab in a window of its own

VS Code's Move into New Window: `editor.detach` in the palette and in a
tab's right-click menu — for the tab clicked, in its own group — or a tab
let go of beyond the main window's edge (`Shell.tab-dropped`, which names
the group too, since one file can be a tab in both). One process, a second
Slint window holding one file (`TornWindow`, `ui/torn.slint`), and
`app/torn.rs` for when it opens, what its keys do and how it closes.

- **A second window is a second surface, not a second editor.** Slint gives
  every window its own instance of each global, so a torn window's `Code`
  and `Find` are the main window's properties and callbacks again, of the
  same Rust type. `editor/host.rs` is the one place that knows there are two
  kinds of window (`Host`, `Strong::global`), and every `self.window` in
  `src/editor/` goes through it. `Surface::beside` makes the new window's
  surface, which shares the main surface's documents (`Open`), servers,
  painting, margin and metrics, and keeps a view of its own: rows, caret,
  popups, the hover card, the find bar, the Vim machine.
- **One file open in two windows is one document, the split's rule.** An
  edit in either draws both (`Surface::follows`, called where the split's
  shade already followed: an edit, a painting, the server's colours and
  hints, what a server found), the history is the document's, the servers
  hear about the file once — from whichever surface made the edit — and
  the last view to close asks about changes not saved (`App::views_of`
  counts the windows). The cursors and the folds are the document's too,
  as they are across the split, so typing in one window moves the caret
  the other draws; what the other view had open over the text — a popup,
  the card, the caret's marks — goes, since it was about the text before.
- **An answer finds the surface that asked by number** (`Surface::id`,
  `Views`). Work comes back through `work::spawn`, whose answer must be
  `Send`, and a surface is not: a completion, a hover, a signature, the
  colours, a repaint name their surface, and `app.editor().view(id)` finds
  it or drops the answer, its window having closed. A repaint, which frees
  its document for the next one, lands on the main surface then.
- **What is about the file happens in its window; what is about the
  project is the main window's, which comes forward.** Typing, completion,
  the hover card and its fixes, find, folds, save, undo, Vim's `:w` and `:q`
  (`Surface::on_act`) belong to the torn window. The palette, the finder, a
  list of places, a run and a jump to another file land in the main
  window, brought to the front with winit's `focus_window` — the one use of
  Slint's `unstable-winit-030` feature. An action asked for there runs with
  `App::keyed` set to its surface, so F12, Shift+F12, Ctrl+., F2, F8 and the
  call hierarchy ask about the caret in that window, and the quick fix
  picked from the main window's list is applied to the surface that asked;
  a definition in the same file is gone to in the torn window. Go to Line
  and the split are refused there, in its status line, since both act on
  the main window's editor. A right-click there has no menu yet: the menu
  is the main window's overlay.
- **Its keys are the main window's keymap** (`App::key_in`,
  `chord_key_in`). A key the code there does not take goes from the code
  straight to that window's own handler (`Strong::invoke_key`), as the main
  window's goes to its shell rather than bubbling, and is looked up in the
  one `Keymap`, rebound keys included; the action runs as one asked for
  there (`torn_act`). A chord's first stroke there is said in its status
  line, and its second is the chord's there too — the code asks first
  (`Surface::on_chord`). Alt+Left, Alt+Right and the mouse's side buttons
  walk the main window's history, which comes forward, and an edit there
  moves the history's places as one in the main window does (`on_lines`).
  Ctrl over a name draws the link there, and a click follows it as a
  definition found from there goes (`App::follow_link` takes the asking
  surface): in the same file, in that window.
- **The editor's text is one size in every window.** Ctrl and the wheel
  over the code there are `content_zoom("editor")`, the main window's, and
  `Surface::set_scale` sizes every live view: the windows measure their
  characters into one table and keep one place per document, both in
  pixels of the size the text is drawn at, so a window at a size of its
  own would be measured wrong by the others. The table and the places are
  done once (`set_scale`), each view is drawn again (`fit_text`, `refit`),
  and a new window takes the size before it shows its file (`take_scale`).
  Its rows take the wheel as the main window's do — the same `CodeView`.
  The interface's zoom is worked out per window by the main window's rule
  (`zoom::apply_to`, with what that window last asked for), again whenever
  it takes a new size, since it may be on another display.
- **It belongs to its process.** New Window is a second process with its
  own `App` and main window, sharing no memory; a window a tab was moved
  into is the `App`'s it was moved out of, Move into Main Window means that
  process's main window, it closes with that main window through its exit,
  and it is no instance (`instances.rs` lists processes, not windows).
- **Closing.** A torn window closes with its tab — its ×, Ctrl+W, `:q`, the
  system's close button — and its file's last view with changes asks first
  (`After::CloseWindow` when the answer is save). Closing it never closes
  the main window. The main window closing, or the project switching, keeps
  every draft first (one map of documents) and then closes every torn window
  without a question; a torn file is written to the remembered strip after
  the main window's left group, so a restart opens it there. A window let go
  of from inside its own callback is dropped a turn later
  (`Timer::single_shot`). Move into Main Window, the icon on its strip, puts
  the tab back in the group with the keys, where its view was scrolled.
- **It follows the tree as a tab does.** A rename or a move there moves the
  document for every window at once (`Surface::moved`, the map being one),
  and the torn window takes the new name (`App::torn_moved`, from
  `follow_move`); a delete closes the window without a question and keeps
  nothing for the file (`App::torn_deleted`, from `follow_delete`).

## What the editor draws beside the text

Indent guides, sticky scroll, inlay hints, a minimap and more than one
cursor — VS Code's, each but the cursors switchable in Settings ▸ Editor
(`EditorView`, `[editor]` in `workbench.toml`).

- **Inlay hints are drawn in the line, as VS Code draws them**:
  `let total: f32 = both();`, `sample(sensor: &gyro)`. Drawn at the end of
  the line for a release, they looked wrong, in the user's word. A hint takes
  room, so from a line's first hint on, where a character is drawn is not
  where the text alone would put it: everything that turns a column into an
  x or an x into a column — the pieces of a row, the caret, a selection, a
  squiggle, a click, the popups — goes through one walk of a line's
  characters and hints (`editor/inlays.rs`), with the metrics' tab rule
  inside it, since a tab after a hint goes to the stop counted from past the
  hint.
- **A hint belongs to one side of its column.** A type after a name, a
  chain's type, a closing brace's block belong to the code before them; a
  parameter's name to the argument after it. The caret at that column
  stands on the side of the code the hint is not about — before `: f32`,
  after `sensor: ` — so what is typed there lands beside the code the hint
  describes, and `rusty_lsp::follow::hints` moves hints through every edit
  by the same rule. Dropped until the next answer, a hint takes its width
  with it and the rest of the line slides under the caret and back.
  Parameter names are asked for — rust-analyzer's default.
- **Hints arrive whole only once the server has settled.** rust-analyzer
  answers `inlayHint` with nothing while it loads, and its
  `workspace/inlayHint/refresh` — which reaches only a client declaring
  `refreshSupport`, and is passed on as `LspEvent::Refresh` — can come
  while it is still loading, so what is asked then is empty too; a
  restarted server answered every refresh with no hints and then said
  nothing more, the hints appearing only once somebody typed. The client
  sends `Refresh` itself on the turn to `quiescent` as well, once per
  settling, and the editor asks for colours and hints together 300 ms after
  the last (`editor/analysis.rs`).
- **A hints answer is drawn only over the text it is about.** A hint is a
  line and a column, right for one text alone, and the editor carries an
  answer from the text it asked over to the one on screen — which assumed
  the server answered about the text asked over. It did not always:
  reported as every hint a line below its code, after a Tab replaced two
  selected lines with one, because the question reached the server before
  the change it followed. So the answer names its text (`InlayHints::about`,
  `rusty_lsp::text_mark` of what the client had sent when the question went
  out, read and written in one turn — see *What the client records and what
  it writes go in one order*), and an answer about another text is asked
  for again rather than drawn; the hints already there have followed every
  edit and are left standing meanwhile.
- **An answer places the caret; it never moves the view** (`place_caret`,
  `editor/mod.rs`). Colours and hints landing redrew the caret through
  `show_caret`, which also scrolls to it — and a long file's answer is
  asked for *because* the view moved away from the caret, so the wheel's
  work was taken back the moment it was answered. Found by the zoom
  probe's plain wheel, which scrolled and then read the view back at the
  top whenever rust-analyzer happened to answer in between.
- **Sticky scroll is the fold regions read the other way**
  (`rusty_edit::sticky`, pure and tested; drawn by `editor/overview.rs`): the
  headers of the regions holding the line under the stuck rows, outermost
  first, at most five. **The search for that line only ever adds rows.** A
  stuck row covers the line it sits on, which changes which regions hold the
  first line still showing, which changes the count: at a block's closing
  brace one row fewer uncovers a line of the block and one more covers the
  brace, and the first version took turns between the two — nothing stuck at
  all at the offset where it mattered. `stuck` grows from the first row's
  headers until the line under them adds none, a fixed point rather than an
  oscillation.
- **The minimap is an image painted in Rust** (`editor/overview.rs`): two
  pixels a row and one a character, scrolled against the editor with VS
  Code's arithmetic (`rusty_edit::minimap::frame`, tested), and painted again
  only when the rows it shows or their painting change. Drawn as elements, a
  file of any length would be thousands of them.
- **Indent guides** come from the indentation
  (`rusty_edit::guides::indent_levels`, counted once per version of the text
  in `editor/rows.rs`);
  a blank line takes the deeper of its neighbours, or every empty line in a
  block would break its guide.
- **More than one cursor is arithmetic over the text** (`editor/cursors.rs`,
  pure and tested): every key that edits or moves is applied at every cursor
  in one pass — one document, one undo step — and positions are document
  bytes on character boundaries, the first cursor always the one the keys and
  the input method land at. Text typed or composed goes in at every cursor;
  a paste with one line per cursor gives each its line in document order, as
  VS Code's does; Ctrl+D takes the next occurrence and wraps.

## The Git panel

The repository, in two places, as VS Code and Fork split it. The Git panel
in the left dock is VS Code's shape: what is checked out and how it stands
against its upstream, fetch, pull and push (the counts they would move in
their tooltips), the commit box with amend, the staged and unstaged lists
with their verbs on hover, and the stashes. The History page in the editor
area is Fork's: every ref down the left — the local branches, each with how
far it is ahead of and behind its upstream and whether that upstream is
gone; the remotes, grouped by remote; the tags — a click going to the
commit and a double-click checking a branch out; the graph of lanes beside
the commits, labels on the commits that carry branches and tags; a search
over the log that dims what does not match and steps through what does;
and a commit opened below with its files and their patch, one column or
two. A file's diff is a page of its own. `app/git.rs` reads and writes,
`app/git_view.rs` draws, and the pure halves are `rusty_git`'s (`graph`,
`log`, `plan`, `diff`, `parse`).

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
  index alone, the status; the stash, the stash list. A change on disk
  re-reads the status (`app/watch.rs`), and the stamp is asked every 2.5 s
  while the panel or the page shows (`POLL`) — which is how a commit made in
  a terminal, invisible to the watcher, arrives. Where the git directory is
  comes from one `rev-parse --git-dir --git-common-dir` per root, cached, so
  a worktree's stamp reads the refs it shares.
- **No read runs twice at once.** One of each read is in flight with at
  most one more asked for after it, never a queue (`Gate`, `app/git.rs`), so a
  burst of saves is two reads, not ten; and a history answer for a revision
  or a length no longer asked for is not shown.
- **Background reads take no locks** (`GIT_OPTIONAL_LOCKS=0`). `git status`
  refreshes the index as a side effect and takes `index.lock` to do it, so a
  status every few seconds would sooner or later collide with the user's own
  `git commit` in a terminal: `Unable to create '…/index.lock': File exists`.
- **An opened commit is kept by hash** (the last 32, `CACHED`), which names
  one content for ever — never under `stash@{n}`, which names a different
  stash after every push — so stepping back to one needs no `git` at all, and
  an answer for a commit no longer selected is kept and not shown. A patch
  past 4,000 rows draws that many and counts the rest.
- **Showing the panel is not opening it.** Read everything again each
  time it was shown, it dropped the opened commit with it; a root it has
  read is only probed now (`App::git_shown`, the stamp), and a new root
  starts clean.
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
  copy stays, and the question says so — and aborting a stopped operation ask
  first, in the system's own dialog (`confirm`, `app/git.rs`). Merge, `branch -d` and push do not: git's own
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
- **Reads run on a thread; writes are dock commands.** The log, a commit, the status,
  the stash list and one path's diff answer with model types and touch
  nothing. Commit, stash, checkout, branch, fetch, pull and push run through
  the same runner every `cargo` uses, so the exact `git` line and everything
  it says back are in the dock — a checkout refused on a dirty tree or a
  rejected push is a paragraph there, not a banner nobody can act on. The
  one quiet write is staging: `git add` on a path is instant and reversible,
  and a dock line per click would bury the commands that matter.
- **A file's diff is a page, at full height, not a strip under the
  lists.** Stacked, the diff at a fixed fraction and the commit box at its
  natural height took the space between them, and the file lists — the thing
  the view is for — were three rows tall in a panel two thousand pixels wide.
- **A diff is read once, in `rusty_git::diff`, and laid out two ways.** git
  speaks unified text and nothing else; side by side is a reading of it —
  removed and added runs paired index for index, the longer run overhanging,
  `\ No newline at end of file` on the side of the line it is about — and the
  pure reader is under tests that pin Fork's exact shape. The hunk carries
  both readings (`rows` and `lines`), because rebuilding git's order from the
  paired rows puts a note after the additions it preceded. The view only
  decides what a row looks like. The hunk header
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
  (`Target::Change`), because discard means three different things across
  the two lists. Every write in it is the same dock command a button would
  run, and it is the window's one right-click menu (`app/menu.rs`).
- **Remotes are read from the config, not inferred from their branches.**
  The refs used to be grouped `refs/remotes/` by remote name, so a remote
  nothing had been fetched from did not exist on screen — and neither did
  anywhere to add one: a repository made with `git init` could not be
  connected to GitHub from the panel at all, and Push ran `push -u origin
  main` into git's "'origin' does not appear to be a git repository".
  `repo::remotes` is one `git config -z --get-regexp` over `url` and
  `pushurl` (exit 1 is "none"; a name may hold dots, so it is what lies
  between `remote.` and the suffix), read only when the config moves — the
  stamp's `config` part — since remotes change far less than anything else
  here. The remotes are always listed, one with no branches saying so; a
  right-click fetches, changes the URL, renames, copies the URL or removes it
  (asked first). **Push with no remote opens the remote form and pushes once
  it is added** (`PromptKind::Remote { push }`), told the remote's name
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
  merge is not mistaken for a stash. And
  the log runs with `--exclude=refs/stash` before `--all`: a stash's parents
  are commits, and one `git stash` put three rows and two lanes that no
  branch owns into the graph — stashes are read in their own view.
- **A commit message is one argument however many lines it has.** A line
  split on whitespace tears a message at its first space; a git command is an
  argument vector handed to the process as it is (`CommandPlan`), and `shown`
  (`app/git.rs`) quotes each argument *for the dock's line only*. Anything
  passing user prose to a command goes that way.
- **`git diff` exits 1 when there is a difference**, which is the answer
  wanted, so `run_allowing` treats the codes a caller names as success and
  `run` is the `&[0]` case. An untracked file's diff is `--no-index` against
  `/dev/null`, a name git resolves itself on every platform, Windows included.
- **Status is porcelain v2 with `-z`**, the one format whose field layout is
  a contract: branch headers, `1`/`2`/`u`/`?` entries, a rename carrying its
  old path as the next NUL-separated token. Parsed once, in `parse::status`,
  under tests pinning the real output.
- **A letter in a file list says what it means, and git's `?` is not one.**
  The Changes list wore porcelain's own code for an untracked file, a grey
  `?`, and on a new repository that is every row — asked about in exactly
  those words. It is VS Code's green `U` now, and every letter carries its
  meaning on hover (`glyph`, `app/git_view.rs`, shared with a commit's file
  list), not only the two odd ones. **A row's stage button shows on the row under the
  pointer**, as VS Code's inline actions do, and on keyboard focus: a boxed
  `+` on every row was a column of clutter beside the one row anybody was
  looking at.
- **Branch delete is `-d`, never `-D`.** A branch whose work is merged
  nowhere is refused, and that refusal in the dock is the right answer;
  force-deleting is a decision for a terminal, not a button.
- **A push with no upstream sets one** (`-u origin <head>`), because a bare
  `git push` on a new branch refuses with a hint nobody reads. Stash is
  `push --include-untracked` — "everything I have" is what the button says.
- **The graph is laid out once, in `rusty_git`, and only drawn by the window.**
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
- **A row draws the half of every line that reaches it from above and the
  half that leaves it, clipped to itself**, so a filled row — the selection,
  the pointer — covers nothing of a line. A line drawn whole by the row it
  left, running on over the next, was painted over by the next row's fill,
  and the graph looked cut at whichever row the pointer was on. A lane a
  commit *opens* for a second parent has no line arriving from above, and the
  layout must not emit one — it did, and every merge grew a stray tail. **A
  line is Fork's shape, not a slant** (`rusty_git::log::edge_line`, pure and
  tested): straight down a lane, a quarter circle where it joins a commit on
  the lane to its left — out of a merge at the top, into the commit a branch
  grew from at the bottom — and an S where a lane only shifts, with no commit
  to turn at. Which end has a commit depends on the next row, so a row is
  drawn knowing the next row's lane; a turn is coloured as the lane it runs
  down, so a branch's curve into the commit it grew from is the branch's
  colour.
- **Lane colours are fixed hex, the board sheet's exemption applied again**:
  a commit graph is the same colours in every client that draws one, and a
  lane that changed colour with the theme would read as a different branch.
  **A branch's label is its line's colour**: a fixed colour
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
  not an error: it is an ordinary thing to open — and the one refusal the
  panel can fix, so it carries an Initialize button that runs `git init`
  through the dock. The panel knows *which* refusal it got from the error's
  kind (`rusty_git::Error::NotARepository`), never by matching the English.
- **A merge is shown against its first parent** (`-m --first-parent`), as
  Fork does — `git show` on a clean merge prints an empty combined diff,
  which reads as "this merge changed nothing".
- **Clone is the one git command that runs with no project open**, because
  it is how a project arrives (`app/clone.rs`): it runs in the dock like any
  other command, and the window opens the checkout once it succeeds. The
  directory it creates is `rusty_git::repo_name(url)` — what
  `git clone` itself would pick — and the dialog says so before running.
- **An image is to be compared as pictures, not as a patch**, and the
  Slint window does not do it yet. `rusty_git::is_image_path` decides by
  extension; which two versions to compare is the caller's knowledge — a
  commit's first parent against the commit, `HEAD` against the index for a
  staged change, the index against the disk for an unstaged one, and no old
  side at all for a file git has never seen.
- **After any write, everything is read back** (`after_git`): history,
  refs, status and stashes, a new stamp as the baseline, the project, and the
  open files nobody has edited, so the panel never shows a state git has
  already left.
- **The history follows the disk, including the part the watcher cannot
  see.** `.git/` is a dot directory and unwatched: a working-tree batch
  re-reads the status, and the stamp is asked on a timer while the panel or
  the page shows, which is what catches a commit, checkout or fetch made in a
  terminal — and only then, so a project nobody looks at the history of
  costs no `git` on a timer.

## The tree's own verbs

Move, copy, rename and delete what the tree shows go through
`rusty_edit::entries` — confined to the root like every other write, every
answer the relative path it produced. The tree is VS Code's explorer
(`app/tree.rs`, its pure half `crate::tree`): a right-click on a folder
offers New file…, New folder…, Reveal in File Explorer, Cut, Copy, Paste,
Copy path, Copy relative path, Rename… and Delete; on a file, Open to the
side in place of the three that make something; below the rows, the root's
New file, New folder and Paste. The same verbs are keys while the tree has
them — the arrows, Enter, Space, F2, Delete, Ctrl+X, Ctrl+C, Ctrl+V,
Escape — and drag and drop moves; a new file or folder in the folder
selected is also the dock's buttons and the palette's. `RUSTY_PROBE_TREE` drives all of it by the
pointer and the keys and photographs it.

- **A new entry is named where it will be**, VS Code's box: a row among its
  future siblings — a folder first in its folder, a file after the folders
  — the folder opened first (`tree::with_box`), and a rename in the entry's
  own row with the name before its extension selected. Enter makes it,
  Escape puts it away, a click elsewhere makes it when it can be made and
  puts it away when it cannot. A new file opens in the editor. A new entry
  may be a path (`src/x.rs` makes `src`), as VS Code's may; a rename is a
  name.
- **A name is refused under the box while it is typed, by the library's
  rule** (`rusty_edit::name_problem`, which `rename_entry` keeps — so the
  box and the write cannot disagree) and by the tree's own entries for a
  name that is taken, never the disk on the thread that draws. **Only
  committed text is judged**: the box judges when the input method has
  nothing composing (`preedit-text`), so `f'l'y` on the way to 飞 is never a
  name anybody typed. Enter over a refused name leaves the box saying why.
- **The box is never made again under the keys typing into it.** The rows
  are one model changed in place (`patch`: the rows before and after a
  change keep their elements), because replaced whole on every sync — the
  watcher's re-read, a tab switch — the box was a new element each time,
  and the text being typed and the keys went with the old one.
- **A click keeps the keys in the tree; a double-click or Enter gives them
  to the file**, VS Code's single click: the file opens and the arrows still
  walk the tree (`open_file_then`, which takes the keys back after the
  editor has put them in the code). The tree follows the file in front —
  selected, the folders above it opened, brought into view — once each time
  it changes, so a folder closed over it stays closed (`explorer.autoReveal`).

- **Move refuses an existing name; copy takes a free one.** An editor that
  silently replaces a file eats work, so a move or rename onto a taken name
  is `Error::Exists`; a copy pasted where its name is taken is ` copy`,
  ` copy 2`… before the extension, because pasting beside the original is
  the ordinary case. A directory into itself is `IntoItself`; a rename with
  a separator in it is `BadName` — a rename is a name, a path is a move to
  somewhere the tree did not show.
- **Delete is the recycle bin, never `remove_dir_all`** (`trash`), asked
  first. **On a thread of its own**: the recycle bin is a COM call, `trash`
  initialises COM in apartment mode, and on a pooled thread that something
  else had already initialised the other way it *panics* — `Call to
  CoInitializeEx failed. HRESULT(0x80010106)` — so whether a Delete worked
  depended on which worker it landed on. A fresh thread has no such history,
  and a panic there becomes an error. Every tab under the entry closes
  without a second question: the file is gone and a draft of it has nowhere
  to be saved.
- **Paths that move take their state with them, before the watcher hears**
  (`App::follow_move`, after a rename, a drop and a cut pasted): the tabs in
  both groups, the documents with their text, cursors, folds and history
  (`Surface::moved`, painted again where the extension changed), the drafts
  kept for them, the expanded folders, the breakpoints, the selection and
  what is cut, and the servers, which hear a moved file's old name closed
  and its new one opened. A path is under a folder by its separator, not its
  prefix — `src2/a.rs` is not under `src` (`rusty_edit::moved_path`).
- **A drop target is a pure rule, and a highlight follows it**
  (`rusty_edit::drop_target`): a file's row stands for its folder, a folder
  never accepts itself or anything below it, and the folder an entry is
  already in is not a move. The drag is the row's own: a press pulled four
  pixels carries it, and the row it is let go over is counted from the
  pressed row in rows, since the pointer is that row's until it is let go;
  past the last row is the root, lit as a frame round the list. Escape gives
  a drag up, then a cut. No question before a drop, unlike VS Code's
  default — a move is undone by dragging it back.
- **Paste goes into the folder right-clicked, the folder selected, or a
  selected file's folder** — the root below the rows; Paste is listed only
  when something was cut or copied, since a menu row cannot be greyed. A cut
  is drawn faint and used up by its paste; a copy stays to be pasted again.
- **Reveal is one function, `crate::reveal`**, the tree's and a tab's; the
  menu names the platform's file manager (`reveal::label_key`).
- **Reveal is Explorer's `/select,"<path>"` as one raw argument** — a space
  after the comma makes Explorer open the home folder, and **so does a
  forward slash in the path**. The tree and the tabs name a file relative to
  the project with `/`, and joined onto the root that is
  `E:\proj\core/src/lib.rs`: every file API takes it, and Explorer reads the
  slashes as switches. Measured with the Shell's own window list, it opened
  the Desktop with Documents selected, for every file below the project's top
  level, until the path went over with backslashes and quoted through
  `raw_arg` so a space in a folder name holds. A path a program takes and a
  command line is handed are two different contracts. `open -R` on macOS,
  and the containing folder through `xdg-open` elsewhere, where no file
  manager takes a selection portably. A tab's menu reveals, copies a path
  and opens to the side through the tree's own functions (`tree_reveal`,
  `tree_copy_path`, `open_to_side`), so the two menus cannot disagree.

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
is `notify` behind a quiet window; `app/watch.rs` decides what to do with
a batch.

- **`target/` is not watched.** One `cargo build` writes tens of thousands of
  files, and a watcher that reported them would spend the build storming the
  window. Dot directories go the same way — the tree does not draw them.
- **Batched, and structural changes are told apart from content ones.** One
  Ctrl+S in another editor is up to four notifications on Windows; a `git
  checkout` is one action that takes a second of syscalls. Re-reading one open
  file is free and walking the project is not, so a modify says "this file"
  and a create/remove/rename says "the tree".
- **An unsaved draft is never reloaded.** Silently replacing a draft with
  the disk's copy is an editor eating work, and a modal prompt per file is
  unusable after a checkout that touched a dozen: an open file nobody has
  edited takes the disk's text, and an edited one keeps its draft
  (`App::follow_changed`). **The disk is read before anything is decided**,
  against the state after the read, because typing goes on while the read
  does: a draft typed into while the disk was read wins. A draft the disk
  moved under is to be marked on its tab, which the Slint window does not do
  yet; when it does, the disk holding what the document already says, or
  what this window last wrote there — a hash noted *before* the write, since
  the debounced watcher can overtake the write's answer — is no news. Marked
  on the watcher's word alone, a save followed by more typing put ⚠ beside a
  tab only rusty had written.

## The build directory

A Rust `target/` only grows: every `cargo update` leaves the old version of
each moved dependency compiled beside the new one, every dropped dependency
leaves its artifacts, every crate's incremental cache outlives its last
change. One machine here had 140 GB under one project's `target/` and a
full disk, and the compiler's report of that was `IO failure on output
stream`. `rusty_core::disk` is the answer; the Crates page's build-directory section,
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
- **A removal re-scans; the window never sends paths.** `sweep` takes a
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
  (`disk_auto_sweep`), read after every build that succeeds (`app/build.rs`,
  `App::disk_auto_sweep`), which says what it removed when it removed
  anything. Off by default: deleting without being asked is not a
  default even when the deletion is safe.
- **rusty does not write the user's cargo config.** A shared build
  directory (`build.target-dir` in `~/.cargo/config.toml`) is the biggest
  saving of all — each dependency compiled once for every project — and the
  Crates page says so, shows the exact snippet with the absolute path, and
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
said so only if somebody found the toolchain page and worked out which of six
buttons to press first. Every piece needed to fix that already existed — the
probe, the recipes, the archive downloads — and none of it ran unless asked.

**Most of it does not have to be fetched at all now: the installer carries
it.** `scripts/bundle-tools.sh` puts rusty's QEMU, both esp-gdbs, espflash and
the LLDB adapter into `target/bundled/`, which every package ships beside the
executable, where the window finds it as it starts (`bundled_dir` in
`crates/rusty-desk/src/main.rs`) — three hundred megabytes unpacked, and the
difference between an install that is a workbench and one that is a list of
things to go and find.

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
  Environment page asks for those, which is the honest shape for a dependency the
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
  The Environment page and the first-run note read the same
  `ToolchainReport` through it, so they cannot disagree; it is pure, so the
  ordering rules are tests rather than something discovered on a laptop.
- **Order is not cosmetic.** Without `rustup` nothing else can install, so
  that case collapses to one item and a link. `espup` comes before the Xtensa
  target it provides, because `rustup target add xtensa-…` without it fails
  complaining about an unknown target. Everything blocking comes before
  anything optional, so a queue somebody interrupts halfway has fixed the
  parts that mattered.
- **The queue is sequential and stops at the first failure.** Two `cargo
  install`s at once fight over the package-cache lock, and a queue that
  carried on past a failure would end by reporting a ready machine that is
  not.
- **It says where each thing lands, before running anything.** Three homes are
  involved and only one is rusty's: `~/.cargo/bin` is cargo's (redirecting it
  with `--root` puts espflash where flashing cannot find it), rustup's home is
  rustup's, and the data directory is the one the user may move.
- **It speaks only when the machine cannot build**: a note, once, with the
  Environment page one click away (`app/environment.rs`). An optional tool
  missing is worth offering, not worth interrupting anybody for. A first-run
  check that shows up on a working machine is one people dismiss without
  reading, and then dismiss the time it mattered. The palette's Environment
  is the way in when nothing interrupted, and the page says everything a
  first-run sheet would and what is installed besides.
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
  *new* processes. rusty ran espup itself and was already running, so the
  `cargo build` it spawned next died with `linker xtensa-esp32-elf-gcc not
  found`, on a machine it had just called ready. `esp_env.rs` reads espup's export file (its own statement of what
  it installed), falls back to the layout under `RUSTUP_HOME`, and
  `process::command` appends what exists to every child's PATH and sets
  `LIBCLANG_PATH` when the user has none. The user's environment is never
  written; `tools::find` consults the same directories so the page cannot
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

- **Flash is one key and builds first** (`app/device.rs`): the build, then
  the plan against the image that build just made, then the part's flasher,
  staying attached as a monitor. It used to flash whatever had last been
  built — a board that disagrees with the code on screen for a reason
  nothing names. The command reaches the dock before it runs.
- **The device is chosen once, in the title bar** (`app/device.rs`), and
  kept. With nothing chosen, the one board plugged in is the answer
  (`only_candidate`, pure and tested): ports that look like boards when the
  chip has a serial bootloader — a C3 on native USB is a port *and* a probe,
  and counting both would ask every time about one board — and probes when
  it has none. Anything else opens the list, and the verb that opened it
  waits there (`Devices::pending`) and runs when a row is picked, rather than
  asking for the key again. A write the plan warns about is asked about
  first.
- **A monitor holding the port is let go first** (`Devices::after_stop`), as
  PlatformIO's Upload does. The flash starts from the old session's own end,
  not from the stop's answer: that is the only moment the port is certainly
  free, and the stop's answer can land after the build has begun.
- **Monitor attaches without writing, and needs no build.** The planner's
  ELF is optional for a serial monitor (a board flashed last week is exactly
  the one somebody wants to watch) and passed when it exists, for defmt and
  panics. A probe's monitor is `probe-rs attach`; it was `run`, which
  rewrote the flash of a board somebody had asked only to watch.
- **The status bar says what runs** (`rusty_embed::activity`, pure and
  tested; `app/activity.rs`): an icon for the kind of run in its state's
  colour, the crate being compiled and a clock while it runs, and then the
  verdict until the next run, whole on hover — `Build succeeded · 12.4 s ·
  Flash 85.3 KB · RAM 20.1 KB / 320 KB (6%) · 2 warnings`. The counts are
  cargo's own summary lines, kept **per unit**: a crate that fails says its
  warnings twice (`generated 1 warning`, then `…; 1 warning emitted`), and
  summing lines reported two for one — found by driving a failing build, not
  by reading cargo. A flash that goes on monitoring reports *flashed* at
  espflash's `Flashing has completed!` and carries on as a monitor of the
  same port; a run somebody stopped has no verdict; a command's success says
  nothing, since a git write's success is the panel moving. The image's size
  comes from the memory report, which a build refreshes — `target/` is not
  watched, so nothing else would.
- **The Environment page answers before it lists** (`app/environment.rs`,
  `ui/environment.slint`): a verdict with the one button that fixes it —
  *Install what is missing*, which installs exactly the rows it marked, not
  a plan with the optional tools in it — then each tool in the group of work
  it serves, its state and its Install on the right. What is needed is read
  off the report's own fields as the setup plan reads them, plus what the
  project's blocking problems name: counted from problems alone, the page
  said "one missing" above two rows marked missing. The Xtensa toolchain
  appears only when the chip needs it; "absent — not needed here" at
  headline size was the old page's most confusing line. Installs run one at a
  time, the first failure ending the queue.
- **The keys are the ones people bring**: Ctrl+Shift+B builds and F5,
  Ctrl+F5 and Shift+F5 debug, run and stop, as in VS Code — F5 on a paused
  debug session resumes it — and Ctrl+U flashes and Ctrl+Shift+M monitors,
  as in Arduino. Not PlatformIO's Ctrl+Alt letters: on a layout with AltGr
  they type characters, which is why this binding system leaves Alt letters
  alone. Every verb is also a menu row and a palette entry — the same
  `Action`.
- **The debugger's keys are VS Code's too** — F6 pauses, F10, F11 and
  Shift+F11 step over, into and out (`actions.rs`), and the transport's
  tooltips name them (`ui/debug.slint`). **A tooltip that names a key nothing
  is bound to is worse than none**: "Step over (F10)" was once written beside
  no binding at all, so a user pressed F10, got nothing, and asked whether
  Vim had eaten it. A key a tooltip names is a key in `actions::ALL`. F5
  resumes only when `DebugState::stopped` says — attached, at rest, not
  exited (`App::debug_verb`); live whenever the target was not running, the
  steps were live through the second or two gdb spends attaching.
- **The Environment page lists what the open project can use**
  (`toolchain::relevant`). An STM32 user was told "4 optional tools not
  installed" — espup, espflash, esp-generate and ldproxy — and a CMake user
  that Rust itself was missing. With a project and a catalogued part, a
  flasher appears only where the part is flashed with it, a generator only
  where it starts the part's projects, and Rust's own tools only where Rust
  is built; rustup's row says `required: false` on a PlatformIO or CMake
  project, which is what keeps `setup::plan` from collapsing to "install
  Rust first" there. With no project every row stays. pio, CMake, Ninja and
  the cross compilers sit under Build. **"Install what is missing" is shown
  only when rusty can install one of them**: a C project's tools are the
  user's to install, and the button that ran and installed none of them
  now says so and points at the rows' commands.
- **The welcome page offers the three ways to have a project** — Open,
  New project and Clone, as VS Code's does — and says a PlatformIO or CMake
  folder opens as well as a Cargo one.
- **A part's flash comes from the project's `memory.x` where the catalogue
  has none** (`memory::analyze_project`, one door for the Memory page,
  `rusty-cli size` and the assistant). An RP2040's and RP2350's flash is on the board,
  so the catalogue rightly has no number, and the Memory page said
  "capacity unknown" over a Pico. The reader sums the `FLASH` regions and
  the RP2040's `BOOT2` and refuses a length it cannot evaluate rather than
  half-reading it; every template's own script is its test.

## Output, its filter, and the terminal's key

Three things v0.7.1's user found missing beside VS Code: a run left the
last run's errors on screen, Output had no filter, and Ctrl+` did nothing.

- **Every run somebody asks for starts on an empty dock, once, with the
  last build's problems gone** (`run::starts_afresh`, `App::begin_run` in
  `app/output.rs`): Build, the simulator's Run and its restart, Flash,
  Monitor and the serial link, a test and its lens, an example's ▶ Run,
  Debug. Flash builds first and Run builds then boots, and neither clears
  between its steps — the build began the run, and its errors are what
  somebody reads when the write never comes. A command between runs (a git
  write, an install, a `cargo add`) is added below what is there, as a
  terminal's history is. **It was one `runner.clear()` per entry point,
  and the entry points disagreed.** Build alone dropped the last build's
  errors (`BuildLog`); a test, its lens, an example's ▶ Run, the
  simulator's Run and a test's Debug emptied the dock and left *The last
  build* in Problems, listing an error in code that had since been mended —
  reproduced by `RUSTY_PROBE_OUTPUT` on the window as it was before: the
  dock fresh after a test's lens, Problems still on `mismatched types` at
  `lib.rs` 34:27 — and Monitor and the serial link cleared nothing, so a
  board's lines landed under the last build's. A rule spelled once at each
  of eight doors is eight rules; it is one function now, and a door that
  continues a run (a flash's write) says so by not calling it.
- **A run refused before it starts clears nothing.** `begin_run` asks
  whether another run is going *before* it empties anything: the test
  lens's old order — clear, then run — wiped a running monitor's lines for
  a test that `Runner::run` then refused.
- **The filter decides what is drawn and keeps every line**
  (`crate::filter`, pure and tested; `Runner::view`: Slint's `FilterModel`
  over the lines and a `MapModel` over that, which cuts each line drawn
  into pieces where the box's text is, so every match is marked as a
  find's is). A line arriving while the filter is in force is judged as it
  arrives; a click on a drawn line finds the line drawn
  (`FilterModel::unfiltered_row`, `Runner::line`); Copy all copies every
  line kept, as VS Code's does under a filter; the count beside the box
  (`3 of 120`) is said only while it narrows. The text is found in any
  case, by Unicode lower case, with each folded byte remembering which
  character of the line it came from — `İ` is two characters in lower case,
  and a mark placed by folded offsets lands a byte early. Three switches
  beside the box narrow to errors, to warnings and to the rest, by
  `run::kind_of`'s colour; none on is every level, so the row is quiet
  until somebody wants it. Problems has the same box and switches
  (`filter::narrow_problems`): a problem shows when its message or place
  holds the text, or its file's name or folder does, and a file's heading
  only over what is still shown, counting that. The status bar's counts
  stay what the servers found. Ctrl+F — whatever Find is bound to — while
  Output or Problems has the keys (a click in it gives them) goes to its
  box (`App::panel_key`); Escape clears it, in the box or the panel.
- **The filter judges committed text, never an input method's
  composition** (`preedit-text` in `PanelFilter`, `ui/docks.slint`), the
  tree box's rule: filtering on `f'l'y` on the way to 飞 would empty the
  panel and fill it again at every syllable.
- **Ctrl+` is VS Code's Toggle Terminal, and the shell never hears it**
  (`terminal.toggle`, `docks::toggle_terminal`, pure and tested). From a
  closed dock or another panel the terminal comes forward with the keys;
  in front without them it takes them; with them the dock goes and the
  keys return to the code. The terminal is the one place the key is heard
  with the terminal holding the keys, so that is how the toggle knows:
  `term_key` asks the keymap first, before the shell is written to and
  whether or not one is running, and an action in `actions::SKIPS_SHELL`
  (VS Code's `commandsToSkipShell`) is the window's. The encoder sends an
  unbound Ctrl+punctuation to the shell as the character (a test holds
  that), so asked second, Ctrl+` would type a backquote — PowerShell's
  escape character — at the prompt; the probe reads the shell's screen
  after the key and finds none. From
  the code it reaches the keymap as every key the editor does not take
  does (`Shell::invoke_key`). The key is named as Slint names it, `` ` ``
  with Ctrl, which winit takes off the layout (`logical_key`), not off the
  input method — the US layout a Chinese IME runs on gives the backquote
  with Ctrl held whatever the IME's punctuation mode; a real IME was not
  driven, only the platform's events.

## The playground

Wokwi's new project, without the folder, the generator or the account: one
click from the welcome page or the project menu opens code beside a board
that runs it. `rusty_embed::playground` writes one project per chip into
`<data dir>/playground/<chip>/`; the window's half is `app/playground.rs`.

- **The templates are the proven projects, not new ones**
  (`data/templates/<name>/`, a directory and a `template.toml` each, which
  `build.rs` compiles in — the ones whose manifest gives a `playground`
  place are the list, in that order, so a new playground is a directory and
  no code; the wizard writes a part's project from the template its
  catalogue `generator` names): the C3's is
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
  templates and then opens the playground again from the disk, its drafts
  dropped with it, so no draft of the old code is left to be saved back over
  the example.
  *Keep as project…* copies everything but `target/` into an empty folder
  (refused when the folder has anything in it, or is inside the playground)
  and opens that.
- **Out of the recents list, and marked by the window, not by detection.**
  The recents are the projects somebody works on, and the playground has its
  own doors (`App::open_playground`). `EmbeddedProject.playground` names the
  chip; detection cannot know it (it is a fact about where rusty keeps its
  data), so the window sets it from `playground::chip_of` as it opens any
  folder (`project.rs`), which is why the playground's folder opened through
  Open is laid out as one too.
- **Code beside the board is every project's layout**: the board is the
  right dock's (`docks.rs`), and a run brings it to the front there
  (`app/sim.rs`) — Wokwi's layout, not a mode only the playground has.
- **What runs is what is on screen — the code and the board.** Build, Run,
  Test and Flash write every unsaved draft first (`App::write_all_then`),
  moving each document forward to the bytes written, so a key pressed during
  the round trip is not replaced by the disk's copy. The board's file is
  written four hundred milliseconds after its last change
  (`save_board_soon`), because the pin channel's polarities, the buses and
  the knobs are read off the file.
- **Run while a simulation runs restarts it**, Wokwi's loop — change the
  code, run it again. It is the flash-waits-for-the-monitor mechanism
  generalised: the run is stopped, and the new one starts from the old one's
  own end (`sim.restart`, `app/sim.rs`), the one moment the old session's end
  cannot clear the new one's running state.

## Updating itself

The running app asks the release feed a moment after launch and, when a
newer version exists, says so in a note with *Download and install*;
Settings ▸ Updates shows the release's notes, the download's progress and
*Restart now*. `rusty_embed::update` is the feed, the comparison and the
check; `rusty-desk`'s `app/updating.rs` is when to ask and how the verified
download is put in place.

- **The feed is rusty's own, and nothing before 0.7.0 reads it.**
  `rusty-update.json` on the newest release: the version, the notes, when
  it was published, and per platform the download's `url` and the text of
  its `.minisig`, an ordinary minisign signature anybody can check with
  `minisign -Vm <file> -P <PUBLIC_KEY>`. Copies before 0.7.0 poll a file of
  another name, so they find nothing to update to. 0.7.0 is
  installed by hand, beside the old one rather than over it — its
  installer does not take the old one over, so two entries named rusty in
  Settings ▸ Apps is the honest picture until the old one is uninstalled.
- **A signature names the version it was made for, and the feed's must
  match.** The feed is not signed, so its version number proves nothing
  about the file its `url` names; `rusty-release sign` puts
  `version:<v>` in the trusted comment, which the signature covers, and
  `verify` refuses a mismatch or a signature naming none.
- **The launch check fails silently; the manual one says what it found
  either way.** No network is the normal state of a bench, and a note at
  every launch teaches people to dismiss notes. Check for updates opens
  Settings ▸ Updates whatever the answer. A skipped version
  (`skipped_update` in `workbench.toml`) is quiet at launch and shown when
  asked for.
- **Download and restart are two gestures.** The download runs in the
  background — `install::download_counted`, the resuming download with byte
  counts — its progress in the status bar, and is verified before it is
  offered; the restart that installs it asks first while something runs,
  since a restart stops it, and an install cut off halfway is how a
  toolchain loses its compiler. Drafts are kept as at any exit.
- **How the verified file goes in place is the system's.** Windows: the
  installer, run as `--quiet --run --wait <pid>` — no questions, this
  process gone first, then rusty opened again. macOS: the `.app.tar.gz`
  unpacked beside the running bundle, swapped for it, opened with
  `open -n`. Linux: `$APPIMAGE` replaced. A
  `.deb` install has no `$APPIMAGE`, and the update says to get the new
  package rather than touching `/usr`.
- **The whole flow is testable without a release.** A debug build honours
  `RUSTY_UPDATE_FEED=<url>`; serve a `rusty-update.json` and an installer
  signed with `rusty-release sign --key <the key file>` over
  `python -m http.server`, and probe it with `RUSTY_PROBE_UPDATE=1`. The
  installer honours `RUSTY_SETUP_DIR`, `RUSTY_SETUP_KEY` and
  `RUSTY_SETUP_LINKS` (the install directory, the registry key under HKCU,
  the shortcuts' folder) so it can be installed and uninstalled in a
  sandbox, with `--silent` — or through its window, whose buttons are
  accessible, so UI Automation can press Install and Done as a person
  would. **`RUSTY_PROBE_UPDATE=restart` takes it the whole way**: those
  variables set, a debug `rusty.exe` packed (`rusty-release pack`) into an
  old installer and installed by it, the same packed again with one more
  file as the served version — the payload's file times are fixed, so a
  replaced file cannot be told by its time — and the probe checks,
  downloads, verifies and restarts; the installer waits for it, installs
  into the sandbox and opens rusty again with the same environment, and
  that copy writes `relaunched.txt` and leaves rather than update for ever.
  Proven 2026-10-16: the extra file in the install and in `install.txt`,
  the relaunch from the install directory, and the sandbox's uninstaller
  then taking files, shortcuts and registry key away.
- **An update closes no other window, and the restart says so.** The
  Tauri window's NSIS installer ended every rusty process; `rusty-setup`
  does not — a file another window holds is renamed aside (`*.old`) and
  swept up by the next install — so the other windows keep running the
  version they started with until they are closed. The restart's question
  counts them (`instances::others`, *More than one rusty*) whenever there
  are any, because a window still on the old version after *Restart now*
  reads as an update that did not take.

## More than one rusty

New Window (Ctrl+Shift+N, VS Code's chord, in the palette and the project
menu) opens another rusty on the welcome page, beside the one it was asked
from. `src/instances.rs` starts it and keeps the count;
`app/new_window.rs` is when to ask.

- **A second project is a second process.** One rusty holds one project —
  its language servers, its watcher, its runs, the emulator — so New Window
  starts this executable again with `--new-window`, and `main` then stays
  on the welcome page instead of reopening `recent_projects[0]`: that
  project is open in the window it was asked from. From an AppImage the
  image is started (`$APPIMAGE`), not the binary inside its mount, which
  goes when the first does. The child is reaped on a thread, gets a process
  group of its own on Unix so a Ctrl+C in the terminal that ran the first
  leaves it alone, and no console window (a debug build is a console
  program).
- **Here or in a new window, as VS Code offers it.** A recent project in
  the project menu opens here; Ctrl+Enter or the window button on its row
  opens it in a new window, and so do Ctrl+click and the hover button on
  the welcome page's recents. The Open dialog has a second row, *Open
  project in new window…*. A project is never started a second time: the
  same window says it already has it, another window's is said too.
- **The windows know of each other through files** (`instances::directory`:
  `%LOCALAPPDATA%\rusty\instances`, or beside `RUSTY_CONFIG_DIR` so a test
  counts only its own windows — never the data directory, which may be
  synced). Each instance holds an exclusive lock on `<pid>.lock` for as long
  as it lives and names the project it has open in `<pid>.project` beside
  it — not in it: a locked file is unreadable to others on Windows. A lock
  nobody holds is a window that has gone, however it went, and the next
  look removes both files; a registration is locked under a `.pending` name
  and renamed into place, so a count running at that moment cannot take it
  for a dead one.
- **Two windows on one project are refused, not raced.** Two rust-analyzers
  would take turns at one build directory, and two hot exits would keep
  drafts of the same files over each other. `open_project` asks
  `instances::holding` first and says the project is open in another
  window; a plain `rusty` started while the last project is open elsewhere
  opens on the welcome page. rusty cannot bring that other window forward —
  VS Code can, through its main process — so the note is the whole answer.
- **Every instance shares one `workbench.toml`, so every write is a
  read-modify-write of only what changed, under `config::update`'s lock**
  (`workbench.lock`, held across processes). The recents, each project's
  tab strip and each key are their own entries, and what another window
  wrote is read back where it is shown (`App::reread_workbench`): the
  recents when the project menu or the welcome page is looked at, the keys
  when Keyboard Shortcuts opens.

## Keyboard shortcuts

`actions::ALL` is the table of every action and its default key;
`actions::Keymap` is the table with what somebody rebound over it, and the
only thing that answers "which key runs this" or "what does this key run".
The page is VS Code's Keyboard Shortcuts (Ctrl+K Ctrl+S, the palette, and
Settings ▸ Keyboard); `app/shortcuts.rs` runs it and `ui/keys.slint` draws
it.

- **The binding in force is the key shown, everywhere.** The window's key
  handler, the palette, the editor's, the tabs' and the tree's menus (the
  rows that are actions — the tree's cut, copy, paste, rename and delete
  are its own keys, like the editor's), the debugger's tooltips
  (`Shell.key-of`, a pure callback handed `keys-version` so it is asked
  again when a key changes) and the welcome page's line all read the
  keymap — never `Action::chord`, which is only the default.
- **A rebinding is `workbench.toml`'s `keybinds`**: action id → chord, the
  changed keys only, so a new default in a newer rusty reaches everyone who
  has not rebound that action. Written at once through `config::update`;
  a key set back to its default removes the entry. Read, never trusted
  (`Keymap::read`): an id this window has no listed action for, a chord that
  names no key and one no shortcut may be are left out, and every spelling
  — the Tauri window's `Shift+Alt` order, `cmd+k`, `ctrl+left` — reads as
  the one a press makes (`keys::Stroke`: Ctrl, Alt, Shift, then the key).
- **Recorded as VS Code records it**: a click on a row's key, the strokes
  pressed (two at most), Enter to keep them, Escape — or the page losing
  the keys — to leave the key as it was. Searched by name, id or key; the
  keyboard button beside the search takes keys pressed instead, quoted to
  match exactly.
- **A shortcut is a key nothing types** (`actions::refusal`): a function
  key alone, or Ctrl, or Alt with a named key (Alt+ArrowLeft is Back) —
  never a bare character or Enter, and never Alt with a character, which is
  how a layout with AltGr types one (AltGr is Ctrl+Alt to the system). The
  table's own Ctrl+Alt+B for the right dock predates the rule.
- **What a key collides with is said on its row, not prevented**: another
  action on the same key, a key whose first stroke is another's whole key
  (which could then never run), and on a rebound key one the code takes
  for itself while it has the keys (`editor::takes`, the editor's own
  keymap — Ctrl+D, Ctrl+Z). Two actions on one key run the one somebody
  bound.
- **Two-stroke chords wait for the rest, wherever the first was pressed.**
  A stroke that begins a chord in force is held, said in a note VS Code's
  words ("(Ctrl+K) was pressed. Waiting for the second key of the chord…"),
  and the next stroke completes it or is said not to be a command — taken
  either way. Every key the code does not take reaches the keymap through
  `Shell::invoke_key` (*Two editor groups*, where Alt+Left once never
  reached Back), chords' first strokes
  included; while a chord is half pressed the code asks the window before
  its own keymap (`Surface::on_chord`), so `Ctrl+K Ctrl+C` is never a
  copy. A text field
  other than the code still types a bare second stroke, which is why a
  chord's second stroke follows the first's rule.

## The gutter, and one line height

The margin carries two things beside each line — a fold chevron and the
breakpoint dot (a click beside a number sets one) — and for a while carried
a third, a run arrow for a `#[test]`, which has since moved beside the item
as a lens (next section). The chevrons show while the pointer is over the
margin, VS Code's default, and always where a block is collapsed.

- **One row height, and nothing computes its own.** `Code.row-height` is the
  one property the rows, the margin, the caret and every overlay take, and
  Rust reads the same one (`get_row_height`) for a click, the hover and a
  scroll. Two layers that each rounded a fractional line height their own way
  were once a whole line apart by row eighty, invisible on any one row.
- **The text's size, the row height and the margin's width are set
  together, by Rust** (`Surface::set_scale` over `metrics::text_size`: 12.5
  pixels on rows of 19 and a margin of 64 at the size as designed, the row
  always whole pixels). Ctrl and the wheel over the code move the scale a
  tenth a notch, VS Code's `editor.mouseWheelZoom`, between half and three
  times; every character is measured again at the new size, every row
  placed again, and each view keeps the line at its top. The margin was a
  64 in the `.slint` and another 64 in Rust's sideways scroll, which a
  larger text would have overrun: it is `Code.gutter`, read by both.
- **The icons scale with the row** (`min(12px, row-height - 6px)` in
  `ui/editor.slint`). A fixed icon taller than the row once the editor is
  zoomed out pushes every number below it down.
- **Whatever the margin holds has to fit its width, or it vanishes without
  a word**: a column reserved for the run arrows and then taken by the
  chevron is how they were invisible for a while, pushed off the margin's
  edge. A new mark in the margin is measured against its width.
- Fold chevron right of the number, hard against the code, as VS Code puts
  it. Each control is its own click target, because one glyph that means two
  things depending on where you hit it is how you set a breakpoint when you
  meant to run a test — the reason the run arrow was never folded into the
  dot, and the reason it is a lens now.

## Test lenses

`▶ Run Test | Debug` beside every `#[test]` and every module holding one,
where VS Code puts it, in place of the margin's run arrow. The user's
complaint was exact: an entry point at the far edge of the margin is not
where anyone looks for it, and the margin had no room to say "Debug".

- **Beside the item, never a row of its own.** VS Code inserts a row above
  the item; a row drawn on screen that is not in the document is a line
  number and a caret that drift. So `lens_line` (`rusty_edit::lens`, pure,
  tested) puts the lens on the attribute line above the item — the row VS
  Code's lens occupies — and on the item's own line when nothing is above
  it, after everything drawn there, hints included; a lens whose line is
  inside a collapsed fold is not drawn.
- **Run is what the arrow did**: `App::run_test`, a substring filter
  with `--show-output`, rust-analyzer's own default for its lens. It was
  `--nocapture`, and every panic went to stderr the moment it happened,
  ahead of the verdicts and among every other test's lines, while
  `failures:` came out with nothing under it — reported as "it failed and
  nothing says why". Captured, each test's output and its panic are printed
  under its own `---- name stdout ----` at the end. A test run, an example
  run and a debugged test start with an empty dock, as rust-analyzer's
  runnables clear their terminal: appended, the dock was several runs'
  `FAILED`s in one scroll.
- **What a run says is read as it goes** (`rusty_edit::testrun`, pure and
  tested against a captured run; `app/testing.rs` feeds it every line the run
  prints): each test's verdict, and for each failed one where it panicked
  and what it said. The lens carries a tick or a cross from the last run — a
  module's is the worst of its tests', found by the file's module path
  (`module_of`) and never by a bare suffix, which would put another file's
  result on this lens. **A panic is a failure only where the verdict says
  so**: a `#[should_panic]` test that passed prints its panic under
  `successes:`. A run replaces the verdicts of the tests it ran and keeps
  the rest, as VS Code's do. The dock marks a `panicked at` line as an
  error (`run::kind_of`). Still to come: the failing line given the red line
  every problem gets with the panic's first line after it (VS Code's test
  message) and the whole message on its card — the reader has both. Not in
  Problems, which lists what stops the build.
- **Debug builds, asks, then runs.** `App::debug_test` runs `cargo test
  --no-run` visibly, then the same with `--message-format=json` to learn
  where each test executable landed, then asks each binary `<exe> <filter>
  --list` and starts the *one* that lists a match under gdb
  (`rusty_dbg::Target::Host`). Which binary holds a test is cargo's private
  knowledge — `src/lib.rs` tests live in the library's, `tests/x.rs` in its
  own, a `#[path]` module anywhere — so the binaries are asked rather than
  the path read. Two matches are refused by name; none is refused because
  `cargo test` with such a filter exits zero having run nothing.
- **A host program is run, not attached to.** `Target::Host` sends
  `-exec-arguments` instead of `-target-select`, is `attached` from the start
  (pushed at once, so the window places its breakpoints *before* the first
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
- **The build shows first.** A test's Debug builds the test binaries in the
  dock (`host_debug::build_plan`) and the Debug panel takes over on attach; a
  Debug panel saying "starting" over a two-minute compile looked hung.
- **A breakpoint on the first line of a multi-line `assert!(` is placed on
  its condition** (`rusty_dbg::placement`, both debuggers). The compiler
  gives that line no code but the call to the panic handler: in a Windows
  test binary's line table, line 139 (`assert!(`) had four addresses, all
  after line 142's `r.norm()` on the failing path, and line 140's condition
  came first. LLDB bound the breakpoint there, the assertion held, and the
  test ran to its end — reported as "Debug does not start". `debug_assert!`
  is the same; `assert_eq!(`, `assert_ne!(`, `println!(` and `vec![` do
  their work on their first line and stop there as clicked, each measured
  with `dap_probe`. The line clicked stays `Breakpoint::requested`, so the
  dot moves to where the session will stop: the DAP answer is paired with
  its request by `request_seq`, in order — taken as the adapter's line, a
  breakpoint moved in its first answer never moved its dot — and gdb's
  `original-location`, which quotes the line placed on, is mapped back.
  **A program that ran to its end without once stopping says so** in the
  dock (`dock.debug.ran-past`), since the panel it leaves behind is the one
  a session that never started leaves.

## A Markdown page, and a picture

A `.md` file opens as a page (`src/markdown.rs` reads, `app/markdown_view.rs`
makes the rows, `ui/markdown.slint` draws) and an image file as a picture
(`src/picture.rs` reads, `app/pictures.rs` shows, `ui/picture.slint` draws).
The book they are measured against is a flight-control textbook in Chinese:
4,504 formulas, 140 figures, raw HTML round every figure.

- **Formulas are RaTeX's** (`markdown/math.rs`): KaTeX's layout in Rust,
  drawn as an SVG of glyph outlines from KaTeX's own fonts, so nothing
  depends on the fonts of the machine showing it. Chosen by measuring four
  engines on every formula of the book (`docs/slint.md`, *Formulas*):
  latex-rust drew `\operatorname{Re}` as the symbol ℜ, set text-style
  fractions at full size and refused all ninety `\text{中文}`; typst with
  mitex is 46 MB and printed `[2pt]` into a `cases`; MathJax on boa is 72 ms
  a formula and lacks `\boldsymbol`. RaTeX drew all 4,504, and the
  characters each draws are MathJax's on every formula MathJax could draw
  but 19, which differ only in how one symbol is drawn (`⊥` for `⟂`, an
  arrow as a path rather than a glyph). The version is pinned exactly: a
  layout engine's minor release moves formulas.
- **The lessons of the old renderer are tests** (`math.rs`): `v_i^2` is a
  superscript and a subscript stacked on the `v`, never a superscript on the
  subscript — the reading `latex2mathml` once shipped; `aligned` rows are
  rows with their `=` at one x; a control space draws nothing but space
  (the old page printed `&nbsp;` there); `\operatorname{Re}` is two letters.
- **A formula is black, and the page colours it** (`colorize` with the
  text's colour), so a theme switch costs no formula. It is 1.2 ems of the
  text's size, as KaTeX sets math beside text. A formula RaTeX refuses is
  its source in a code span with the reason on the pointer — never dropped,
  never part of it. Drawn off the UI thread on every core, kept by source.
- **`StyledText` cannot hold a picture, so a paragraph with a formula is
  laid out by rusty** (`markdown/flow.rs`, pure and tested): cut into atoms
  — words, spaces, each Chinese character, formulas — measured by the
  window's own `StyledText` (`md-probe` in `app.slint`, so a line broken in
  Rust fits in Slint), broken greedily with CJK's rules (anywhere between
  ideographs, never before `，。）` nor after `（`, a formula glued to the
  punctuation beside it; a line with nowhere to break runs over rather than
  begin with a comma), and set as one non-wrapping `StyledText` per run of
  text and a picture per formula, all on the line's baseline: a tall
  formula grows its line, never overlaps the next. Too wide, it is shrunk to
  the column, never cut. Paragraphs without a formula stay one wrapping
  `StyledText`, as before.
- **The width comes back from the view.** A flowing paragraph's `Flow`
  reports its width and text size as it appears and whenever either
  changes (`Shell.md-flow-width`); the page lays out again, on the next
  turn, only the blocks whose width moved (`App::md_reflow`). A zoom
  changes the text size and so lays everything out again — nothing in Rust
  assumes a size.
- **Raw HTML is read, never run** (`markdown/html.rs`, a small tag reader):
  a `<figure>` centres its picture with the `<figcaption>`'s Markdown under
  it, an `<img>` is a picture wherever it is (a linked one too), `<b>`
  `<i>` `<code>` `<a>` `<br>` are what Markdown's words for them are, a
  `<span>` shows its text; a `<script>`, `<style>`, `<iframe>`, `<object>`
  is named and its body never shown; any other tag stays the text it is.
- **A picture a page names is read the way a picture file is** (one
  `picture::read`): resolved against the page's own folder, never above the
  project's root (`rusty_edit::absolute`), and never fetched from another
  machine — a remote one is its alt text with the reason on the pointer,
  because fetching it tells its host who opened the file. Read off the UI
  thread, kept until the watcher says the file changed.
- **An image file opened from the tree is a picture, not text**
  (`Tab::Picture`): a PNG, JPEG, GIF, BMP, WebP or ICO never reaches the
  editor or a language server; it is decoded with `image` so a failure says
  why in the decoder's words (Slint's loader says only that it failed),
  fitted to the pane and never enlarged past its own size until the wheel
  zooms it, on a checkerboard. An SVG is text as well: it opens in the
  editor with its picture over it and the page's toggle, the picture drawn
  from the document — an unsaved edit shows the moment the toggle flips
  back. To everything else a picture tab is a file (`Tab::of` decides
  which a path opens as): the tab menu copies its path, reveals it and
  splits it right, the tree follows it through a rename or a move and
  closes it with a delete. `RUSTY_PROBE_BOOK` photographs all of it.

## Languages

`rusty-i18n` is one TOML catalogue per language plus a `t!` macro. English is
the source; every other file is checked against it, so a key added without a
translation fails a test rather than reaching a screen. The window names its
words by key twice over — `t!("…")` in the Rust, `@tr("…")` in the `.slint`
files, which `build.rs` bundles as gettext from the same TOML.

- **The setting is `workbench.toml`** (`locale`; absent is "follow the
  system"), and changing it changes the window in place (`App::set_language`):
  `rusty_i18n`'s locale and Slint's bundled translation are set together
  (`apply_locale`), then everything is drawn again. **The order at start
  matters**: Slint's translations are bundled with the component, and asked
  for before the window exists the choice is lost and Slint follows the
  system — the two halves of the window then spoke two languages
  (`main.rs`).
- **Text from the libraries is translated by its key, not by its English.** A
  tool's purpose arrives as prose; the *name* beside it is the stable half,
  so the window looks up `tool.<name>` and falls back to the English when
  there is no entry (`app/environment.rs`). Refuse-rather-than-guess applied
  to translation: no entry means no claim. `rusty_i18n::translate` is the
  `Option` form that makes the fallback possible — `lookup` asserts on a
  missing key, which is right for `t!` and wrong here.
- **A `Problem` carries a `kind` and its `args` beside the English.** Prose
  with values baked into it cannot be looked up, so the stable name travels
  next to the sentence and the values travel apart from it — the window
  refills `problem.<kind>-title` / `-detail` (`app/part.rs`), and the CLI
  prints the English it always did. Each `Problem::new` names its kind as a
  *literal*, so a scan can read them off the source; a computed kind is
  unscannable and silently unchecked.
- **A scalar key and a table cannot share a name.** `[dock]` held the tab
  titles and the panels below then needed sections of their own, so the tabs
  moved to `[dock.tab]`; `menu.file` became `menu.bar.file` for the same
  reason. TOML rejects the collision, and the test that parses every
  catalogue is where it surfaces.
- **What the tests hold.** `rusty-i18n`'s: every catalogue parses, every
  language has exactly English's keys, placeholders survive translation, and
  no catalogue translates a command or a target triple.
  `catalogue.rs`'s: every key the window names — after `t!(` in the Rust and
  `@tr(` in the `.slint`, skipping the whitespace rustfmt puts after a
  macro's paren — is in the catalogue. **The scan asks whether `t!(` starts
  a word**: `format!(` ends in `t!(` too, and read as a translation every
  format string was a missing key. And every action's title translates
  (`actions.rs`). **Not held by any test today**: a `Problem::new` kind with
  no catalogue entry (it falls back to English, correctly and silently,
  which is how a gap survives a release), and a sentence in the window's
  source that never became a key — invisible to a key check, and how some
  sixty English sentences once sat in the Chinese window.
- **Group headings and templated titles are keys too.** A heading written as
  a literal beside translated rows, or a suffix added with `format!`, is a
  sentence that never became a key; such words take their values as
  arguments of a key instead.
- Not translated, deliberately: command lines, tool names, chip ids, target
  triples, and the dock's output. Users retype them, search them, and paste
  them into issues.
- **`t!` returns `String`.** A `&'static str` field or return type on the
  path to a label has to widen. That is most of the work in a new file and
  all of the compile errors.

## UI conventions

`docs/slint.md` (*The window, designed again*) is the design; these are the
rules it settled, each the answer to something that went wrong.

- **The title bar is the run loop.** On the left, rusty's mark — the
  window's own icon, never a gear, which read as a Settings button that did
  nothing — opening the window's menu as a Windows app's icon opens its own;
  which project (its menu is
  recents, open, new, clone, the playgrounds) and which branch; in the
  middle, Xcode's shape — the run button and *where* it runs, the simulator,
  a serial port, a probe or a USB bootloader, one list chosen once
  (`app/device.rs`) — and stop; on the right, the finder, the palette and the
  window's own controls. Build, Test, Debug, Flash and Monitor are keys, menu
  rows and the palette's (`actions.rs`). **What is running is not said
  there**: an activity view stood beside the button for one phase, and the
  user's verdict was that no IDE puts progress at the top centre — it is the
  status bar's, as in VS Code and JetBrains.
- **Three docks, each a set of panels**, Zed's model (`docks.rs`): left the
  project's tree, search and Git; bottom Output, Problems, the terminal,
  what a run brings — Plot, Waves, Draw, Flight, Signals, Debug, Registers
  — and Calls, the call hierarchy; right the board and the assistant, so
  the board stands beside the code for every project. Each dock opens and
  closes from the status bar and a key, keeps its size, and remembers which
  panel was in front. **A tab's right-click takes its panel off the strip**,
  VS Code's *Hide 'Output'* (`docks::hide`, kept in the dock's `hidden`):
  never the last on a strip, which would leave a dock nothing could be
  opened from, and a hidden panel is back whenever it is shown — by its
  key, by a run that brings it, or by the *Show 'Search'* rows the menu of
  any tab still on that strip lists, since a hidden panel has no tab of its
  own to be found by.
- **A run's panels are listed when they have something to say.** Output,
  Problems and Terminal always are; Plot arrives with the first sample or
  tunable, Waves with the first edge, Flight with a declared sensor, Draw
  with the first scene, Debug and Registers with a session, Calls with the
  first hierarchy asked for — which, being asked for, comes forward
  (`App::sync_docks`). **Arriving is not taking the dock**: a panel that
  switched under somebody reading Output is a workspace that moves by
  itself, so a new panel is listed and only a run's first scene brings Draw
  forward (a drawing is what the program was run for).
- **Pages are documents.** What is read or edited at length rather than
  consulted beside code opens as a tab among the files — the welcome page,
  settings, the environment, the memory report, the crates, the math
  toolbox, the Git history, a new project. There is no rail: a panel is
  reached by its dock's tab or its key, a page by the palette or a link.
- **The status bar says the state, and each item is a door**: the docks,
  problems by count, what is running — an icon for the kind of run in its
  state's colour, its clock while it runs, the whole verdict on hover — the
  language server, the caret, the encoding, the line endings, the language
  and the chip. **Icons first, the words in the tooltip**, the user's rule
  for all chrome: text appears in a control only where it carries a value —
  a count, a clock, a position, a zoom. **A server's own words never reach
  the bar**: rust-analyzer's progress is a crate name per piece of work, and
  for `Roots Scanned` a whole path — `Roots Scanned 92% 13/14:
  C:\Users\…\lib/rustlib/src/rust/library\std` — which pushed the bar's
  other items off the edge; it is the server item's tooltip.
- **Messages are notifications**, a stack in the corner with their actions
  on them and a history behind a bell in the status bar (`App::note`,
  `note_once`, `note_transient`) — not a banner over the work, and never a
  row that pushes the workspace down: a banner that arrived as a row moved
  everything under it forty pixels, and a click already in flight landed on
  whatever had moved under the pointer.
- **A refusal is said where the click landed.** Test on a root that is its
  own firmware (`EmbeddedProject::root_is_firmware`) builds its tests for the
  chip, which has no harness, so it is refused — in the dock and a
  notification naming why (`app/testing.rs`), not by a disabled control: disabled with the reason only in a tooltip, it
  was refused in silence, and the previous build's output still in the dock
  read as "Test ran a build".
- **The keyboard reaches everything.** The palette lists every action with
  its key; the keys are VS Code's wherever a VS Code key exists (Ctrl+P,
  Ctrl+Shift+P, Ctrl+B, Ctrl+`, Ctrl+\, F12, Ctrl+., Ctrl+Shift+N, Ctrl+K Ctrl+S),
  Arduino's for the board (Ctrl+U flashes, Ctrl+Shift+M monitors), and
  never an Alt letter, which a layout with AltGr types characters with.
  Every one can be rebound (*Keyboard shortcuts*), and what shows a key
  shows the one in force.
- **Every surface answers a right-click with its own menu or not at all**
  (`app/menu.rs`, one menu for the window, filled by whichever surface was
  clicked). VS Code's for each (`app/context.rs`, which keeps what was under
  the pointer until a row is picked): Output copies the line, copies all and
  clears; a problem goes to itself, copies itself as VS Code's JSON of it
  and copies its message, and a file's heading opens and copies its path;
  the terminal — whose cells a drag selects, Ctrl+Shift+C copying them, and
  Ctrl+C too while something is selected, as VS Code's does on Windows —
  copies its selection, pastes, selects all, clears (the screen
  and its history, then the form feed that is the shell's own Ctrl+L, so
  ConPTY's screen and the one drawn agree), kills and restarts; a search
  hit opens, copies its line and its path; a Markdown page shows its source
  and copies its path; a picture copies its path and reveals it — and an
  SVG's shows its source; a dock's tab hides its panel. A row that would do
  nothing is not there — the page's text cannot be selected, so its menu has
  no Copy. **A path is copied as the tree copies one** (`tree_copy_path`):
  VS Code's *Copy path*, absolute, and *Copy relative path*, both with the
  system's separator. The Git menus' *Copy path* is still Fork's, relative,
  beside *Copy full path* — one name meaning two things, left for the
  owner to settle.
- **Ctrl and the wheel size the text under the pointer; Ctrl+= and Ctrl+-
  size the window**, VS Code's split. Over the code it is the editor's text,
  over a Markdown page the page's own scale (prose and a listing are read at
  different sizes; its formulas with it, inline ones laid out again for the
  new size and drawn from their SVG at it, and its figures at their own
  size), over the terminal its font — each a tenth a notch
  between half and three times (`zoom::stepped`; a touchpad's small turns
  summed into notches, `zoom::wheel_steps`), each in `workbench.toml`'s
  `[layout]`, each put back by a palette action. The wheel alone still
  scrolls. Over a picture the wheel is the picture's own zoom, with Ctrl
  or without, and the page's scale stays where it was.
- **Lists of things the window has are generated from the thing.** The
  palette lists `actions::ALL` and the docks list `Dock::panels`; a list
  spelled out by hand beside them is a list that drifts, and some of the old
  window's panels were reachable from nowhere but a click on a strip.
- **A thing has one name wherever it is named.** The Environment page was
  "Environment" in one place and "Re-scan toolchain" in another; Memory was
  "内存占用" in one and "内存报告" in the next. And a Chinese name is the word a
  Chinese user would look for: the Flight tab is 飞控 (it was 飞行,
  "flying"), Draw is 图形 beside Plot's 曲线, the chip catalogue is 芯片库
  (目录 reads as a folder).
- **Settings are macOS System Settings' shapes, and prose is one line under
  the group** (`ui/pages.slint`: a `Group` is a rounded box of `Setting`
  rows divided by hairlines, a `Setting` its label on the left and its control
  on the right; `Segments` for a handful of exclusive choices, `Field` for a
  value a machine reads). A paragraph under every field and a summary under
  every sidebar entry was the first version, and the user's verdict was
  exact: cluttered, and written to be admired rather than read. Labels are one
  to three words; a footer is one sentence stating a fact ("Changing this
  restarts the terminal"), never a rationale; the rationale lives in the code
  comment. Same for the assistant: an empty transcript is one line and the
  composer.
- **The open file goes with a question**, as VS Code sends the active editor:
  a chip above the composer names it, its × drops it for that question, and
  it travels as `Content::Attachment { path, text, lines }` — its own block,
  so the transcript draws the chip and only the providers render it as prose
  (`Content::prose`, the one place the framing is spelled). The text is the
  document on screen, unsaved edits included: whole when it is short, and the
  lines around the selection or the cursor when it is not (`Content::attach`,
  and *A question costs every round of it* above).
- **The board sheet is dark in both themes, on purpose.** The sheet, the
  devkit and the parts are drawn in fixed colours (`#101216` and friends,
  `rusty_embed::board::art`, `ui/board.slint`) rather than the theme's, the
  way a schematic sheet is the same colour in every editor; the panel around
  it follows the theme. It is the one surface exempt from the theme, and
  this sentence is what makes that a decision rather than an omission.
- **Every colour has both themes in one place** (`ui/theme.slint`: each
  property `dark ? … : …`), so a colour cannot exist in one theme and leak
  into the other — the failure that made code unreadable twice, light ink on
  a dark ground and pastels on white, when each theme was a block of its
  own.

## Where state lives

One rule decides: **if the CLI, the assistant or another window could ever
care, it is a file; high-volume queryable data picks its own format when the
feature that needs it lands.** The window keeps nothing anywhere else: there
is no browser storage, and what the window remembers between runs is
`workbench.toml`.

- The data directory (`config::data_dir()`) holds `boards/` and
  `workbench.toml` — plain TOML, user-readable, checked by tests. Its location
  is configurable: a fixed anchor (`%APPDATA%\rusty`) holds `location.toml`
  pointing at the real directory. Relocation copies and switches the pointer;
  the originals stay until the user deletes them. Pointing it at a synced
  folder is the cloud-sync story.
- `workbench.toml` (`config::WorkbenchState`) is the recents, the window's
  layout and theme and scale (`WindowLayout`: each dock's size, whether it is
  open, which panel was in front and which were taken off its strip, where
  the editor groups part, and the editor's, the page's and the terminal's
  text scales), the editor's switches (`vim`,
  `EditorView`), the language, the assistant's profile, each project's tab
  strip (`open_tabs`, keyed by the directory as the filesystem sees it, so
  another spelling of the same directory finds it), the rebound keys
  (`keybinds`), a skipped update and the auto-sweep. Every window shares
  it (*More than one rusty*). Drafts are files of their own, under the
  data directory's `backups/` (`editor/backup.rs`).
- Secrets stay in the OS credential store, never in the data directory — a
  synced directory must never sync a key.
- Per-project, team-shared things live in the project's `.rusty/`, where they
  are diffed and reviewed: board overlays, the simulated board (`sim.toml`,
  which is what the sheet editor writes), user-defined parts (`parts/`, one
  TOML per part — see *A part is a declaration* above) and the math sheet
  (`math.toml`).
- **A window's state is grouped by concern, not laid end to end**: the
  `App`'s state is a struct per concern (`sim`, `git`, `debug`, `lab`,
  `math` …), and a new piece goes in the group it belongs to, or a new group
  is added. A flat struct of a hundred fields carrying their group's name as
  a prefix is a struct nobody can read and a boundary nothing enforces.

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
  A second test of the same parts had the same blind spot, setting `rot` and
  leaving `flip` alone. A fixture now sets every optional field to something
  that is not its default.
- **Geometry and protocol get tests; views get driven.** The board canvas got
  its arithmetic wrong three times while none of it was reachable from a test.
  The pure halves now live apart from any view: `rusty_embed::board::geometry`
  (pin points on the grid, the devkit's generated symbol against its row
  points, turned and mirrored parts, wire paths, hit-testing) and
  `board::edit` (what placing, wiring, renaming, rotating, deleting,
  duplicating and undoing *do* to the parts and the wires). What genuinely
  needs the window — a drag, a click, a key — is driven by its probe
  (`RUSTY_PROBE`, `src/probe.rs`) and asserted on what it reads back and
  photographs.
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
  and a moved file's old name close now (`Servers::closed`), the client drops
  the file's pulled analysis with it, and the puller skips a file closed
  while its pull was on the way.
- **What the client records and what it writes go in one order.** Requests
  once ran each on a thread of its own, and `did_open`, `did_change` and
  `did_close` each recorded the document and then wrote to the server with
  nothing held between. Two changes could swap on the wire — each is a delta
  against the text before it, so the server's copy was wrong from then on
  and said nothing — a close could land after the open that followed it,
  and a question asked after a change could be written ahead of it and
  answered about the text before. Each now records and writes in one turn
  (`Shared::order`), and a question whose answer is positions
  (`Shared::ask_about`: the text read and the request written in the same
  turn, the answer waited for outside it — `transport::ask` and `answer` are
  `request` in two halves) knows which text it was answered about. **A lock
  of its own, not the documents'**: the thread reading the server takes the
  documents' lock for every pushed diagnostic, and a write to a server whose
  pipe is full waits on that thread — the documents' lock held across the
  write would be two pipes waiting on each other. **The two tests that hold
  it fail five runs in five with the old ordering put back**: four threads
  changing one file and the server's copy rebuilt from what it was sent, and
  a file changed as fast as a thread can while another asks about it. A race
  that needs two threads is not rare when every keystroke's completion, the
  colours and hints and a hover each send a change of their own.
  **And a question carries its text** (`rusty_lsp::Draft`), because the lock
  orders what reaches the client and nothing orders the threads that reach
  it: a change and then a question are two pieces of work, and either could
  start first — as could two changes, the older written over the newer. The
  window's open, change, close and save go through one thread in the order
  they happened (`lsp.rs`), and a question gives the server its draft first,
  in its own thread (`sync_draft`). A draft is numbered (`Servers::draft`),
  and the client skips one older than the number the document holds —
  `drafts_sent_at_once_leave_the_server_with_the_newest` fails five runs in
  five without that line. Completion is one round trip per keystroke where
  it was two.
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
  — and the client sent each to the window as it arrived, so an empty pull
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
- **A flattened `"cargo.buildScripts.enable"` key beside a `"cargo"` object is
  silently ignored** in rust-analyzer's initializationOptions. The first
  attempt at the fix above failed while looking applied, because the sibling
  `procMacro` object *did* take effect. Nest keys in their object.
- **Nothing goes on the critical path of a project switch that the new
  project's first paint does not need.** Until a project is open the window
  is still showing the last one, so every millisecond spent opening it is a
  millisecond of a workbench that looks frozen. Two things were in there
  that nothing on screen reads:
  - **The Cargo analysis.** `Workspace::load` resolves the whole dependency
    graph: measured at 113 ms on a small embedded project, 812 ms on this
    workspace, and unbounded on one whose lockfile does not exist yet —
    which is exactly the project somebody has just generated, and exactly
    the case reported. Only the Crates page reads it, and it is loaded the
    first time that page shows, or by the first assistant tool that needs it
    (`LazyWorkspace`), and kept (`App::crates_keep_workspace`).
  - **The outgoing language server's funeral.** `LspClient::drop` asks
    rust-analyzer to shut down and waits for the process: measured at 540 ms
    against this workspace — the server answers `shutdown` in a moment and
    then outlives `EXIT_GRACE`, so the poll runs out and kills it.
    `Servers::install` drops the outgoing clients on a thread of their own;
    nothing waits on a corpse.

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
  showing". `tab_hints` (`app/strip.rs`, pure and tested) is VS Code's rule:
  the name alone while it is unique, otherwise the shortest suffix of the
  directories above it that separates the whole group, grown one segment at
  a time. `core/src/lib.rs` against `firmware/src/lib.rs` needs two; a `bin`
  against a `src` needs one. A file at the project root keeps a bare name
  even when it shares one — having nothing above it *is* the distinguishing
  mark, and inventing a word for it would be a label the path does not
  contain.
- **A remembered tab is not a file the user asked for.** The strip comes
  back from `workbench.toml`, keyed on the project *directory* — and a
  directory can hold a different project than it did last week, which is
  exactly what the wizard does when somebody generates over a path they used
  before. Reopened through the loud door, a freshly generated project greeted
  its author with a red *could not read firmware/src/bin/main.rs* about a
  file nobody had asked for, and the first click on each of the other tabs
  did the same. The strip is read in one pass with the drafts and the text
  of every file on it (`app/strip.rs`), and a file gone or no longer text is
  left out: gone means off the strip, quietly.
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
  normal project, so nothing about a normal build changed. `App::root()`
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
  two disagreed. Hints stay in the editor.
- **A problem at a point is no columns wide, and it was drawn over none.**
  rust-analyzer's `expected SEMICOLON` and rustc's ``expected `;` `` are
  both ranges of no width at the end of the line the `;` is missing from.
  The editor marked the characters a range covers and widened a point to the
  one after it — which, at the end of a line, does not exist. So Problems
  listed two errors on line 16 of a file the editor drew clean, the only
  mark on screen being rustc's `unexpected token` hint dotted under the
  next line's `pub`; and the hover matched a problem by its own half-open
  range, which a point never satisfies, so it could not be asked about
  either. `rusty_lsp::squiggle::drawn_on` is now the one reading of where a
  squiggle stands, VS Code's (`_createDecorationRange`): a point past the
  last thing written on its line is a cell after everything on the line —
  after its hints too, so nothing the overlays measure moves — and anywhere
  else it widens to the word it touches. The editor draws it there, the hover
  finds it there, and the card's range is the drawn span, since "moved
  away" measured against a range of no width is moved away already.
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
  switching shells — and each alone was enough: a reopen started while the
  close was still on its way, and the close then killed the new session;
  and a finished session's clean-up, whose contract was "kill whatever this
  replaces", killed its own successor. The window kills the old shell before
  it starts the next and counts shells (`Term::generation`), so an update or
  an exit from one already replaced is dropped.
- **A GUI-subsystem executable cannot be a pseudoconsole's program.** A
  release build of the window is one on Windows (`windows_subsystem`, so no
  console window opens behind it), and the built-in shell was once that same
  executable re-entered with a flag. portable-pty starts every child with
  `STARTF_USESTDHANDLES` and invalid handles; a console program's startup
  swaps them for the pseudoconsole's, a window program's does not, and
  `CONIN$`, `CONOUT$` and `AllocConsole` do not reach it either — each
  measured with a window-subsystem probe in a pty. So the installed app's
  terminal read end-of-input at once and said "The shell exited with status
  0." over an empty screen, every time, while the debug build — a console
  program — worked every time, which is how it read as "sometimes". The
  built-in shell is `rusty-shell` now, a console program that is nothing but
  the shell (`rusty-term`'s bin, built by the release workflow into
  `bundled/`); the window's terminal starts the system shell
  (`rusty_term::default_shell`), and any shell it is ever given a choice of
  must be a console program. `cargo run -p rusty-term --example pty_probe`
  is the check, and it can be pointed at any executable.
- **On Windows a pty read never reports end-of-file.** The master keeps the
  pseudoconsole open however dead the child is, so exit has to be detected by
  polling `Child::try_wait` on its own thread. Inferring it from the reader
  works on Unix and hangs here.
- **An internally-tagged enum cannot have a newtype variant wrapping a string.**
  `#[serde(tag = "type")] enum Content { Text(String) }` compiles, and then
  fails at *runtime* with "cannot serialize tagged newtype variant" — there is
  nowhere inside a bare string to put the discriminant. For `Content` that
  meant every assistant answer failing where it was serialised, nowhere near
  the declaration. Use a struct variant (`Text { text: String }`). Any type that
  crosses the wire deserves a round-trip test; `rusty-ai/src/model.rs` has one.
- **`RUSTUP_TOOLCHAIN` leaks into every cargo rusty spawns.** rustup's proxy
  sets it for whatever toolchain binary it runs — rusty started with `cargo
  run` included — and rustup lets it outrank the project's
  rust-toolchain.toml, so a spawned `cargo build` compiles an esp-pinned
  Xtensa project with stable and dies with "can't find crate for `core`".
  `process::spawn` strips the variable.
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
- **A flag cleared from a shared error path belongs to nobody.** Every
  command once shared one failure path, and it cleared "a session is
  running" — so one unrelated error told a *running* simulation it had
  ended, the Stop button vanished, and QEMU kept going with nothing in the
  window able to reach it. Only the run that started a session may say it
  ended: its end arrives through the callback it was started with
  (`App::run_session`). The same reasoning says a debug run's Stop must stop
  what the Debug key started, since that is what booted QEMU.
- **A debugger reading a different build than the one running answers every
  question, fluently, about the wrong binary.** Debug runs build unoptimised
  (`--config profile.dev.opt-level=0`) into `target/<triple>/debug/`, and the
  debugger was once handed the ELF from a plan made for an ordinary run, so
  gdb read the *release* ELF while QEMU booted the unoptimised image.
  Symptoms, none of which point at the cause: the breakpoint is reported six
  lines below where it was set (the release line table), it never hits (the
  address means nothing in the running image), and the Debug panel sits on
  "Running" for ever. Only the run that built and booted the image may say
  what the debugger reads: the boot step hands its own ELF to
  `App::debug_attach` (`app/sim.rs`). When something has one right answer and
  two computations of it, delete a computation — a test can only catch the
  drift after someone reintroduces it.
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
  succeeds, the board never hears it. That is why `serial::open` exists
  (`App::link`) and why the Plot panel's tunables are writable only while
  what runs takes input (`Runner::takes_input`) rather than whenever a
  session runs: a slider that silently does nothing reads as firmware
  ignoring the change. The trade is explicit — rusty's own link is plain
  text, and defmt decoding stays espflash's.
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
- **A port `bind(0)` gave and was let go of is the likeliest source port
  for the next connection on the machine.** `free_port` used to be that,
  and two failures came of it, both found by running rusty-mcu's launch
  tests in eight processes at once beside the headless test whose channel
  dials a port nobody listens on — the one flake that had reached CI, made
  to happen about once in a thousand runs. First, a connection tried before
  the emulator listened was given the port it dialled as its own source and
  met itself (TCP's simultaneous open): `connect` succeeded, and the read
  timed out ten seconds later with both ends `127.0.0.1:61652`. Then, with
  that refused, the emulator's own bind failed (`os error 10048`), and
  `netstat` at that moment showed the port as the source of another run's
  connection to *its* emulator, three ports along. `free_port` walks
  20000–32000 now, below every system's ephemeral range, where neither can
  happen; `connect_local` still refuses a socket joined to itself, since a
  machine's range can be moved; the in-process emulator listens before
  `launch` returns; and the headless test's unconnected channel dials port
  0. Twelve thousand launches after, none failed. **A flake is a race with
  a witness somewhere: make the failure print both ends of the socket and
  who holds the port**, and two rounds of it named what an afternoon of
  reasoning would have guessed at.
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
- Child processes get `CREATE_NO_WINDOW` on Windows; the Environment page
  probes six tools on open and would otherwise flash six console windows.
- **Mirror, do not rotate, to face a part at the chip.** Rotating 180° does
  bring a part's pins to the near edge — and reverses their order, so seven
  wires to a seven-segment cross on the way in. `mirror` mirrors: near edge,
  same order. Neither transform touches the writing: pin names, numbers,
  the reference and the value are placed in sheet coordinates *after* the
  turn (`pin_labels`), so nothing is ever drawn mirrored.
- **A part is drawn once, and moving it draws nothing again.**
  `rusty_embed::board::picture` draws each part alone in its own frame, an
  image the window places where the part is (`app/board.rs`); what changes
  while a run goes on — a lamp's glow, a pressed cap, a screen — is the
  window's own, drawn over the pictures. The models those live layers read
  are made once and changed row by row: a model replaced under a held cap
  took the cap's press area with it, and the release went nowhere.
- **Both ends of a wire are a place to start it.** Any pin drags to any
  other — a part's to the devkit's, the devkit's to a part's, part to part
  — through one `edit::connect`, because two gestures that agreed about what
  wiring means only in prose would drift. `pin_under` works in sheet units
  so the reach does not shrink with the zoom, and answers nothing when
  nothing is in reach — a wire that landed on a pin forty pixels from the
  pointer would be a connection nobody made. A pin to itself and a pair
  already joined are refused as wires that mean nothing.
- **And a wire is drawn click by click, the way every schematic editor
  draws one.** A press on a pin that travels less than `CLICK_SLOP` starts a
  `Drawing`; each click on the sheet fixes a corner; a click on a pin
  finishes it, and on a wire makes the T; the route follows the pointer the
  whole way rather than appearing once the wire exists. Reported by the user
  as the one way wiring works everywhere else, and it was: pressing and
  holding across the sheet was the only gesture there was. Space turns the
  live leg the other way round (KiCad's posture), Backspace takes back one
  click and then the drawing, Escape abandons it, and a drag let go on bare
  sheet becomes the first corner rather than a wire dropped on the floor. **A
  `Drawing` is not a drag, because it outlives every press.** **What is
  drawn is what is made**: the corners are stored already square, stub out
  of the pin included, so `orthogonalize` has nothing to add between two of
  them and the wire draws the preview's polyline point for point
  (`edit::connect_drawn`, and a test that holds the two equal).
- **Wire bends belong to the sheet, not to the part — KiCad semantics.**
  Dragging a part stretches only the leg from its pin to the nearest bend,
  at whichever end of the wire the part is (`wire_legs`, `follow_bend`);
  every bend the user placed stays put, and the orthogonal pass grows the
  elbow the stretched leg needs. (An earlier fix translated bends with the part;
  that read a rendering artefact as a semantics bug and inverted the
  behaviour every schematic editor has taught.)
- **A tab is not a character wide, and everything measures one the way the
  text draws it.** A tab goes to the next stop of four columns
  (`editor/metrics.rs`, `TAB_COLUMNS`), and the same rule places the text,
  the caret, a click, a squiggle and every popup — counted from past a hint
  when a hint stands before it (`editor/inlays.rs`). Measured one character
  wide, a find match, the completion popup, the Vim cursor and the hover's
  hit-testing once all sat up to three columns left of the text on a
  tab-indented line.
- **A server hears only about the files its language is.** A didOpen of
  `.git/info/exclude` to rust-analyzer produced a syntax error per line —
  sixty-eight problems from a file that was never code. Every open, change,
  save and close goes to the server the language table names for the path
  (`language::of`, `Servers::for_path`), and to none when it names none —
  never on the strength of "it is open in the editor".
- A future built from `&SomeStruct { .. }` inline borrows a temporary that dies
  at the end of the statement. Bind the struct, then move it into an `async`
  block.
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
  boundary.** A splice that computed the common prefix of an old and a new
  text byte by byte and then sliced there met two CJK characters that share
  their first two bytes — 中 and 世, the ordinary case in one block — put the
  slice inside a character and panicked the window. `rusty_lsp::positions`
  has the boundary walk, and the editor's own diffing of a changed text
  (`editor/document.rs`) backs up the same way.
- **A thread that owns the `Arc` it waits on lives for ever.** rust-analyzer's
  pull loop held an `Arc<Shared>` whose `poke` field held the sender the
  loop was blocked receiving on; dropping the client killed the server but
  the loop, the `Shared` and every open document's text stayed. The loop
  holds a `Weak` now and the client sends `shutdown`/`exit` before `kill`.
- **rustup answers for the directory it is run from.** `rustup target list
  --installed` in rusty's own cwd reported the default toolchain's targets
  for a project pinned to `nightly-…`, so the toolchain page said "target
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
- **A long-running session owns a slot, and ends because the slot was
  replaced.** A loop that stopped only when its consumer said it had gone ran
  until the window closed — one more file watcher per project switch, an
  assistant question that could not be cancelled — because the consumer
  never said. A watcher, a terminal, a debug session, a run and a question
  each own a slot in the window's state, and whatever arrives from one
  already replaced is dropped by its generation or its ticket (`Watching` in
  `app/watch.rs`, `Term::generation`, the assistant's `ticket`).
- **`workbench.toml` is written through `config::update`, and only that.**
  Every writer is a read-modify-write of the whole file, and two of them —
  the tab strip on every tab switch, the recents on every open — interleaved
  and lost one. `update` holds a lock across the read and the write, in the
  process and across processes (`workbench.lock` beside the file,
  `File::lock` — why `rust-version` is 1.89), because every rusty process
  shares the one file; the temporary file carries the process id, because
  two processes sharing one `workbench.toml.tmp` produced a file that was
  neither's. The lock file is never written, so a data directory in a synced
  folder sees it once. The file has its own private structs (`config::file`),
  so the model's types can be renamed without dropping a key from
  everybody's file.
- **A child's stderr is read or it is `null()`; it is never `piped()` and
  forgotten.** gdb's was piped and unread, and a gdb with a Python warning
  per startup filled the pipe and blocked on it with every MI answer still
  to come. And gdb writes `exit-code` and every non-ASCII byte in *octal* —
  `"012"` is ten, `\346\227\245` is 日 — so both are decoded as octal, into
  bytes, not chars.
- **The pty's exit poll wakes the window once, on exit.** It used to send a
  wake every 250 ms as a way to notice the consumer had gone, and every wake
  was drawn — four unchanged frames a second from an idle terminal.
  Liveness is read off `Arc::strong_count` instead. And a DSR query is
  answered *after* the bytes before it in the same write have reached the
  emulator, or a program that prints and asks gets the cursor from before
  its own output.
- **A reader loop that peeks and does not consume must consume *something*
  on every path, or it is a freeze waiting for the right file.** The
  Markdown block reader broke on an `End` it was not waiting for — the
  closer of an HTML block, a construct it did not list — and looped there
  for ever; it ran on the thread that draws the window, so opening a chapter
  with a `<figure>` froze the whole window with no error anywhere. Two
  defences now: every construct `pulldown-cmark` can emit for the enabled
  options is handled, *and* the loop eats an event it made no progress on.
  The second is the one that holds when the first drifts.
  `RUSTY_MD_CORPUS=<dir>` runs the reader and syntect over a real book under
  a time bound; without it the test skips and says so.
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
  is still indexing, which is why the server item's tooltip shows its
  progress (`workDoneProgress` on, `$/progress` folded into
  `LspEvent::Progress`) instead of "rust-analyzer" in green over a minute of
  empty replies. gdb reads
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
- **The build follows the chip; the tests follow the user.** Builds run in
  `firmware_root`, which for the standard layout is the *excluded*
  bare-metal crate — and `cargo test` there fails with "can't find crate for
  `test`", because a `no_std` target has no test harness, which is the whole
  reason that crate is excluded. Tests run at the opened project, where the
  testable members are (`app/testing.rs`): the Run Test lens, Test and a
  test's Debug alike — and detection says which layout it found as a value,
  `firmware_dir`, beside the prose in `chip_source`, so Test is refused on a
  root that is its own firmware without matching English.
- **CI's clippy is today's stable; the machine's is whenever `rustup update`
  last ran.** v0.3.1 was tagged with clippy green here on 1.97 and failed on
  the runners' 1.98 — `chunks_exact(5)` where `as_chunks::<5>()` now exists,
  and three imports that `use super::*` already supplied. Neither is wrong code; both are `-D warnings`. Before a tag, run
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
  appends to it.** The bundle's directory once came back canonicalised, and
  QEMU joins `-L <dir>` to `esp32c3-rom.bin` with a forward slash, which the
  verbatim prefix forbids: `ROM code binary not found`, for a file exactly
  where the bundle put it, on the first run after the bundle shipped.
  `tools::plain` strips the prefix at `set_bundled_dir`, so every path
  derived from the bundle is plain. Anything else that turns a canonicalised
  path into a command-line argument wants the same.
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
  installation. The Environment page's probe follows the same rule now
  (`component_binary`): it did not, and the first-run check declared a
  machine ready above a status bar saying rust-analyzer was missing — one side had
  asked `--version`, the other had only found a file.
- **`Path::join` inserts the host's separator, so a Windows path built with
  it is Windows-only.** A list of Windows shells built with it and tested on
  the Linux runner came out `C:\Program Files/Git\usr\…`, failing the test
  that asserts the spelling a Windows user sees: a path that must read as
  Windows is spelled as text, and `Path::ends_with` compares components, so
  a test matching a Windows suffix on Linux compares the string.
  `Path::strip_prefix` is the same trap from the other side: `rusty-dbg`'s
  tests feed records from real Windows gdb sessions, and on Linux the
  fullname was one component no root could be a prefix of, so `relative`
  relativises textually after bringing both sides to `/`. And
  `cfg!(windows)` is the wrong question for whether a drive letter's case
  folds: a path with a drive letter is a Windows path whatever host reads
  it, so `rusty-lsp`'s `same_path_text` folds the letter everywhere and the
  rest only on Windows. Three crates, one class of bug, found one per push
  because `cargo test` stops at the first failing binary — the CI passes
  `--no-fail-fast` now so a run names them all.
- **A field nobody reads is a reply that never arrives.** Reasoning models
  in the OpenAI dialect stream their thinking as `reasoning_content` (or
  `reasoning`) beside an empty `content`, and can spend the whole
  `max_tokens` there: the user saw a question with nothing under it and
  `4096 out` on the meter, because the field was unread, the stream ended on
  `length`, and the loop pushed no message for a turn with no text. Thinking
  is `ChatEvent::ThinkingDelta` and `Content::Thinking` now — shown folded,
  kept in the history, skipped by both providers on the way back (DeepSeek
  rejects its own reasoning as input; Anthropic would want it signed) — and
  `StopReason::MaxTokens` sets the question's `cut_short`
  (`app/assistant.rs`), which the assistant says under the answer with the
  number, because the returned history cannot carry a
  fact about text that was never produced.
- **One output budget for every provider is possible only because a
  provider that caps lower says so.** `DEFAULT_MAX_TOKENS` is 200 000, far
  above any model's cap, so a reasoning model is never cut off by a default.
  Anthropic, OpenAI and DeepSeek all refuse a larger `max_tokens` with a 400
  that names the cap (`> 64000, which is the maximum allowed number of
  output tokens`, `supports at most 16384 completion tokens`, `the valid
  range of max_tokens is [1, 8192]`); `output_cap_in` reads the largest
  number below the ask out of a refusal about output, the loop asks again at
  that number, and the window keeps it for the session (`app/assistant.rs`)
  so the refusal happens once per provider, not once per question. It reads only
  the provider's words — never this crate's `answered 400` framing, whose
  status code would read as a cap of 400 — and nothing under 256. The
  setting itself is left alone: what the user chose and what the provider
  takes are two facts, and the cut-off note uses the count the provider
  reported rather than either.
- **Two pieces of work started together answer in either order.** Settings
  stored a key and asked whether it was stored, back to back; the check
  overtook the write, "not saved" arrived and stayed, and the next request
  used the key perfectly well — a flag contradicting the store it describes.
  A read that depends on a write goes in the write's answer, never beside it:
  wherever two `work::spawn`s are made in a row, the second may land first.
- **State cleared and later compared against compares against nothing.**
  The update check set the last answer to `None` so the page would show the
  new one, and its completion then read that same state to decide whether
  the verified download was for the same version — it never was, and a
  manual re-check put a downloaded update back to "Download and install".
  Read what a decision needs into a local *before* the reset, and hand the
  local to the closure.
- **In Slint, `row` and `col` are every element's properties** — a
  grid's placement — so a component cannot declare `in property <T> row`:
  the build stops at `Cannot override property 'row'`, which reads like a
  name declared twice. It happened twice in one day (the Environment page's
  tool row, the wizard's choice); name the property after what it holds.
- **Slint's OpenGL renderer draws text half a pixel off sharply, and the
  contents of a rounded clip half a pixel off blurred — so rusty carries a
  patched renderer** (`vendor/i-slint-renderer-femtovg`, through
  `[patch.crates-io]`). Reported as "scrolling looks like behind glass, and
  sharp once it stops". A wheel notch is 60 px eased over 180 ms
  (`items/flickable.rs`), so every frame in between leaves the content at a
  fraction of a pixel — but femtovg puts each glyph run's origin on the
  pixel grid, so that alone blurs no text. An element with `clip: true` *and*
  a `border-radius` is different: femtovg cannot clip to a rounded shape, so
  `visit_clip` draws the children into a texture and copied it to the
  item's unrounded position, sampling between texels. Measured with
  `RUSTY_PROBE_SCROLL` on a Markdown page held still half a pixel off: the
  paragraphs identical to the whole-pixel frame, the code blocks at 0.67
  and 0.86 of its contrast; the code view, which has no rounded clips,
  identical; the software renderer sharp. With the texture put on the grid,
  as femtovg's opacity layers already were, the code blocks match too.
  `RUSTY_PROBE_SCROLL_WHEEL` then turns the wheel as winit hands it over and
  photographs the glide: unpatched, a code block's brightest strokes fell
  from 222 at rest to 177 at half a pixel, and the block was soft at rest
  as well — the page's centred column had set it half a pixel in; patched,
  235 at every phase. So it is not only scrolling: anything clipped round
  (code blocks and tables, a `Field`, the editor's popups) is soft wherever
  its position falls between pixels. Upstream's `visit_clip` was unchanged
  when this was written; when slint moves, check it and copy the crate
  again or drop the patch. A patch whose version slint no longer pins is
  only a warning that it went unused, so `rusty-desk/tests/renderer.rs`
  fails on a lockfile whose renderer came from the registry.
  **The hypothesis that came first — fractional offsets blur text — was
  right about the motion and wrong about the text**, and only a photograph
  held between pixels told the two apart.
- **A Slint `TouchArea`'s `moved` is called only while a button is held**
  (`grabbed`, in `i-slint-core`'s `input_items.rs`), so a hover cannot be
  read there. The code view's hover card was wired to `moved`'s not-pressed
  branch, which never ran: the card came only from the keyboard, and every
  probe had opened it with *Show hover*. A pointer resting is a
  `pointer-event` of kind `move`. Found by dispatching real pointer moves at
  the window, which is also how the next rule was proven: **a touch area
  that must stay hovered while the pointer is on something clickable in it
  has to be that thing's ancestor** — `has-hover` is set on the way down
  (`input_event_filter_before_children`), which a sibling never sees, so the
  card's own touch area beside its fix rows lost the hover to them and put
  the card away under the click.
- **A Slint `StyledText` takes every press, of every button** (with
  `shared-parley`, `items/text.rs`: `Pressed { .. } => EventAccepted`, so
  that its links can be clicked), and nothing under it hears one. A
  right-click on a Markdown page's text would have opened no menu, and a
  `ContextMenuArea` would have fared no better — it, too, hears only what
  its children leave. So the page has a touch area *over* it, short of the
  scrollbar, which takes the right button and Ctrl and the wheel, and
  passes the wheel alone on; a left click it hears is made again underneath
  with the touch area stepped aside (`Shell.page-through`,
  `App::page_click`, after the event and never inside it), which is how a
  link still answers — a link in a paragraph laid round its formulas as
  well, whose pieces are text like any other. What it costs is the pointer
  hand over a link, which the text sets on a move the touch area now takes;
  and hover under it, which it took from the page's tips too (a refused
  formula's reason, a picture not shown) until the touch area put where the
  pointer is in `PagePointer` (`ui/markdown.slint`) and a `TipArea` counted
  as pointed at when that point is inside it as well as when it is
  hovered. **And a link wrapped
  onto two lines answers only on its last** (`link_in_layout` in
  `textlayout/sharedparley.rs` keeps the verdict of the last line's box,
  each line overwriting the one before): a click on its first line goes
  nowhere, with or without the touch area. Slint's, not rusty's: it was
  taken for the touch area's fault until a bare `ScrollView` and
  `StyledText` heard the same synthetic click on a one-line link at once.
- **A Slint `Flickable` keeps a wheel gesture it began for 800 ms**
  (`should_capture_scroll`, `items/flickable.rs`): a wheel turned within
  that time of its own scroll, the pointer not moved two pixels, never
  reaches what is inside it. Ctrl and the wheel straight after scrolling
  the code therefore scrolled it — the probe measured exactly that — so
  the code's rows take the wheel themselves (`CodeView`'s `wheel`): with
  Ctrl the text's size, alone the view moved by as far as the wheel turned,
  at once, which is VS Code's own scroll (`editor.smoothScrolling` is off by
  default). The Markdown page's touch area sits beside its `ScrollView`,
  not in it, for the same reason.

## The sheet

The board editor's parts are schematic symbols — KiCad's drawing of a part
with real pins, wired pin to pin — and `docs/schematic.md` is the design.
`model::symbol` is the drawing (what the window renders), `model::sheet`
the board (placed symbols and wires), `nets` the reading of the wires,
`schematic::kicad_sym`
KiCad's `.kicad_sym` read and written, `schematic::easyeda` an LCSC part
number fetched from EasyEDA's component service and read into the same type,
`simulate::board_file` the file in both its formats, `rusty_embed::board`
the drawing and the editing arithmetic, and `app/board.rs` and
`app/sheet.rs` the window's board. `rusty-cli symbol C2286` is the headless
proof of the import; the Slint board has no import of its own yet. **A Wokwi
`diagram.json` comes across too** (`schematic::wokwi`): the
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
  drawing converts once, in `geometry::local` and `orient`, with `MM_PX` chosen so
  that KiCad's 100 mil pin pitch is one kit row pitch — a symbol's pins then
  sit on the same grid as the devkit's header, which is what lets a snapped
  wire meet both ends. A pin is found by number and then by name, so
  `D1.K` and `D1.2` both land.
- **The devkit is a part like any other: `U1`, whose symbol is generated
  from its rows.** `nets::kit_rows` names the header (`GPIO2`, `GND`,
  `3V3`; the ESP32 devkit's `RX` is `GPIO3` by name) and
  `geometry::kit_symbol` puts one pin on each row, so wiring, hit-testing and the
  file need no second code path for the chip. A wire spells a pin by its
  name when the name is unique in its symbol and by its number otherwise
  (`pin_key`): `U1.GPIO2`, `D1.K`, and `U1.9` only where `GND` repeats.
- **Polarity is wiring.** There is no `active_low` any more. A lamp lights
  when its anode's net is high and its cathode's low; on 3V3 with the GPIO
  sinking the cathode, it lights when the pin is low. A switch to ground
  drives its GPIO low while pressed, one to 3V3 high — `nets::button_drives`
  says which, and the run reads the *same* function as it starts to set the
  pin channel's polarity, so the emulator and the sheet cannot
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
  zero is pin `1`'s end, and the run sends the opening counts as it starts,
  from the same `start` prop the slider reads, so the panel and the
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
- **The rules run in one place** (`nets::evaluate`, read through
  `rusty_embed::period` from the parts, the wires, the firmware's levels and
  the held switches), and every part's face reads its answer:
  `is_lit` for a lamp, `is_pin_lit` per channel for an RGB lens or a digit
  against its `COM`, `level` for a motor's direction pins, `gpio_of` for
  what a knob or a source is on. Findings — a lamp with no series resistor,
  rails shorted, GPIOs fighting, a switch that reaches nothing, a wire to a
  pin that is not there — are `Warning`s with a stable kind, translated by
  the window and printed in English by the CLI.
- **A rail is a rail wherever it is drawn, and a name is a wire.**
  `rusty:GND` and `rusty:Supply` put their net at a level without a wire
  running back to the devkit, and two `rusty:Label` parts carrying the same
  value are one net — the two things a schematic uses instead of drawing a
  wire across the whole sheet. Both live in `nets`, so the rules, the probe
  and the run's button polarity all read them the same way. A label with
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
  pin can be asked what it is joined to — high, low or floating, and every
  pin in its net, which is what a probe on a wire is to say; the Slint board
  has no probe yet. The union-find was computing it already and throwing it
  away.
- **A sensor is a part.** `rusty:Sensor`'s value names the channel the
  firmware declared with `[rusty:sensor]`; the sliders under it on the sheet
  feed that channel with the same `I<name>=` line (`rusty_embed::sensor_line`)
  the Flight panel's plant sends, and
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
  decides where the pins are** (`rusty_embed::board::art`). A 5 mm lamp
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
  them, so the picture is a pure function of the symbol and its value and
  is drawn once (`board::picture`). A lit lamp is its own colour with a drop shadow, a dark
  one that colour dimmed — a red LED is red on the desk with the power off.
- **A part rusty has never heard of is a package**: pins down the two long
  sides, numbered as a DIP is, with its name on the body. An import whose
  reference prefix says what it is — `LED`, `R`, `SW` — gets that part's
  drawing instead, which is what makes importing from LCSC worth doing.
- **Text is never turned.** The body turns and mirrors as the part does; pin names, the reference and the value are placed in sheet
  coordinates *after* the turn (`pin_labels`), beside the lead rather than
  beyond its tip, where the wire goes, and drawn as the window's own text.
  Numbers are not drawn at all: a real
  part has none printed on its legs. `every_part_can_be_drawn_to_a_sheet_
  for_looking_at` writes every part into one SVG when `RUSTY_ART_SVG` names
  a file — the check a unit test cannot be, since whether a drawing looks
  like the thing needs eyes.
- **While a run goes on a switch is pressed, not dragged**: the cap sinks,
  the rules see it conducting, and the GPIO it reaches is driven through the
  same `B<pin>=1` the first boards sent as well as the pin channel, so
  firmware written for the text protocol hears it too.
- **A running board is not a drawing.** From Run to the run's end, build
  included, nothing moves, nothing is rewired and nothing is selected: a
  switch is pressed, a knob turned, a slider slid, and a drag pans. It was an
  editor for the whole run once — wires that dragged, an inspector over the
  board at every click — which the user named as exactly that. Which wire the
  pointer is near is asked of the geometry (`wire_under`), never of whatever
  is drawn on top, because the wires are drawn over the parts and a wire
  routed across a switch's cap took every press meant for the switch. What a
  part reads while it runs — `D1 · 1.97 V · 6.03 mA`, or a net's level and
  voltage — belongs at the sheet's foot for whatever the pointer is over,
  not in a panel over the board; the Slint board does not show it yet.

## Laying the sheet out

`rusty_embed::board::layout` is pure and tested, and it exists because
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
sheet, measured off the rendered drawing. `drawn_box` grows the
box by a row, and the same measurement then says zero.

**`arrange` is a command, never a rule that runs** — and not offered by the
Slint board yet. A board somebody laid
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
- **The solver is read through one door**, `circuit::operating_point` —
  bridge, solve, and one error type over both halves — and the board reads
  it through `rusty_embed::period` beside the rules, from the same parts,
  wires, held switches and reported levels. A net's reading is what it is
  *at*; a part's, what is across it, through it and dissipated in it. On a
  divider the two say different true things about one net: the rules say
  nothing is driving it, because a net holding both rails through resistance
  has no level, and the solver says 1.10 V. The Slint board shows the solver
  only as each lamp's glow so far; a probe and a part's reading are to read
  it here too.
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
- **A reading is what a meter shows.** `rusty_embed::board::readout` is
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
  solves, and the wire colours and the findings are worked out apart for the
  same reason. A pin no wire reaches is never ranked, or a quad's four
  motors would cost five readings of a sheet they are not on.
- **What a meter shows over a period is three averages, not two and a
  product** (`period::Measured`): a lamp lit half the time dissipates half
  its power, and its average volts times its average amps is a quarter. A
  reading that differs between moments is `steady: false` and says
  *average* on screen — a lamp under PWM never sits at its average voltage,
  and `0.59 V` beside a lit LED with nothing saying why is a number somebody
  would go and check. A net under PWM is read as `PWM, high 30% of the
  time`, never as floating.
- **The glow is the current to the power 1/2.2** (`rusty_embed::board::glow`). A
  pixel's value is its light to that power, so drawn this way the light
  leaving the screen is in proportion to the lamp's and the eye does the
  rest: a duty ramped in a straight line brightens fast and then barely,
  here as on the desk. Full is 10 mA, where `vf` is quoted. Where the solver
  refuses, the rules' lit share stands in at full current — what every lamp
  was drawn as before there were numbers. An RGB lens mixes its channels'
  shares through the eight colours it was always drawn in, and a digit's
  segments dim the same way. The lamp's light is drawn over the dark body the
  part's picture already holds, so a duty changing a hundred times a second
  redraws a light and not the part.
- **A pin is a level or a duty, never both, and the newer report decides**
  (`App::absorb`: a `[rusty:gpio]` report takes the pin out of the duties, a
  `[rusty:pwm]` one out of the levels). A level left from before
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
`wave` puts it on the pin channel; `rusty_embed::lab` is the instruments'
arithmetic (the records, a sweep's plan and measure, the designs' curves),
and in the window `app/lab.rs` is the Signals tab — the four instruments,
the sweep and the console feed — which `ui/signals.slint` draws.

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
  samples a second inside its declared range, **with the host's pace said
  above the fields**.
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
- **Every instrument reads a record** (`rusty_embed::lab::record`): a telemetry
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
  inside each try. And `filter_probe` runs the whole chain with nothing mocked (`examples/
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
- **A filter that is host code is tried with no board** (`docs/signals.md`,
  *A filter tried in a test*). Asked for as "the instruments need a
  simulation and a circuit; I want to write the signals myself and see the
  filter keep the slow one". A test feeds the filter a signal it computes
  and samples what goes in and what comes out with rusty-draw's `Trace`
  (stamped telemetry on the trace's own clock), and the lab reads that as it
  reads a firmware's. **Response → Measure recorded** (`lab::sweep::recorded`)
  is the sweep's measurement without the stepping: the input's tones found
  in its spectrum (`tones_in`, Blackman, since Hann's first sidelobe is
  read as a second tone), each fitted on both records over the stretch both
  cover less its first quarter, drawn as dots and never joined. Held to the
  exponential average's closed form in a test and in the app: 2 and 200 Hz
  through α = 0.05 at 1 kHz, −0.25 dB −13° and −27.2 dB −52°. Two things
  the path needs that a simulation never had: a sample after libtest's
  `test name ... ` is still a sample (`protocol::split_telemetry`), and a
  run's stamps start at zero again, so a second run appended to the first
  reads as one record at a rate neither ran at — the simulator's start clears
  the capture, and a test or example run must too, which the Slint window
  does not do yet.

## The math toolbox

The arithmetic a flight controller is made of, worked out a row at a time and
drawn: an attitude as the aircraft it is, every step of the working in space,
and the firmware's own estimate beside the plant's truth while a run goes on.
`rusty_embed::spatial` is the arithmetic and the sheet's language, pure and
tested; `rusty_embed::scene` (with `scene::math`) the drawing; `app/math.rs`
the page. The sheet
lives in the project as `.rusty/math.toml`, beside the board.

- **Every convention is stated once, in `spatial`'s header, because each is
  somebody's afternoon**: Hamilton quaternions, `w` first; a rotation turns a
  vector, and an attitude turns the body's axes onto the world's; `a ⊗ b` is
  `b` first; Euler angles Z-Y-X intrinsic; and which way is up (`Frame`).
  cf-drone-rs flies Z up with the body forward-left-up — read off its
  source, not assumed — and PX4 and ArduPilot fly Z down. The default is
  the user's own firmware's.
- **The same Euler angles are two physical attitudes.** A positive pitch is
  the nose down with Z up (about a left wing) and up with Z down (about a
  right one). The arithmetic is identical in both; gravity (`Frame::at_rest`,
  `tilt`) and the picture are not. So the attitude indicator reads the nose's
  elevation, the right wing's dip and the heading off where the body's axes
  point (`spatial::instrument`), never off the three angles — and so does
  the assistant's `math_sheet`, which hands a model that reading beside the
  angles: the words for an attitude come from the instrument, never from a
  sign.
- **rusty's plant said "X forward, Y right, Z up" for months**, which is no
  right-handed frame at all. Its numbers were always one consistent set —
  the world's +Z in the body's axes, level `[0, 0, 1]` — and its tests are
  worded in the Z-down reading. The comment says both readings now, and
  `truth()` hands the plant's quaternion over as numbers: the frame changes
  the words, not the numbers, so firmware integrating the plant's gyro
  agrees with it either way.
- **A worksheet, not a form.** Testing attitude code is a chain — an
  attitude from three angles, gravity into the body with it, the tilt read
  back, checked against what the firmware printed — and each link wants a
  name the next can use. Rows read top to bottom: a name from below is
  `Later`, one given twice is `Twice`, and a row that does not even parse
  still says what it meant to be called, so the rows using it say *it* is
  broken rather than that the name is unknown.
- **The language refuses what it cannot decide, and says so by name.** A
  vector times a vector names `dot` and `cross`; four numbers in brackets
  are refused because `(w, x, y, z)` and `(x, y, z, w)` are both somebody's
  convention; a plain number past a turn where an angle goes is taken as
  radians *and said* (`Note::BareAngle` — it was probably degrees); an
  attitude off the unit sphere is said where it turns something.
- **Every operation leaves its working**: the formula with the numbers in,
  what to draw, and the turn it makes. `euler` is three steps whose turns
  chain — yaw, then pitch about the new Y, then roll about the newest X — so
  playing the row shows the order, which is the thing people get wrong;
  `rotate` is the sandwich product and then Rodrigues' three vectors head to
  tail; `integrate_linear` says how far first order leaves the unit sphere.
  **The formula lines are formulae and nothing else**: the first draft had
  English clauses in them ("the area a and b span"), which a Chinese window
  would have shown as English. Prose is a `Remark`, named in the arithmetic
  and worded by the panel.
- **`check` names the mistake, not the distance.** Attitude code is seldom
  nearly right; it is right with one convention crossed — the inverse, `w`
  written last, roll-first order, the other frame, degrees for radians — and
  each is a closed form away from the answer. "Differs by 108°" sends
  somebody through the code; "`w` written last" is the fix.
- **Live rows are refreshed ten times a second, not per line of
  telemetry** (`App::math_live`, only for a sheet whose rows mention `tel(`
  or `truth`): a flight controller prints hundreds of lines a second,
  and a sheet re-evaluated and redrawn per line is a window that stops
  answering.
- **No 3-D library**: a camera orbiting a target, a perspective projection,
  and shapes sorted back to front (`rusty_embed::scene`) — a few hundred
  paths, crisp at any zoom and coloured by the theme, drawn by the same
  pieces as the Draw tab (`SpacePieces`, `ui/draw.slint`). The plane's view
  is the same renderer, flat.
- **The sheet's file never reads as empty.** A `.rusty/math.toml` that does
  not parse is refused by name and nothing is saved over it — the panel
  saves as it is typed in, and an empty reading would be written back over
  somebody's rows.
- **The sheet is a tool as well** (rule 4, `math_sheet`): the assistant and
  every MCP client work rows out through the panel's evaluator, the
  project's sheet included, with the window's live values sent along with a
  question to the assistant.

## Drawing from code

A program draws into rusty the way it plots: by printing. `rusty-draw`
writes `[rusty:draw] vector 1 0.4 0 a` — a verb, the fixed count of numbers
that verb takes, and a label that is the rest of the line — `rusty_embed::draw`
reads it and gathers the lines into scenes (`Sketchbook`), and `app/draw.rs`
is the dock's Draw tab (`ui/draw.slint`). Asked for as "I want to see the cross
product of two vectors": the Math page draws what its own language
computes, and the vectors in question were the user's, in a flight library
of their own. So the picture comes from their code.

- **One reader, every stream.** The lines go through `absorb` as telemetry
  does, so a test run from its lens, an example run from its lens, the
  simulator's console and a board's port all draw — firmware too, through
  `SceneOn` and esp-println's `Printer`; the crate builds for
  `riscv32imc-unknown-none-elf` with no `std`. Text before the marker is
  kept for the log (`split_draw`): a harness running one test at a time
  writes `test name ... ` on the line the test's first print lands on.
- **A scene is printed whole, in one `print!`.** `cargo test` runs tests on
  threads, and two scenes written a line at a time interleave, each getting
  the other's arrows; one call holds stdout for the whole scene. Dropped by
  a panic, it goes out without its `end` and is shown as unfinished — what
  a failing test had drawn is the thing to look at — and so is a scene a
  run began and never ended (a run's end calls `App::drawing_ends`).
- **Shown when it ends, not mark by mark.** The scene being received is
  held by the `Sketchbook` and draws nothing; only a finished scene reaches
  the tab (`App::drawn`). A scene drawn again under its title replaces itself in place: a
  firmware drawing its attitude every loop is one scene fifty times a
  second, not fifty scenes. The first scene of a run brings the tab
  forward, and only the first — after that it would take the dock back
  every time somebody looked at Output.
- **Every scene a run drew is a row, not an option.** The tab followed the
  newest scene and kept the rest in a drop-down, so an example drawing
  three scenes was reported as "where did a and b go" — the two before
  the last read as never drawn. The list on the left has a row per scene
  with the one on the page lit, the page carries its scene's title, and a
  click on the newest row follows the newest again.
- **Names do not sit on each other** (`scene::declutter`, after the depth
  sort): a label whose box meets one already placed moves down a line, in
  page order so a scene always stacks the same way. An arrow's head, a
  point and a second arrow's head at one place were three names written
  into one smudge. And a frame names each of its axes (`body x`, `body y`,
  `body z`): named at its X alone, the other two arrows were nobody's and
  the name read as a label of whatever ended near the X tip.
- **A colour rides on the verb** (`vector#e5484d 1 0 0 a`), set by
  `Scene::color` for every shape after it and handed back by
  `auto_color`; a colour that is not six hex digits makes the line no
  drawing. On the verb and not as a word before the label, because the
  label is the rest of the line and may begin with anything. A coloured
  frame draws all three axes in its colour — two attitudes told apart —
  and an uncoloured one keeps red, green and blue. The drawing playground
  (`PLAYGROUND_DRAW`, a host crate carrying rusty's own copy of
  rusty-draw) shows all of it.
- **The angles are worked out, not left to the eye.** Perspective
  foreshortens every angle, so a right angle cannot be judged in a turned
  view. `draw::angles` measures every pair of arrows sharing a tail from
  the numbers printed, the list writes the angles in degrees, and the view
  draws its little square only where the cosine is under 1e-4 — a
  two-hundredth of a degree, a thousand times the rounding in an `f32`
  cross product and thousands of times smaller than a product with a
  component in the wrong place. **A right angle cannot tell `a × b` from
  `b × a`**, which is at right angles to both as well; `draw::handedness`
  can, and says which hand three arrows from one tail make, in amber when
  it is the left.
- **A number an `f32` held is written as that `f32`.** Widened for
  printing, `0.4_f32` is `0.4000000059604645`, and every line an `f32`
  library drew read like that. `Number` writes the `f32`'s own shortest text
  when an `f32` holds the value exactly and the `f64`'s otherwise; a `NaN`
  stays `NaN`, drawn as broken rather than as a zero.
- **Framed to the page, not to a sphere** (`Camera::framing`). The dock is
  short and wide, and a sphere fitted round the scene left most of it empty
  with the arrows the size of a thumbnail. The distance is found by halving
  until every mark, every name and every axis's end is inside the page, a
  margin in from the edge. **Each axis is as long as the marks reach along
  it** (`axis_lengths`): one length for all three let the tallest decide
  the page. A new scene, one grown or shrunk past a factor of two, or a
  view reshaped by a third is framed again (`Framed::stale`); anything less
  keeps the zoom somebody chose.
- **▶ Run over an example's `main`** (`tests_in::example_main`), beside the
  tests' lenses: `cargo run --example <name>` at the project root, which
  cargo resolves across the workspace's members. **Withheld wherever
  `cargo run` would flash a board** — a root that is its own firmware, and
  anything inside the excluded firmware crate (`runs_examples`) — because
  `cargo run` there hands the image to espflash. An example has no Debug:
  the host debugger finds a test's binary by asking the test binaries.
- **The crate carries the workspace's license, GPLv3** (the workspace's
  since the window moved to Slint under its GPLv3 option). For the one
  crate here that other people link into their own firmware that is a
  real condition: firmware that links it and is distributed is GPLv3 too.
  Whether this crate alone should be permissive, and whether to publish it
  to crates.io rather than be taken by git, is the owner's call and not
  made here.
- **`SpacePieces` (`ui/draw.slint`) is the drawing half of
  `rusty_embed::scene`** — each piece's shape, and each `Ink`'s colour
  (`ink_of`, `app/draw.rs`) — shared by the Math page and the Draw tab, so a
  vector is the same arrow in the same colours in both.
- **The same crate traces** (`Trace`, `TraceOn`): named channels sampled
  `rate` times a second, one `[rusty:tel@<µs>]` line a sample, stamped
  `n / rate` because a test has no clock and the lab reads only stamped
  samples. `Trace` prints whole on drop, as `Scene` does, so two tests
  tracing at once cannot put two signals in one channel; `TraceOn` writes a
  line a sample to a `fmt::Write`. rusty-embed's protocol test reads
  `TraceOn`'s lines back, so the writer is held to the reader.

## WCH's parts: the CH32V003 and the CH32X035

WCH's small RISC-V parts, through ch32-rs's crates: ch32-hal (from git — it
is not on crates.io), qingke-rt, wlink. The CH32V003 (16 KB, a QingKe V2A)
came first and most of what follows was learned on it; the CH32X035F8U6
(62 KB, a V4C, USB and USB PD) followed, and the last bullets say what it
changed. Everything rusty does for an ESP32 it does for either except
debugging, and the simulator is its own.

- **One catalogue entry per package**, `ch32v003j4m6`, `…f4p6`, `…f4u6`,
  because ch32-hal selects the part by package feature and probe-rs names
  it by package, and the package decides which pins exist. A pin is
  numbered eight to a port (PC4 is 20) everywhere it travels, and *named*
  by port wherever it becomes words — `nets::pin_label` over the part's
  catalogue `ports`, the one place, used
  by the devkit's rows, the Waves panel, `rusty-cli sim` and the assistant's
  `simulate`. `gpio_named` does **not** read `PC4` as a pin: chip-agnostic,
  it would bind an imported STM32 schematic's `PA1` to an ESP32's GPIO1.
- **The target is a description the project carries**,
  `riscv32ec-unknown-none-elf.json`, which detection names by its stem — the
  directory cargo builds under — and which needs nightly and `rust-src`
  (`-Zbuild-std=core`). `ToolchainRequirement::NightlyBuildStd` says so;
  the report checks nightly with its source and that the project builds
  with it (`rust-toolchain.toml`, a rustup override, or a nightly default)
  instead of a `rustup target add` that cannot exist, and
  `ToolchainReport::builds_std` keeps the Environment page and the setup
  plan from offering one. ch32-hal's own examples pin nothing, so they
  report `nightly-not-pinned` — the playground and the wizard's projects
  carry the pin. rust-analyzer is given no `cargo.target` for such a
  project: the stem is a triple no rustc knows, and its cargo reads the
  config's `build.target` for itself. **But not the `[unstable]` table,
  where it loads the sysroot**: rust-analyzer runs `cargo metadata` for
  the standard library's own workspace from the sysroot's directory, and
  nightly cargo refuses a `.json` target there without
  `json-target-spec` — the status bar said "partly loaded", with `core::`
  answering nothing, on the playground. `discover::cargo_env` passes the
  key as `CARGO_UNSTABLE_JSON_TARGET_SPEC` in `cargo.extraEnv` for a project
  (or linked firmware crate) whose config turns it on, and the same window
  then loaded whole, ch32-hal's types in its inlay hints.
- **Flashing is wlink first, probe-rs when its probe is picked**
  (`flash::uses_wlink`). `wlink list`'s probes are listed beside probe-rs's
  as `wlink:<index>`, the number its `-d` takes. Flash-and-monitor is
  `wlink -v flash --enable-sdi-print --watch-serial`; a monitor resets
  rather than writes, since wlink turns SDI print on only across a reset;
  `flash::firmware_line` takes off the timestamp `--watch-serial` puts in
  front of every line, or no protocol line on hardware would read as one.
  Proven against wlink's documented CLI and its source, **not against a
  board**: nobody here had one.
- **The simulator is rusty's own** (`rusty-mcu`), run on a thread of the
  process that wants it, not a program to find. The plan names it as one —
  `rusty-mcu --chip … <elf>` — so the dock shows what ran, and
  `simulate::launch` is the one place that knows it is not a file:
  `process::Session` is its lines, its input, a stopper and a wait, and
  nothing about a process beyond that, so the app, `rusty-cli sim` and the
  MCP `simulate` drive both the same way. To them it is a QEMU with rusty's
  models: a pin channel it listens on and waits for (`--pins`), the same
  `[rusty:gpio@<us>]`/`[rusty:pwm@<us>]` lines, and enough QMP for the
  panel's pause (`--qmp`). Paced to the wall clock in slices of 20 ms of
  the part's time; 2 s of the PWM example run in about 230 ms on this
  machine, so pacing is all it needs.
- **What it models is what ch32-hal's ordinary firmware reaches for**: the
  clock tree (HSI, the PLL, HPRE — the part starts at 8 MHz), GPIO with
  AFIO's remap tables (transcribed from ch32-data), EXTI, TIM1/TIM2 with
  preload and update events, SysTick (every `Delay`), the PFIC, USART1 and
  SDI print. Interrupts go through the vector table of addresses, with the
  hardware stack qingke-rt turns on (`csrw 0x804, 3`) — its handlers save
  only `ra`, so without it the first interrupt corrupts whatever it
  interrupted. **What it does not model it names on first touch** — ADC,
  I2C, SPI, DMA, the watchdogs — with what the firmware will find missing;
  their registers read back what was written. **HSE never comes ready**: a
  J4M6 board has no crystal, and blinky's `SYSCLK_FREQ_48MHZ_HSE` waits for
  ever there as on the desk, said once with the HSI preset to use. The plan
  says all of it before the run (`SimLimit` `ch32-model`), and that the
  J4M6's shared package pins are separate pins here.
- **No M extension, refused as the chip refuses it**: `mul` is an illegal
  instruction, so firmware built for the wrong target fails here as it
  would on the desk. Timing is one instruction per HCLK cycle; anything
  timed by SysTick or a timer keeps the part's time exactly.
- **A preloaded duty lands at the next update event, and is reported
  then.** A step of a 10 ms ramp is 10 ms of `Delay` plus the loop's own
  microseconds, so one in a few dozen lands 11 ms after the last: the
  drift crossing a 1 kHz period. The test that held every step to 10 ms
  was wrong, not the model; `a_duty_lands_on_the_update_event_after_it_is_
  written` holds the property instead. What the test *did* find was a
  register read that brought a timer up to date and dropped the "duty
  changed" answer — a timer now keeps that until the machine takes it
  (`take_changed`), whoever caused the update.
- **The playground and the wizard write the same proven project**: ch32-hal
  at a pinned revision with its lockfile, the target description, nightly
  pinned, a `build.rs` of its own (esp-hal's links another script), a
  breathing LED on PC4 with a button on PC1, and the board — redrawn per
  package by the wizard, because a ground wire lands on a row number and
  an F4P6's row 6 is a GPIO. SDI print is printed once: it waits for a
  debugger to take each line, and a board with no WCH-LinkE would wait for
  ever at the second.
- **One machine, two parts; what differs is data** (`ch32/part.rs`): the
  core, the memories, the die's pins, how wide a port is, which timers and
  USARTs sit where and which interrupt each raises, the blocks and what is
  missing. Where a layout differs in kind — the X035's RCC has only its
  48 MHz HSI and a divider, its AFIO two `EXTICR`s and a remap field per
  timer, its GPIO `CFGHR`/`CFGXR`/`BSXR` — the peripheral asks `Family`.
  The V003's machine was generalised, not copied: eight hundred lines of
  run loop twice is two run loops that drift.
- **The X035's ports are twenty-four pins wide, so it is numbered
  twenty-four to a port** (PB12 is 36, PC19 67), where the V003 is eight.
  The width is per part on both sides of the wire — `Part::width` in the
  emulator, the catalogue's `ports` for every name a pin is given — and a test
  holds the two equal for every pin either part has; a WCH family nobody
  has measured gets no port names at all rather than eight to a port.
- **The V4C is not the V2A with more registers.** RV32IMAC: thirty-two
  registers, multiply and divide (the fixture's image has twelve), the
  atomics; `GINTENR` (CSR 0x800), qingke's critical sections' window onto
  MIE and MPIE, without which `gintenr::set_enable()` enabled nothing; SDI
  print at 0xE0000380, not 0xE00000F4; a 64-bit SysTick with `INIT`; and a
  hardware stack that keeps sixteen registers. Each is a field of `Core` or
  a `wide` SysTick, and the V2A still refuses everything it lacks.
- **The X035 builds with stable Rust**, for `riscv32imc-unknown-none-elf`,
  a target rustup ships — ch32-hal's examples' own — so it is `stock` in
  the catalogue and none of the nightly machinery applies. The playground
  and the wizard's template share the V003's lockfile and `build.rs`:
  ch32-hal's dependencies do not change with the part, and cargo writes
  the same lock (`--locked` builds it). Its PWM is ch32-hal's example's
  PB12, TIM1 channel 4 under remap 2, with the button on PB1.
- **Its remap tables are generated, never typed** (`scripts/x035_pins.py`
  over ch32-metapac's `metadata_0036.rs`): four timers' and four USARTs'
  pins by remap value. The F8U6's nineteen GPIOs were read off the
  datasheet's QFN20 pinout, since ch32-data lists the family's pins and not
  a package's. **Proven in the emulator and against ch32-hal's code, not
  against a board**: nobody here has one.
- **"Has no serial bootloader" was never true.** The refusal for a serial
  flash said so of every part without espflash, and an STM32 has a UART
  bootloader, the X035 WCH's ISP. What is true is that espflash speaks only
  Espressif's ROM protocol; the refusal says that now.

## Cortex-M parts: STM32F4, RP2040 and RP2350, nRF52840

The first parts added as data alone (*A part is data, and so is a
vendor*, under rule 6): two vendors (`rpi`, `nordic`) and seven entries, no
code that names any of them. Each is built with stable Rust for a target
rustup ships, flashed and logged through probe-rs (defmt over RTT), and
started by the wizard from one of rusty's templates; none is simulated, and
the plan says so by name.

- **One entry per package where the package is the HAL's feature**:
  `stm32f411ce` and `stm32f401cc` (the WeAct Black Pill's two), because
  embassy-stm32 selects the part as `stm32f411ce` and probe-rs names it
  `STM32F411CEUx` — so these, unlike the bare `stm32f411` above them, can be
  flashed without asking. `rp235xa` and `rp235xb` are embassy-rp's own
  names for the RP2350's two packages.
- **Pins are named as each vendor prints them**: ST's lettered ports
  sixteen to a port (PC13 is 45), Nordic's numbered ports thirty-two to a
  port (`P0.13`, `P1.05` is 37 — `Ports { numbered }`), the RP parts by
  GPIO number.
- **The templates are proven, not typed** (`data/templates/stm32f411ce`,
  `rp2040`, `rp235xa`, `nrf52840`): embassy blinkies with a button, each
  built for its target with its lockfile in the template, and the projects
  the wizard writes from them — the F401CC and the RP2350B through another
  part's template — built `--locked` from what the wizard wrote. **Built,
  not run on a board**: nobody here has one. A template whose part shares
  another's has its feature and probe-rs's name for the part replaced
  (`template_files`), or an F401CC is flashed as an F411CE.
- **The RP2040's second-stage bootloader is placed by memory.x, not by
  embassy-rp's `link-rp.x`.** That script is a `SECTIONS` with no `INSERT`,
  and rust-lld then places `.boot2` as an orphan after the code — measured at
  0x10003be0 in a binary that linked without a word, where the boot ROM
  reads the first 256 bytes of flash and finds nothing. `INSERT BEFORE
  .text` in memory.x puts it at 0x10000000. The RP2350 has the same shape
  of trap: its boot ROM looks for the image definition in the first 4 KB,
  so memory.x places `.start_block` straight after the vector table
  (0x10000114). Check where a boot-critical section *landed*
  (`llvm-objdump -h`), not that the link succeeded.
- **A board in its USB bootloader is a device like a port or a probe**
  (`Transport::Uf2`, `Transport::Dfu`; `device::list_boot_devices`). A Pico
  held in BOOTSEL mounts a drive whose `INFO_UF2.TXT` says `Board-ID:
  RPI-RP2` or `RP2350`; an STM32 with BOOT0 high answers USB DFU as
  `0483:df11`, which `dfu-util -l` lists. The catalogue says which part has
  which (`uf2 = { family, flash, board_id }`, `dfu = { device, alt, address
  }`) and puts `uf2` or `dfu` among its `flashers`, so the title bar's
  picker lists only the bootloaders the open project's part can be written
  through, and one plugged in alone is the one board. Debug and Monitor
  never pick one: nothing runs in a bootloader.
- **UF2 is written by rusty, not by a tool** (`rusty_embed::image`):
  `rusty-uf2` is an in-process program like `rusty-mcu`, run through
  `flash::launch` — it reads the ELF's `PT_LOAD` segments by their physical
  address, cuts them into 256-byte pages padded out to each 4 KB sector, as
  picotool's `elf2uf2` does, stamps the part's family ID (pico-sdk's
  `boot/uf2.h`: RP2040 `0xe48bff56`, RP2350 Arm secure `0xe48bff59`), and
  copies the file onto the drive. picotool's RP2350-E10 absolute block is
  added only by its `--abs-block`, which is off by default, so it is not
  added here either. Checked against the two templates' real images: every
  byte of every loaded segment in a block at its address, and a segment
  outside flash refused by its address rather than written.
- **DFU is dfu-util, handed a binary rusty cuts** (`rusty-dfu`: the
  segments laid out from the part's flash base, `0xFF` between them, then
  `dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D <bin>`, with `-S`
  when the device has a serial). On Windows the system bootloader needs
  WinUSB first (Zadig), which the Environment page's dfu-util row says.
  **Neither path has met a board here** — no Pico, no Black Pill — so
  what is proven is the conversion and the plan, not the bootloaders.
- **Debug on the board is probe-rs's own debug adapter, not gdb.** For a
  release Debug meant the simulator and nothing else, so an STM32, RP or nRF
  user — anybody whose part QEMU does not model — could not debug
  at all. `probe-rs dap-server` flashes the image and debugs it in one
  session, for every part probe-rs knows, with no cross gdb to install, and
  rusty already spoke DAP for LLDB: `rusty_dbg::Board` on a `DapLaunch` is
  the whole difference (`App::debug_board`). The launch is probe-rs's own
  `SessionConfig`, read off its source rather than its docs, and so is the
  rest — RTT arrives as `probe-rs-rtt-data`, not `output`, and only after
  the client answers `probe-rs-rtt-channel-config` with `rttWindowOpened`;
  a refused launch carries its reason in the error body's `format`, while
  `message` says only `cancelled`. Measured against probe-rs 0.32 with no
  probe plugged in: refused within a second, in its own words ("No
  connected probes were found."), where the first version waited out the
  five minutes a flash may take. `tests/probe_rs.rs` holds that. **Which
  Debug runs is a rule, not a setting** (`App::debug_verb`): the board when a
  probe is the chosen target, the simulator otherwise, so a C3 with its
  serial port chosen stays in the simulator; a WCH-Link, which wlink drives
  and probe-rs cannot, is refused by name. The image debugged is the image
  Flash writes — release, which every template builds with `debug = 2` — and
  probe-rs flashes the file it is handed, the one the build just made. **Not run against a board**:
  nobody here has a probe.
- **Watching a board needs no bootloader.** Monitor over a serial port was
  refused for every part espflash cannot talk to, because the refusal came
  before the action was looked at — an nRF's USB-CDC, an STM32 on a
  USB-UART. It is rusty's own serial link for those now (`App::link`),
  through the same `absorb`.
- **The probe-rs name is the project's runner's first**
  (`EmbeddedProject::probe_chip`, read off `runner = "probe-rs run --chip
  STM32F411CEUx"` in `.cargo/config.toml`, a string or an argument list):
  it is what `cargo run` already flashes with, and the catalogue's die
  entries (`stm32f411`) have no package and so no name to give. Two probe-rs
  runners naming different parts name neither. The refusal for a part with
  no name says to put one there.
- **The pin map reads every HAL's pin names** (`pins::pin_of`): `.GPIO5`,
  `.PIN_25`, and through the part's `ports`, `.PC13` and `.P0_13`
  (`Ports::number`, the inverse of `Ports::name`), and names a pin as its
  vendor does on screen. A part that is not Espressif's is mapped from the
  catalogue's `gpio` list, with a note that the catalogue knows which pins
  exist and nothing about what each can do — it told an STM32 user that
  esp-hal's description of their part could not be found.
- **The wizard lists what it can create**: a part with no generator is one
  to open a project for, not to start one with, and picking `stm32f411`
  ended at the last step on "no generator". A part with no `std` target
  gets no ESP-IDF row, a nightly part is said to need nightly, and a part
  flashed through a probe says so before the project exists.

## Meeting C

Embedded Rust does not live alone: vendor SDKs are C, `esp-idf-sys` wraps a C
framework, and teams migrate by putting a Rust module inside C firmware. So
the workbench knows about C without becoming a C IDE.

- **Detection reports, never guesses.** `project::detect` fills `c_interop`
  with what it found — `cc`, `bindgen`, `esp-idf-sys`, a `staticlib`
  crate-type, C sources in the project — and each claim carries the file
  that proves it.
- **Scaffolding refuses before it writes.** `scaffold::c_interop` writes
  three directions — Rust calls C, Rust calls C++, C calls Rust — and stops
  on the first path that exists: half a scaffold over somebody's code cannot
  be undone by an error message. It does not edit `Cargo.toml` either —
  `cargo add cc --build` is the official path, visible in the dock like
  every other command.
- **An existing `build.rs` is joined, not refused.** The C build goes into
  `build_c.rs` and the build script gains exactly two lines, `mod build_c;`
  and `build_c::compile();` at the top of `main` (`merge_build_script`).
  Every embedded template has a build script — the link scripts are passed
  there — so refusing one refused C for every part but the Espressif ones.
  A `main` that is not one `fn main() {` line is not guessed at: the refusal
  names the two lines. The build script names the part's cross compiler
  (`Chip::c_compiler`), since `cc`'s guess for a bare-metal triple is a
  generic toolchain that may not be installed; for C++ it is the matching
  `g++`, with no exceptions, no RTTI, no guarded statics and no libstdc++
  linked — and the C++ sits behind an `extern "C"` function, because Rust
  calls C, never C++. Both directions were built for a Black Pill with the
  Arm toolchain and read back with `nm`.
- **The memory report names a C file as it names a crate.** C has no crate,
  and everything without one was "unattributed"; `vendor.cpp` is an entry
  of its own now, found two ways: the image's DWARF, whose non-Rust compile
  units say which addresses each source covers (`gimli`), and without debug
  information — a CMake Release build — the `STT_FILE` symbol a file's own
  `static`s are listed under. A global with neither stays unattributed.
  **Most of that bucket was never C**: a v0 symbol for an impl's method
  demangles to `<u64 as core::fmt::Display>::fmt` or `<Executor>::spawn`,
  which begin with `<` and name no crate, so every method of every impl was
  unattributed — 5.5 KB of a 13.8 KB image. `v0_crate` reads the defining
  crate off the mangled name (the impl's own path is encoded first), and
  the names lose the `[26b6afaf38d9ac34]` hash the plain form prints.
- **`.h` is C here.** syntect gives the extension to Objective-C, whose
  grammar colours a firmware header wrongly in ways that read as a broken
  highlighter.
- **A C or C++ project is opened, built and flashed** (*Build systems*,
  below): rusty builds it with the project's own tool, finds its image,
  sizes and flashes it, and clangd reads its C and C++ in the editor (*C
  and C++ in the editor*, below).
- **And the wizard starts one, for a part with a C template**
  (`WizardChoice.language`, the catalogue's `c_template`; `c-stm32f4` for
  both Black Pills, `c-nrf52840`): one CMake project, no vendor SDK — a
  `startup.c` with the vector table, `.data` copied and `.bss` cleared, a
  `link.ld` with the part's own memory filled in from the catalogue, and a
  blinky written register by register — plus `CMakePresets.json` naming
  the `cmake/rusty-toolchain.cmake` written beside it, so `cmake --preset
  firmware` builds it outside rusty too. `RUSTY_CHIP` in the CMakeLists is
  how rusty knows the part. It is the runtime step's third row, shown only
  where the part has a template; a workspace, an option or another runtime
  with C is refused by name. **Linked with `-nostartfiles`**, and
  `startup.c` runs the constructors itself: newlib's crt0 brings `exit()`,
  stdio's clean-up and the four "is not implemented and will always fail"
  syscall warnings with it, and its `.init_array` put a writable section in
  flash's segment — `LOAD segment with RWX permissions`. Built here, all
  three parts, through `rusty-cli build` with xPack GCC 14.2.1, CMake 3.31
  and Ninja, with no warning; the vector tables read back with the stack at
  the top of each part's RAM. `RUSTY_C_PROJECTS=<dir>` makes the wizard's
  test keep a copy of each for exactly that. Not run on a board.

## Build systems: Cargo, PlatformIO, CMake

`EmbeddedProject.build` says which tool builds a project, decided by the file
at its root: `Cargo.toml`, then `platformio.ini`, then `CMakeLists.txt`
(`buildsys::system_at`), and a folder with none is refused naming all three.
`rusty_embed::buildsys` answers for the two that are not Cargo the questions
`project` answers for Cargo: what the chip is, the commands that build it
(`build_plans`), how it is flashed, where its images are (`images`) and what
it needs installed (`tools_for`). The window's Build plans
(`buildsys::build_plans`, `app/build.rs`) and runs the steps in order;
`rusty-cli build` runs the same.

- **The chip is read where the project's own tool reads it, or not at
  all**: a PlatformIO environment's `board_build.mcu`, else its board through
  the catalogue's `platformio` board ids, else the board's JSON in an installed
  PlatformIO; the Pico SDK's `PICO_BOARD` / `PICO_PLATFORM` (its own default,
  the Pico, when neither is set); ESP-IDF's `CONFIG_IDF_TARGET` in `sdkconfig`
  (its own default, the ESP32); STM32CubeMX's `.ioc`; and in a CMake project
  with none of those, a `set(RUSTY_CHIP <part>)` line, which the C projects
  rusty writes carry. Every name goes through
  the catalogue — an ordering code like `STM32F411CEU6` is a chip's
  `aliases` entry, matched whole, never by prefix: `esp32` is a prefix of a
  dozen parts it is not. No chip is a warning that says where the chip would
  be written, not a guess off a compiler flag.
- **PlatformIO is read as PlatformIO reads it** (`parse_ini`): `default_envs`'
  first, else the file's first environment; `[env]`, then what `extends`
  names, then the environment's own keys; indented continuation lines; an
  inline `;` comment after whitespace. Flashing and monitoring are
  PlatformIO's own (`pio run -t upload`, `pio device monitor`), handed the
  port the title bar chose and nothing about a probe, which the environment's
  `upload_protocol` decides.
- **CMake configures once, with Ninja and compile commands.** The first build
  of a directory with no `CMakeCache.txt` is `cmake -S . -B build -G Ninja
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_EXPORT_COMPILE_COMMANDS=ON` (Ninja,
  because the host's default generator — Visual Studio's on Windows — cannot
  cross-compile), or `cmake --preset <first visible preset>` where
  `CMakePresets.json` has one, its `binaryDir` followed through `inherits`;
  every build after is `cmake --build <dir>`. ESP-IDF is `idf.py build`.
  `compile_commands.json` is what clangd will read.
- **A CMake project that names no compiler is given the part's**
  (`buildsys::ensure_toolchain_file`, run by the window's Build and
  `rusty-cli build` before planning). Configured as it stands, it would
  build a host program with the machine's own compiler and say nothing.
  So where nothing names one — no SDK, no `CMAKE_TOOLCHAIN_FILE`,
  `toolchainFile`, `CMAKE_C_COMPILER` or `CMAKE_SYSTEM_NAME` anywhere in
  the CMakeLists or a preset (`names_its_toolchain`, read loosely: a mention
  counts, because a project that does name one must not have rusty's put
  over it) — `cmake/rusty-toolchain.cmake` is written for the part from the
  catalogue (`c_compiler.flags`, the core's `-mcpu`/`-mfpu`), once, never
  over a file that is there, and the configure step names it. The file is
  the project's: in its tree, readable, committable, and used by a build
  without rusty. A part with no flags on file is refused rather than
  configured for the host.
- **A build directory's firmware is an Arm, RISC-V or Xtensa ELF
  executable** (`embedded_elf`: the header's type and machine), not any ELF:
  a CMake tree is full of ELF object files and, on Linux, of host tools the
  Pico SDK builds for itself. ESP-IDF's bootloader and CMake's own
  bookkeeping directories are not looked in.
- **What only Cargo has is refused by name for the others**: the simulator
  ("rusty simulates firmware built with cargo"), Test, the Crates page's
  analysis, and rust-analyzer — whose status reads *off*, not *missing*,
  because "rust-analyzer missing" over a C project sends somebody to
  install a server it would not use. The Environment page asks for the build
  system's tools instead of rustup's targets, and for the part's cross C
  compiler on a CMake project (PlatformIO and ESP-IDF install their own).
- **Proven against a real build, not only the tests**: a bare-metal
  STM32F411CE CMake project with a `.ioc`, built by Arm GNU Toolchain 15.2
  and Ninja through `rusty-cli build` — configured once, built, a second
  build finding nothing to do — then `rusty-cli size` finding `build/
  blinky.elf`. PlatformIO is proven by its tests alone: nobody here has it
  installed.

## C and C++ in the editor

A PlatformIO or CMake project gets clangd where a Cargo one gets
rust-analyzer: one server per project, chosen by the build system detection
found (`lsp::start`), and `rusty_lsp::ServerKind` says which files are its —
`.rs` for rust-analyzer, C and C++ and their headers for clangd. Every
request the editor makes goes to the server the language table names for
its path (`language::of`, `Servers::for_path`); the sixteen places that once
asked `ends_with(".rs")` were the whole of what kept the editor Rust-only,
and the client under them was already plain LSP.

- **clangd reads `compile_commands.json`, which the build writes**: CMake's
  configure exports it into the build directory, and a PlatformIO build adds
  `pio run -t compiledb` once (`buildsys::compile_commands_dir`). Before the
  first build there is none, and clangd guesses flags.
- **`--query-driver` is what makes cross C readable, measured**: without it
  clangd parses an Arm file with its own headers, and `#include <string.h>`
  is `file not found` (`clangd --check`, a newlib header); with it clangd
  runs `arm-none-eabi-gcc` for its system include paths and the same file
  has no errors. The globs come from the catalogue's `c_compiler` names
  (`buildsys::query_driver_globs`), never `**/*`: clangd *runs* whatever
  matches, and a glob that matches anything runs anything a compile
  database names. Windows paths in the database match `**/` globs.
- **clangd declares no pull diagnostics, so nothing is pulled.** rust-
  analyzer's pushes are only the check's half and its pulls the rest;
  clangd's pushes are the whole answer. `Shared::pulls` is read off the
  `initialize` reply's `diagnosticProvider`, and a server without one is
  never asked — a mock server that does not declare it gets no pulls either,
  which is why the unit tests' fake now declares what rust-analyzer does.
  rust-analyzer's own extensions (`runFlycheck`) go to rust-analyzer only.
- **`tests/clangd.rs` drives a real clangd** — a pushed diagnostic on the
  right column under a CJK comment, completion and hover through the same
  client — and skips aloud where none is installed, as `tests/analyzer.rs`
  does. Format-on-save stays Rust's (rustfmt); a C file is saved as typed.
- **A Cargo project with C runs both servers**, each for its own files: the
  C a `cc` build script compiles goes to a clangd beside rust-analyzer
  (`lsp::start`), and every request is routed by its path
  (`Servers::for_path`) — to the server that serves it, or to none, never the
  other. `cc` records no command line, so rusty writes the compile database
  it would have used (`buildsys::write_cargo_compile_commands` over
  `cargo_compile_commands`: the part's compiler by name, which
  `--query-driver` asks for newlib's headers, freestanding, every header
  directory included) under the data directory, never into the project.
  The companion's diagnostics and refreshes share the first server's
  stream; its progress, health and exit do not (`lsp::passed_from_beside`),
  since the status bar is the first server's — which reads `rust-analyzer +
  clangd`. Which server a file goes to is the language table's answer, not
  which servers are up, so a C file never reaches rust-analyzer while clangd
  is starting.

## After every feature: review before moving on

Not a release ritual — a step in finishing the feature.

1. **Dead code.** Did this leave an unused `pub fn`, a struct field nothing
   reads, a helper duplicated in two modules? `cargo clippy --all-targets`
   catches private ones; public API needs a grep for call sites.
2. **Did the seams hold?** Does the new code keep a crate's data apart from
   its work (rule 1), file formats apart from model types, and
   refuse-rather-than-guess? If it needed an
   exception, that is a design signal, not a licence.
3. **Is anything now stale?** A pivot leaves debris: config pointing at deleted
   directories, a CLI that predates the domain, docs describing the old
   positioning.
4. **Should the architecture change?** Cheap now, expensive later — the chip
   catalogue became data before boards were referenced everywhere; the AI tool
   context was generalised before a second consumer existed.
5. **Update this file** if a convention was established or a trap was found.
