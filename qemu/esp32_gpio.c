/*
 * ESP32 GPIO emulation
 *
 * Copyright (c) 2019 Espressif Systems (Shanghai) Co. Ltd.
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License version 2 or
 * (at your option) any later version.
 */

/*
 * The stock model answered GPIO_STRAP and discarded every write, so a pin had
 * no state in either direction: the guest could not read back what it had
 * driven, and nothing could be driven at it. Whatever watched the simulation
 * had to rely on the firmware describing its own pins over the UART, which is
 * only as truthful as the firmware remembers to be.
 *
 * This keeps the registers that carry pin state and reports changes on a
 * chardev of its own, so a host can watch pins without sharing the console
 * the firmware is using — and can drive the input register back, which is
 * what lets unmodified firmware read a button.
 *
 * **Four peripherals, one file, one channel.** The GPIO is the first; the
 * SAR ADC, the I2C master and the SPI master follow it as further MMIO
 * regions on the same device. That is a decision and not an accident.
 * Everything here exists to carry the host's view of one board — what is on
 * a pin, what is on a bus — and they share the socket that carries it, so
 * keeping them together is what makes "one channel, one protocol, one
 * reader" true on the emulator's side as well as rusty's. Separate devices
 * would each need a chardev of their own or a link back to this one for
 * every access; separate *files* would each need an entry in upstream's
 * build system, and every one of those is a way for a build to fail that has
 * nothing to do with what is being modelled. The file keeps upstream's name
 * because it replaces upstream's file, not because the GPIO is all it holds.
 *
 * Every register name here is prefixed `RUSTY_`. Upstream has its own
 * `esp32_i2c.h` and `esp32c3_spi.h` whose `REG32(I2C_CTR, …)` expands to the
 * same enumerator, and `hw/xtensa/esp32.c` includes both that header and
 * this one — so unprefixed names are a redeclaration error in a file neither
 * of us wrote.
 */

#include "qemu/osdep.h"
#include "qemu/log.h"
#include "qemu/error-report.h"
#include "qemu/timer.h"
#include "qapi/error.h"
#include "hw/hw.h"
#include "hw/sysbus.h"
#include "hw/registerfields.h"
#include "hw/irq.h"
#include "hw/qdev-properties.h"
#include "hw/qdev-properties-system.h"
#include "hw/gpio/esp32_gpio.h"


/* What a pin reads as: an output reports the level the guest drove, an input
 * the level the host is driving. Reporting `out` for an input would be
 * reporting a number nothing can observe. */
static inline int esp32_gpio_level(Esp32GpioState *s, uint64_t bit)
{
    uint64_t source = (s->enable & bit) ? s->out : s->resolved_in;
    return (source & bit) ? 1 : 0;
}

/* The pad's own pull, or -1 where the guest has configured none. */
static int esp32_gpio_pull(Esp32GpioState *s, int pin)
{
    uint32_t mux = s->io_mux[pin];

    if (mux & ESP32_IOMUX_WPU) {
        return 1;
    }
    if (mux & ESP32_IOMUX_WPD) {
        return 0;
    }
    return -1;
}

/* What a closed switch puts on this pad, or -1 where none does.
 *
 * A switch joins two pads; the level comes from whichever of them is being
 * *driven*, which is the pin the firmware has made an output. A matrix key
 * is exactly that — the row is an output for the moment it is scanned, and
 * the column is an input with a pull-up the rest of the time. Two closed
 * switches driving one pad from two directions take the first declared,
 * because on a real board that is a short between two outputs and there is
 * no right answer to report. */
static int esp32_gpio_tied(Esp32GpioState *s, int pin)
{
    for (unsigned i = 0; i < ESP32_GPIO_SWITCHES; i++) {
        Esp32GpioSwitch *sw = &s->switches[i];
        int peer;

        if (!sw->present || !sw->closed) {
            continue;
        }
        if (sw->a == pin) {
            peer = sw->b;
        } else if (sw->b == pin) {
            peer = sw->a;
        } else {
            continue;
        }
        if (s->enable & (1ULL << peer)) {
            return (s->out & (1ULL << peer)) ? 1 : 0;
        }
    }
    return -1;
}

/*
 * What every pad reads, in the order a pad is actually decided.
 *
 * A pin somebody is driving through a closed switch wins; then a level the
 * host stated for the pad itself; then the pad's own pull; then what it was
 * left at, which is zero from reset. The order is the physics: a driver
 * beats a pull, and a pull only answers when nothing is driving.
 *
 * An output's own pad is whatever it is driving, so it is settled here too
 * — a switch from an output to an input carries that level, and an output
 * reading itself back is `esp32_gpio_level`'s business.
 */
static uint64_t esp32_gpio_resolve(Esp32GpioState *s)
{
    uint64_t level = 0;

    for (int pin = 0; pin < ESP32_GPIO_PINS; pin++) {
        uint64_t bit = 1ULL << pin;
        int at;

        if (s->enable & bit) {
            at = (s->out & bit) ? 1 : 0;
        } else if ((at = esp32_gpio_tied(s, pin)) >= 0) {
            /* driven through a switch */
        } else if (s->host_driven & bit) {
            at = (s->in & bit) ? 1 : 0;
        } else if ((at = esp32_gpio_pull(s, pin)) >= 0) {
            /* pulled up or down by the pad itself */
        } else {
            at = (s->in & bit) ? 1 : 0;
        }
        if (at) {
            level |= bit;
        }
    }
    return level;
}

/* Defined below, with the rest of the channel's writers. */
static void esp32_gpio_report(Esp32GpioState *s, uint64_t changed);
static void esp32_gpio_int_update(Esp32GpioState *s, uint64_t edges);

/*
 * Work out what every pad is at, and tell whoever needs to know.
 *
 * Everything that can change a level without the guest writing `GPIO_OUT`
 * ends here: a host line, a switch opening or closing, a pull being
 * configured, an output changing under a closed switch. The report and the
 * edges come from the difference, so a pin that did not move says nothing
 * and a pin that did interrupts the firmware exactly once.
 *
 * `reported` is what the caller has already accounted for — the guest's own
 * `GPIO_OUT` write reports itself, and reporting it twice would put a
 * repeat on the channel.
 */
static void esp32_gpio_settle(Esp32GpioState *s, uint64_t reported)
{
    uint64_t before = s->resolved_in;
    uint64_t changed;

    s->resolved_in = esp32_gpio_resolve(s);
    changed = (before ^ s->resolved_in) & ~reported;
    if (changed == 0) {
        return;
    }
    esp32_gpio_report(s, changed);
    /* Only pads the guest is *reading* can interrupt it on this path: an
     * output's own edge is raised where the guest wrote it. */
    esp32_gpio_int_update(s, changed & ~s->enable);
}

/* Whether this pad is `GPIO_OUT`'s to drive, or a peripheral's.
 *
 * The matrix sends one signal to each pad, and `GPIO` (128) is the one that
 * means "whatever the guest put in GPIO_OUT". A pad pointed at LEDC or RMT
 * is driven by that peripheral, and reporting `GPIO_OUT`'s bit for it is
 * how a lamp on a dimmed pin reads as dark while the firmware fades it —
 * the peripheral speaks for its own pad. Zero is the reset value, which
 * every pad the firmware has not configured still holds. */
static inline bool esp32_gpio_is_plain(Esp32GpioState *s, int pin)
{
    unsigned signal = s->func_out[pin] & ESP32_GPIO_OUT_SEL_MASK;

    return signal == ESP32_GPIO_OUT_SEL_GPIO || signal == 0;
}

/*
 * Which pins are asking the CPU for an interrupt right now.
 *
 * A pin reaches the line the interrupt matrix carries only if its
 * `INT_ENA` names one of the CPU sources; the NMI bits go to a source this
 * device is not wired to, and raising the ordinary line for them would be
 * inventing an interrupt nobody asked for.
 */
static uint64_t esp32_gpio_cpu_mask(Esp32GpioState *s)
{
    uint64_t mask = 0;

    for (int pin = 0; pin < ESP32_GPIO_PINS; pin++) {
        unsigned ena = (s->pin_cfg[pin] >> ESP32_GPIO_PIN_INT_ENA_SHIFT)
                       & ESP32_GPIO_PIN_INT_ENA_MASK;

        if (ena & ESP32_GPIO_INT_ENA_CPU) {
            mask |= 1ULL << pin;
        }
    }
    return mask;
}

/* Defined below, beside the other things that write to the pin channel. */
static void esp32_gpio_say_irq(Esp32GpioState *s, bool raised);
static void esp32_gpio_say_i2c(Esp32GpioState *s, int address, const char *verb,
                               const uint8_t *bytes, unsigned count);
static Esp32I2cDevice *esp32_i2c_declare(Esp32GpioState *s, int address);
static void esp32_ledc_say_all(Esp32GpioState *s);
static Esp32I2cDevice *esp32_i2c_device(Esp32GpioState *s, int address);

/*
 * Latch what has fired, drop what no longer holds, and tell the interrupt
 * matrix — the whole of GPIO interrupts.
 *
 * `edges` names the pins whose level moved since the last call, which is
 * what tells the two edge types from the two level types: an edge is a
 * moment and stays latched until the firmware writes the bit back, a level
 * is a state and follows it. Silicon does exactly this, and the difference
 * is the one a driver notices — a level interrupt that latched would go on
 * firing after the button came back up.
 *
 * Called after anything that can change the answer: a host-driven pin, a
 * write to OUT or ENABLE, a write to a pin's configuration, and the
 * firmware clearing a bit. Missing one of those is an interrupt that only
 * arrives when something else happens to move.
 */
static void esp32_gpio_int_update(Esp32GpioState *s, uint64_t edges)
{
    uint64_t before = s->status;
    bool raise;

    for (int pin = 0; pin < ESP32_GPIO_PINS; pin++) {
        uint64_t bit = 1ULL << pin;
        unsigned type = (s->pin_cfg[pin] >> ESP32_GPIO_PIN_INT_TYPE_SHIFT)
                        & ESP32_GPIO_PIN_INT_TYPE_MASK;
        int level = esp32_gpio_level(s, bit);

        switch (type) {
        case ESP32_GPIO_INT_RISING:
            if ((edges & bit) && level) {
                s->status |= bit;
            }
            break;

        case ESP32_GPIO_INT_FALLING:
            if ((edges & bit) && !level) {
                s->status |= bit;
            }
            break;

        case ESP32_GPIO_INT_ANYEDGE:
            if (edges & bit) {
                s->status |= bit;
            }
            break;

        /* Not latched: the bit is the level, so a firmware that clears it
         * while the button is still down sees it come straight back — which
         * is what a level interrupt is, and why drivers mask it instead. */
        case ESP32_GPIO_INT_LOW:
            s->status = level ? (s->status & ~bit) : (s->status | bit);
            break;

        case ESP32_GPIO_INT_HIGH:
            s->status = level ? (s->status | bit) : (s->status & ~bit);
            break;

        /* Turned off: nothing new latches, and anything already pending on
         * this pin goes — a pin somebody stopped listening to would
         * otherwise hold the line down for ever. */
        default:
            s->status &= ~bit;
            break;
        }
    }

    raise = (s->status & esp32_gpio_cpu_mask(s)) != 0;
    if (raise != s->irq_level || s->status != before) {
        s->irq_level = raise;
        qemu_set_irq(s->irq, raise);
        /* Said on the pin channel, so a host watching the board can tell
         * "the peripheral raised the line" from "the firmware ran its
         * handler" — two accounts of one interrupt, and the only way a
         * test can name which half is broken when nothing happens. */
        esp32_gpio_say_irq(s, raise);
    }
}

/* Fold a 32-bit register write into one half of a 64-bit field.
 *
 * `bank` is 0 for pins 0..31 and 1 for 32..39. Doing it here rather than at
 * each case is what keeps the twelve registers from becoming twelve chances
 * to shift by the wrong amount. */
static inline uint64_t esp32_gpio_half(uint64_t whole, unsigned bank,
                                       uint32_t word, int op)
{
    uint64_t mask = 0xffffffffULL << (bank * 32);
    uint64_t bits = (uint64_t)word << (bank * 32);

    switch (op) {
    case 0:  /* assign */
        return (whole & ~mask) | bits;
    case 1:  /* set */
        return whole | bits;
    default: /* clear */
        return whole & ~bits;
    }
}

/*
 * Report every pin whose level or direction changed.
 *
 * Timestamped with the guest's own clock, because a host plotting these
 * beside firmware output needs one clock, and the guest's is the one the
 * firmware also reads. The format is the line rusty already parses, so a
 * host reading pins from the emulator and a firmware printing them itself
 * produce the same text and can be checked against each other.
 */
static void esp32_gpio_report(Esp32GpioState *s, uint64_t changed)
{
    /* Forty pins at up to "39=1," each, plus the timestamped prefix. Sized so
     * a report of every pin at once still fits rather than tripping the
     * truncation guard below. */
    char line[384];
    int at;
    bool first = true;

    if (changed == 0 || !qemu_chr_fe_backend_connected(&s->pins)) {
        return;
    }

    at = snprintf(line, sizeof(line), "[rusty:gpio@%" PRId64 "] ",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL));

    for (int pin = 0; pin < ESP32_GPIO_PINS; pin++) {
        uint64_t bit = 1ULL << pin;

        if ((changed & bit) == 0) {
            continue;
        }
        if (at > (int)sizeof(line) - 10) {
            break;
        }
        /* A pad a peripheral drives belongs to that peripheral's report. */
        if (!esp32_gpio_is_plain(s, pin)) {
            continue;
        }
        at += snprintf(line + at, sizeof(line) - at, "%s%d=%d",
                       first ? "" : ",", pin, esp32_gpio_level(s, bit));
        first = false;
    }

    /* Every pin that changed belonged to a peripheral, so this report has
     * nothing to say. */
    if (first) {
        return;
    }
    at += snprintf(line + at, sizeof(line) - at, "\n");
    qemu_chr_fe_write_all(&s->pins, (const uint8_t *)line, at);
}

/*
 * What the interrupt line is doing, on the same channel the pins travel.
 *
 * `[rusty:irq@<us>] <pins>` names every pin currently asking, or nothing
 * after the last one is cleared. It is a report, not a protocol the guest
 * can see: firmware learns about its interrupts by being interrupted.
 *
 * `unconnected` when nothing is on the other end of the line. A machine
 * that never called `sysbus_connect_irq` leaves `s->irq` null, and
 * `qemu_set_irq` on a null line returns without doing anything — so the
 * model would go on reporting interrupts it raised into nothing, and a
 * firmware that was never interrupted would look identical to a firmware
 * that ignored one. Said here because this is the only place that can.
 */
