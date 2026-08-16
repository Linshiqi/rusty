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
static inline int esp32_gpio_level(Esp32GpioState *s, uint32_t bit)
{
    uint32_t source = (s->enable & bit) ? s->out : s->in;
    return (source & bit) ? 1 : 0;
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
static void esp32_gpio_report(Esp32GpioState *s, uint32_t changed)
{
    char line[160];
    int at;
    bool first = true;

    if (changed == 0 || !qemu_chr_fe_backend_connected(&s->pins)) {
        return;
    }

    at = snprintf(line, sizeof(line), "[rusty:gpio@%" PRId64 "] ",
                  qemu_clock_get_us(QEMU_CLOCK_VIRTUAL));

    for (int pin = 0; pin < 32; pin++) {
        uint32_t bit = 1u << pin;

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
 * Host input: `<pin>=<level>` a line, driving the input register.
 *
 * This is the half that lets firmware read a button through the GPIO it
 * actually reads, instead of through a side channel it had to be written to
 * expect.
 */
static void esp32_gpio_host_read(void *opaque, const uint8_t *buf, int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);

    for (int i = 0; i < size; i++) {
        if (buf[i] == '\n' || buf[i] == '\r') {
            unsigned pin, level;

            s->host_line[s->host_at] = '\0';
            if (sscanf(s->host_line, "%u=%u", &pin, &level) == 2 && pin < 32) {
                uint32_t bit = 1u << pin;
                uint32_t before = s->in;

                s->in = level ? (s->in | bit) : (s->in & ~bit);
                /* Only a real change is reported, so a host holding a button
                 * down does not fill the channel with one repeated line. */
                esp32_gpio_report(s, before ^ s->in);
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
        r = s->out;
        break;

    case A_GPIO_ENABLE:
        r = s->enable;
        break;

    /* An output pin reads back its own driven level, which is what the
     * silicon does and what firmware toggling a pin by read-modify-write
     * depends on. */
    case A_GPIO_IN:
        r = (s->in & ~s->enable) | (s->out & s->enable);
        break;

    default:
        break;
    }
    return r;
}

static void esp32_gpio_write(void *opaque, hwaddr addr,
                       uint64_t value, unsigned int size)
{
    Esp32GpioState *s = ESP32_GPIO(opaque);
    uint32_t before_out = s->out;
    uint32_t before_enable = s->enable;
    uint32_t word = (uint32_t)value;

    switch (addr) {
    case A_GPIO_OUT:
        s->out = word;
        break;

    /* The set and clear aliases. esp-hal drives a pin through these rather
     * than through OUT, so a model handling only OUT would see nothing at
     * all from ordinary firmware. */
    case A_GPIO_OUT_W1TS:
        s->out |= word;
        break;

    case A_GPIO_OUT_W1TC:
        s->out &= ~word;
        break;

    case A_GPIO_ENABLE:
        s->enable = word;
        break;

    case A_GPIO_ENABLE_W1TS:
        s->enable |= word;
        break;

    case A_GPIO_ENABLE_W1TC:
        s->enable &= ~word;
        break;

    default:
        return;
    }

    /* A direction change alters what a pin reports even when its level did
     * not move, so both registers decide what counts as changed. */
    esp32_gpio_report(s, (before_out ^ s->out) | (before_enable ^ s->enable));
}

static const MemoryRegionOps uart_ops = {
    .read =  esp32_gpio_read,
    .write = esp32_gpio_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

static void esp32_gpio_reset_hold(Object *obj, ResetType type)
{
    Esp32GpioState *s = ESP32_GPIO(obj);

    s->out = 0;
    s->enable = 0;
    s->in = 0;
    s->host_at = 0;
}

static void esp32_gpio_realize(DeviceState *dev, Error **errp)
{
    Esp32GpioState *s = ESP32_GPIO(dev);

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
