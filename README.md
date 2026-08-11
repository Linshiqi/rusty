# rusty

An embedded Rust workbench. ESP32 first, STM32 next.

Not an editor. rusty owns the half of embedded Rust that an editor does not:
which chip you are targeting, whether your toolchain can actually build for it,
what is filling your flash, what is on the serial port — and an assistant that
can call every one of those analyses instead of guessing at them.

> Status: engines complete and tested, frontend in progress.

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

## Licence

MIT OR Apache-2.0
