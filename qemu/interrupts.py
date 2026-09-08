"""Everything the interrupt path needs that Espressif's QEMU does not have.

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

Anchored insertions rather than patch files: what changes here is a handful
of lines inside functions of hundreds, so a unified diff would be mostly
context and its line numbers the fragile part. Each edit finds the text it
is inserting after, insists on seeing it exactly once, and stops otherwise —
the same discipline as `upstream.sha256`, which pins all three files so a
rewrite upstream fails loudly here rather than quietly doing nothing.

The ESP32's matrix has the same hole and is not fixed here: it keeps no
level state at all (`hw/xtensa/esp32_intc.c` forwards straight to the CPU's
external lines), and its status registers live in a different device
altogether, so answering them means new state and a link between two
upstream models — with no gate in this repository that could prove it. The
line is still wired on that machine, which is what ESP-IDF-style firmware
dispatching on the CPU line needs. `qemu/README.md` says so rather than
letting the release imply otherwise.

    python qemu/interrupts.py <path to the qemu source tree>
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
