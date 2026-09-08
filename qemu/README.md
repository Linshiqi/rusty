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
- **And it can never interrupt.** Firmware that asks to be woken by an edge
  — how nearly every real button is read — waits for ever, which looks like
  a hang in the user's own code rather than a hole in the emulator.
- **And there is no analog side, and no bus.** Nothing answers at the SAR
  ADC's registers or the I2C master's either, so `adc.read_oneshot()` polls a
  done bit nothing can set and a driver's first transaction waits on an
  interrupt nothing can raise. Same shape of failure, in the user's own
  `read` call.

`esp32_gpio.c` here fills in the model: the output and enable registers,
their set/clear aliases, the input register, and the interrupt half — the
status register, its set/clear aliases, the CPU's pending view and each
pin's trigger configuration, with the edge types latched and the level
types following the level, as the silicon does. All forty pins, since the
original ESP32 keeps GPIO32..39 in a second bank and that is where its
input-only pins live. Pin changes leave on their own chardev — not the
UART, which belongs to the firmware — and input can be driven back in, so
unmodified firmware reads a real pin.

Replacement files rather than a patch: what is being replaced is a stub, so
almost every line changes and a unified diff's line numbers would be the
fragile part of an otherwise total substitution. `upstream.sha256` does the
job a patch's context would — if Espressif edits either file, the build stops
and says so rather than silently discarding their change.

## Two holes between the pin and the handler

A model that raises an interrupt is not yet an interrupt the firmware takes,
and both of the things in the way fail *silently* — the firmware simply
never runs its handler, which reads as a hang in the user's own code.
`patches.py` fills them, as anchored insertions after text it insists on
seeing exactly once; the files are pinned in `upstream.sha256` like the GPIO
ones.

- **Neither machine connects the device's interrupt line.** `sysbus_init_irq`
  gives the model a line, and nothing was ever on the other end of it,
  because the stub never raised one. `qemu_set_irq` on an unconnected line
  returns without doing anything, so the peripheral reports into nothing.
  Two lines, one per machine (`hw/riscv/esp32c3.c`, `hw/xtensa/esp32.c`).
- **The C3's interrupt matrix does not say which source is asserting.**
  `INTERRUPT_CORE0_INTR_STATUS_0/1` are the two registers a dispatcher reads
  to find out what to call, and `hw/riscv/esp32c3_intmatrix.c` answers zero
  for both. ESP-IDF gives each source its own CPU line and dispatches on the
  line number, so it never reads them and upstream never needed them;
  esp-hal shares one CPU line between sources and reads them on every
  interrupt. The state is already there in `irq_levels` — only the two
  registers reading it were missing. Until they answered, the CPU took the
  interrupt, esp-hal found nothing pending, and returned: from the
  firmware's side, identical to a line that was never raised. That is what
  gate 6 caught, with the model's own witness insisting it had raised it.

**The ESP32 gets the first fix and not the second, and its interrupts are
therefore unproven.** `hw/xtensa/esp32_intc.c` keeps no level state at all —
it forwards straight to the CPU's external lines — and its status registers
live in DPORT, a different device, so answering them needs new state and a
link between two upstream models. There is no gate here that could prove
that, and a release note claiming it would be exactly the confident wrong
answer this emulator exists to stop giving. ESP-IDF-style firmware, which
dispatches on the CPU line, has what it needs on that machine.

## And the analog half

A pin is not only high or low. A knob, a divider, a light sensor and a
battery are all read through the SAR ADC, and **nothing is mapped at its
registers on the C3** — so `adc.read_oneshot()` does not return a wrong
number, it waits: the driver polls a done bit that nothing can set, and the
firmware hangs inside the user's own `read` call with nothing anywhere to
say why. Measured, not feared: the analog probe on the stock build prints
`the conversion never finished` and stops, which is the only reason it is
visible at all.

`esp32_gpio.c` answers for it as a second MMIO region, and `patches.py` maps
that region. Deliberately the same device: the analog value on a pin and its
digital level are two readings of one wire, they arrive on the same channel
from the same host, and a separate device would need a link back to this one
for every conversion. A second *file* would also mean a new entry in
upstream's build system, and every one of those is a way for a build to fail
that has nothing to do with what is being modelled.

What it models is one-shot conversion and nothing else: the channel and start
bits, the two data registers, the done bit and its clear. Everything else in
the window is shadowed, so a driver's read-modify-write of a register this
has no opinion about keeps what it put there. A conversion is instant —
the silicon takes microseconds and the driver waits for the bit either way,
so a timer here would only add a way to lose one.

**Counts, not volts**, on the host's side and the model's. rusty does not
know anybody's divider or reference, and a voltage the emulator converted
itself would be a confident number the firmware's arithmetic disagreed with.
`A<pin>=<counts>` puts a value on a pin, `[rusty:adc@<us>] <pin>=<counts>`
reports what was taken off it — per *change*, because a driver polling in a
loop converts thousands of times a second and a line each would drown the
channel the console and the board share.

