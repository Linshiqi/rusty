# The GPIO the stock emulator does not have

Espressif's QEMU boots our firmware faithfully — the CPU, the UART, the
timers are all real — but its GPIO peripheral is a stub. In
`hw/gpio/esp32_gpio.c` the write handler is *empty*, and reads answer only
the boot-strapping register:

```c
static void esp32_gpio_write(void *opaque, hwaddr addr,
                       uint64_t value, unsigned int size)
{
}
```

So nothing is stored, and that is why probing the real register addresses
over QMP reads zero on both esp32 and esp32c3: there is no state to read.

Two consequences, and they are the ceiling on everything the board view can
honestly claim:

- **The firmware has to narrate its own pins.** A LED lights because the
  firmware printed `[rusty:gpio] 0=1`, not because a pin went high. Code
  that does not print tells the simulator nothing.
- **A pin cannot be read at all.** `Input::new(…).is_high()` is always
  false in the simulator, which is why a button press has to be injected as
  `B14=1` over the UART instead of through the GPIO the firmware actually
  reads.

`esp32_gpio.c` here fills in the model: the output and enable registers,
their set/clear aliases, and the input register. Pin changes leave on their
own chardev — not the UART, which belongs to the firmware — and input can be
driven back in, so unmodified firmware reads a real pin.

Replacement files rather than a patch: what is being replaced is a stub, so
almost every line changes and a unified diff's line numbers would be the
fragile part of an otherwise total substitution. `upstream.sha256` does the
job a patch's context would — if Espressif edits either file, the build stops
and says so rather than silently discarding their change.

## Building it

`.github/workflows/qemu.yml` clones `espressif/qemu` at the tag rusty pins
(read out of `QEMU_RELEASE` in `crates/rusty-embed/src/simulate.rs`, not
repeated), verifies the checksums, copies these two files in and builds
`riscv32-softmmu`. Run it from the Actions tab; it uploads the binary as an
artifact.

## What it is proven to do

Five gates, each able to fail:

1. The upstream files still hash to what this was written against.
2. The built binary contains this model — `strings | grep '\[rusty:gpio@'`,
   by string because a sysbus device may not answer `-device help` at all.
3. Booting `examples/blink-rust` puts **real pin reports on the chardev**, so
   `-global driver=esp32.gpio,property=pins,value=pins` does reach a device
   the machine created, and the model does see the `W1TS`/`W1TC` writes
   esp-hal makes — it never touches `GPIO_OUT`, so a model handling only that
   register would have passed 1 and 2 and reported nothing.
4. The emulator's account of GPIO0 alternates **and contains the firmware's
   own** `[rusty:gpio]` narration of the same pin, in order.

5. A level driven **from the host** reaches the firmware's `is_high()`.

Gate 4 is what makes 3 mean something: a model reporting a stuck level, or
the wrong pin, passes everything above it. The two accounts are independent —
one is the register file, the other is a `println!` — so agreement is
evidence and disagreement names which is wrong.

Gate 5 is the other direction, and it needs its own firmware because blinky
never reads a pin. `gpio-probe/` configures GPIO4 as an ordinary input and
prints its level on change, knowing nothing about the simulator; the test
drives `4=1` then `4=0` down the chardev and requires the firmware to have
read 0, 1, 0. Both writes, because a model that ORed every write into the
input register would pass a test that only ever drove a pin high.

It first ran as:

```
emulator : [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]
firmware : [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0]
```

and the check rejected it for disagreeing on the first level. It was wrong to.
`Output::new(peripherals.GPIO0, Level::Low, …)` drives the pin *before* the
loop that prints, so the emulator legitimately holds one transition the
firmware never announced — the model was rejected for being more truthful
than the firmware, which is the entire reason it exists. The check now aligns
the two and reports the lead.

## What each desktop needed

Nothing, on Linux and macOS. Windows needed four things, and one platform
each needed a dependency nobody had named — none of it in the emulator, whose
own source compiled clean on the first attempt that reached it, everywhere:

- **A prefix with a drive letter.** QEMU's configure defaults the mingw prefix
  to `/qemu` and meson 1.5 will not call that absolute, so it stops at
  `../meson.build:1:0` before reading anything else. Nothing is installed from
  here, so this only has to satisfy the validation.
