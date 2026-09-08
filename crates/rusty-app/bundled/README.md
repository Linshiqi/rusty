# Bundled tools

What the installer ships beside the app, so that a fresh install is a
workbench rather than a list of things to go and fetch. `scripts/bundle-tools.sh`
fills this directory and the release workflow runs it before every build.

- `qemu/` — rusty's build of Espressif's QEMU, the one with the GPIO, ADC,
  I2C and SPI models.
- `riscv32-esp-elf-gdb/`, `xtensa-esp-elf-gdb/` — Espressif's debuggers,
  where they publish them. Not macOS, which has CodeLLDB instead.
- `espflash/` — the flasher, prebuilt, so a machine that has just installed
  Rust does not have to compile it first.
- `codelldb/` — the LLDB adapter, on Windows and macOS. Not on Linux: it
  carries a hundred and thirty megabytes of host LLDB, and the AppImage
  bundler walks every ELF among the app's resources — which is how QEMU's
  firmware directory once ended the Linux build. That desktop has esp-gdb in
  the bundle already.

**Rust itself is deliberately absent** — rustup, cargo, the standard library
and espup's Xtensa fork. Those belong in the user's own `~/.cargo` and rustup
home: which toolchain a project needs is decided by its `rust-toolchain.toml`,
rustup is the only thing that installs them correctly, and a copy frozen into
an installer goes stale in six weeks. The first-run setup screen asks for
those, which is the honest shape for a dependency the user has to own.

**The bundle is a fallback, not a preference.** `rusty_embed::tools::find`
reaches it after the data directory, cargo's bin and PATH, so a copy the user
installed on purpose still wins. The one exception is rusty's QEMU: a stock
`qemu-system-riscv32` wears the same name and has none of the peripherals, so
letting one on PATH win would quietly take the board view apart.

None of it is committed — three hundred megabytes of binaries belong in a
release asset, not in git. Without it (a checkout that never ran the script)
the app falls back to the data directory and then to the download ladder, as
it always did.

`PLATFORM` names the target the binaries were built for; a bundle for another
architecture is ignored rather than tried and blamed on the firmware.