## And the bus

Almost every real board has something on I2C: a display, an IMU, a
temperature sensor. Nothing was mapped at the C3's I2C master either, so the
first transaction a driver ran waited on an interrupt nothing could raise —
the same shape of failure as the converter, inside the user's own `read`.

The master is modelled as a third region on the same device, and its bus is
**register files the host declares**. A sensor is a set of registers a driver
reads and a display is a stream of bytes somebody wants to see; 256 bytes and
a pointer serve both. `i2c 68:75=68` puts a byte behind an address,
`i2c 3c=+` declares one with nothing to read, `i2c 3c=-` takes it off.

**An address nobody declared is not answered.** The transaction NACKs, which
is what a bus scan needs and what tells "the part is not on this board" from
"the part is there and quiet". Answering zeros instead would make every scan
find every address — worse than finding none, because firmware that probes
for an optional device would find it every time.

`[rusty:i2c@<us>] 3c w 00ae` reports a write, `… 68 r 68` a read and
`… 68 nak` an address that answered nothing. The same transaction twice in a
row is said once: a driver reading an accelerometer at a kilohertz is the
ordinary case, and a line each would put twenty kilobytes a second down the
channel the console and the board share.

## Building it

`.github/workflows/qemu.yml` clones `espressif/qemu` at the tag rusty pins
(read out of `QEMU_RELEASE`, wherever in `rusty-embed` it lives — searched
for rather than named, because naming it is how this workflow sat broken
through a refactor that moved the file), verifies the checksums, copies the
two model files in, runs `patches.py` over the three it patches, and
builds both `riscv32-softmmu` and `xtensa-softmmu` — the C3 and C6 on one,
the ESP32 and S3 on the other. Run it from the Actions tab; it packages each
platform as an artifact.

## What it is proven to do

Nine gates, each able to fail:

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

6. A pin edge **interrupts** the firmware, on the C3. `irq-probe/` asks to
   be woken by both edges of GPIO4 and never reads the pin outside its
   handler, so a printed count is an interrupt the peripheral raised and the
   CPU took — the one thing polling cannot fake. It must also be *quiet*
   until the pin moves, and fire once per edge: a model that raised the line
   on configuration, or raised it and left it raised, passes neither.

   Three independent accounts, because "nothing happened" is the least
   useful sentence a gate can end on and this one ended on it twice. The
   device announces every change of its interrupt line on the pin channel
   (`[rusty:irq@<us>] 4`, or `unconnected` when nothing is on the other end
   of the line). The probe prints what it programmed — the pin's
   configuration, the matrix's routing and enables, the machine CSRs — and
   the interrupt latch again whenever it moves. And it counts the handler's
   *entry* separately from its completion, so a handler that ran and died is
   not reported as one that never ran. Its panic handler talks, for the same
   reason. Each of those was added the round after a failure that could not
   say which half was broken; the second of them is what found the interrupt
   matrix's missing status registers, with the model insisting it had raised
   the line and the firmware insisting it had never been interrupted.

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

7. The **board** lights. `board_probe` boots `examples/blink-rust` with its
   own `.rusty/sim.toml`, replays the sheet's rules over the pins the
   emulator reports, and requires every lamp wired to a GPIO to have lit
   and gone out. That is the panel's whole claim — this LED is on that pin
   — checked by a machine instead of by somebody looking at it.

8. An **analog value** on a pin reaches `adc.read_oneshot()`. `adc-probe/`
   configures ADC1 channel 3 and prints every reading that changes; the test
   drives `A3=1234`, then `3000`, then `99999`, and requires the firmware to
   have read 1234, 3000 and 4095 — two values because a converter answering
   with a constant passes any single one, and a third out of range because a
   slider dragged past full scale must read as full scale rather than wrap
   to nothing. The emulator's own `[rusty:adc]` account has to carry the same
   numbers.

   The failure it exists for is worse than a wrong reading. With nothing
   mapped at the converter's registers the driver polls a done bit nothing
   can set, and the firmware hangs inside the user's own `read` call. That
   is what the stock build does, measured before the model was written —
   which is why the probe's poll is bounded and prints `the conversion never
   finished` instead of spinning: a witness that reports a hang as silence
   is no witness.

9. A **device on the I2C bus** answers the firmware's driver. `i2c-probe/`
   watches three addresses, each carrying its own assertion: a sensor whose
   registers the host loaded, which must answer *those bytes* and answer six
   of them from one register onwards; a display, whose writes must reach the
   host; and an address nobody declared, which must **not** answer. The last
   is the one the others cannot make — a bus where every address
   acknowledges is a bus where a missing part looks exactly like a present
   one. Then the sensor is taken off the bus and must stop answering, which
   a scan that remembered rather than asked would fail.

   Its `SoftwareTimeout` is there for the same reason the analog probe's
   poll is bounded: without it the driver's first transaction hung, and the
   probe reported the hole as silence.


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
only if every gate passes does it attach the packages to a Release.
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