- **`--disable-guest-agent`.** qemu-ga's VSS provider needs Microsoft's Volume
  Shadow Copy SDK. It is a program that runs inside a guest OS and has nothing
  to do with the emulator.
- **`static: false` on the slirp dependency.** Espressif's fork writes
  `dependency('slirp', …, static: true)` where upstream 9.2 does not. On MSYS2
  that resolves libslirp's `Libs.private` statically and pulls in
  `libglib-2.0.a` while QEMU links glib as a DLL, so the final link dies in
  289 lines of `multiple definition of g_*`. Not something to silence with
  `--allow-multiple-definition`: two glibs in one binary is two allocators and
  two main contexts.
- **`--disable-debug-info`**, to keep a very large PE within what mingw's ld
  handles. Nothing shipped needs it.

And **libgcrypt**, which only appeared once the xtensa target was added:
`hw/misc/esp32_flash_enc.c` models that part's flash encryption and includes
`<gcrypt.h>`. Ubuntu's runner image happens to ship the headers and the other
two do not, so the Linux build passed while macOS and Windows failed on a
dependency nobody had chosen — this project's own recorded trap, and it fails
first on the platform you were not looking at. Named on all three now.

That one `static: true` produced three different failures before it was
found — undefined `__imp_slirp_*` at link time, then `--disable-slirp` having
no effect at all, then duplicate glib. The middle one is worth knowing about
on its own: the fork wraps the dependency in `declare_dependency`, which
always returns a *found* object, so with slirp disabled `net/slirp.c` is still
added to the build while its include path is gone. configure's own summary
says `slirp: disabled` and `slirp support: YES` on the same page.

The behavioural gates stay on Linux. What differs across these platforms is
glib, pixman and ninja, not `s->out |= word`, and booting firmware three times
would mostly test whether espflash and a TCP port behave the same everywhere.

## Licence

QEMU is GPL-2.0. These files are derivatives of it and carry the same terms,
not rusty's licence. They are kept here, outside the cargo workspace and
applied at build time, so that stays unambiguous.

## Shipped

`qemu-release.yml` runs on a `qemu-v*` tag: it calls the build workflow, and
only if every gate passes does it attach the four packages to a Release.
`qemu_download` in `crates/rusty-embed/src/simulate.rs` asks for that Release
first and Espressif's second, so there is nothing for a user to install by
hand and a failure to reach ours degrades to the emulator rusty has always
used rather than to no emulator at all.

Each package carries **both** emulators — `qemu-system-riscv32` for the C3 and
C6, `qemu-system-xtensa` for the ESP32 and S3 — laid out as `qemu/bin` beside
`qemu/share/qemu`, which is Espressif's own layout and therefore a drop-in for
a downloader that already knew how to unpack theirs.

Windows carries its mingw DLLs and macOS is run through `dylibbundler`,
because a dynamically linked build otherwise only runs on the machine that
built it. Linux relies on the system's glib, pixman and slirp, exactly as
Espressif's does; it is built on Ubuntu 22.04, so that is the oldest glibc it
is known to run against.

ARM Linux and Intel macOS are not built here. Espressif publishes both,
`qemu_download` says so rather than 404ing, and those users get the stock
emulator with everything working as it always has, minus real pin state.

Intel macOS is absent for a dull reason worth recording: GitHub retired the
`macos-13` runner, so that job sat queued for 103 minutes while the other
three finished in four to fifteen, and would never have been picked up.
Adding it back needs a runner label somebody has watched work — guessing one
costs an hour of queue to disprove.

## Which one is running

Never assumed. `has_gpio_model` reads the binary for `[rusty:gpio@`, the
marker only this model emits — a version file beside the binary would answer
about the install rather than about the emulator, and a user who dropped
Espressif's build into the same directory would get the wrong answer.

The run then announces `[rusty:pins] emulator` and the board's caption follows
it. That matters more than it sounds: a caption promising register-level truth
over a stock build sends somebody with a dark LED to check their wiring when
the bug is a missing `println!`, and the reverse sends them to re-read
firmware that was right all along.
