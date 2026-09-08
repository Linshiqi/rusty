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
    uint64_t source = (s->enable & bit) ? s->out : s->in;
    return (source & bit) ? 1 : 0;
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

/* Defined below, beside the other thing that writes to the pin channel. */
static void esp32_gpio_say_irq(Esp32GpioState *s, bool raised);

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
        at += snprintf(line + at, sizeof(line) - at, "%s%d=%d",
                       first ? "" : ",", pin, esp32_gpio_level(s, bit));
        first = false;
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

/*
 * Host input, a line at a time: `<pin>=<level>` drives the input register,
 * `A<pin>=<counts>` puts an analog value on the pin for the converter.
 *
 * This is the half that lets firmware read a button through the GPIO it
 * actually reads, and a knob through the ADC it actually reads, instead of
 * through a side channel each had to be written to expect. The two forms
 * cannot be confused: a decimal pin number never begins with `A`.
 */
static void esp32_gpio_host_read(void *opaque, const uint8_t *buf, int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);

    for (int i = 0; i < size; i++) {
        if (buf[i] == '\n' || buf[i] == '\r') {
            unsigned pin, level;

            s->host_line[s->host_at] = '\0';
            if (sscanf(s->host_line, "A%u=%u", &pin, &level) == 2
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
                /* Only a real change is reported, so a host holding a button
                 * down does not fill the channel with one repeated line. */
                esp32_gpio_report(s, before ^ s->in);
                /* And it is an edge on that pin, so firmware waiting on an
                 * interrupt runs — a button that worked only when polled
                 * was the gap this closes. Masked by `enable`, because a
                 * pin the guest is driving does not hear the host. */
                esp32_gpio_int_update(s, (before ^ s->in) & ~s->enable);
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
     * depends on. */
    case A_GPIO_IN:
        r = (uint32_t)((s->in & ~s->enable) | (s->out & s->enable));
        break;

    case A_GPIO_IN1:
        r = (uint32_t)(((s->in & ~s->enable) | (s->out & s->enable)) >> 32);
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
        }
        return;
    }

    /* A direction change alters what a pin reports even when its level did
     * not move, so both registers decide what counts as changed. */
    esp32_gpio_report(s, (before_out ^ s->out) | (before_enable ^ s->enable));
    /* A pin the guest drives is a pin that can interrupt the guest — the
     * loopback silicon has, and what firmware testing its own handler
     * depends on. The clear path lands here too, with no edges at all. */
    esp32_gpio_int_update(s, (before_out ^ s->out) & s->enable);
}

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
static int esp32_saradc_pin_for(unsigned unit, unsigned channel)
{
    /* ESP32-C3, from esp-idf's soc/esp32c3/adc_channel.h: ADC1 channels
     * 0..4 are GPIO0..GPIO4, and ADC2 has one channel, GPIO5. */
    static const int adc1[] = { 0, 1, 2, 3, 4 };
    static const int adc2[] = { 5 };

    if (unit == 0) {
        return channel < ARRAY_SIZE(adc1) ? adc1[channel] : -1;
    }
    return channel < ARRAY_SIZE(adc2) ? adc2[channel] : -1;
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
static void esp32_saradc_convert(Esp32GpioState *s, uint32_t onetime)
{
    unsigned channel = (onetime >> ESP32_SARADC_ONETIME_CHANNEL_SHIFT)
                       & ESP32_SARADC_ONETIME_CHANNEL_MASK;
    unsigned unit;
    unsigned counts;
    bool moved;
    int pin;

    /* Neither unit selected is a start with nothing to start: the silicon
     * has no converter running, and neither has this. */
    if (onetime & ESP32_SARADC_ONETIME_ADC1) {
        unit = 0;
    } else if (onetime & ESP32_SARADC_ONETIME_ADC2) {
        unit = 1;
    } else {
        return;
    }

    pin = esp32_saradc_pin_for(unit, channel);
    counts = pin >= 0 ? s->analog[pin] : 0;
    moved = pin != s->adc_pin[unit] || counts != s->adc_data[unit];
    s->adc_pin[unit] = pin;
    s->adc_data[unit] = counts;
    s->adc_reg[R_SARADC_INT_RAW] |= unit == 0 ? ESP32_SARADC_DONE_ADC1
                                              : ESP32_SARADC_DONE_ADC2;
    if (moved) {
        esp32_gpio_say_adc(s, unit, channel, pin, counts);
    }
}

static uint64_t esp32_saradc_read(void *opaque, hwaddr addr, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    unsigned word = addr / 4;

    if (word >= ESP32_SARADC_WORDS) {
        return 0;
    }
    switch (addr) {
    case A_SARADC_1_DATA:
        return s->adc_data[0];

    case A_SARADC_2_DATA:
        return s->adc_data[1];

    /* The masked view beside the raw one. A polling driver reads the raw
     * bit; a driver using the interrupt reads this. */
    case A_SARADC_INT_ST:
        return s->adc_reg[R_SARADC_INT_RAW] & s->adc_reg[R_SARADC_INT_ENA];

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
    switch (addr) {
    /* A conversion's result and the fact that it happened are the model's
     * to say; a driver writing them would be telling itself a story. */
    case A_SARADC_1_DATA:
    case A_SARADC_2_DATA:
    case A_SARADC_INT_RAW:
    case A_SARADC_INT_ST:
        break;

    case A_SARADC_INT_CLR:
        s->adc_reg[R_SARADC_INT_RAW] &= ~(uint32_t)value;
        break;

    case A_SARADC_ONETIME:
        before = s->adc_reg[R_SARADC_ONETIME];
        s->adc_reg[R_SARADC_ONETIME] = (uint32_t)value;
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
