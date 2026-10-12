# rusty

An embedded Rust workbench: Espressif, STM32, RP2040/RP2350, nRF52 and WCH
CH32 parts, and the C and C++ that live beside the Rust.

rusty owns the half of embedded work that a general-purpose editor does not:
which chip you are targeting, whether your machine can actually build for it,
what is filling your flash, what is on the serial port, what the loop is
doing while it runs — and an assistant that calls every one of those
analyses instead of guessing at them.

It edits code too: files, highlighting, rust-analyzer for Rust and clangd for
C and C++ behind them, a Git panel, and optional Vim keys. A tool you cannot
change a file in is a dashboard about work you do somewhere else.

**Download:** [the latest release](../../releases/latest) — Windows installer,
macOS universal DMG, Linux `.deb` or `.AppImage`. Nothing is code-signed yet,
so SmartScreen and Gatekeeper will both object the first time. The installer
carries the emulator, the debuggers and espflash; Rust itself comes from
[rustup](https://rustup.rs), and rusty's setup sheet installs the rest.

## Why

Embedded Rust fails in ways whose error messages point away from the cause.

```
error: toolchain 'stable' does not support target 'xtensa-esp32-none-elf'
```

Nothing in that mentions `espup`, and the fix is not discoverable from it.

```
region `FLASH` overflowed by 4096 bytes
```

That names a number and says nothing about what filled it. `cargo size` gives
you section totals, which still does not name the dependency.

rusty computes both answers, and hands them to the assistant as tools so it
cannot make one up.

## Getting started

1. Install Rust with [rustup](https://rustup.rs) — or skip it for a
   PlatformIO or CMake project, which builds no Rust.
2. Open rusty. On a machine that cannot build yet, the setup sheet lists
   what is missing, in the order it has to be installed, and installs it.
3. Then any of:
   - **Open** a folder with a `Cargo.toml`, a `platformio.ini` or a
     `CMakeLists.txt`.
   - **New project** — pick a part; rusty writes a project that is known to
     build for it, as one crate or as a host-testable workspace beside the
     firmware.
   - **Playground** — code beside a simulated board that runs it, no project
     needed: ESP32-C3, ESP32, CH32V003, CH32X035, and a drawing playground.
4. **Build**, **Run** in the simulator, **Debug**, **Flash** — the title bar,
   or Ctrl+Shift+B, Ctrl+F5, F5 and Ctrl+U.

## What it does

- **Project check** — reads the project's own build files (`Cargo.toml`,
  `.cargo/config.toml`, `rust-toolchain.toml`; `platformio.ini`; CMake and
  the Pico SDK, ESP-IDF or STM32CubeMX) and cross-checks them. They routinely
  disagree, and when they do the compiler blames none of them.
- **Environment** — what is installed against what this project needs:
  targets, nightly where a part builds `core` itself, espup's Xtensa
  toolchain, the flasher, PlatformIO, CMake and the part's cross compiler.
- **Simulator** — the firmware booted in rusty's emulator with a schematic on
  its pins: lamps, buttons, knobs, I2C sensors, SPI, a display, a keypad, LED
  strips, PWM, signal generators played into the ADC, and a circuit solver
  that says the volts and amps. Espressif parts run in rusty's own build of
  QEMU; WCH's CH32 parts in rusty's own emulator.
- **Debug** — in the simulator through gdb, on the board through probe-rs
  (flash, breakpoints, stepping, variables, RTT output), and a host test from
  the lens beside it.
- **Flash and monitor** — espflash, probe-rs, wlink or PlatformIO, one click
  that builds first, with defmt decoded and the command shown before it runs.
- **Memory** — flash and RAM per section, per crate, and per C or C++ source
  file, against the part's real capacity.
- **Pin map** — which pins the part has and which the source already uses,
  named the way the vendor names them.
- **Plot, tune and measure** — telemetry plotted as it arrives, tunables with
  the range the firmware gave, a flight-controller plant to close the loop
  at a desk, and a signal lab: spectrum, filter design, frequency sweeps.
- **Math toolbox** — vectors, quaternions and Euler angles worked out a row
  at a time and drawn as the aircraft they describe; and drawing from your
  own code with the `rusty-draw` crate.
- **Mixed C and Rust** — scaffolding in either direction (Rust calls C or
  C++, C calls Rust), clangd beside rust-analyzer, and C's bytes attributed.
- **Cargo features and disk** — what a feature costs after workspace-wide
  unification, and a build directory swept of what the lockfile no longer
  needs.
- **Git** — history graph, changes, branches, remotes and stashes, the way
  Fork lays them out.
- **Assistant** — bring your own model. Keys live in the OS credential store
  and never reach the WebView.

## Bring your own model

Anthropic, OpenAI, DeepSeek, Moonshot/Kimi, Zhipu GLM, DashScope/Qwen,
SiliconFlow, OpenRouter — plus Ollama, LM Studio, and vLLM running on your own
machine, where nothing leaves the device.

The same analyses serve other assistants over the Model Context Protocol:

```bash
claude mcp add rusty -- rusty-cli mcp /path/to/project
```

## Adding your board or part

Six lines of TOML in `<project>/.rusty/boards/`, checked in so your team gets it
too:

```toml
[[board]]
id = "acme-sensor-node"
name = "ACME Sensor Node rev C"
chip = "esp32c6"
flash_bytes = 16777216
[[board.usb]]
vendor_id = 0x1A86
product_id = 0x55D4
```

Your files layer over the built-ins, so you can correct a shipped entry without
forking. Parts and vendors are data the same way — see
[docs/extensibility.md](docs/extensibility.md).

## Without the window

`rusty-cli` is the same analyses for a terminal or CI:

```bash
rusty-cli check .                 # why it will or will not build; non-zero on a blocker
rusty-cli build .                 # cargo, PlatformIO or CMake, as the window builds
rusty-cli size .                  # where the newest firmware's bytes went
rusty-cli sim . --expect ready    # build, boot in the emulator, watch; 0 passed
rusty-cli disk . && rusty-cli sweep . --apply
rusty-cli mcp .                   # the assistant's tools, for another assistant
```

`--json` gives the payload the desktop app renders; `rusty-cli help <command>`
says the rest.

## Examples

[examples/](examples/README.md) — each one a small, complete project that
proves one thing: a blinking LED, a control loop tuned while it runs, a
quadcopter rate loop flown at a desk, a filter judged against a signal, C and
Rust calling each other, and vectors drawn from code.

## Building

Rust 1.89+, and [Trunk](https://trunkrs.dev) for the frontend. There is no Node
in this repository — Trunk drives the wasm build and fetches the standalone
Tailwind binary itself.

```bash
cargo test --workspace
cd crates/rusty-app && cargo tauri dev
```

## Contributing

Bug reports, "this was confusing", and reports from real hardware — above all
the STM32, RP, nRF and CH32 parts, whose projects build here but have not
met a board — are the most useful things you can send. **Please open an
issue before writing code** — see [CONTRIBUTING.md](CONTRIBUTING.md) for why
a one-person project has to work that way.

## Licence

[PolyForm Noncommercial 1.0.0](LICENSE.md). Read it, run it, change it, share
your changes — but not commercially. Open an issue if you want a commercial
arrangement.
