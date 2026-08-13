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
# Everything, the way CI runs it
cargo test --workspace
cargo clippy --workspace --all-targets

# The frontend links only the model layers, so these must stay green
cargo check -p rusty-core -p rusty-embed -p rusty-ai -p rusty-term \
  -p rusty-edit -p rusty-lsp --no-default-features \
  --target wasm32-unknown-unknown

# Frontend alone, on http://localhost:1425 — much faster to iterate on than a
# full `tauri dev` rebuild. Anything needing the backend reports that it cannot
# run; the layout and styling are real. To exercise backend flows here anyway,
# crates/rusty-ui/mock.js stubs the IPC surface: add
#   <link data-trunk rel="copy-file" href="mock.js" /><script src="/mock.js"></script>
# to index.html while debugging and REMOVE IT BEFORE COMMITTING. It is inert in
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

# The workbench without the window
cargo run -p rusty-cli -- check .
cargo run -p rusty-cli -- size target/riscv32imc-unknown-none-elf/release/app
```

## Layout

| Crate | Does |
|---|---|
| `rusty-core` | Cargo workspace analysis: dependency graph, duplicates, feature unification |
| `rusty-embed` | Chips, boards, project detection, toolchain, memory, flashing, wizard, simulation |
| `rusty-ai` | Bring-your-own-LLM providers, the tool registry, the agent loop |
| `rusty-term` | A real terminal: portable-pty (ConPTY) + vt100, rendered by the frontend |
| `rusty-edit` | File tree, syntax highlighting (semantic tokens, not colours), read/write, rustfmt, project search on ripgrep's engine |
| `rusty-lsp` | rust-analyzer client: stdio JSON-RPC, diagnostics, completion, hover, definition, signature help, code actions, semantic tokens |
| `rusty-ipc` | Command-name constants both sides `use`; a test in rusty-app pins each to a real handler |
| `rusty-app` | Tauri backend — thin, no analysis lives here |
| `rusty-ui` | Leptos frontend (Trunk + Tailwind, no npm) |
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

Debugging rides the same boot: `-s -S` freezes the CPU with the gdbstub on
:1234, and the terminal attaches the matching esp-gdb (`break main` lands in
the user's source with full backtraces — proven against the real blinky
image). Espressif's prebuilt QEMU has the plugin interface compiled OUT
("plugin interface not enabled in this build"), so register-level tracing
needs our own QEMU build one day; until then the gdbstub's watchpoints are
the honest bridge to register truth.

That is a deliberate ceiling. The QEMU peripheral models expose no GPIO
readback — probed with QMP on the real register addresses, esp32 and esp32c3
both read zero — so the board shows *what the firmware says it set*, and the
panel says so in as many words. A part therefore needs no code in rusty to
exist, which is why `.rusty/parts/*.toml` can add one.

### 6. Extensibility is data first

See `docs/extensibility.md`. Chips and boards are TOML in three layers
(built-in < user config < `<project>/.rusty/`); simulator parts are TOML in
`<project>/.rusty/parts/`. Code extensions go through MCP. UI contributions are
declarative — extensions never ship markup or styles.

## UI conventions

Chrome actions are icon buttons with a `title` tooltip — flat like VSCode's,
no ring, no fill; colour lands on the glyph (accent Play, crimson Stop). Text
appears in a control only when it carries state (a zoom %, a grid size).
Dot-entries never show in the file tree. Every dock surface answers a
right-click with its own menu or not at all — the browser's default menu is
always a bug.

## Where state lives

One rule decides: **if the backend, the CLI, or another window could ever care,
it is a file; if only this WebView cares and losing it costs a shrug, it may be
localStorage; high-volume queryable data picks its own format when the feature
that needs it lands.**

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
- Theme, divider positions, the editor's text zoom and the interface scale
  are localStorage, and that is all that is.

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
- **Geometry and protocol get tests; views get driven.** The board canvas got
  its arithmetic wrong three times while none of it was reachable from a test.
  The pure half now lives in `simulate/geometry.rs` under tests that pin the
  real pinmap, rotated anchor points, orthogonality and endpoint anchoring.
  What genuinely needs a browser — a drag, a right-click — is driven through
  `mock.js` and asserted on numbers read back from the DOM, in a *separate*
  call from the one that dispatched the event.
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
- **espflash `save-image` does not create parent directories** — a missing
  `target/rusty-sim/` fails as `os error 3`, which reads like a broken tool
  rather than a missing mkdir. `simulate::prepare` runs first.
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
