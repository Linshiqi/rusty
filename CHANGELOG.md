# Changelog

What each release changed, written for the people downloading it. The release
workflow reads the section matching the tag and publishes it verbatim as the
release body — so this file, not the commit log, is what users see. The commit
log stays in the private repository where it belongs.

One `## v<version>` heading per release, newest first.

## v0.2.0

The first public build, and the first with the source published. An embedded
Rust workbench for ESP32: it knows which chip a project is for, what fits in
flash, what is on the serial port, and it runs firmware without hardware.

**New since the last build**

- **Vim keys**, off by default, switched on in Settings > Editor. Modes,
  motions, operators, text objects, visual mode and `.`, with the mode in the
  status bar. Ctrl+S, Ctrl+A, Ctrl+C and the rest are untouched, and insert
  mode behaves exactly as it does with this off.
- **A way back.** Jumping to a definition had none: Back and Forward now walk
  the positions the caret has visited, from the View menu, `Alt+←/→`, or
  Vim's `Ctrl+O`/`Ctrl+I`.
- **Live telemetry and tuning.** The Plot panel draws named channels from a
  running board and changes gains **without a reflash**, over the serial line
  the firmware is already printing to.
- **Completion while you type a name**, not only after a dot.
- **Hovering a squiggle** answers what is wrong with it, not what type it is.

**Editing** — files, syntax highlighting, and rust-analyzer behind it for
completion, diagnostics, hover, go-to-definition, signature help and quick
fixes. Multi-tab, project-wide search on ripgrep's engine, format on save.

**The device** — flash and monitor over espflash or a probe, with defmt
decoded. Serial ports are named by the board they look like rather than by
their COM number. A memory report attributes flash and RAM per crate from the
ELF, and refuses to guess a capacity for a chip it does not know.

**Without hardware** — build, image and boot the same binary espflash would
burn, in Espressif's QEMU. A board view lights LEDs, digits and displays from
what the firmware prints; buttons and potentiometers travel back the same way.
Waveforms are captured with the firmware's own timestamps and export as VCD.

**Debugging** — breakpoints in the gutter, stepping, the call stack and
variables, and the chip's registers read live from the vendor's own SVD.

**Tuning** — a Plot panel draws named telemetry channels from a running board
and changes gains **without a reflash**, over the serial line the firmware is
already printing to. `examples/pid-tune` is a whole tuning loop in 200 lines.

**Meeting C** — vendor SDKs are C, so the workbench detects `cc`, `bindgen`,
`esp-idf-sys` and C sources, scaffolds both FFI directions, and refuses before
writing over anything.

Known limits, stated rather than discovered: the board view shows what the
firmware *says* it set, because QEMU's peripheral models expose no GPIO
readback. In-app updating checks and links; it does not install. STM32 is
detected but not yet served the way ESP32 is.
