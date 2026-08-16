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

`esp-gpio.patch` fills in the model: the output and enable registers, their
set/clear aliases, and the input register. Pin changes leave on their own
chardev — not the UART, which belongs to the firmware — and input can be
driven back in, so unmodified firmware reads a real pin.

## Building it

`.github/workflows/qemu.yml` clones `espressif/qemu` at the tag rusty pins,
applies this patch and builds `riscv32-softmmu`. Run it from the Actions tab;
it uploads the binary as an artifact.

The patch is against the tag in `QEMU_RELEASE` (`crates/rusty-embed/src/simulate.rs`).
When that pin moves, this patch has to be re-checked against the new source —
`git apply` failing loudly is the point.

## Licence

QEMU is GPL-2.0. This patch is a derivative of it and carries the same terms,
not rusty's licence. It is kept as a patch rather than a fork so that stays
unambiguous.
