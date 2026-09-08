"""The source edits rusty's emulator needs beyond the two files it replaces.

Three of them today: two on the interrupt path, and one line that maps the
SAR ADC. Each is small enough that a patch file's line numbers would be the
fragile part, and each is silent when missing — the reason every one of them
insists on seeing its anchor exactly once.

## The interrupt path

Two holes, both silent, and firmware waiting on a GPIO edge falls into
whichever it reaches first.

**The GPIO device's interrupt line goes nowhere.** The model in
`esp32_gpio.c` raises an interrupt when a pin fires and `sysbus_init_irq`
gives it a line to raise, but neither machine connects that line, because
the stub it replaces never raised one. `qemu_set_irq` on an unconnected
line returns without doing anything, so the peripheral goes on reporting
interrupts into nothing.

**The C3's interrupt matrix does not say which source is asserting.**
`INTERRUPT_CORE0_INTR_STATUS_0/1` (`0x0f8`, `0x0fc`) are the two registers a
dispatcher reads to find out what to call. ESP-IDF gives each source its own
CPU line and dispatches on the line number, so it never reads them and
upstream never needed them; esp-hal shares one CPU line between sources and
reads them on every interrupt. Answering zero means the CPU takes the
interrupt, esp-hal finds nothing pending, and returns — indistinguishable,
from the firmware's side, from a line that was never raised. That was this
gate failing while the model's own witness said it had raised the line.

The ESP32's matrix has the same second hole and is not fixed here: it keeps
no level state at all (`hw/xtensa/esp32_intc.c` forwards straight to the
CPU's external lines), and its status registers live in a different device
altogether, so answering them means new state and a link between two
upstream models — with no gate in this repository that could prove it. The
line is still wired on that machine, which is what ESP-IDF-style firmware
dispatching on the CPU line needs. `qemu/README.md` says so rather than
letting the release imply otherwise.

## The analog one

**Nothing is mapped at the C3's SAR ADC.** `esp32_gpio.c` answers for it —
the analog value on a pin and its digital level are two readings of one
wire, arriving on one channel — as a second MMIO region, so the machine
needs one line to map it. Until it was mapped, `adc.read_blocking` did not
return a wrong number: it polled a done bit nothing could set, and the
firmware hung in the user's own code.

    python qemu/patches.py <path to the qemu source tree>
"""

import sys
from pathlib import Path

# (file, the text to insert after, what to insert). Both machines already
# have `intmatrix_dev` in scope at that point and already include the header
# that defines `ETS_GPIO_INTR_SOURCE`; both facts are what make the wiring
# two lines rather than a rewrite.
EDITS = [
    (
        "hw/riscv/esp32c3.c",
        "        memory_region_add_subregion_overlap(sys_mem, DR_REG_GPIO_BASE, mr, 0);\n",
        "        sysbus_connect_irq(SYS_BUS_DEVICE(&ms->gpio), 0,\n"
        "                           qdev_get_gpio_in(intmatrix_dev, ETS_GPIO_INTR_SOURCE));\n",
    ),
    (
        "hw/xtensa/esp32.c",
        "    esp32_soc_add_periph_device(sys_mem, &s->gpio, DR_REG_GPIO_BASE);\n",
        "    sysbus_connect_irq(SYS_BUS_DEVICE(&s->gpio), 0,\n"
        "                       qdev_get_gpio_in(intmatrix_dev, ETS_GPIO_INTR_SOURCE));\n",
    ),
    # Anchored on the same upstream line as the wiring above, not on the
    # wiring's own text, so the two are independent: an edit that anchored on
    # another edit's output would break the moment somebody reordered this
    # list, and break by silently doing nothing.
    (
        "hw/riscv/esp32c3.c",
        "        memory_region_add_subregion_overlap(sys_mem, DR_REG_GPIO_BASE, mr, 0);\n",
        "        /* The same device's other regions: the SAR ADC and the I2C\n"
        "         * master. One model, because all three carry the host's view\n"
        "         * of the board and they share its channel. */\n"
        "        memory_region_add_subregion_overlap(sys_mem, DR_REG_APB_SARADC_BASE,\n"
        "            sysbus_mmio_get_region(SYS_BUS_DEVICE(&ms->gpio), 1), 0);\n"
        "        memory_region_add_subregion_overlap(sys_mem, DR_REG_I2C_EXT_BASE,\n"
        "            sysbus_mmio_get_region(SYS_BUS_DEVICE(&ms->gpio), 2), 0);\n"
        "        /* And SPI2, the one a project puts a display on. SPI1 is the\n"
        "         * flash controller and stays upstream's. */\n"
        "        memory_region_add_subregion_overlap(sys_mem, DR_REG_SPI2_BASE,\n"
        "            sysbus_mmio_get_region(SYS_BUS_DEVICE(&ms->gpio), 3), 0);\n",
    ),
    (
        "hw/riscv/esp32c3_intmatrix.c",
        "#define SET_BIT(reg, bit)   do { (reg) |= BIT(bit); } while(0)\n",
        "\n"
        "/* INTERRUPT_CORE0_INTR_STATUS_0/1: which interrupt *sources* are\n"
        " * asserting, one bit each, sources 0..31 and 32..61. The state is\n"
        " * already kept in `irq_levels`; only the two registers reading it are\n"
        " * missing. */\n"
        "#define ESP32C3_INTMATRIX_IO_STATUS0_REG (0x0f8 / sizeof(uint32_t))\n"
        "#define ESP32C3_INTMATRIX_IO_STATUS1_REG (0x0fc / sizeof(uint32_t))\n",
    ),
    (
        "hw/riscv/esp32c3_intmatrix.c",
        "        r = s->irq_enabled;\n",
        "    } else if (index == ESP32C3_INTMATRIX_IO_STATUS0_REG ||\n"
        "               index == ESP32C3_INTMATRIX_IO_STATUS1_REG) {\n"
        "        /* A dispatcher that shares one CPU line between sources reads\n"
        "         * these to learn what to call. Raw, as the silicon reports\n"
        "         * them: before the mapping and before the enable, which the\n"
        "         * caller applies itself. */\n"
        "        const unsigned half =\n"
        "            (unsigned)(index - ESP32C3_INTMATRIX_IO_STATUS0_REG);\n"
        "        r = (uint32_t)(s->irq_levels >> (half * 32));\n",
    ),
]

def read(path):
    """The file's bytes as text, with its line endings left alone.

    `newline=""` on both halves, because Python's default is to translate
    on the way in *and* on the way out: on the Windows runner a read-change-
    write of an LF file rewrites every line of it. Nothing here breaks if
    that happens, which is precisely why it would go unnoticed.
    """
    with open(path, encoding="utf-8", newline="") as handle:
        return handle.read()


def write(path, text):
    with open(path, "w", encoding="utf-8", newline="") as handle:
        handle.write(text)


root = Path(sys.argv[1])
for name, anchor, addition in EDITS:
    path = root / name
    text = read(path)
    if addition in text:
        print(f"{name}: already patched")
        continue
    found = text.count(anchor)
    if found != 1:
        print(
            f"::error::{name}: the line this inserts after appears {found} times, "
            f"not once. Espressif has moved it; read the file and update "
            f"qemu/interrupts.py.",
            file=sys.stderr,
        )
        raise SystemExit(1)
    write(path, text.replace(anchor, anchor + addition))
    print(f"{name}: patched")