static void esp32_gpio_say_irq(Esp32GpioState *s, bool raised)
{
    char line[256];
    int at;
    bool first = true;

    if (!qemu_chr_fe_backend_connected(&s->pins)) {
        return;
    }
    at = snprintf(line, sizeof(line), "[rusty:irq@%" PRId64 "] ",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL));
    if (!s->irq) {
        at += snprintf(line + at, sizeof(line) - at, "unconnected ");
    }
    if (!raised) {
        at += snprintf(line + at, sizeof(line) - at, "-");
    }
    for (int pin = 0; raised && pin < ESP32_GPIO_PINS; pin++) {
        uint64_t bit = 1ULL << pin;

        if ((s->status & bit) == 0 || at > (int)sizeof(line) - 8) {
            continue;
        }
        at += snprintf(line + at, sizeof(line) - at, "%s%d", first ? "" : ",", pin);
        first = false;
    }
    at += snprintf(line + at, sizeof(line) - at, "\n");
    qemu_chr_fe_write_all(&s->pins, (const uint8_t *)line, at);
}

/* One hex digit, or -1. */
static int esp32_hex_digit(char c)
{
    if (c >= '0' && c <= '9') {
        return c - '0';
    }
    if (c >= 'a' && c <= 'f') {
        return c - 'a' + 10;
    }
    if (c >= 'A' && c <= 'F') {
        return c - 'A' + 10;
    }
    return -1;
}

/* A run of hex pairs into bytes, stopping at the first thing that is not
 * one. A half pair at the end stops the run rather than becoming a byte:
 * a truncated hex string is a valid, wrong one. */
static unsigned esp32_hex_bytes(const char *text, uint8_t *out, unsigned max)
{
    unsigned count = 0;

    while (count < max) {
        int high, low;

        if (text[0] == '\0' || (high = esp32_hex_digit(text[0])) < 0) {
            break;
        }
        if (text[1] == '\0' || (low = esp32_hex_digit(text[1])) < 0) {
            break;
        }
        out[count++] = (uint8_t)((high << 4) | low);
        text += 2;
    }
    return count;
}

/*
 * What the model did with a switch, on the channel the pins travel.
 *
 * `[rusty:sw@<us>] 4-6=1`. Two accounts of one thing, as everywhere else
 * here: the host says what it pressed and the device says what it joined,
 * and a key that reached a build too old to know the line says nothing at
 * all rather than looking like a key that does nothing.
 *
 * It is also **the marker this generation is recognised by**. A host
 * scanning the binary for `[rusty:sw@` learns that this emulator ties pads
 * *and* that it models the pads' pulls, because the two are in this one
 * file and are built together — not a proxy for each other but two halves
 * of the same replacement, which cannot be half present.
 */
static void esp32_gpio_say_switch(Esp32GpioState *s, const Esp32GpioSwitch *sw)
{
    char line[64];
    int at;

    if (!qemu_chr_fe_backend_connected(&s->pins)) {
        return;
    }
    at = snprintf(line, sizeof(line), "[rusty:sw@%" PRId64 "] %u-%u=%u\n",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL), sw->a, sw->b,
                  sw->closed ? 1u : 0u);
    qemu_chr_fe_write_all(&s->pins, (const uint8_t *)line, at);
}

/*
 * A switch between two pads.
 *
 *   sw 4-5=1     the key joining GPIO4 and GPIO5 is held down
 *   sw 4-5=0     it is released — the pads part again
 *
 * Declared by pressing it: the first `sw a-b=` for a pair makes the switch,
 * and every later line for the same pair moves the one that is there. A
 * matrix keypad is sixteen of these, and nothing about it is special — the
 * rows are outputs while they are scanned and the columns are inputs with
 * pull-ups, and a held key is what carries one to the other.
 *
 * **Not a level.** `4=0` says what the *host* is driving onto a pad; this
 * says two pads are connected and lets whichever of them the firmware is
 * driving decide. That difference is the whole of why a matrix could not be
 * simulated before: during a scan the row is an output for a moment, and a
 * host driving the column low instead would be pressing every key in that
 * column at once.
 *
 * A pair past the table is refused by name rather than dropped, because a
 * keypad that silently lost its last row would read as a broken model.
 */
static void esp32_gpio_host_switch(Esp32GpioState *s)
{
    unsigned a, b, closed;
    Esp32GpioSwitch *free_slot = NULL;

    if (sscanf(s->host_line, "sw %u-%u=%u", &a, &b, &closed) != 3
        || a >= ESP32_GPIO_PINS || b >= ESP32_GPIO_PINS || a == b) {
        return;
    }
    for (unsigned i = 0; i < ESP32_GPIO_SWITCHES; i++) {
        Esp32GpioSwitch *sw = &s->switches[i];

        if (!sw->present) {
            free_slot = free_slot ? free_slot : sw;
            continue;
        }
        if ((sw->a == a && sw->b == b) || (sw->a == b && sw->b == a)) {
            sw->closed = closed != 0;
            esp32_gpio_say_switch(s, sw);
            esp32_gpio_settle(s, 0);
            return;
        }
    }
    if (free_slot == NULL) {
        char line[64];
        int at = snprintf(line, sizeof(line),
                          "[rusty:pins] switches full, %u-%u refused\n", a, b);

        qemu_chr_fe_write_all(&s->pins, (const uint8_t *)line, at);
        return;
    }
    free_slot->present = true;
    free_slot->closed = closed != 0;
    free_slot->a = (uint8_t)a;
    free_slot->b = (uint8_t)b;
    esp32_gpio_say_switch(s, free_slot);
    esp32_gpio_settle(s, 0);
}

/*
 * What the host puts on the two buses.
 *
 *   i2c 68:75=68     device 0x68, register 0x75 onwards, one byte 0x68
 *   i2c 3c=+         declare it with every register zero
 *   i2c 3c=-         take it off the bus, so an address stops answering
 *   spi 0=68010203   what chip select 0 answers with, from the start of
 *                    every transfer; anything that is not hex clears it
 *
 * Hex for the bytes and for an I2C address, because that is how a datasheet
 * writes both. The chip select is decimal, because it is a line number.
 * Declaring is what makes an I2C address answer at all — see the note on
 * the engine about why an undeclared one must NACK.
 */
static void esp32_gpio_host_bus(Esp32GpioState *s)
{
    unsigned address, reg;
    char payload[ESP32_GPIO_HOST_LINE];
    char sign;

    if (sscanf(s->host_line, "i2c %x:%x=%500s", &address, &reg, payload) == 3) {
        uint8_t bytes[256];
        unsigned count;
        Esp32I2cDevice *device;

        if (address > 0x7f || reg > 0xff) {
            return;
        }
        count = esp32_hex_bytes(payload, bytes, sizeof(bytes));
        device = esp32_i2c_declare(s, (int)address);
        if (!device) {
            esp32_gpio_say_i2c(s, (int)address, "full", NULL, 0);
            return;
        }
        for (unsigned i = 0; i < count; i++) {
            device->regs[(uint8_t)(reg + i)] = bytes[i];
            device->has_regs = true;
        }
        return;
    }
    if (sscanf(s->host_line, "spi %u=%500s", &address, payload) == 2
        && address < ESP32_SPI_SELECTS) {
        s->spi_miso_len[address] =
            esp32_hex_bytes(payload, s->spi_miso[address], ESP32_SPI_BUFFER);
        return;
    }
    if (sscanf(s->host_line, "i2c %x=%c", &address, &sign) == 2 && address <= 0x7f) {
        if (sign == '-') {
            Esp32I2cDevice *device = esp32_i2c_device(s, (int)address);

            if (device) {
                device->present = false;
            }
        } else if (sign == '+' && !esp32_i2c_declare(s, (int)address)) {
            esp32_gpio_say_i2c(s, (int)address, "full", NULL, 0);
        }
    }
}

/*
 * Host input, a line at a time: `<pin>=<level>` drives the input register,
 * `A<pin>=<counts>` puts an analog value on the pin for the converter, and
 * `i2c …` / `spi …` put devices on the two buses.
 *
 * This is the half that lets firmware read a button through the GPIO it
 * actually reads, a knob through the ADC it actually reads, and a sensor
 * through the bus it actually reads, instead of through a side channel each
 * had to be written to expect. The forms cannot be confused: a decimal pin
 * number begins with none of `A`, `i` or `s`.
 *
 * A line that matches none of them is dropped. There is one writer on the
 * other end of this socket and its spellings are under test; a malformed
 * line here is a bug in that, not something to guess the meaning of.
 */
static void esp32_gpio_host_read(void *opaque, const uint8_t *buf, int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);

    for (int i = 0; i < size; i++) {
        if (buf[i] == '\n' || buf[i] == '\r') {
            unsigned pin, level;

            s->host_line[s->host_at] = '\0';
            if (strncmp(s->host_line, "sw ", 3) == 0) {
                esp32_gpio_host_switch(s);
            } else if (s->host_line[0] == 'i' || s->host_line[0] == 's') {
                esp32_gpio_host_bus(s);
            } else if (sscanf(s->host_line, "A%u=%u", &pin, &level) == 2
                && pin < ESP32_GPIO_PINS) {
                /* Stored and nothing more: an ADC reads when it is told to,
                 * and a value that moved a data register on its own would be
                 * a reading nobody took. Clamped rather than wrapped, so a
                 * slider dragged past full scale reads as full scale. */
                s->analog[pin] = MIN(level, ESP32_SARADC_FULL_SCALE);
            } else if (sscanf(s->host_line, "%u=%u", &pin, &level) == 2
                       && pin < ESP32_GPIO_PINS) {
                uint64_t bit = 1ULL << pin;
                uint64_t before = s->in;

                s->in = level ? (s->in | bit) : (s->in & ~bit);
                /* Said, so a pull no longer answers for this pad: the host
                 * has taken it over. Without this a `4=0` against a
                 * configured pull-up would be read back as the pull's 1. */
                s->host_driven |= bit;
                (void)before;
                /* Only a real change is reported, so a host holding a button
                 * down does not fill the channel with one repeated line —
                 * and it is an edge on that pin, so firmware waiting on an
                 * interrupt runs. Both are `settle`'s, which is where every
                 * other way a pad can move ends too. */
                esp32_gpio_settle(s, 0);
            }
            s->host_at = 0;
        } else if (s->host_at < sizeof(s->host_line) - 1) {
            s->host_line[s->host_at++] = buf[i];
        } else {
            s->host_at = 0;
        }
    }
}

static int esp32_gpio_host_can_read(void *opaque)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);

    return sizeof(s->host_line);
}

static uint64_t esp32_gpio_read(void *opaque, hwaddr addr, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    uint64_t r = 0;
    switch (addr) {
    case A_GPIO_STRAP:
        r = s->strap_mode;
        break;

    case A_GPIO_OUT:
        r = (uint32_t)s->out;
        break;

    case A_GPIO_OUT1:
        r = (uint32_t)(s->out >> 32);
        break;

    case A_GPIO_ENABLE:
        r = (uint32_t)s->enable;
        break;

    case A_GPIO_ENABLE1:
        r = (uint32_t)(s->enable >> 32);
        break;

    /* An output pin reads back its own driven level, which is what the
     * silicon does and what firmware toggling a pin by read-modify-write
     * depends on. An input pin reads the *settled* pad — what a closed
     * switch or the pad's own pull put there, not the raw `in` the host
     * last wrote. Reading `in` here left every pull-up invisible to the
     * firmware while the pin channel reported it correctly: a keypad whose
     * columns rested high for the host and low for the guest. */
    case A_GPIO_IN:
        r = (uint32_t)((s->resolved_in & ~s->enable) | (s->out & s->enable));
        break;

    case A_GPIO_IN1:
        r = (uint32_t)(((s->resolved_in & ~s->enable) | (s->out & s->enable)) >> 32);
        break;

    case A_GPIO_STATUS:
    case A_GPIO_STATUS_W1TS:
    case A_GPIO_STATUS_W1TC:
        r = (uint32_t)s->status;
        break;

    case A_GPIO_STATUS1:
    case A_GPIO_STATUS1_W1TS:
    case A_GPIO_STATUS1_W1TC:
        r = (uint32_t)(s->status >> 32);
        break;

    default:
        /* The CPU's own pending view and the per-pin configuration, whose
         * offsets differ between the original ESP32 and everything after
         * it. A `switch` cannot hold an address that is not a constant, so
         * they are answered here. */
        if (addr == s->pcpu_int_reg) {
            r = (uint32_t)(s->status & esp32_gpio_cpu_mask(s));
        } else if (addr == s->pcpu_int_reg + ESP32_GPIO_PCPU_INT1_STRIDE) {
            /* The second bank's view, on the part that has one. */
            r = (uint32_t)((s->status & esp32_gpio_cpu_mask(s)) >> 32);
        } else if (addr >= s->pin0_reg
                   && addr < s->pin0_reg + 4 * ESP32_GPIO_PINS) {
            r = s->pin_cfg[(addr - s->pin0_reg) / 4];
        } else if (addr >= s->func_out_reg
                   && addr < s->func_out_reg + 4 * ESP32_GPIO_PINS) {
            r = s->func_out[(addr - s->func_out_reg) / 4];
        }
        break;
    }
    return r;
}

