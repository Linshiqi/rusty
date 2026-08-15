# rusty

An embedded Rust workbench. ESP32 first, STM32 next.

rusty owns the half of embedded Rust that a general-purpose editor does not:
which chip you are targeting, whether your toolchain can actually build for
it, what is filling your flash, what is on the serial port, what the loop is
doing while it runs — and an assistant that can call every one of those
analyses instead of guessing at them.

It edits code too: files, highlighting, and rust-analyzer behind it for
completion, diagnostics and navigation, with optional Vim keys. A tool you
cannot change a file in is a dashboard about work you do somewhere else.

**Download:** [the latest release](../../releases/latest) — Windows installer,
macOS universal DMG, Linux `.deb` or `.AppImage`. Nothing is code-signed yet,
so SmartScreen and Gatekeeper will both object the first time.

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

## What it does

- **Project check** — reads `Cargo.toml`, `.cargo/config.toml`,
  `rust-toolchain.toml` and cross-checks them. They routinely disagree, and when
  they do the compiler blames none of them.
- **Toolchain** — what is installed versus what this project needs. Xtensa
  parts need espup's forked LLVM; RISC-V ones do not, and conflating the two
  sends half of ESP32 users to install something they will never use.
- **Memory** — flash and RAM per section *and per crate*, against the chip's
  real capacity.
- **Flash and monitor** — espflash or probe-rs, with defmt decoding, and the
  command shown before it runs.
- **Feature matrix** — what a Cargo feature actually costs after workspace-wide
  unification. On a microcontroller that is not tidiness, it is whether the
  binary fits.
- **Boards** — a plugged-in device is named, not guessed at.
- **Assistant** — bring your own LLM. Keys live in the OS credential store and
  never reach the WebView.

## Bring your own model

Anthropic, OpenAI, DeepSeek, Moonshot/Kimi, Zhipu GLM, DashScope/Qwen,
SiliconFlow, OpenRouter — plus Ollama, LM Studio, and vLLM running on your own
machine, where nothing leaves the device.

## Adding your board

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
forking.

## Try it headless

```bash
cargo run -p rusty-cli -- check .
```

Exits non-zero if anything blocking was found, so it drops into CI. Add
`--json` for the same payload the desktop app renders.

## Building

Rust 1.88+, and [Trunk](https://trunkrs.dev) for the frontend. There is no Node
in this repository — Trunk drives the wasm build and fetches the standalone
Tailwind binary itself.

```bash
cargo test --workspace
cd crates/rusty-app && cargo tauri dev
```

## Contributing

Bug reports, "this was confusing", and hardware reports from parts that are
not an ESP32-C3 are the most useful things you can send. **Please open an
issue before writing code** — see [CONTRIBUTING.md](CONTRIBUTING.md) for why
a one-person project has to work that way.

## Licence

[PolyForm Noncommercial 1.0.0](LICENSE.md). Read it, run it, change it, share
your changes — but not commercially. Open an issue if you want a commercial
arrangement.
