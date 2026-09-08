"""Wire the GPIO device's interrupt line to the interrupt matrix.

The model in `esp32_gpio.c` raises an interrupt when a pin fires, and
`sysbus_init_irq` gives it a line to raise — but neither machine connects
that line to anything, because the stock model never raised it. Two lines,
one per machine, and firmware waiting on a GPIO interrupt runs.

An anchored insertion rather than a patch file: what changes here is two
lines inside functions of a thousand, so a unified diff would be mostly
context and its line numbers the fragile part. This finds the text it is
inserting after, insists on seeing it exactly once, and stops otherwise —
the same discipline as `upstream.sha256`, which pins these two files so a
rewrite upstream fails loudly here rather than silently doing nothing.

    python qemu/wire-gpio-irq.py <path to the qemu source tree>
"""

import sys
from pathlib import Path

# (file, the text to insert after, what to insert). Both machines already
# have `intmatrix_dev` in scope at that point and already include the header
# that defines `ETS_GPIO_INTR_SOURCE`; both facts are what make this two
# lines rather than a rewrite.
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
]

root = Path(sys.argv[1])
for name, anchor, addition in EDITS:
    path = root / name
    text = path.read_text(encoding="utf-8")
    if addition in text:
        print(f"{name}: already wired")
        continue
    found = text.count(anchor)
    if found != 1:
        print(
            f"::error::{name}: the line the GPIO interrupt is wired after appears "
            f"{found} times, not once. Espressif has moved it; read the file and "
            f"update qemu/wire-gpio-irq.py.",
            file=sys.stderr,
        )
        raise SystemExit(1)
    path.write_text(text.replace(anchor, anchor + addition), encoding="utf-8")
    print(f"{name}: GPIO interrupt wired to the matrix")