static void esp32_gpio_write(void *opaque, hwaddr addr,
                       uint64_t value, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    uint64_t before_out = s->out;
    uint64_t before_enable = s->enable;
    uint32_t word = (uint32_t)value;

    switch (addr) {
    case A_GPIO_OUT:
        s->out = esp32_gpio_half(s->out, 0, word, 0);
        break;

    /* The set and clear aliases. esp-hal drives a pin through these rather
     * than through OUT, so a model handling only OUT would see nothing at
     * all from ordinary firmware. */
    case A_GPIO_OUT_W1TS:
        s->out = esp32_gpio_half(s->out, 0, word, 1);
        break;

    case A_GPIO_OUT_W1TC:
        s->out = esp32_gpio_half(s->out, 0, word, 2);
        break;

    case A_GPIO_ENABLE:
        s->enable = esp32_gpio_half(s->enable, 0, word, 0);
        break;

    case A_GPIO_ENABLE_W1TS:
        s->enable = esp32_gpio_half(s->enable, 0, word, 1);
        break;

    case A_GPIO_ENABLE_W1TC:
        s->enable = esp32_gpio_half(s->enable, 0, word, 2);
        break;

    /* The same six for GPIO32..39, which only the original ESP32 has. */
    case A_GPIO_OUT1:
        s->out = esp32_gpio_half(s->out, 1, word, 0);
        break;

    case A_GPIO_OUT1_W1TS:
        s->out = esp32_gpio_half(s->out, 1, word, 1);
        break;

    case A_GPIO_OUT1_W1TC:
        s->out = esp32_gpio_half(s->out, 1, word, 2);
        break;

    case A_GPIO_ENABLE1:
        s->enable = esp32_gpio_half(s->enable, 1, word, 0);
        break;

    case A_GPIO_ENABLE1_W1TS:
        s->enable = esp32_gpio_half(s->enable, 1, word, 1);
        break;

    case A_GPIO_ENABLE1_W1TC:
        s->enable = esp32_gpio_half(s->enable, 1, word, 2);
        break;

    /* Clearing a pending interrupt is a write of the bit back. `STATUS`
     * itself is writable too, and a bit the firmware sets by hand is
     * honoured: that is a driver asking for its own handler to run. */
    case A_GPIO_STATUS:
        s->status = esp32_gpio_half(s->status, 0, word, 0);
        break;

    case A_GPIO_STATUS_W1TS:
        s->status = esp32_gpio_half(s->status, 0, word, 1);
        break;

    case A_GPIO_STATUS_W1TC:
        s->status = esp32_gpio_half(s->status, 0, word, 2);
        break;

    case A_GPIO_STATUS1:
        s->status = esp32_gpio_half(s->status, 1, word, 0);
        break;

    case A_GPIO_STATUS1_W1TS:
        s->status = esp32_gpio_half(s->status, 1, word, 1);
        break;

    case A_GPIO_STATUS1_W1TC:
        s->status = esp32_gpio_half(s->status, 1, word, 2);
        break;

    default:
        /* A pin's own configuration — how it triggers, and which CPU line
         * it feeds. Storing it is what makes the type mean anything, and a
         * level type can be true the moment it is written. */
        if (addr >= s->pin0_reg && addr < s->pin0_reg + 4 * ESP32_GPIO_PINS) {
            s->pin_cfg[(addr - s->pin0_reg) / 4] = word;
            esp32_gpio_int_update(s, 0);
        } else if (addr >= s->func_out_reg
                   && addr < s->func_out_reg + 4 * ESP32_GPIO_PINS) {
            /* The matrix: which signal this pad carries. Kept because it is
             * the only way to follow a peripheral's output to a pin, and
             * because a pad sent to a peripheral is one GPIO must stop
             * speaking for. */
            int pin = (addr - s->func_out_reg) / 4;
            bool was_plain = esp32_gpio_is_plain(s, pin);

            s->func_out[pin] = word;
            /*
             * **Who speaks for the pad changed, not what the pad is doing.**
             * A routing write is not a level change, and reporting one as
             * though it were puts a repeat in the pin's account: esp-hal
             * writes this register while configuring an ordinary output, so
             * every `Output::new` said the pin's level a second time and
             * blinky's GPIO0 came back as 0, 0, 1, 0, 1 … — which is what
             * gate 4 rejects, and rightly, since a model that repeats a
             * level is one that could be missing an edge.
             *
             * So the report is for the one transition that needs it: a pad
             * coming *back* to GPIO after a peripheral had it, where GPIO
             * has said nothing about it in the meantime. The other way
             * round needs no line here — the peripheral says its own.
             */
            if (was_plain != esp32_gpio_is_plain(s, pin)) {
                if (!was_plain) {
                    esp32_gpio_report(s, 1ULL << pin);
                }
                esp32_ledc_say_all(s);
            }
        }
        return;
    }

    /* A direction change alters what a pin reports even when its level did
     * not move, so both registers decide what counts as changed. */
    uint64_t reported = (before_out ^ s->out) | (before_enable ^ s->enable);

    esp32_gpio_report(s, reported);
    /* A pin the guest drives is a pin that can interrupt the guest — the
     * loopback silicon has, and what firmware testing its own handler
     * depends on. The clear path lands here too, with no edges at all. */
    esp32_gpio_int_update(s, (before_out ^ s->out) & s->enable);
    /* And the pads on the other side of a closed switch, which this write
     * has just moved: a scanned row drags its column down with it, which is
     * the whole of what a matrix key does. Told what has been reported
     * already, so the row itself is not said twice. */
    esp32_gpio_settle(s, reported);
}

/*
 * Fill in which IO_MUX word belongs to which pad, for this part.
 *
 * The ESP32's table is in *pad-name* order — `MTDI` is GPIO12 and sits four
 * words before `GPIO0` — so it is transcribed rather than computed. It is
 * the field order of the `io_mux` register block in the vendor's own SVD,
 * whose registers are all 32 bits: GPIO36 first at 0x04, then 37, 38, 39,
 * 34, 35, 32, 33, 25, 26, 27, 14, 12, 13, 15, 2, 0, 4, 16, 17, 9, 10, 11,
 * 6, 7, 8, 5, 18, 19, 20, 21, 22, 3, 1, 23, 24. There is no GPIO28..31.
 *
 * Everything else in the family puts the pads after `IO_MUX_PIN_CTRL` in
 * pin order, which is the arithmetic this used to do for every part.
 */
static void esp32_iomux_map(Esp32GpioState *s, bool esp32)
{
    static const uint8_t pads[] = {
        36, 37, 38, 39, 34, 35, 32, 33, 25, 26, 27, 14, 12, 13, 15, 2,
        0, 4, 16, 17, 9, 10, 11, 6, 7, 8, 5, 18, 19, 20, 21, 22, 3, 1,
        23, 24,
    };

    memset(s->iomux_at, -1, sizeof(s->iomux_at));
    if (esp32) {
        for (unsigned i = 0; i < ARRAY_SIZE(pads); i++) {
            s->iomux_at[(ESP32_IOMUX_PIN0 / 4) + i] = (int8_t)pads[i];
        }
        /* GPIO34..39 are input-only on this part: no output driver and no
         * pull circuitry, so their two bits read back zero. */
        s->iomux_no_pull = 0x3full << 34;
    } else {
        for (unsigned pin = 0; pin < ESP32_GPIO_PINS; pin++) {
            unsigned word = (ESP32_IOMUX_PIN0 / 4) + pin;

            if (word < ESP32_IOMUX_WORDS) {
                s->iomux_at[word] = (int8_t)pin;
            }
        }
        s->iomux_no_pull = 0;
    }
}

/* Which pad a word of the IO_MUX window belongs to, or -1 for none.
 *
 * A table the part filled in, never arithmetic: see the header. A word
 * outside the table is `IO_MUX_PIN_CTRL`, a reserved word, or a pad this
 * part does not have, and all three are nobody's pull. */
static int esp32_iomux_pin(Esp32GpioState *s, hwaddr addr)
{
    if (addr & 3 || addr / 4 >= ESP32_IOMUX_WORDS) {
        return -1;
    }
    return s->iomux_at[addr / 4];
}

static uint64_t esp32_iomux_read(void *opaque, hwaddr addr, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    int pin = esp32_iomux_pin(s, addr);

    return pin < 0 ? 0 : s->io_mux[pin];
}

static void esp32_iomux_write(void *opaque, hwaddr addr, uint64_t value,
                              unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    int pin = esp32_iomux_pin(s, addr);
    uint32_t word = (uint32_t)value;

    if (pin < 0) {
        return;
    }
    /* An input-only pad has no pull circuitry, so its two bits read back as
     * zero however they were written — the firmware then floats on the desk
     * and must float here. Dropped on the way in rather than ignored on the
     * way out, so a driver reading the register back is told. */
    if (s->iomux_no_pull & (1ull << pin)) {
        word &= ~(ESP32_IOMUX_WPU | ESP32_IOMUX_WPD);
    }
    /* Stored whole otherwise, so a driver's read-modify-write of a field
     * this has no opinion about keeps what it put there. */
    s->io_mux[pin] = word;
    /* A pull configured is a pad that may have just moved — an input with
     * `Pull::Up` reads high from that instant, which is what
     * `Input::new(pin, Pull::Up)` means and what every button is read
     * through. */
    esp32_gpio_settle(s, 0);
}

