# rusty — an embedded Rust workbench

A desktop IDE for embedded Rust, ESP32 first. It knows which chip a project is
for, which toolchain it needs, what fits in flash, what is on the serial port —
and it runs firmware without hardware.

**This repository holds downloads and issues only.** The source is not public
yet. Everything here is built from it automatically.

## Install

Grab the installer for your desktop from the
[latest release](../../releases/latest):

| Platform | File |
|---|---|
| Windows | `.exe` (NSIS installer) |
| macOS | `.dmg` (one image, Intel and Apple Silicon) |
| Linux | `.deb`, or `.AppImage` to run without installing |

`rusty-cli-*` is the same analysis without a window, for CI and for bug
reports.

Nothing is code-signed yet, so Windows SmartScreen and macOS Gatekeeper will
both object the first time. On macOS: right-click the app → Open. On Windows:
More info → Run anyway.

## What it does

- **Edits.** Files, highlighting, and rust-analyzer behind it — completion,
  diagnostics, hover, go-to-definition, quick fixes, project-wide search.
- **Knows the chip.** Target triple, toolchain channel, `build-std`, the
  `esp-hal` feature — checked against each other, and it says which one is
  wrong rather than that the build failed. Switching a project's chip rewrites
  all four and tells you the pins it cannot rewrite.
- **Flashes and monitors** over espflash or a probe, with defmt decoded.
  Serial ports are named by the board they look like.
- **Runs without hardware.** The same image espflash would burn, booted in
  Espressif's QEMU, with a board view that lights from what the firmware
  prints and buttons that travel back to it.
- **Debugs.** Breakpoints, stepping, stack, variables, and the chip's
  registers read live from the vendor's SVD.
- **Tunes a running loop.** Named telemetry channels plotted live, and gains
  changed **without a reflash**.
- **Meets C**, because vendor SDKs are C: detection, and scaffolding in both
  FFI directions.

## Reporting something

[Open an issue](../../issues/new/choose). What helps most:

- The version, from the title bar or `rusty-cli --version`
- Your chip and board
- What you expected, and what happened instead
- The Output panel's text if a command failed — right-click → Copy

Feature requests are welcome in the same place. So is "this was confusing",
which is worth as much as a crash.

## Not there yet

Stated rather than left to be discovered:

- The board view shows what the firmware *says* it set — QEMU's peripheral
  models expose no GPIO readback, so it cannot show what a pin really is.
- Updating checks and links; it does not install in place.
- STM32 is detected, but not served the way ESP32 is.
- No code signing, hence the warnings above.
