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

## Licence

QEMU is GPL-2.0. These files are derivatives of it and carry the same terms,
not rusty's licence. They are kept here, outside the cargo workspace and
applied at build time, so that stays unambiguous.

## Not shipped

The binaries are artifacts to test against. Shipping them means committing to
building a QEMU fork for three desktops on every upstream bump, so the app
still downloads Espressif's build and the board view's caption — pin levels
are what the firmware *says* it set — stays true of what users run.
