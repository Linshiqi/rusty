# Extensibility: what we commit to now, and what we defer

**Status:** accepted, amended for the embedded pivot · **Applies from:** M1

## Amendment: the long tail here is hardware

This record was written when rusty was a general Rust workbench. The pivot to
embedded changes the *shape* of the extension pressure, not the conclusions
below — but it adds a category they did not cover.

For a general Rust IDE the long tail is software: linters, frameworks,
languages. Dozens of things, all of them code. For an embedded workbench it is
**parts and boards** — thousands of things, and almost all of them *data*.

That difference matters because data extension is an order of magnitude cheaper
than code extension. A board definition needs a file format, not a runtime, not
a sandbox, and not an ABI.

So a fourth commitment, implemented rather than deferred:

### Commitment 4 — the hardware catalogue is a file format

Chips and boards live in TOML, layered:

| Layer | Where | For |
|---|---|---|
| built-in | compiled into the binary | the common parts |
| user | `%APPDATA%\rusty` or `~/.config/rusty` | boards you own |
| project | `.rusty/` in the open project | boards your team owns, checked in |

Later layers win by `id`, so a project can correct a built-in without forking.

Two rules make this survivable:

- **The file format is a separate type from the wire format.** `catalog.rs`
  parses into its own structs and converts. The file format is a public contract
  with users; `model` is an internal contract with the frontend. Coupling them
  would mean a UI refactor silently breaking everybody's board files.
- **A malformed file is reported, not swallowed and not fatal.** One bad file
  must not blank the catalogue, and ignoring it silently would leave the user
  staring at a board that never appears. `Catalog::problems()` carries them to
  the UI.

The payoff beyond extensibility: a board record carries USB vendor and product
ids, so a plugged-in device is *named* rather than guessed at — "Seeed XIAO
ESP32C3 on COM3" instead of "COM3 (CP210x)".

#### A chip entry says what a capability needs, so silence is a refusal

Two of a chip's fields are optional, and their absence is the answer rather
than a gap:

| Field | Absent means |
|---|---|
| `probe_rs_target` | rusty will not guess a probe-rs name; it tells the user to run `probe-rs chip list` |
| `hal` | rusty will not offer to switch a project to or from this part |

`hal` names the crate a project selects the part through, **and asserts that
selecting it means putting this chip's `id` in that crate's feature list**.
That is exactly how esp-hal works, so every Espressif entry carries
`hal = "esp-hal"` and switching between any two of them is mechanical: a
feature name, a target triple, a toolchain channel, `build-std`.

It is not how the STM32 HALs work — `stm32f1xx-hal` wants `stm32f103c8`, a full
part number the die alone does not determine, and F103 and F411 are different
crates besides. So those entries carry no `hal`, and the chip switch refuses
with the reason rather than rewriting four files into a project that cannot
build. Adding a part to the catalogue is therefore **safe by default**: it
works everywhere else in the workbench immediately, and offers a migration only
once somebody states how a project names it.

This is the general shape to copy when adding a capability that only some parts
can support: put the precondition in the data as an optional field, let its
absence refuse, and say why in the refusal. The alternative — a `match` on chip
id in the code — is a capability that silently does the wrong thing for the
next part somebody adds.

### What this does not change

Code extensions still go through MCP, UI contributions are still declarative,
and there is still no bespoke plugin runtime. The categories below stand.

## The question

Does rusty need a plugin system designed up front, or can one be added later
without a rewrite?

## Decision

**No bespoke plugin runtime now.** Three cheap commitments instead, each of
which is expensive to retrofit and nearly free to make today.

The reasoning: retrofitting a plugin *runtime* (wasmtime, a process host) is
additive work — it does not disturb what already exists. What genuinely forces a
rewrite is retrofitting the things a runtime sits on top of:

| Actually expensive to retrofit | Status |
|---|---|
| Stable, serializable data contracts | Done — `rusty-core::model` is an explicit wire contract |
| An invocation seam third parties can plug into | Done — `ToolRegistry` |
| A declared permission model | Done — `Capabilities` |
| A UI contribution model | **Committed below, not yet implemented** |
| Async, out-of-process-safe boundaries | Done — everything crosses JSON |

Four of five were already satisfied, because the analysis layer was built for
three consumers from the start (desktop UI, CLI, MCP). Designing for more than
one consumer is what bought the extensibility; it was not a separate effort.

## Commitment 1 — everything crosses a JSON boundary

No frontend or extension surface receives a Rust type it cannot also receive as
JSON. The Tauri command layer never hands out `PackageGraph`, `PackageMetadata`,
or any other guppy handle — only `model` types.

This is what makes the in-process/out-of-process question a later decision
rather than a foundational one.

## Commitment 2 — the tool registry is the extension registry

`ToolRegistry::register` is the seam. A tool is a name, a description, a JSON
Schema, a declared `Capabilities`, and a `call(args) -> Value`. That is already
a plugin interface: no shared memory, no ABI, no linkage.

Tools from outside are namespaced by source (`mcp__<server>__<tool>`), so a
third party can never shadow a built-in. Silent shadowing of `workspace_report`
would be both a correctness bug and an attack.

### Third-party analyses go through MCP, not a plugin API

