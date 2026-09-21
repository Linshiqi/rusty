"""The source edits rusty's emulator needs beyond the two files it replaces.

The interrupt path, the peripheral windows on each machine, and one line in
the CPU. Each is small enough that a patch file's line numbers would be the
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

## The windows a machine has to open

`esp32_gpio.c` answers for seven peripherals — the pins, the SAR ADC, the
I2C master, SPI2, LEDC, RMT and IO_MUX — because all seven carry the host's
view of one board and share its channel. They are MMIO regions of one
device rather than seven devices, so a machine opens each with a line, and
a region no machine maps costs nothing.

**Nothing was mapped on either machine but the pins.** On the C3 that meant
`adc.read_oneshot()` polling a done bit nothing could set — the firmware
hanging in the user's own code rather than returning a wrong number — a
bus scan finding nothing, and every `Input::new(pin, Pull::Up)` button
reading as *held down* from reset, because a pad with no pull keeps
whatever it last read and that is zero. All six are opened there now.

**And on the ESP32 they were not opened at all**, which is why rusty told
every ESP32 user that its converter and its buses were not modelled. They
are the same six regions, so the machine needs the same six lines — with
three differences that are the machine's rather than the model's. The
converter is at `SENS` rather than at an `APB_SARADC` this part does not
have, and its window there is 0x400 rather than 0x1000, so the region is
aliased down to fit rather than laid over IO_MUX next door. Upstream
already maps models of its own at I2C, SPI2 and LEDC — none of which can
reach rusty's channel, so none of which can answer for anything on the
sheet — so rusty's go over them at a higher priority rather than by
deleting somebody else's device. And every peripheral on this part is
reachable at two addresses, the DPORT one and an APB mirror, so each
region is mapped twice exactly as upstream's own helper does it.

## The CPU

**An ESP32 application's first float faults**, because `CPENABLE` resets to
zero and nothing in esp-hal or its runtime writes it — and the exception
handler saves the floating-point registers, so the handler faults too and
the CPU spins in the double-exception vector. The symptom is silence. The
emulator says it once, at the exception, in the terms the one-line fix is
written in.

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
        "            sysbus_mmio_get_region(SYS_BUS_DEVICE(&ms->gpio), 3), 0);\n"
        "        /* And LEDC, which reaches its pad through the GPIO matrix:\n"
        "         * the duty behind every servo, motor and dimmed lamp. */\n"
        "        memory_region_add_subregion_overlap(sys_mem, DR_REG_LEDC_BASE,\n"
        "            sysbus_mmio_get_region(SYS_BUS_DEVICE(&ms->gpio), 4), 0);\n"
        "        /* And RMT, whose window holds its channels' RAM as well:\n"
        "         * the codes an addressable LED strip is sent as. */\n"
        "        memory_region_add_subregion_overlap(sys_mem, DR_REG_RMT_BASE,\n"
        "            sysbus_mmio_get_region(SYS_BUS_DEVICE(&ms->gpio), 5), 0);\n"
        "        /* And IO_MUX, which is two bits per pad: the pull-up and the\n"
        "         * pull-down every button is read through. Mapped on this\n"
        "         * machine only — the ESP32's IO_MUX registers are a table in\n"
        "         * pad-name order, not pin order, so the same arithmetic would\n"
        "         * put one pin's pull on another's register. */\n"
        "        memory_region_add_subregion_overlap(sys_mem, DR_REG_IO_MUX_BASE,\n"
        "            sysbus_mmio_get_region(SYS_BUS_DEVICE(&ms->gpio), 6), 0);\n",
    ),
    # The same six regions on the original ESP32, which had none of them.
    # Anchored on the same upstream line as the ESP32 wiring above, for the
    # reason given there. A loop rather than six pairs of calls: the APB
    # mirror is easy to give one window and not the next, and a window
    # reachable at one of its two addresses is a peripheral that works
    # until somebody's driver uses the other spelling.
    (
        "hw/xtensa/esp32.c",
        "    esp32_soc_add_periph_device(sys_mem, &s->gpio, DR_REG_GPIO_BASE);\n",
        "    {\n"
        "        /* rusty: the same device's other regions — the SAR ADC, the\n"
        "         * I2C master, SPI2, LEDC, RMT and IO_MUX. One model, because\n"
        "         * all of them carry the host's view of one board and share\n"
        "         * its channel.\n"
        "         *\n"
        "         * Priority 1, because upstream maps models of its own at\n"
        "         * I2C, SPI2 and LEDC and none of them can reach that\n"
        "         * channel. The converter's window at SENS is 0x400, not the\n"
        "         * region's own 0x1000, so it is aliased down rather than\n"
        "         * laid over IO_MUX, which begins 0x800 later. */\n"
        "        static const struct {\n"
        "            int region;\n"
        "            hwaddr base;\n"
        "            uint64_t size;\n"
        "        } rusty_windows[] = {\n"
        "            { 1, DR_REG_SENS_BASE,    0x400  },\n"
        "            { 2, DR_REG_I2C_EXT_BASE, 0x1000 },\n"
        "            { 3, DR_REG_SPI2_BASE,    0x1000 },\n"
        "            { 4, DR_REG_LEDC_BASE,    0x1000 },\n"
        "            { 5, DR_REG_RMT_BASE,     0x1000 },\n"
        "            { 6, DR_REG_IO_MUX_BASE,  0x1000 },\n"
        "        };\n"
        "\n"
        "        for (unsigned i = 0; i < ARRAY_SIZE(rusty_windows); i++) {\n"
        "            MemoryRegion *whole =\n"
        "                sysbus_mmio_get_region(SYS_BUS_DEVICE(&s->gpio),\n"
        "                                       rusty_windows[i].region);\n"
        "            MemoryRegion *cut = g_new(MemoryRegion, 1);\n"
        "            MemoryRegion *apb = g_new(MemoryRegion, 1);\n"
        "            uint32_t at = (uint32_t)rusty_windows[i].base;\n"
        "            char *cut_name = g_strdup_printf(\"rusty-0x%08x\", at);\n"
        "            char *apb_name = g_strdup_printf(\"rusty-apb-0x%08x\", at);\n"
        "\n"
        "            memory_region_init_alias(cut, OBJECT(&s->gpio), cut_name,\n"
        "                                     whole, 0, rusty_windows[i].size);\n"
        "            memory_region_add_subregion_overlap(sys_mem,\n"
        "                rusty_windows[i].base, cut, 1);\n"
        "            memory_region_init_alias(apb, OBJECT(&s->gpio), apb_name,\n"
        "                                     cut, 0, rusty_windows[i].size);\n"
        "            memory_region_add_subregion_overlap(sys_mem,\n"
        "                rusty_windows[i].base - DR_REG_DPORT_APB_BASE\n"
        "                    + APB_REG_BASE, apb, 1);\n"
        "            g_free(cut_name);\n"
        "            g_free(apb_name);\n"
        "        }\n"
        "\n"
        "        /* And the source-status words the CPU's dispatcher reads,\n"
        "         * which on this part are DPORT's and read as zero — so a\n"
        "         * GPIO edge was taken by the CPU and then returned from,\n"
        "         * with esp-hal finding nothing pending. Region 7 answers\n"
        "         * for this device's own source, which it is told here\n"
        "         * rather than having written down. */\n"
        "        s->gpio.intr_source = ETS_GPIO_INTR_SOURCE;\n"
        "        memory_region_add_subregion_overlap(dport_mem,\n"
        "            DPORT_PRO_INTR_STATUS_0,\n"
        "            sysbus_mmio_get_region(SYS_BUS_DEVICE(&s->gpio), 7), 1);\n"
        "    }\n",
    ),
    # `INTERRUPT_CORE0_INTR_STATUS_0`, three words at 0xec of DPORT. Named
    # here because upstream's `esp32_dport.c` has no constant for a register
    # it does not answer, and a bare 0xec in the mapping above would be a
    # number nobody could check.
    (
        "hw/xtensa/esp32.c",
        "#include \"hw/sd/dwc_sdmmc.h\"\n",
        "\n"
        "/* rusty: where the per-source interrupt status words live on this\n"
        " * part. DPORT + 0xec, three of them, from the `dport` block of the\n"
        " * ESP32's own SVD. */\n"
        "#define DPORT_PRO_INTR_STATUS_0 0x0ec\n",
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
    # The FPU is usable from reset on the silicon, and upstream's system
    # emulation leaves it switched off.
    #
    # `CPENABLE` is architecturally undefined at reset, and QEMU sets it only
    # in user mode (to 0xff); a softmmu CPU starts at zero. The ESP32 does not:
    # nothing in its ROM, in the second-stage bootloader or in an esp-hal
    # application ever writes CPENABLE — disassembled, all three, and only a
    # read turned up — and yet esp-hal's interrupt entry saves the
    # floating-point registers unconditionally (`float-save-restore`, a
    # default feature), which traps with CP0 off, and esp-hal's interrupts
    # work on real boards. So on the board it is on, and here it was not: the
    # first interrupt an application took faulted inside its own context save
    # and spun in the double-exception vector, which is every Embassy timer
    # and every `listen()`, not only the applications that multiply. ESP-IDF
    # clears CPENABLE on purpose at start-up for its lazy coprocessor switch,
    # which is the other half of the evidence that the hardware does not.
    # Only on cores whose coprocessor 0 is an FPU.
    (
        "target/xtensa/cpu.c",
        "    env->sregs[VECBASE] = env->config->vecbase;\n",
        "    /* rusty: the FPU is usable from reset, as it is on the ESP32's\n"
        "     * silicon. See qemu/patches.py for how that was established. */\n"
        "    if (xtensa_option_enabled(env->config,\n"
        "                              XTENSA_OPTION_FP_COPROCESSOR)) {\n"
        "        env->sregs[CPENABLE] |= 1;\n"
        "    }\n",
    ),
    # And when something *does* switch coprocessor 0 off — xtensa-lx-rt
    # does inside every interrupt when esp-hal's `float-save-restore` is not
    # enabled, and firmware can itself — a float taken then traps, and with
    # the floating-point save enabled the handler traps too and the CPU spins
    # in the double-exception vector for ever. The symptom is silence, the
    # last line before the float and nothing after it, which is what made
    # rusty tell everybody for months that the emulator stopped at the first
    # float. So the emulator says what happened, once, at the exception.
    (
        "target/xtensa/exc_helper.c",
        "    env->sregs[EXCCAUSE] = cause;\n",
        "    /* rusty: name the one exception whose symptom is silence. */\n"
        "    if (cause == COPROCESSOR0_DISABLED) {\n"
        "        static bool rusty_said_cp0;\n"
        "        if (!rusty_said_cp0) {\n"
        "            rusty_said_cp0 = true;\n"
        "            fprintf(stderr,\n"
        "                \"[rusty:cpu] coprocessor 0 is disabled and the \"\n"
        "                \"application used it at pc=0x%08x: something wrote \"\n"
        "                \"CPENABLE with bit 0 clear, and the FPU is on from \"\n"
        "                \"reset.\\n\"\n"
        "                \"[rusty:cpu] an exception handler that saves the \"\n"
        "                \"floating-point registers faults too, and the CPU \"\n"
        "                \"spins in the double-exception vector. xtensa-lx-rt \"\n"
        "                \"clears CPENABLE inside interrupts unless esp-hal's \"\n"
        "                \"float-save-restore feature is on.\\n\", pc);\n"
        "            fflush(stderr);\n"
        "        }\n"
        "    }\n",
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
            f"qemu/patches.py.",
            file=sys.stderr,
        )
        raise SystemExit(1)
    write(path, text.replace(anchor, anchor + addition))
    print(f"{name}: patched")