static const MemoryRegionOps iomux_ops = {
    .read = esp32_iomux_read,
    .write = esp32_iomux_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

/*
 * Which interrupt sources are asserting, as the CPU's dispatcher reads it.
 *
 * One bit, this device's own, in whichever of the three words holds it. See
 * the header: a wired line the dispatcher cannot attribute is an interrupt
 * that is taken and then returned from, and that is what a GPIO edge on an
 * ESP32 did until this answered.
 */
static uint64_t esp32_intr_status_read(void *opaque, hwaddr addr,
                                       unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    unsigned word = addr / 4;

    if (s->intr_source < 0 || !s->irq_level
        || word >= ESP32_INTR_STATUS_WORDS
        || (unsigned)s->intr_source / 32 != word) {
        return 0;
    }
    return 1u << ((unsigned)s->intr_source % 32);
}

/* Read-only: the status of a source is the peripheral's to say, and a
 * driver clears it by clearing what raised it. */
static void esp32_intr_status_write(void *opaque, hwaddr addr, uint64_t value,
                                    unsigned int size)
{
}

static const MemoryRegionOps intr_status_ops = {
    .read = esp32_intr_status_read,
    .write = esp32_intr_status_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

static const MemoryRegionOps uart_ops = {
    .read =  esp32_gpio_read,
    .write = esp32_gpio_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

/*
 * ============================ the analog half ============================
 *
 * The SAR ADC, so that firmware reading a knob reads it the way firmware
 * does — `adc.read_blocking(&mut pin)` — rather than through a text channel
 * it had to be written to expect. Without it that call does not return a
 * wrong number, it *waits*: the driver polls a done bit that a peripheral
 * nobody models can never set, and the firmware hangs in the user's own
 * code with nothing on screen to say why.
 *
 * Only what a one-shot conversion touches is modelled. Everything else in
 * the window is shadowed, so a driver's read-modify-write of a register this
 * has no opinion about keeps what it put there.
 */

/*
 * Which pin a unit's channel reads.
 *
 * A table rather than arithmetic: the mapping is a property of the die's
 * bonding, and a formula that happened to fit this part would quietly read
 * the wrong pin on the next one. -1 is "this unit has no such channel",
 * which reads as zero and says so on the channel rather than silently
 * answering with some other pin's value.
 */
static int esp32_saradc_pin_for(Esp32GpioState *s, unsigned unit,
                                unsigned channel)
{
    /* ESP32-C3: ADC1 channels 0..4 are GPIO0..GPIO4; ADC2's one usable
     * channel is GPIO5. */
    static const int c3_adc1[] = { 0, 1, 2, 3, 4 };
    static const int c3_adc2[] = { 5 };
    /* The original ESP32, whose channels are scattered across the die: the
     * eight ADC1 channels are the input-only pads and the two pairs beside
     * them, and ADC2's ten are ordinary GPIOs. Neither is arithmetic. */
    static const int esp32_adc1[] = { 36, 37, 38, 39, 32, 33, 34, 35 };
    static const int esp32_adc2[] = { 4, 0, 2, 15, 13, 12, 14, 27, 25, 26 };

    const int *table;
    size_t count;

    if (s->saradc_esp32) {
        table = unit == 0 ? esp32_adc1 : esp32_adc2;
        count = unit == 0 ? ARRAY_SIZE(esp32_adc1) : ARRAY_SIZE(esp32_adc2);
    } else {
        table = unit == 0 ? c3_adc1 : c3_adc2;
        count = unit == 0 ? ARRAY_SIZE(c3_adc1) : ARRAY_SIZE(c3_adc2);
    }
    return channel < count ? table[channel] : -1;
}

/*
 * What was converted, on the same channel the pins travel.
 *
 * The other half of every reading a panel shows: the host said what was on
 * the pin, and this says what the firmware actually took off it and when.
 * A conversion of a channel with no pin behind it says so as `adc<n>ch<c>`
 * rather than inventing a pin number.
 *
 * Reported per *change*, like a pin is — a driver polling in a loop
 * converts thousands of times a second, and a line each would drown the
 * channel the console and the board share. The caller decides what counts
 * as a change; here it is the pin or the value moving.
 */
static void esp32_gpio_say_adc(Esp32GpioState *s, unsigned unit,
                               unsigned channel, int pin, unsigned counts)
{
    char line[64];
    int at;

    if (!qemu_chr_fe_backend_connected(&s->pins)) {
        return;
    }
    at = snprintf(line, sizeof(line), "[rusty:adc@%" PRId64 "] ",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL));
    if (pin >= 0) {
        at += snprintf(line + at, sizeof(line) - at, "%d=%u\n", pin, counts);
    } else {
        at += snprintf(line + at, sizeof(line) - at, "adc%uch%u=?\n",
                       unit + 1, channel);
    }
    qemu_chr_fe_write_all(&s->pins, (const uint8_t *)line, at);
}

/*
 * One conversion: latch what is on the channel's pin and raise the done bit
 * the driver is polling.
 *
 * Instant, because there is nothing useful to model in the delay. The
 * silicon takes microseconds and the driver waits for the bit either way;
 * a timer here would only add a way for a conversion to be lost.
 */
static void esp32_saradc_sample(Esp32GpioState *s, unsigned unit,
                                unsigned channel)
{
    int pin = esp32_saradc_pin_for(s, unit, channel);
    unsigned counts = pin >= 0 ? s->analog[pin] : 0;
    bool moved = pin != s->adc_pin[unit] || counts != s->adc_data[unit];

    s->adc_pin[unit] = pin;
    s->adc_data[unit] = counts;
    if (moved) {
        esp32_gpio_say_adc(s, unit, channel, pin, counts);
    }
}

static void esp32_saradc_convert(Esp32GpioState *s, uint32_t onetime)
{
    unsigned channel = (onetime >> ESP32_SARADC_ONETIME_CHANNEL_SHIFT)
                       & ESP32_SARADC_ONETIME_CHANNEL_MASK;
    unsigned unit;

    /* Neither unit selected is a start with nothing to start: the silicon
     * has no converter running, and neither has this. */
    if (onetime & ESP32_SARADC_ONETIME_ADC1) {
        unit = 0;
    } else if (onetime & ESP32_SARADC_ONETIME_ADC2) {
        unit = 1;
    } else {
        return;
    }

    esp32_saradc_sample(s, unit, channel);
    s->adc_reg[R_RUSTY_SARADC_INT_RAW] |= unit == 0 ? ESP32_SARADC_DONE_ADC1
                                              : ESP32_SARADC_DONE_ADC2;
}

/*
 * The original ESP32's converter: one register per unit, and the whole
 * conversation in it.
 *
 * `SAR_MEAS_STARTn` is read back by the driver between every step — the
 * done bit and the counts are polled out of the same word it writes the
 * start bit into — so what the guest wrote and what the model answers are
 * kept apart: the word is stored as written, and the two fields the
 * converter owns are put over it on the way out. A model that stored the
 * whole word would answer with whatever the driver's last `modify()`
 * happened to carry, which is the previous reading.
 */
static bool esp32_sens_meas(hwaddr addr, unsigned *unit)
{
    if (addr == ESP32_SENS_MEAS_START1) {
        *unit = 0;
        return true;
    }
    if (addr == ESP32_SENS_MEAS_START2) {
        *unit = 1;
        return true;
    }
    return false;
}

/* Which channel a pad-enable bitmap asks for: the bit that is set.
 *
 * The silicon takes a bitmap because the ULP coprocessor can sweep several
 * pads; a driver doing one conversion sets exactly one bit. None set is a
 * start that names no pad, which reads nothing rather than channel 0. */
static int esp32_sens_channel(uint32_t word)
{
    uint32_t pads = (word >> ESP32_SENS_EN_PAD_SHIFT) & ESP32_SENS_EN_PAD_MASK;

    return pads ? ctz32(pads) : -1;
}

static uint64_t esp32_sens_read(Esp32GpioState *s, hwaddr addr)
{
    unsigned word = addr / 4;
    unsigned unit;

    if (esp32_sens_meas(addr, &unit)) {
        /* Everything the guest wrote, with the counts laid over the data
         * field. The done bit is already the model's in `adc_reg`. */
        return (s->adc_reg[word] & ~ESP32_SENS_DATA_MASK) | s->adc_data[unit];
    }
    return s->adc_reg[word];
}

static void esp32_sens_write(Esp32GpioState *s, hwaddr addr, uint32_t value)
{
    unsigned word = addr / 4;
    unsigned unit;
    uint32_t before;
    int channel;

    if (!esp32_sens_meas(addr, &unit)) {
        s->adc_reg[word] = value;
        return;
    }
    before = s->adc_reg[word];
    /* The done bit is the model's, never the guest's: a driver's
     * read-modify-write carries the bit it just read back in, and storing
     * that would leave the *next* conversion looking finished before it
     * started. */
    s->adc_reg[word] = value & ~ESP32_SENS_DONE;

    if ((~before & value & ESP32_SENS_START) != 0) {
        channel = esp32_sens_channel(value);
        if (channel >= 0) {
            esp32_saradc_sample(s, unit, (unsigned)channel);
            s->adc_reg[word] |= ESP32_SENS_DONE;
        }
    } else if ((value & ESP32_SENS_START) == 0) {
        /* Lowering start is how esp-hal begins each conversion, and on the
         * silicon that is when the done bit goes away. Without this the
         * driver's first poll sees the *previous* conversion's flag and
         * reads a stale sample every time but the first. */
        s->adc_reg[word] &= ~ESP32_SENS_DONE;
    }
}

static uint64_t esp32_saradc_read(void *opaque, hwaddr addr, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    unsigned word = addr / 4;

    if (word >= ESP32_SARADC_WORDS) {
        return 0;
    }
    if (s->saradc_esp32) {
        return esp32_sens_read(s, addr);
    }
    switch (addr) {
    case A_RUSTY_SARADC_1_DATA:
        return s->adc_data[0];

    case A_RUSTY_SARADC_2_DATA:
        return s->adc_data[1];

    /* The masked view beside the raw one. A polling driver reads the raw
     * bit; a driver using the interrupt reads this. */
    case A_RUSTY_SARADC_INT_ST:
        return s->adc_reg[R_RUSTY_SARADC_INT_RAW] & s->adc_reg[R_RUSTY_SARADC_INT_ENA];

    default:
        return s->adc_reg[word];
    }
}

static void esp32_saradc_write(void *opaque, hwaddr addr, uint64_t value,
                               unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    unsigned word = addr / 4;
    uint32_t before;

    if (word >= ESP32_SARADC_WORDS) {
        return;
    }
    if (s->saradc_esp32) {
        esp32_sens_write(s, addr, (uint32_t)value);
        return;
    }
    switch (addr) {
    /* A conversion's result and the fact that it happened are the model's
     * to say; a driver writing them would be telling itself a story. */
    case A_RUSTY_SARADC_1_DATA:
    case A_RUSTY_SARADC_2_DATA:
    case A_RUSTY_SARADC_INT_RAW:
    case A_RUSTY_SARADC_INT_ST:
        break;

    case A_RUSTY_SARADC_INT_CLR:
        s->adc_reg[R_RUSTY_SARADC_INT_RAW] &= ~(uint32_t)value;
        break;

    case A_RUSTY_SARADC_ONETIME:
        before = s->adc_reg[R_RUSTY_SARADC_ONETIME];
        s->adc_reg[R_RUSTY_SARADC_ONETIME] = (uint32_t)value;
        /* On the *rising* edge of START and only there. A driver that
         * leaves the bit set and writes the register again for another
         * reason — changing the channel, say — would otherwise convert
         * twice and the second reading would answer the first request. */
        if ((~before & (uint32_t)value & ESP32_SARADC_ONETIME_START) != 0) {
            esp32_saradc_convert(s, (uint32_t)value);
        }
        break;

    default:
        s->adc_reg[word] = (uint32_t)value;
        break;
    }
}

static const MemoryRegionOps saradc_ops = {
    .read =  esp32_saradc_read,
    .write = esp32_saradc_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

/*
 * ============================= the I2C master =============================
 *
 * Enough of the peripheral for a driver to run a transaction, and a bus
 * whose devices are register files the host declares. That covers what
 * nearly every board here has on it: a sensor is a set of registers a
 * driver reads, a display is a stream of bytes somebody wants to look at.
 *
 * The transaction is the command list. A driver fills `COMD0..7` with the
 * steps — start, write these bytes, start again, read that many, stop —
 * puts the outgoing bytes in the TX FIFO and sets `TRANS_START`; this
 * walks the list, moves the bytes, and sets the done bit on each step and
 * the completion interrupt at the end. Nothing is timed: the silicon takes
 * microseconds at 100 kHz and the driver waits for the interrupt either
 * way, so a clock here would add only a way to lose a byte.
 *
 * **An address nobody declared is not answered.** The transaction stops
 * with `NACK` and `RESP_REC` clear, which is exactly what a bus scan reads
 * and what tells "the part is not on this board" from "the part is there
 * and quiet". Answering zeros instead would make every scan find every
 * address, which is worse than finding none.
 */

/* Which slot of the last-report memory a verb uses: a write, a read, and
 * everything else. See the note beside `ESP32_BUS_VERBS`. */
static unsigned esp32_bus_verb(const char *verb)
{
    switch (verb[0]) {
    case 'w':
        return 0;
    case 'r':
        return 1;
    default:
        return 2;
    }
}

/* The device at an address, or NULL. Linear over sixteen entries: the list
 * is short and the alternative is a 128-entry table mostly full of nothing. */
static Esp32I2cDevice *esp32_i2c_device(Esp32GpioState *s, int address)
{
    for (int i = 0; i < ESP32_I2C_DEVICES; i++) {
        if (s->i2c_devices[i].present && s->i2c_devices[i].address == address) {
            return &s->i2c_devices[i];
        }
    }
    return NULL;
}

/* The device at an address, made if there is room. NULL means the bus is
 * full, which the host is told rather than left to wonder about. */
static Esp32I2cDevice *esp32_i2c_declare(Esp32GpioState *s, int address)
{
    Esp32I2cDevice *found = esp32_i2c_device(s, address);

    if (found) {
        return found;
    }
    for (int i = 0; i < ESP32_I2C_DEVICES; i++) {
        if (!s->i2c_devices[i].present) {
            s->i2c_devices[i].present = true;
            s->i2c_devices[i].address = (uint8_t)address;
            s->i2c_devices[i].pointer = 0;
            memset(s->i2c_devices[i].regs, 0, sizeof(s->i2c_devices[i].regs));
            return &s->i2c_devices[i];
        }
    }
    return NULL;
}

/*
 * What happened on the bus, on the same channel the pins travel.
 *
 * `[rusty:i2c@<us>] 3c w 00af` is a write and `… 68 r 68` a read; `… 68 nak`
 * is an address that answered nothing.
 *
 * The same transaction twice in a row is said once. A driver reading an
 * accelerometer at a kilohertz is the ordinary case, and a line each would
 * put twenty kilobytes a second down the channel the console and the board
 * share — the flooding rule the pins and the duty channel already follow.
 * One deep and no deeper, because interleaved traffic is a driver doing
 * different things and all of it is worth seeing; only the repetition is
 * noise.
 *
 * **A device with no registers is written to rather than read from, and
 * every one of its writes is said.** That is what a display is, and its
 * writes repeat by their nature: clearing a screen is the same sixteen
 * zero bytes sixty-four times over, each landing somewhere else in its
 * memory. Suppressed, sixty-three of them vanish and whatever reads the
 * stream draws a screen with one line on it. The rule is the declaration's:
 * an address the host gave registers is a sensor, an address it declared
 * bare is a display.
 */
/* Whether this address has registers behind it, which is the difference
 * between a sensor and a display: the host declares the first with
 * `i2c 68:75=68` and the second with a bare `i2c 3c=+`. */
static bool esp32_i2c_talks_back(Esp32GpioState *s, int address)
{
    Esp32I2cDevice *device = esp32_i2c_device(s, address);

    return device != NULL && device->has_regs;
}

static void esp32_gpio_say_i2c(Esp32GpioState *s, int address, const char *verb,
                               const uint8_t *bytes, unsigned count)
{
    char body[ESP32_I2C_REPORT];
    char line[ESP32_I2C_REPORT + 40];
    int at;

    if (!qemu_chr_fe_backend_connected(&s->pins)) {
        return;
    }
    at = snprintf(body, sizeof(body), "%02x %s", address, verb);
    if (count) {
        at += snprintf(body + at, sizeof(body) - at, " ");
    }
    for (unsigned i = 0; i < count && at < (int)sizeof(body) - 4; i++) {
        at += snprintf(body + at, sizeof(body) - at, "%02x", bytes[i]);
    }
    if (!esp32_i2c_talks_back(s, address) && esp32_bus_verb(verb) == 0) {
        /* A display: say every write, and remember none of them. */
        s->i2c_last_report[esp32_bus_verb(verb)][0] = '\0';
    } else if (strcmp(body, s->i2c_last_report[esp32_bus_verb(verb)]) == 0) {
        return;
    } else {
    /* `snprintf` rather than a copy: `pstrcpy` lives in `qemu/cutils.h`,
     * which `osdep.h` does not pull in, and `strcpy` into a fixed field is
     * the wrong habit to reach for even when the source is known short. */
        snprintf(s->i2c_last_report[esp32_bus_verb(verb)],
                 sizeof(s->i2c_last_report[0]), "%s", body);
    }

    at = snprintf(line, sizeof(line), "[rusty:i2c@%" PRId64 "] %s\n",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL), body);
    qemu_chr_fe_write_all(&s->pins, (const uint8_t *)line, at);
}

/* Take one byte the guest queued, or zero when it queued too few — which is
 * a driver bug rather than a bus event, so it is not reported as traffic. */
static uint8_t esp32_i2c_take(Esp32GpioState *s)
{
    if (s->i2c_tx_at < s->i2c_tx_len) {
        return s->i2c_tx[s->i2c_tx_at++];
    }
    return 0;
}

static void esp32_i2c_give(Esp32GpioState *s, uint8_t byte)
{
    if (s->i2c_rx_len < ESP32_I2C_FIFO) {
        s->i2c_rx[s->i2c_rx_len++] = byte;
    }
}

/*
 * One `WRITE` step: the address byte when the bus has just been started,
 * data for the addressed device after that.
 *
 * Returns false when nothing answered, which stops the transaction where
 * the silicon would.
 */
static bool esp32_i2c_write_step(Esp32GpioState *s, unsigned bytes)
{
    uint8_t data[ESP32_I2C_FIFO];
    unsigned taken = 0;
    Esp32I2cDevice *device;

    if (s->i2c_expect_address && bytes > 0) {
        uint8_t header = esp32_i2c_take(s);

        s->i2c_address = header >> 1;
        s->i2c_expect_address = false;
        bytes--;

        if (!esp32_i2c_device(s, s->i2c_address)) {
            s->i2c_reg[R_RUSTY_I2C_INT_RAW] |= ESP32_I2C_INT_NACK;
            s->i2c_reg[R_RUSTY_I2C_SR] &= ~ESP32_I2C_SR_RESP_REC;
            esp32_gpio_say_i2c(s, s->i2c_address, "nak", NULL, 0);
            return false;
        }
        s->i2c_reg[R_RUSTY_I2C_SR] |= ESP32_I2C_SR_RESP_REC;
    }

    device = esp32_i2c_device(s, s->i2c_address);
    while (taken < bytes && taken < ESP32_I2C_FIFO) {
        data[taken++] = esp32_i2c_take(s);
    }
    if (!device) {
        /* Data with nobody addressed. On a real bus that is a driver fault;
         * here it was the model's, and returning quietly is what made a
         * transaction that did nothing look like a bus with nothing on it. */
        esp32_gpio_say_i2c(s, s->i2c_address < 0 ? 0 : s->i2c_address,
                           s->i2c_address < 0 ? "?unaddressed" : "nak",
                           NULL, 0);
        return false;
    }
    /* The first data byte of a write moves the register pointer and the
     * rest land from there — an I2C register file, which is what a sensor
     * is. A display ignores the pointer and cares only that the bytes were
     * seen, which the same rule gives for free. */
    for (unsigned i = 0; i < taken; i++) {
        if (i == 0) {
            device->pointer = data[i];
        } else {
            device->regs[device->pointer++] = data[i];
        }
    }
    if (taken) {
        /* `w+` is more of the transaction already going: a driver sending a
         * framebuffer writes a thousand bytes through a thirty-two byte
         * FIFO, so one transaction is a run of steps, and whatever reads
         * the stream has to know that the first byte of the second step is
         * *not* the first byte of a message. A display's control byte comes
         * once per transaction and decides what every byte after it means;
         * read as though each step began one, the picture is nonsense. */
        esp32_gpio_say_i2c(s, s->i2c_address, s->i2c_continues ? "w+" : "w",
                           data, taken);
        s->i2c_continues = true;
    }
    return true;
}

/* One `READ` step: bytes from the addressed device's registers, from the
 * pointer onwards, exactly as a sensor answers. */
static bool esp32_i2c_read_step(Esp32GpioState *s, unsigned bytes)
{
    uint8_t data[ESP32_I2C_FIFO];
    unsigned given = 0;
    Esp32I2cDevice *device = esp32_i2c_device(s, s->i2c_address);

    if (!device) {
        s->i2c_reg[R_RUSTY_I2C_INT_RAW] |= ESP32_I2C_INT_NACK;
        s->i2c_reg[R_RUSTY_I2C_SR] &= ~ESP32_I2C_SR_RESP_REC;
        esp32_gpio_say_i2c(s, s->i2c_address, "nak", NULL, 0);
        return false;
    }
    while (given < bytes && given < ESP32_I2C_FIFO) {
        data[given] = device->regs[device->pointer++];
        esp32_i2c_give(s, data[given]);
        given++;
    }
    s->i2c_reg[R_RUSTY_I2C_SR] |= ESP32_I2C_SR_RESP_REC;
    esp32_gpio_say_i2c(s, s->i2c_address, s->i2c_continues ? "r+" : "r",
                       data, given);
    s->i2c_continues = true;
    return true;
}

/*
 * Run the command list.
 *
 * `END` pauses rather than finishes: the driver refills the FIFO and starts
 * again, and the bus is still held by the same device in the same
 * direction — which is why the address survives across it.
 */
static void esp32_i2c_run(Esp32GpioState *s)
{
    for (unsigned i = 0; i < s->i2c_commands; i++) {
        uint32_t command = s->i2c_reg[R_RUSTY_I2C_COMD0 + i];
        unsigned op = (command >> ESP32_I2C_CMD_OP_SHIFT) & ESP32_I2C_CMD_OP_MASK;
        unsigned bytes = command & ESP32_I2C_CMD_BYTES_MASK;

        /*
         * A zero word is *not* an empty slot, however much it looks like
         * one. On the original ESP32 `RSTART` is opcode zero with no byte
         * count and no ack bits, so a start command and an unused slot are
         * the same thirty-two bits — and the silicon needs no way to tell
         * them apart, because it stops at the `STOP` or `END` a driver
         * always ends with.
         *
         * Breaking on a zero word here made every transaction execute
         * nothing at all: it completed, with no acknowledgement, so every
         * address on the bus looked absent and not one byte was ever
         * reported. Which is precisely how it read — a bus with nothing on
         * it, from a model that had never got as far as looking.
         */
        s->i2c_reg[R_RUSTY_I2C_COMD0 + i] = command | ESP32_I2C_CMD_DONE;

        /* An if-chain rather than a switch, because which number means
         * which step is the part's and not a constant. */
        if (op == s->i2c_op_rstart) {
            s->i2c_expect_address = true;
            s->i2c_continues = false;
        } else if (op == s->i2c_op_write) {
            if (!esp32_i2c_write_step(s, bytes)) {
                return;
            }
        } else if (op == s->i2c_op_read) {
            if (!esp32_i2c_read_step(s, bytes)) {
                return;
            }
        } else if (op == s->i2c_op_stop) {
            s->i2c_reg[R_RUSTY_I2C_INT_RAW] |= ESP32_I2C_INT_TRANS_COMPLETE;
            s->i2c_expect_address = false;
            s->i2c_continues = false;
            s->i2c_address = -1;
            return;
        } else if (op == s->i2c_op_end) {
            s->i2c_reg[R_RUSTY_I2C_INT_RAW] |= ESP32_I2C_INT_END_DETECT;
            return;
        } else {
            /* A step this model does not know says so, rather than being
             * skipped. Skipping is how the op codes being wrong stayed
             * invisible: the transaction ran, did nothing, reported
             * nothing, and the bus read as empty. Saying it is what turned
             * the ESP32's own numbering from three rounds of "the firmware
             * found nothing" into one line of `?op0`. */
            char unknown[12];

            snprintf(unknown, sizeof(unknown), "?op%u", op);
            esp32_gpio_say_i2c(s, s->i2c_address < 0 ? 0 : s->i2c_address,
                               unknown, NULL, 0);
        }
    }
    /* A list that ran off its end without a stop still completed: the
     * driver is waiting on the interrupt and nothing else will set it. */
    s->i2c_reg[R_RUSTY_I2C_INT_RAW] |= ESP32_I2C_INT_TRANS_COMPLETE;
}

static uint64_t esp32_i2c_read(void *opaque, hwaddr addr, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    unsigned word = addr / 4;

    if (word >= ESP32_I2C_WORDS) {
        return 0;
    }
    switch (addr) {
    case A_RUSTY_I2C_DATA:
        return s->i2c_rx_at < s->i2c_rx_len ? s->i2c_rx[s->i2c_rx_at++] : 0;

    /* What is *left* in each FIFO, which is what the driver counts. */
    case A_RUSTY_I2C_SR: {
        uint32_t status = s->i2c_reg[R_RUSTY_I2C_SR]
                          & ~((0x3fu << ESP32_I2C_SR_RXFIFO_CNT_SHIFT)
                              | (0x3fu << ESP32_I2C_SR_TXFIFO_CNT_SHIFT)
                              | ESP32_I2C_SR_BUS_BUSY);

        status |= (uint32_t)(s->i2c_rx_len - s->i2c_rx_at)
                  << ESP32_I2C_SR_RXFIFO_CNT_SHIFT;
        status |= (uint32_t)(s->i2c_tx_len - s->i2c_tx_at)
                  << ESP32_I2C_SR_TXFIFO_CNT_SHIFT;
        return status;
    }

    case A_RUSTY_I2C_INT_STATUS:
        return s->i2c_reg[R_RUSTY_I2C_INT_RAW] & s->i2c_reg[R_RUSTY_I2C_INT_ENA];

    default:
        return s->i2c_reg[word];
    }
}

static void esp32_i2c_write(void *opaque, hwaddr addr, uint64_t value,
                            unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    unsigned word = addr / 4;

    if (word >= ESP32_I2C_WORDS) {
        return;
    }
    switch (addr) {
    case A_RUSTY_I2C_DATA:
        if (s->i2c_tx_len < ESP32_I2C_FIFO) {
            s->i2c_tx[s->i2c_tx_len++] = (uint8_t)value;
        }
        return;

    /* Read-only: what the bus did is the model's to say. */
    case A_RUSTY_I2C_SR:
    case A_RUSTY_I2C_INT_STATUS:
        return;

    case A_RUSTY_I2C_INT_CLR:
        s->i2c_reg[R_RUSTY_I2C_INT_RAW] &= ~(uint32_t)value;
        return;

    case A_RUSTY_I2C_FIFO_CONF:
        s->i2c_reg[R_RUSTY_I2C_FIFO_CONF] = (uint32_t)value;
        /* The driver sets each reset bit and clears it again, so the act is
         * the bit going up. */
        if (value & ESP32_I2C_TX_FIFO_RST) {
            s->i2c_tx_len = 0;
            s->i2c_tx_at = 0;
        }
        if (value & ESP32_I2C_RX_FIFO_RST) {
            s->i2c_rx_len = 0;
            s->i2c_rx_at = 0;
        }
        return;

    /* The bus-clear a driver runs after a NACK: nine SCL pulses to unstick a
     * slave that is holding SDA down. There is no bus here to unstick, so it
     * is over the moment it is asked for — and the bit has to come back
     * clear, because that is what `ClearBusFuture` waits on. Left set, every
     * recovery costs the driver's fifty-millisecond timeout, and esp-hal
     * recovers after every NACK. */
    case A_RUSTY_I2C_SCL_SP_CONF:
        s->i2c_reg[R_RUSTY_I2C_SCL_SP_CONF] =
            (uint32_t)value & ~ESP32_I2C_SCL_RST_SLV_EN;
        return;

    case A_RUSTY_I2C_CTR:
        /* All three of `CTR`'s write-triggered bits come back clear: the
         * hardware acts on them and clears them, and a driver reads them
         * back to find out that it has. Storing them is a bit that never
         * falls. */
        s->i2c_reg[R_RUSTY_I2C_CTR] =
            (uint32_t)value & ~ESP32_I2C_CTR_SELF_CLEARING;
        if (value & ESP32_I2C_TRANS_START) {
            /* Each start reads the FIFO the driver has just filled, from
             * the beginning, and produces a fresh answer. */
            s->i2c_tx_at = 0;
            s->i2c_rx_len = 0;
            s->i2c_rx_at = 0;
            esp32_i2c_run(s);
            s->i2c_tx_len = 0;
            s->i2c_tx_at = 0;
        }
        return;

    default:
        s->i2c_reg[word] = (uint32_t)value;
        return;
    }
}

static const MemoryRegionOps i2c_ops = {
    .read =  esp32_i2c_read,
    .write = esp32_i2c_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

/*
 * ============================= the SPI master =============================
 *
 * Upstream models `SPI1`, the flash controller the machine boots through,
 * and nothing at `SPI2` — the one a project puts a display or a sensor on.
 * So a driver's first transfer sets `USR` and polls it for ever, which is
 * the same hang the converter and the bus had before they were modelled.
 *
 * Bytes out of the sixteen data words and bytes back into them, in one
 * step. No timing and no clock: a CPU-driven transfer is over before the
 * driver's next instruction either way, and a delay here would only be
 * another way to lose a byte.
 */

/* Which chip select is asserted: `MISC` bits 0..5 *disable* each one, so
 * the active line is the lowest bit that is clear. Zero when the driver has
 * disabled them all, which is a transfer to nothing in particular — still
 * worth reporting, since the bytes went out. */
static unsigned esp32_spi_select(Esp32GpioState *s)
{
    uint32_t disables = s->spi_reg[s->spi_cs_reg / 4];

    for (unsigned cs = 0; cs < s->spi_selects; cs++) {
        if ((disables & (1u << cs)) == 0) {
            return cs;
        }
    }
    return 0;
}

/* One byte of the data words, which are little-endian as the bus is. */
static uint8_t esp32_spi_byte(Esp32GpioState *s, unsigned at)
{
    uint32_t word = s->spi_reg[s->spi_w0_reg / 4 + at / 4];

    return (uint8_t)(word >> (8 * (at % 4)));
}

static void esp32_spi_put(Esp32GpioState *s, unsigned at, uint8_t byte)
{
    unsigned shift = 8 * (at % 4);
    uint32_t *word = &s->spi_reg[s->spi_w0_reg / 4 + at / 4];

    *word = (*word & ~(0xffu << shift)) | ((uint32_t)byte << shift);
}

/* What crossed the wire, with the same one-deep repeat rule the bus has and
 * for the same reason: a display refreshing at sixty hertz is a transfer a
 * frame, and a driver polling a sensor is far more. */
static void esp32_gpio_say_spi(Esp32GpioState *s, unsigned cs, const char *verb,
                               const uint8_t *bytes, unsigned count)
{
    char body[ESP32_I2C_REPORT];
    char line[ESP32_I2C_REPORT + 40];
    int at;

    if (!qemu_chr_fe_backend_connected(&s->pins)) {
        return;
    }
    at = snprintf(body, sizeof(body), "%u %s ", cs, verb);
    for (unsigned i = 0; i < count && at < (int)sizeof(body) - 4; i++) {
        at += snprintf(body + at, sizeof(body) - at, "%02x", bytes[i]);
    }
    if (s->spi_miso_len[cs] == 0 && strcmp(verb, "w") == 0) {
        /* A device that says nothing back is one that is written to — a
         * display — and every transfer to it is a different part of its
         * screen. See the I2C report above for why none is suppressed. */
        s->spi_last_report[esp32_bus_verb(verb)][0] = '\0';
    } else if (strcmp(body, s->spi_last_report[esp32_bus_verb(verb)]) == 0) {
        return;
    } else {
        snprintf(s->spi_last_report[esp32_bus_verb(verb)],
                 sizeof(s->spi_last_report[0]), "%s", body);
    }

    at = snprintf(line, sizeof(line), "[rusty:spi@%" PRId64 "] %s\n",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL), body);
    qemu_chr_fe_write_all(&s->pins, (const uint8_t *)line, at);
}

/*
 * One transfer: the bytes the driver put in the data words go out, and what
 * the host declared for this chip select comes back into the same words.
 *
 * From the start of the buffer every time, deliberately. SPI has no
 * addressing to key an answer on, so any other rule would be a convention
 * this model invented — and a driver that sends a command byte and reads the
 * reply in the same transfer gets it at the offset it expects, which is what
 * full duplex means.
 */
static void esp32_spi_transfer(Esp32GpioState *s)
{
    uint32_t user = s->spi_reg[s->spi_user_reg / 4];
    unsigned bits = (s->spi_reg[s->spi_dlen_reg / 4] & ESP32_SPI_DLEN_MASK) + 1;
    unsigned bytes = MIN((bits + 7) / 8, (unsigned)ESP32_SPI_BUFFER);
    unsigned cs = esp32_spi_select(s);
    uint8_t moving[ESP32_SPI_BUFFER];

    if (user & ESP32_SPI_USER_MOSI) {
        for (unsigned i = 0; i < bytes; i++) {
            moving[i] = esp32_spi_byte(s, i);
        }
        esp32_gpio_say_spi(s, cs, "w", moving, bytes);
    }
    if (user & ESP32_SPI_USER_MISO) {
        for (unsigned i = 0; i < bytes; i++) {
            /* Past what the host declared is zero, which is what an
             * undriven MISO line reads as. Not an error: a display has
             * nothing to say and every transfer to one lands here. */
            moving[i] = i < s->spi_miso_len[cs] ? s->spi_miso[cs][i] : 0;
            esp32_spi_put(s, i, moving[i]);
        }
        esp32_gpio_say_spi(s, cs, "r", moving, bytes);
    }
    s->spi_reg[s->spi_done_reg / 4] |= s->spi_done_bit;
}

static uint64_t esp32_spi_read(void *opaque, hwaddr addr, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    unsigned word = addr / 4;

    return word < ESP32_SPI_WORDS ? s->spi_reg[word] : 0;
}

static void esp32_spi_write(void *opaque, hwaddr addr, uint64_t value,
                            unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    unsigned word = addr / 4;

    if (word >= ESP32_SPI_WORDS) {
        return;
    }
    if (addr == ESP32_SPI_CMD) {
        /* Both bits are self-clearing: `UPDATE` latches the configuration
         * and `USR` runs the transfer. Storing either would leave the
         * driver polling a bit that never falls, which is the hang this
         * whole model exists to remove. */
        s->spi_reg[word] =
            (uint32_t)value & ~(s->spi_cmd_usr | s->spi_cmd_update);
        if (value & s->spi_cmd_usr) {
            esp32_spi_transfer(s);
        }
        return;
    }
    if (addr == s->spi_done_clr_reg) {
        if (s->spi_done_w1c) {
            /* A one puts the flag away and the register itself holds
             * nothing worth keeping. */
            s->spi_reg[s->spi_done_reg / 4] &= ~(uint32_t)value;
        } else {
            /* The flag lives in this register: what the driver wrote is
             * what it holds, which is how it clears the bit. */
            s->spi_reg[word] = (uint32_t)value;
        }
        return;
    }
    s->spi_reg[word] = (uint32_t)value;
}

static const MemoryRegionOps spi_ops = {
    .read =  esp32_spi_read,
    .write = esp32_spi_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

/*
 * ============================== the dimmer ==============================
 *
 * LEDC. What a driver does to it is short and exact: set the clock source,
 * give a timer a divider and a resolution and latch it, point a channel at
 * that timer and enable its output, write a duty, start it, latch it. This
 * models that, and reports the result as a duty and a frequency on the pin
 * the GPIO matrix sends the channel to.
 *
 * Everything the model has no opinion about is shadowed, so a driver's
 * read-modify-write keeps what it put there.
 */

/* Which pad a peripheral signal reaches, or -1 when the matrix sends it
 * nowhere. The first pad wins: the same signal on two pads is a board
 * nobody builds, and a rule that answered with the last would depend on
 * the order a driver happened to configure them in. */
static int esp32_gpio_signal_pin(Esp32GpioState *s, unsigned signal)
{
    for (int pin = 0; pin < ESP32_GPIO_PINS; pin++) {
        if ((s->func_out[pin] & ESP32_GPIO_OUT_SEL_MASK) == signal
            && (s->enable & (1ULL << pin))) {
            return pin;
        }
    }
    return -1;
}

/* What a channel's timer counts to, and how fast.
 *
 * `res` is the resolution in bits and `div` the divider in Q10.8, both as
 * the timer last latched them. A timer nobody has configured counts to
 * nothing, and a channel following it has no duty to report — said as
 * "not driving" rather than as zero, because a servo commanded to its
 * lowest angle and a servo nobody has configured are different boards.
 */
/* Which timer a channel follows.
 *
 * `TIMER_SEL` is two bits, and on a part with two halves it names a timer
 * *within the channel's own half*: low-speed channel 0 pointed at timer 1
 * means `LSTIMER1`, which the model numbers 5. A model that read it flat
 * would give every low-speed channel a high-speed timer's resolution, and
 * a duty divided by the wrong denominator is a wrong angle rather than a
 * missing one.
 */
static unsigned esp32_ledc_timer_of(Esp32GpioState *s, unsigned channel)
{
    unsigned sel = s->ledc_ch[channel].conf0 & ESP32_LEDC_CONF0_TIMER_MASK;
    unsigned base = channel < s->ledc_hs_channels ? 0 : s->ledc_hs_timers;

    return base + sel;
}

/* Whether this part makes the index wait for a `PARA_UP`.
 *
 * The ESP32's high-speed half has no such bit: what is written to it is
 * what is driving. Waiting for a latch that does not exist would leave
 * every high-speed channel reported as not driving for ever. */
static bool esp32_ledc_latched(unsigned index, unsigned high_speed)
{
    return index >= high_speed;
}

static bool esp32_ledc_shape(Esp32GpioState *s, unsigned channel,
                             double *duty, double *hz)
{
    Esp32LedcChannel *ch = &s->ledc_ch[channel];
    unsigned which = esp32_ledc_timer_of(s, channel);
    Esp32LedcTimer *timer = &s->ledc_timer[which];
    double source;
    double full;

    if (!(ch->conf0 & ESP32_LEDC_CONF0_SIG_OUT_EN)) {
        return false;
    }
    if (timer->res == 0 || timer->div == 0
        || (timer->conf & (s->ledc_timer_pause | s->ledc_timer_rst))) {
        return false;
    }

    if (s->ledc_clock_per_timer) {
        /* Each timer names its own: the APB clock, or REF_TICK. */
        source = (timer->conf & ESP32_LEDC_TIMER_TICK_SEL_ESP32)
                     ? ESP32_LEDC_CLK_APB
                     : ESP32_LEDC_CLK_REF_TICK;
    } else {
        switch (s->ledc_conf & ESP32_LEDC_CLK_SEL_MASK) {
        case 2:
            source = ESP32_LEDC_CLK_RC_FAST;
            break;
        case 3:
            source = ESP32_LEDC_CLK_XTAL;
            break;
        default:
            /* Zero is "no clock chosen", which every driver leaves behind
             * the moment it configures a timer; treating it as the APB
             * clock keeps a frequency reportable for firmware that never
             * wrote CONF. */
            source = ESP32_LEDC_CLK_APB;
            break;
        }
    }

    full = (double)(1u << timer->res);
    *duty = (double)ch->live / full;
    if (*duty > 1.0) {
        *duty = 1.0;
    }
    /* The divider is Q10.8 against the source, and the timer counts `full`
     * ticks per period. */
    *hz = source / ((double)timer->div / 256.0) / full;
    return true;
}

/*
 * Say what a channel is doing, once per change.
 *
 * `[rusty:pwm@<us>] <pin>=<duty>@<hz>` — the line rusty already reads, with
 * the frequency after it, because a duty alone cannot say what a servo
 * does: 7.5% is the middle of its travel at 50 Hz and nothing at all at
 * 1 kHz. A firmware narrating its own duty still sends the shorter form,
 * and both are the same reading of the same pin.
 *
 * A channel that stops driving says so on the pin it was driving, at the
 * level it idles to, so a board does not keep showing a duty nothing is
 * still producing.
 */
static void esp32_ledc_say(Esp32GpioState *s, unsigned channel)
{
    char line[ESP32_I2C_REPORT];
    double duty = 0.0;
    double hz = 0.0;
    int pin = esp32_gpio_signal_pin(s, s->ledc_sig0 + channel);
    bool driving = pin >= 0 && esp32_ledc_shape(s, channel, &duty, &hz);
    int at;

    if (!driving) {
        /* Nothing to say unless this channel had been driving a pin: the
         * pad is GPIO's again, and GPIO reports its own levels. */
        if (s->ledc_said_pin[channel] < 0) {
            return;
        }
        at = snprintf(line, sizeof(line), "[rusty:pwm@%" PRId64 "] %d=%.4f\n",
                      qemu_clock_get_us(QEMU_CLOCK_VIRTUAL),
                      s->ledc_said_pin[channel],
                      (s->ledc_ch[channel].conf0 & ESP32_LEDC_CONF0_IDLE_LV)
                          ? 1.0 : 0.0);
        s->ledc_said_pin[channel] = -1;
        s->ledc_said[channel][0] = '\0';
        if (at > 0) {
            qemu_chr_fe_write_all(&s->pins, (uint8_t *)line, at);
        }
        return;
    }

    at = snprintf(line, sizeof(line), "%d=%.4f@%.1f", pin, duty, hz);
    if (at <= 0) {
        return;
    }
    /* Per change, like every other report on this channel: a driver that
     * writes the same duty every time round its loop would otherwise put a
     * line on the wire for each. */
    if (pin == s->ledc_said_pin[channel]
        && strncmp(line, s->ledc_said[channel], sizeof(line)) == 0) {
        return;
    }
    /* `snprintf` rather than a copy, for the reason the bus reports give
     * above: `pstrcpy` is in a header `osdep.h` does not pull in. */
    snprintf(s->ledc_said[channel], sizeof(s->ledc_said[channel]), "%s", line);
    s->ledc_said_pin[channel] = pin;

    at = snprintf(line, sizeof(line), "[rusty:pwm@%" PRId64 "] %s\n",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL),
                  s->ledc_said[channel]);
    if (at > 0) {
        qemu_chr_fe_write_all(&s->pins, (uint8_t *)line, at);
    }
}

/* Every channel, after something that could have changed any of them: a
 * timer latched, the clock source chosen, the matrix repointed. */
static void esp32_ledc_say_all(Esp32GpioState *s)
{
    for (unsigned channel = 0; channel < s->ledc_channels; channel++) {
        esp32_ledc_say(s, channel);
    }
}

/* The duty a fade ends on.
 *
 * The silicon walks there over `num` steps of `scale`, one every `cycle`
 * periods, and raises the fade-done interrupt when it arrives. This model
 * arrives at once and raises it, which is the same decision the converter's
 * instant conversion makes: the waiting is what a driver polls for, and the
 * end value is what anybody watching the board can act on. A fade that took
 * its time would need a timer per channel and would report a line per step
 * on the channel the console shares.
 */
static uint32_t esp32_ledc_faded(Esp32LedcChannel *ch, unsigned res)
{
    uint32_t start = ch->duty >> ESP32_LEDC_DUTY_FRACTION;
    uint32_t steps = (ch->conf1 >> ESP32_LEDC_CONF1_NUM_SHIFT)
                     & ESP32_LEDC_CONF1_NUM_MASK;
    uint32_t scale = ch->conf1 & ESP32_LEDC_CONF1_SCALE_MASK;
    uint64_t travel = (uint64_t)steps * scale;
    uint64_t full = 1ULL << res;

    if (ch->conf1 & ESP32_LEDC_CONF1_INC) {
        uint64_t end = (uint64_t)start + travel;
        return (uint32_t)(end > full ? full : end);
    }
    return (uint32_t)(travel > start ? 0 : start - travel);
}

static uint64_t esp32_ledc_read(void *opaque, hwaddr addr, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);

    if (addr < ESP32_LEDC_CH0 + ESP32_LEDC_CH_STRIDE * s->ledc_channels) {
        unsigned channel = (addr - ESP32_LEDC_CH0) / ESP32_LEDC_CH_STRIDE;
        Esp32LedcChannel *ch = &s->ledc_ch[channel];
        switch ((addr - ESP32_LEDC_CH0) % ESP32_LEDC_CH_STRIDE) {
        case ESP32_LEDC_CH_CONF0:
            /* `PARA_UP` is write-triggered and reads back clear. */
            return ch->conf0 & ~ESP32_LEDC_CONF0_PARA_UP;
        case ESP32_LEDC_CH_HPOINT:
            return ch->hpoint;
        case ESP32_LEDC_CH_DUTY:
            return ch->duty;
        case ESP32_LEDC_CH_CONF1:
            /* And so is `DUTY_START`: the fade is over by the time anybody
             * can look, and a driver polling this bit is asking exactly
             * that. */
            return ch->conf1 & ~ESP32_LEDC_CONF1_START;
        case ESP32_LEDC_CH_DUTY_R:
            return ch->live << ESP32_LEDC_DUTY_FRACTION;
        default:
            return 0;
        }
    }
    if (addr >= s->ledc_timer0_reg
        && addr < s->ledc_timer0_reg
                      + ESP32_LEDC_TIMER_STRIDE * s->ledc_timers) {
        hwaddr into = addr - s->ledc_timer0_reg;
        unsigned which = into / ESP32_LEDC_TIMER_STRIDE;

        if (into % ESP32_LEDC_TIMER_STRIDE == 0) {
            return s->ledc_timer[which].conf & ~s->ledc_timer_para_up;
        }
        /* The timer's own count, which nothing here keeps: a counter is the
         * one part of this peripheral the model deliberately does not have. */
        return 0;
    }
    if (addr == s->ledc_int_raw_reg) {
        return s->ledc_int_raw;
    }
    if (addr == s->ledc_int_raw_reg + ESP32_LEDC_INT_ST_AT) {
        return s->ledc_int_raw & s->ledc_int_ena;
    }
    if (addr == s->ledc_int_raw_reg + ESP32_LEDC_INT_ENA_AT) {
        return s->ledc_int_ena;
    }
    if (addr == s->ledc_int_raw_reg + ESP32_LEDC_CONF_AT) {
        return s->ledc_conf;
    }
    return 0;
}

static void esp32_ledc_write(void *opaque, hwaddr addr, uint64_t value,
                             unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    uint32_t word = (uint32_t)value;

    if (addr < ESP32_LEDC_CH0 + ESP32_LEDC_CH_STRIDE * s->ledc_channels) {
        unsigned channel = (addr - ESP32_LEDC_CH0) / ESP32_LEDC_CH_STRIDE;
        Esp32LedcChannel *ch = &s->ledc_ch[channel];
        unsigned res = s->ledc_timer[esp32_ledc_timer_of(s, channel)].res;
        bool latched = esp32_ledc_latched(channel, s->ledc_hs_channels);

        switch ((addr - ESP32_LEDC_CH0) % ESP32_LEDC_CH_STRIDE) {
        case ESP32_LEDC_CH_CONF0:
            ch->conf0 = word;
            /* The latch: what the guest has written to this channel takes
             * effect. esp-hal writes the duty, starts it and then latches,
             * so this is where an ordinary `set_duty` lands — on a half
             * that has a latch at all. */
            if (!latched || (word & ESP32_LEDC_CONF0_PARA_UP)) {
                ch->live = ch->duty >> ESP32_LEDC_DUTY_FRACTION;
            }
            esp32_ledc_say(s, channel);
            break;

        case ESP32_LEDC_CH_HPOINT:
            ch->hpoint = word;
            break;

        case ESP32_LEDC_CH_DUTY:
            ch->duty = word;
            /* Where there is no latch, writing the duty *is* setting it. */
            if (!latched) {
                ch->live = ch->duty >> ESP32_LEDC_DUTY_FRACTION;
                esp32_ledc_say(s, channel);
            }
            break;

        case ESP32_LEDC_CH_CONF1:
            ch->conf1 = word;
            if (word & ESP32_LEDC_CONF1_START) {
                ch->live = esp32_ledc_faded(ch, res ? res : 1);
                s->ledc_int_raw |= 1u << (s->ledc_fade_shift + channel);
                esp32_ledc_say(s, channel);
            }
            break;

        default:
            /* `DUTY_R` is read-only; a write to it is a driver's mistake
             * and not this model's to invent behaviour for. */
            break;
        }
        return;
    }

    if (addr >= s->ledc_timer0_reg
        && addr < s->ledc_timer0_reg
                      + ESP32_LEDC_TIMER_STRIDE * s->ledc_timers) {
        hwaddr into = addr - s->ledc_timer0_reg;
        unsigned which = into / ESP32_LEDC_TIMER_STRIDE;
        Esp32LedcTimer *timer = &s->ledc_timer[which];

        if (into % ESP32_LEDC_TIMER_STRIDE != 0) {
            return;
        }
        timer->conf = word;
        if (!esp32_ledc_latched(which, s->ledc_hs_timers)
            || (word & s->ledc_timer_para_up)) {
            timer->res = word & s->ledc_res_mask;
            timer->div = (word >> s->ledc_div_shift)
                         & ESP32_LEDC_TIMER_DIV_MASK;
        }
        /* Every channel, because a timer is shared and its resolution is
         * the denominator of each of their duties. */
        esp32_ledc_say_all(s);
        return;
    }

    if (addr == s->ledc_int_raw_reg + ESP32_LEDC_INT_ENA_AT) {
        s->ledc_int_ena = word;
    } else if (addr == s->ledc_int_raw_reg + ESP32_LEDC_INT_CLR_AT) {
        s->ledc_int_raw &= ~word;
    } else if (addr == s->ledc_int_raw_reg + ESP32_LEDC_CONF_AT) {
        s->ledc_conf = word;
        esp32_ledc_say_all(s);
    }
}

static const MemoryRegionOps ledc_ops = {
    .read = esp32_ledc_read,
    .write = esp32_ledc_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

/*
 * ============================== the strip ===============================
 *
 * RMT's transmitting half. See the header for what is modelled and what is
 * deliberately not.
 */

/* The bit one pulse code carries: a one is the long half high, a zero the
 * short one. Which half is high is the driver's choice and both are seen in
 * the wild, so the rule is written in terms of the high time rather than of
 * the first half. */
static bool esp32_rmt_bit(uint32_t code)
{
    unsigned first = code & ESP32_RMT_DURATION_MASK;
    unsigned second = (code >> ESP32_RMT_SECOND_SHIFT) & ESP32_RMT_DURATION_MASK;
    bool first_high = (code & ESP32_RMT_LEVEL0) != 0;
    unsigned high = first_high ? first : second;
    unsigned low = first_high ? second : first;

    return high > low;
}

/* A code with a half of no length at all is the end of the transmission:
 * that is what `PulseCode::end_marker()` writes, and what every driver
 * finishes its buffer with. */
static bool esp32_rmt_is_end(uint32_t code)
{
    return (code & ESP32_RMT_DURATION_MASK) == 0
           || ((code >> ESP32_RMT_SECOND_SHIFT) & ESP32_RMT_DURATION_MASK) == 0;
}

/* Say what went out, once a transmission has finished.
 *
 * `[rusty:rmt@<us>] <pin> <hex>` — the bytes, in the order the wire carried
 * them, on the pin the matrix sends the channel to. A transmission the
 * matrix sends nowhere is not reported: the codes went into a pad no part
 * of the board is on.
 */
/* Where this part reports a channel's two events in `INT_RAW`. Computed
 * rather than shifted: the ESP32 packs three bits per channel in a run and
 * the C3 gathers each event's bits into a band of its own. */
static unsigned esp32_rmt_end_bit(Esp32GpioState *s, unsigned channel)
{
    return channel * s->rmt_end_stride;
}

static unsigned esp32_rmt_thr_bit(Esp32GpioState *s, unsigned channel)
{
    return s->rmt_thr_shift + channel;
}

static void esp32_rmt_say(Esp32GpioState *s, unsigned channel)
{
    Esp32RmtChannel *ch = &s->rmt_ch[channel];
    char line[2 * ESP32_RMT_BYTES + 64];
    int pin = esp32_gpio_signal_pin(s, s->rmt_sig0 + channel);
    int at;

    if (pin < 0 || ch->byte_count == 0) {
        return;
    }
    at = snprintf(line, sizeof(line), "[rusty:rmt@%" PRId64 "] %d ",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL), pin);
    for (unsigned i = 0; i < ch->byte_count && at < (int)sizeof(line) - 8; i++) {
        at += snprintf(line + at, sizeof(line) - at, "%02x", ch->bytes[i]);
    }
    if (ch->dropped) {
        /* Said rather than silently cut: a strip longer than this buffer
         * would otherwise look like a shorter one. */
        at += snprintf(line + at, sizeof(line) - at, " +%u", ch->dropped);
    }
    at += snprintf(line + at, sizeof(line) - at, "\n");
    qemu_chr_fe_write_all(&s->pins, (uint8_t *)line, at);
}

/* One bit onto the end of what this transmission has carried. */
static void esp32_rmt_push(Esp32RmtChannel *ch, bool bit)
{
    unsigned index = ch->bit_count / 8;

    if (index >= ESP32_RMT_BYTES) {
        ch->dropped++;
        ch->bit_count++;
        return;
    }
    if (ch->bit_count % 8 == 0) {
        ch->bytes[index] = 0;
    }
    ch->bytes[index] = (ch->bytes[index] << 1) | (bit ? 1 : 0);
    ch->bit_count++;
    ch->byte_count = (ch->bit_count + 7) / 8;
}

/*
 * Send the next run of codes: up to the threshold, or to the end marker.
 *
 * The threshold is what the driver refills against — it is set to half the
 * channel's RAM, so one chunk here is one half there — and a channel with
 * no threshold set sends its whole RAM at once, which is what a
 * transmission that fits does anyway.
 */
static void esp32_rmt_send(Esp32GpioState *s, unsigned channel)
{
    Esp32RmtChannel *ch = &s->rmt_ch[channel];
    unsigned limit = ch->tx_lim & ESP32_RMT_TX_LIM_MASK;

    if (!ch->sending) {
        return;
    }
    if (limit == 0 || limit > s->rmt_codes) {
        limit = s->rmt_codes;
    }
    for (unsigned i = 0; i < limit; i++) {
        uint32_t code = s->rmt_ram[channel][ch->read_at];

        ch->read_at = (ch->read_at + 1) % s->rmt_codes;
        if (esp32_rmt_is_end(code)) {
            ch->sending = false;
            esp32_rmt_say(s, channel);
            s->rmt_int_raw |= 1u << esp32_rmt_end_bit(s, channel);
            return;
        }
        esp32_rmt_push(ch, esp32_rmt_bit(code));
    }
    /* Out of codes for now: ask for the next half. */
    s->rmt_int_raw |= 1u << esp32_rmt_thr_bit(s, channel);
    ch->hungry = false;
}

/* The driver has refilled and is looking again, so take the next chunk of
 * every channel that asked for one. Called from the read of the interrupt
 * registers, which is the only thing a blocking driver does between
 * refilling and waiting. */
static void esp32_rmt_feed(Esp32GpioState *s)
{
    for (unsigned channel = 0; channel < s->rmt_tx_channels; channel++) {
        if (s->rmt_ch[channel].hungry) {
            s->rmt_ch[channel].hungry = false;
            esp32_rmt_send(s, channel);
        }
    }
}

/* Which channel's control register an address is, or -1 for none. The
 * stride is the part's, because the ESP32 has a second configuration
 * register between every two of these and the C3 does not. */
static int esp32_rmt_ctrl_channel(Esp32GpioState *s, hwaddr addr)
{
    hwaddr into;

    if (addr < s->rmt_ctrl_reg) {
        return -1;
    }
    into = addr - s->rmt_ctrl_reg;
    if (into % s->rmt_ctrl_stride != 0
        || into / s->rmt_ctrl_stride >= s->rmt_tx_channels) {
        return -1;
    }
    return into / s->rmt_ctrl_stride;
}

static uint64_t esp32_rmt_read(void *opaque, hwaddr addr, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    int channel;

    if (addr >= s->rmt_ram_at
        && addr < s->rmt_ram_at + 4 * s->rmt_codes * s->rmt_channels) {
        unsigned word = (addr - s->rmt_ram_at) / 4;
        return s->rmt_ram[word / s->rmt_codes][word % s->rmt_codes];
    }
    channel = esp32_rmt_ctrl_channel(s, addr);
    if (channel >= 0) {
        /* `TX_START` and the two resets are write-triggered and read back
         * clear, as every other trigger in this file does. */
        return s->rmt_ch[channel].ctrl
               & ~(ESP32_RMT_TX_START | s->rmt_mem_rd_rst
                   | s->rmt_apb_mem_rst | s->rmt_tx_stop);
    }
    if (addr >= s->rmt_tx_lim_reg
        && addr < s->rmt_tx_lim_reg + 4 * s->rmt_tx_channels) {
        return s->rmt_ch[(addr - s->rmt_tx_lim_reg) / 4].tx_lim;
    }
    if (addr == s->rmt_int_raw_reg) {
        esp32_rmt_feed(s);
        return s->rmt_int_raw;
    }
    if (addr == s->rmt_int_raw_reg + ESP32_RMT_INT_ST_AT) {
        esp32_rmt_feed(s);
        return s->rmt_int_raw & s->rmt_int_ena;
    }
    if (addr == s->rmt_int_raw_reg + ESP32_RMT_INT_ENA_AT) {
        return s->rmt_int_ena;
    }
    /* Everything else as the firmware left it — see `rmt_reg`. */
    return addr / 4 < ESP32_RMT_WORDS ? s->rmt_reg[addr / 4] : 0;
}

static void esp32_rmt_write(void *opaque, hwaddr addr, uint64_t value,
                            unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    uint32_t word32 = (uint32_t)value;
    int channel;

    if (addr >= s->rmt_ram_at
        && addr < s->rmt_ram_at + 4 * s->rmt_codes * s->rmt_channels) {
        unsigned word = (addr - s->rmt_ram_at) / 4;
        s->rmt_ram[word / s->rmt_codes][word % s->rmt_codes] = word32;
        return;
    }
    channel = esp32_rmt_ctrl_channel(s, addr);
    if (channel >= 0) {
        Esp32RmtChannel *ch = &s->rmt_ch[channel];

        ch->ctrl = word32;
        if (word32 & (s->rmt_mem_rd_rst | s->rmt_apb_mem_rst)) {
            ch->read_at = 0;
        }
        if (s->rmt_tx_stop && (word32 & s->rmt_tx_stop)) {
            ch->sending = false;
        }
        if (word32 & ESP32_RMT_TX_START) {
            ch->sending = true;
            ch->read_at = 0;
            ch->bit_count = 0;
            ch->byte_count = 0;
            ch->dropped = 0;
            ch->hungry = false;
            esp32_rmt_send(s, channel);
        }
        return;
    }
    if (addr >= s->rmt_tx_lim_reg
        && addr < s->rmt_tx_lim_reg + 4 * s->rmt_tx_channels) {
        s->rmt_ch[(addr - s->rmt_tx_lim_reg) / 4].tx_lim = word32;
        return;
    }
    if (addr == s->rmt_int_raw_reg) {
        /* `INT_RAW` is the model's to say; a driver writing it would be
         * telling itself a story. */
        return;
    }
    if (addr == s->rmt_int_raw_reg + ESP32_RMT_INT_ENA_AT) {
        s->rmt_int_ena = word32;
    } else if (addr == s->rmt_int_raw_reg + ESP32_RMT_INT_CLR_AT) {
        s->rmt_int_raw &= ~word32;
        /* A cleared threshold is the driver saying it is about to refill;
         * the codes it writes are taken when it next looks at the
         * interrupts, which is after the write. */
        for (unsigned ch = 0; ch < s->rmt_tx_channels; ch++) {
            if (word32 & (1u << esp32_rmt_thr_bit(s, ch))) {
                s->rmt_ch[ch].hungry = true;
            }
        }
    } else if (addr / 4 < ESP32_RMT_WORDS) {
        /* Kept as written, so a driver's read-modify-write of a register
         * this model has no opinion about — the memory size, the divider,
         * the carrier — comes back with what it put there. */
        s->rmt_reg[addr / 4] = word32;
    }
}

static const MemoryRegionOps rmt_ops = {
    .read = esp32_rmt_read,
    .write = esp32_rmt_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

static void esp32_gpio_reset_hold(Object *obj, ResetType type)
{
    Esp32GpioState *s = ESP32_GPIO(obj);

    s->out = 0;
    s->enable = 0;
    s->in = 0;
    s->host_at = 0;
    s->status = 0;
    memset(s->pin_cfg, 0, sizeof(s->pin_cfg));
    /* The analog side goes back with it, including what the host had put on
     * the pins. Arguably the world outside the chip does not reset when the
     * chip does — but the host is the one authority on it, and a host that
     * re-sends what it is driving whenever a run begins is a simpler
     * contract than a device holding state across a reset the host never
     * heard about. rusty sends its levels and analog values on connecting
     * for exactly this reason. */
    memset(s->analog, 0, sizeof(s->analog));
    memset(s->adc_reg, 0, sizeof(s->adc_reg));
    s->adc_data[0] = 0;
    s->adc_data[1] = 0;
    s->adc_pin[0] = -1;
    s->adc_pin[1] = -1;
    /* The bus goes with it, devices and all, for the reason above: the host
     * is the one authority on what is on this board, and it says so when a
     * run begins rather than the device remembering across a reset the host
     * never heard about. */
    memset(s->i2c_reg, 0, sizeof(s->i2c_reg));
    memset(s->i2c_devices, 0, sizeof(s->i2c_devices));
    s->i2c_tx_len = 0;
    s->i2c_tx_at = 0;
    s->i2c_rx_len = 0;
    s->i2c_rx_at = 0;
    s->i2c_address = -1;
    s->i2c_expect_address = false;
    s->i2c_continues = false;
    memset(s->io_mux, 0, sizeof(s->io_mux));
    memset(s->switches, 0, sizeof(s->switches));
    s->host_driven = 0;
    s->resolved_in = 0;
    memset(s->i2c_last_report, 0, sizeof(s->i2c_last_report));
    memset(s->spi_reg, 0, sizeof(s->spi_reg));
    memset(s->spi_miso, 0, sizeof(s->spi_miso));
    memset(s->spi_miso_len, 0, sizeof(s->spi_miso_len));
    memset(s->spi_last_report, 0, sizeof(s->spi_last_report));
    memset(s->ledc_ch, 0, sizeof(s->ledc_ch));
    memset(s->ledc_timer, 0, sizeof(s->ledc_timer));
    memset(s->ledc_said, 0, sizeof(s->ledc_said));
    s->ledc_conf = 0;
    s->ledc_int_raw = 0;
    s->ledc_int_ena = 0;
    for (unsigned channel = 0; channel < ESP32_LEDC_CHANNELS; channel++) {
        s->ledc_said_pin[channel] = -1;
    }
    memset(s->func_out, 0, sizeof(s->func_out));
    memset(s->rmt_ch, 0, sizeof(s->rmt_ch));
    memset(s->rmt_ram, 0, sizeof(s->rmt_ram));
    memset(s->rmt_reg, 0, sizeof(s->rmt_reg));
    s->rmt_int_raw = 0;
    s->rmt_int_ena = 0;
    s->rmt_sys_conf = 0;
    if (s->irq_level) {
        s->irq_level = false;
        qemu_set_irq(s->irq, false);
    }
}

static void esp32_gpio_realize(DeviceState *dev, Error **errp)
{
    Esp32GpioState *s = ESP32_GPIO(dev);

    /* Which part this is, asked of the object rather than passed in: the
     * C3 and the S3 subclass this device, and their interrupt registers sit
     * at different offsets from the original ESP32's. Getting it wrong is
     * silent — the firmware would write its configuration into nothing and
     * wait for an interrupt that could never be raised. */
    bool base = strcmp(object_get_typename(OBJECT(s)), TYPE_ESP32_GPIO) == 0;

    s->pin0_reg = base ? ESP32_GPIO_PIN0_ESP32 : ESP32_GPIO_PIN0_MODERN;
    s->pcpu_int_reg = base ? ESP32_GPIO_PCPU_INT_ESP32
                           : ESP32_GPIO_PCPU_INT_MODERN;
    s->func_out_reg = base ? ESP32_GPIO_FUNC_OUT_ESP32
                           : ESP32_GPIO_FUNC_OUT_MODERN;
    s->saradc_esp32 = base;
    s->i2c_commands = base ? ESP32_I2C_COMMANDS_ESP32
                           : ESP32_I2C_COMMANDS_MODERN;
    s->i2c_op_rstart = base ? ESP32_I2C_OP_RSTART_ESP32
                            : ESP32_I2C_OP_RSTART_MODERN;
    s->i2c_op_write = base ? ESP32_I2C_OP_WRITE_ESP32
                           : ESP32_I2C_OP_WRITE_MODERN;
    s->i2c_op_read = base ? ESP32_I2C_OP_READ_ESP32
                          : ESP32_I2C_OP_READ_MODERN;
    s->i2c_op_stop = base ? ESP32_I2C_OP_STOP_ESP32
                          : ESP32_I2C_OP_STOP_MODERN;
    s->i2c_op_end = base ? ESP32_I2C_OP_END_ESP32
                         : ESP32_I2C_OP_END_MODERN;

    /* LEDC. The ESP32's two halves are numbered as one run — eight
     * high-speed channels then eight low-speed, four of each timer — which
     * works because both the registers and the matrix signals are laid out
     * that way; `ledc_hs_*` is where the first half ends and the latching
     * begins. */
    s->ledc_timer0_reg = base ? ESP32_LEDC_TIMER0_ESP32
                              : ESP32_LEDC_TIMER0_MODERN;
    s->ledc_int_raw_reg = base ? ESP32_LEDC_INT_RAW_ESP32
                               : ESP32_LEDC_INT_RAW_MODERN;
    s->ledc_channels = base ? 16 : 6;
    s->ledc_timers = base ? 8 : 4;
    s->ledc_hs_channels = base ? 8 : 0;
    s->ledc_hs_timers = base ? 4 : 0;
    s->ledc_sig0 = base ? ESP32_LEDC_SIG0_ESP32 : ESP32_LEDC_SIG0_MODERN;
    s->ledc_fade_shift = base ? ESP32_LEDC_INT_FADE_SHIFT_ESP32
                              : ESP32_LEDC_INT_FADE_SHIFT_MODERN;
    s->ledc_res_mask = base ? ESP32_LEDC_TIMER_RES_MASK_ESP32
                            : ESP32_LEDC_TIMER_RES_MASK_MODERN;
    s->ledc_div_shift = base ? ESP32_LEDC_TIMER_DIV_SHIFT_ESP32
                             : ESP32_LEDC_TIMER_DIV_SHIFT_MODERN;
    s->ledc_timer_pause = base ? ESP32_LEDC_TIMER_PAUSE_ESP32
                               : ESP32_LEDC_TIMER_PAUSE_MODERN;
    s->ledc_timer_rst = base ? ESP32_LEDC_TIMER_RST_ESP32
                             : ESP32_LEDC_TIMER_RST_MODERN;
    s->ledc_timer_para_up = base ? ESP32_LEDC_TIMER_PARA_UP_ESP32
                                 : ESP32_LEDC_TIMER_PARA_UP_MODERN;
    s->ledc_clock_per_timer = base;

    /* RMT. The part with eight transmitting channels puts its control bits
     * in a second register per channel and its RAM twice as far in. */
    s->rmt_ctrl_reg = base ? ESP32_RMT_CTRL_ESP32 : ESP32_RMT_CTRL_MODERN;
    s->rmt_ctrl_stride = base ? ESP32_RMT_CTRL_STRIDE_ESP32
                              : ESP32_RMT_CTRL_STRIDE_MODERN;
    s->rmt_tx_lim_reg = base ? ESP32_RMT_TX_LIM_ESP32
                             : ESP32_RMT_TX_LIM_MODERN;
    s->rmt_int_raw_reg = base ? ESP32_RMT_INT_RAW_ESP32
                              : ESP32_RMT_INT_RAW_MODERN;
    s->rmt_ram_at = base ? ESP32_RMT_RAM_ESP32 : ESP32_RMT_RAM_MODERN;
    s->rmt_tx_channels = base ? ESP32_RMT_TX_CHANNELS_ESP32
                              : ESP32_RMT_TX_CHANNELS_MODERN;
    s->rmt_channels = base ? ESP32_RMT_CHANNELS_ESP32
                           : ESP32_RMT_CHANNELS_MODERN;
    s->rmt_codes = base ? ESP32_RMT_CODES_ESP32 : ESP32_RMT_CODES_MODERN;
    s->rmt_sig0 = base ? ESP32_RMT_SIG0_ESP32 : ESP32_RMT_SIG0_MODERN;
    s->rmt_end_stride = base ? ESP32_RMT_END_STRIDE_ESP32
                             : ESP32_RMT_END_STRIDE_MODERN;
    s->rmt_thr_shift = base ? ESP32_RMT_INT_THR_ESP32
                            : ESP32_RMT_INT_THR_MODERN;
    s->rmt_mem_rd_rst = base ? ESP32_RMT_MEM_RD_RST_ESP32
                             : ESP32_RMT_MEM_RD_RST_MODERN;
    s->rmt_apb_mem_rst = base ? ESP32_RMT_APB_MEM_RST_ESP32
                              : ESP32_RMT_APB_MEM_RST_MODERN;
    s->rmt_tx_stop = base ? ESP32_RMT_TX_STOP_ESP32
                          : ESP32_RMT_TX_STOP_MODERN;

    /* SPI2, of which the two parts share only the address of `CMD`. */
    s->spi_user_reg = base ? ESP32_SPI_USER_ESP32 : ESP32_SPI_USER_MODERN;
    s->spi_dlen_reg = base ? ESP32_SPI_DLEN_ESP32 : ESP32_SPI_DLEN_MODERN;
    s->spi_cs_reg = base ? ESP32_SPI_CS_ESP32 : ESP32_SPI_CS_MODERN;
    s->spi_w0_reg = base ? ESP32_SPI_W0_ESP32 : ESP32_SPI_W0_MODERN;
    s->spi_done_reg = base ? ESP32_SPI_DONE_ESP32 : ESP32_SPI_DONE_MODERN;
    s->spi_done_clr_reg = base ? ESP32_SPI_DONE_ESP32
                               : ESP32_SPI_DONE_CLR_MODERN;
    s->spi_done_bit = base ? ESP32_SPI_INT_TRANS_DONE_ESP32
                           : ESP32_SPI_INT_TRANS_DONE_MODERN;
    s->spi_cmd_usr = base ? ESP32_SPI_CMD_USR_ESP32
                          : ESP32_SPI_CMD_USR_MODERN;
    s->spi_cmd_update = base ? ESP32_SPI_CMD_UPDATE_ESP32
                             : ESP32_SPI_CMD_UPDATE_MODERN;
    s->spi_selects = base ? ESP32_SPI_SELECTS_ESP32
                          : ESP32_SPI_SELECTS_MODERN;
    s->spi_done_w1c = !base;

    esp32_iomux_map(s, base);

    /* With no chardev attached this does nothing and the device behaves as
     * it did before — the model is still correct, it simply has nobody to
     * tell. */
    qemu_chr_fe_set_handlers(&s->pins, esp32_gpio_host_can_read,
                             esp32_gpio_host_read, NULL, NULL, s, NULL, true);
}

static void esp32_gpio_init(Object *obj)
{
    Esp32GpioState *s = ESP32_GPIO(obj);
    SysBusDevice *sbd = SYS_BUS_DEVICE(obj);

    /* Set the default value for the strap_mode property */
    object_property_set_int(obj, "strap_mode", ESP32_STRAP_MODE_FLASH_BOOT, &error_fatal);

    /* Nobody has said which source this is. A plain field rather than a
     * qdev property because the machine sets it beside the mapping, after
     * realize, and a property set then asserts; the default is the refusal,
     * so a machine that says nothing gets the behaviour it always had. */
    s->intr_source = -1;

    memory_region_init_io(&s->iomem, obj, &uart_ops, s,
                          TYPE_ESP32_GPIO, 0x1000);
    sysbus_init_mmio(sbd, &s->iomem);
    /* Region 1 is the SAR ADC, mapped only by the machines that have one at
     * these registers. A machine that never maps it pays for a memory
     * region nobody reaches, which is what makes this safe to define for
     * every part in the family. */
    memory_region_init_io(&s->adc_iomem, obj, &saradc_ops, s,
                          TYPE_ESP32_GPIO ".saradc", ESP32_SARADC_REGION);
    sysbus_init_mmio(sbd, &s->adc_iomem);
    /* Region 2 is the I2C master, mapped by the same machines and for the
     * same reason: it speaks to the host over this device's channel. */
    memory_region_init_io(&s->i2c_iomem, obj, &i2c_ops, s,
                          TYPE_ESP32_GPIO ".i2c", ESP32_I2C_REGION);
    sysbus_init_mmio(sbd, &s->i2c_iomem);
    /* And region 3 is SPI2 — the one a project puts a display on. SPI1 is
     * the flash controller and stays upstream's. */
    memory_region_init_io(&s->spi_iomem, obj, &spi_ops, s,
                          TYPE_ESP32_GPIO ".spi", ESP32_SPI_REGION);
    sysbus_init_mmio(sbd, &s->spi_iomem);
    /* Region 4 is LEDC, which drives a pad through the matrix rather than
     * through GPIO_OUT — the peripheral behind every servo, every motor and
     * every dimmed lamp. */
    memory_region_init_io(&s->ledc_iomem, obj, &ledc_ops, s,
                          TYPE_ESP32_GPIO ".ledc", ESP32_LEDC_REGION);
    sysbus_init_mmio(sbd, &s->ledc_iomem);
    /* Region 5 is RMT's transmitting half — an addressable LED strip, and
     * the codes that carry its colours. */
    memory_region_init_io(&s->rmt_iomem, obj, &rmt_ops, s,
                          TYPE_ESP32_GPIO ".rmt", ESP32_RMT_REGION);
    sysbus_init_mmio(sbd, &s->rmt_iomem);

    /* And IO_MUX, where a pad's pull-up and pull-down live. */
    memory_region_init_io(&s->iomux_iomem, obj, &iomux_ops, s,
                          TYPE_ESP32_GPIO ".iomux", ESP32_IOMUX_REGION);
    sysbus_init_mmio(sbd, &s->iomux_iomem);
    /* Region 7 is the source-status words the CPU's dispatcher reads —
     * three registers of somebody else's peripheral, answered for this
     * device's own source alone. */
    memory_region_init_io(&s->intr_status_iomem, obj, &intr_status_ops, s,
                          TYPE_ESP32_GPIO ".intr-status",
                          ESP32_INTR_STATUS_REGION);
    sysbus_init_mmio(sbd, &s->intr_status_iomem);
    sysbus_init_irq(sbd, &s->irq);
}

static Property esp32_gpio_properties[] = {
    /* The strap_mode needs to be explicitly set in the instance init, thus, set
     * the default value to 0. */
    DEFINE_PROP_UINT32("strap_mode", Esp32GpioState, strap_mode, 0),
    /* Where pin changes go and host levels come from. Optional: absent, the
     * emulator runs exactly as it always did. */
    DEFINE_PROP_CHR("pins", Esp32GpioState, pins),
    DEFINE_PROP_END_OF_LIST(),
};

static void esp32_gpio_class_init(ObjectClass *klass, void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    ResettableClass *rc = RESETTABLE_CLASS(klass);

    rc->phases.hold = esp32_gpio_reset_hold;
    dc->realize = esp32_gpio_realize;
    device_class_set_props(dc, esp32_gpio_properties);
}

static const TypeInfo esp32_gpio_info = {
    .name = TYPE_ESP32_GPIO,
    .parent = TYPE_SYS_BUS_DEVICE,
    .instance_size = sizeof(Esp32GpioState),
    .instance_init = esp32_gpio_init,
    .class_init = esp32_gpio_class_init,
    .class_size = sizeof(Esp32GpioClass),
};

static void esp32_gpio_register_types(void)
{
    type_register_static(&esp32_gpio_info);
}

type_init(esp32_gpio_register_types)