The most common thing someone will want to extend is *an analysis* — a new
check, a company-internal policy, a different registry. For that category we
intend to be an **MCP client** rather than to grow a plugin runtime.

This is not a shortcut. It means:

- no sandbox to write, no ABI to version, no runtime to maintain;
- extensions are ordinary programs in any language;
- an ecosystem that already exists.

And symmetrically, rusty should **expose** its own tools as an MCP server, so
the same analyses are available inside Claude Code, Cursor, or anything else
that speaks MCP. For an open-source project that is a distribution channel far
larger than its own install base.

If MCP turns out to be too coarse for some category, WASM (wasmtime component
model, as Zed does) remains available. It is additive to everything above.

## Commitment 3 — UI contributions are declarative, never arbitrary

**Extensions describe what to show. The host decides how it looks.**

A contributed panel is a descriptor — a table, a list, a form, a chart spec,
bound to a tool that supplies the data. It is rendered with rusty's own
components. Extensions cannot ship HTML, cannot ship CSS, and cannot reach the
DOM.

This is the one commitment with a real cost, and it is deliberate:

- **Visual coherence is a product requirement.** rusty's case for existing is
  partly that it is a well-designed workbench. VSCode's model — arbitrary HTML
  in a webview per extension — means every panel looks like a different
  application. That trade is acceptable for a universal editor and is not
  acceptable here.
- **The API surface stays small enough to keep stable.** An extension API
  shaped like "here is a DOM, do what you like" can never be versioned.
- **Theming, light/dark, and accessibility keep working** without every
  extension author having to re-solve them.

### Consequence for M1

The panel layer is built **data-driven from the first commit**, even though
every panel in M1 is built in. Panels register a descriptor; the shell renders
from the registry. There is no hardcoded list of five views anywhere.

This is the specific thing that would otherwise force a rewrite: a shell that
hardcodes its panels cannot later accept panels it does not know about.

## Extension point taxonomy

Decided now so the UI can be built against it. Implementation is deferred.

| Point | Mechanism | Status |
|---|---|---|
| **Chips and boards** | **TOML, three layers** | **done** |
| Analyses / checks | Tool, via MCP | seam exists |
| Panels & views | Declarative descriptor + backing tool | committed |
| Command palette entries | Command descriptor → tool | committed |
| Vendors | Extend the catalogue + `Vendor::chip_feature_crates` | seam exists |
| Flashing recipes | `CommandPlan` is already data; templatable | later |
| Log decoders | Tool, via MCP | later |
| Diagnostics providers | Tool emitting `model::Problem` | later |
| Themes | Design token override file | later |
| Language support | Not an extension point — rusty is Rust-only | never |

## What would prove this decision wrong

- An extension category emerges that genuinely needs its own rendering, and the
  declarative descriptor set keeps growing to chase it. Then reconsider a
  sandboxed UI surface — but reconsider it as an explicit product trade, not by
  quietly adding an HTML escape hatch.
- MCP's per-call overhead turns out to matter for analyses invoked on every
  keystroke. Then that category moves in-process via WASM, while everything
  else stays on MCP.

## Simulator parts

The Simulate panel's part library extends the same way everything else does:
data first. A file in the project's `.rusty/parts/` adds a part:

```toml
# .rusty/parts/relay.toml
name = "relay"
color = "red"      # glow hue: green, blue, red, yellow
```

A part defined this way appears in the library's Custom section and behaves
as a lamp on the gpio report channel. Everything on the board rides two
serial directions:

| line | direction | meaning |
|---|---|---|
| `[rusty:gpio] 26=1,27=0` | firmware → board | pin levels; lamps, RGB mixes and 7-segment digits light from these |
| `[rusty:disp] tick 42` | firmware → board | text for the display part; empty payload clears it |
| `[rusty:tel@1234] gyro_x=1.25,pid_p=-0.5` | firmware → Plot | named numeric channels; the stamp is the firmware's own clock in µs |
| `[rusty:param] kp=2 0..20` | firmware → Plot | "this is a tunable, this is what I hold, this is the range I take" |
| `B14=1` / `B14=0` | board → firmware | button pressed / released |
| `P34=200` | board → firmware | potentiometer moved, 0..255 |
| `Skp=8.5` | Plot → firmware | set a tunable; the firmware answers with the `[rusty:param]` line above |

The firmware side needs nothing but `println!` for the reports and a UART
read loop for the commands — `examples/pid-tune` is the worked example, and
it is a whole tuning loop in 200 lines.

Two rules the telemetry half keeps, both because breaking them makes a plot
that lies rather than a plot that is empty. The panel draws **no slider
without a range**, because a range the tool invented is how somebody sends a
gain of 500 to a motor loop; and a set is answered with what the firmware
*took*, so a clamped value reads as clamped rather than as the number that
was typed.

Writing back needs a port rusty holds open itself — the Plot panel's Connect
does that. `espflash monitor` reads its keyboard through the console rather
than through stdin, so a monitor rusty spawned can only listen; its telemetry
still plots, and its tunables are read-only. Reading richer behaviours
(framebuffers, protocol decoders) grows on these same directions. What does
not exist yet, honestly: I2C/SPI decoding, analog waveform views, and Wokwi
diagram import.
