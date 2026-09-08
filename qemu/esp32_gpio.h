/*
 * ESP32 GPIO emulation
 *
 * Copyright (c) 2019 Espressif Systems (Shanghai) Co. Ltd.
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License version 2 or
 * (at your option) any later version.
 */

#pragma once

#include "hw/sysbus.h"
#include "hw/hw.h"
#include "hw/registerfields.h"
#include "chardev/char-fe.h"

#define TYPE_ESP32_GPIO "esp32.gpio"
#define ESP32_GPIO(obj)             OBJECT_CHECK(Esp32GpioState, (obj), TYPE_ESP32_GPIO)
#define ESP32_GPIO_GET_CLASS(obj)   OBJECT_GET_CLASS(Esp32GpioClass, obj, TYPE_ESP32_GPIO)
#define ESP32_GPIO_CLASS(klass)     OBJECT_CLASS_CHECK(Esp32GpioClass, klass, TYPE_ESP32_GPIO)

REG32(GPIO_STRAP, 0x0038)

/* The registers that carry pin state, which the stock model does not keep.
 * Offsets are from esp-idf's soc/gpio_reg.h; GPIO_STRAP at 0x38 agreeing with
 * the definition that was already here is the check that this is the right
 * map. The low bank is at the same offsets on every part in this family. */
REG32(GPIO_OUT, 0x0004)
REG32(GPIO_OUT_W1TS, 0x0008)
REG32(GPIO_OUT_W1TC, 0x000c)
REG32(GPIO_ENABLE, 0x0020)
REG32(GPIO_ENABLE_W1TS, 0x0024)
REG32(GPIO_ENABLE_W1TC, 0x0028)
REG32(GPIO_IN, 0x003c)

/* The second bank: GPIO32..39, which only the original ESP32 has.
 *
 * Modelling it is not optional for that part. GPIO34/35/36/39 are its
 * input-only pins and are where most boards put their analog inputs, so a
 * model that stops at 31 would leave an ESP32 user with pins that work and
 * pins that silently do not — worse than one that never claimed to.
 *
 * Harmless on the parts without it: the C3 reserves these offsets and its
 * firmware never touches them. */
REG32(GPIO_OUT1, 0x0010)
REG32(GPIO_OUT1_W1TS, 0x0014)
REG32(GPIO_OUT1_W1TC, 0x0018)
REG32(GPIO_ENABLE1, 0x002c)
REG32(GPIO_ENABLE1_W1TS, 0x0030)
REG32(GPIO_ENABLE1_W1TC, 0x0034)
REG32(GPIO_IN1, 0x0040)

/* The interrupt registers.
 *
 * `STATUS` is the latched pending set — one bit per pin, written back with
 * a 1 to clear — and it is the same offset on every part in this family.
 * Where a pin's *configuration* lives is not: the original ESP32 puts
 * `GPIO_PIN0` at 0x88 and the CPU's own pending view at 0x68, while the C3
 * and the S3 put them at 0x74 and 0x5c. Those two are held in the device
 * rather than fixed here, because one model answers for both parts. */
REG32(GPIO_STATUS, 0x0044)
REG32(GPIO_STATUS_W1TS, 0x0048)
REG32(GPIO_STATUS_W1TC, 0x004c)
REG32(GPIO_STATUS1, 0x0050)
REG32(GPIO_STATUS1_W1TS, 0x0054)
REG32(GPIO_STATUS1_W1TC, 0x0058)

/* Where the per-pin configuration and the CPU's pending view sit, by part. */
#define ESP32_GPIO_PIN0_ESP32       0x0088
#define ESP32_GPIO_PIN0_MODERN      0x0074
#define ESP32_GPIO_PCPU_INT_ESP32   0x0068
#define ESP32_GPIO_PCPU_INT_MODERN  0x005c

/* From a part's first CPU-pending register to its second bank's: 0x68 to
   0x7c on the ESP32, 0x5c to 0x70 on the parts that have a second bank. */
#define ESP32_GPIO_PCPU_INT1_STRIDE 0x14

/* A `GPIO_PINn` register: how the pin triggers, and which CPU lines it
 * feeds. Both fields sit in the same bits on every part here. */
#define ESP32_GPIO_PIN_INT_TYPE_SHIFT 7
#define ESP32_GPIO_PIN_INT_TYPE_MASK  0x7
#define ESP32_GPIO_PIN_INT_ENA_SHIFT  13
#define ESP32_GPIO_PIN_INT_ENA_MASK   0x1f

/* `INT_TYPE`: what the pin fires on. The two level types are not latched —
 * silicon holds them while the level holds — and the edge types are, until
 * the firmware writes the bit back. */
#define ESP32_GPIO_INT_OFF     0
#define ESP32_GPIO_INT_RISING  1
#define ESP32_GPIO_INT_FALLING 2
#define ESP32_GPIO_INT_ANYEDGE 3
#define ESP32_GPIO_INT_LOW     4
#define ESP32_GPIO_INT_HIGH    5

/* `INT_ENA`: bit 0 is the CPU's ordinary interrupt and bit 2 the second
 * core's on the parts that have one; bits 1 and 3 are the NMI lines, which
 * go to a source this device is not wired to. Raising the ordinary line for
 * an NMI-only pin would be inventing an interrupt nobody asked for. */
#define ESP32_GPIO_INT_ENA_CPU 0x5

#define ESP32_STRAP_MODE_FLASH_BOOT 0x12
#define ESP32_STRAP_MODE_UART_BOOT  0x0f

/* Longest host line worth accepting: "39=1\n" and slack. Anything longer is
 * garbage and gets dropped rather than truncated into a different command. */
#define ESP32_GPIO_HOST_LINE 32

/* Highest pin number any part in this family has, plus one. The parts with
 * fewer simply never touch the ones above their count. */
#define ESP32_GPIO_PINS 40

/*
 * The SAR ADC, which this device also answers for.
 *
 * A second peripheral in the GPIO model's file, deliberately. The analog
 * value on a pin and its digital level are two readings of the same wire,
 * they arrive on the same channel from the same host, and a separate device
 * would need a link back to this one for every conversion. Keeping them
 * together is what makes "one channel, one protocol" true on the emulator's
 * side as well as rusty's. It is a second MMIO region rather than a second
 * device for the same reason it is not a new file: a new file means a new
 * entry in upstream's build system, and every one of those is a way for a
 * build to fail that has nothing to do with what is being modelled.
 *
 * `esp32.gpio` is instantiated on every part in the family and only the
 * machines that map region 1 get an ADC — which today is the C3, the part
 * whose registers these are. Offsets from esp-idf's
 * soc/esp32c3/apb_saradc_reg.h.
 */
#define ESP32_SARADC_REGION 0x1000

/* Shadowed as a whole, so a read-modify-write of a register this model has
 * no opinion about keeps what the firmware put there. Four kilobytes to
 * remove a class of bug where a driver's `modify()` silently drops bits. */
#define ESP32_SARADC_WORDS (ESP32_SARADC_REGION / 4)

REG32(SARADC_ONETIME, 0x0020)
REG32(SARADC_1_DATA, 0x002c)
REG32(SARADC_2_DATA, 0x0030)
REG32(SARADC_INT_ENA, 0x0040)
REG32(SARADC_INT_RAW, 0x0044)
REG32(SARADC_INT_ST, 0x0048)
REG32(SARADC_INT_CLR, 0x004c)

/* `ONETIME_SAMPLE`: which unit is being asked, the channel, and the edge
 * that starts a conversion. `ATTEN` is stored and ignored — attenuation
 * scales a real voltage onto the converter's range, and this model is
 * handed counts rather than volts precisely so it never has to guess at
 * anybody's divider. */
#define ESP32_SARADC_ONETIME_ADC1  (1u << 31)
#define ESP32_SARADC_ONETIME_ADC2  (1u << 30)
#define ESP32_SARADC_ONETIME_START (1u << 29)
#define ESP32_SARADC_ONETIME_CHANNEL_SHIFT 25
#define ESP32_SARADC_ONETIME_CHANNEL_MASK  0xf

/* The done bits, one per unit, in `INT_RAW`/`INT_ST`/`INT_CLR`. */
#define ESP32_SARADC_DONE_ADC1 (1u << 31)
#define ESP32_SARADC_DONE_ADC2 (1u << 30)

/* The converter is twelve bits. A host that sends more is clamped rather
 * than wrapped: a slider dragged past full scale must read as full scale,
 * not as zero. */
#define ESP32_SARADC_FULL_SCALE 0xfff

typedef struct Esp32GpioState {
    SysBusDevice parent_obj;

    MemoryRegion iomem;
    qemu_irq irq;
    uint32_t strap_mode;

    /* What the guest has driven, and what is being driven at it. `enable`
     * decides which a reader should believe for a given pin: an output pin
     * reports what the guest set, an input pin what the host drove.
     *
     * One 64-bit word per register rather than a pair, because every pin
     * question — level, direction, which changed — is then one expression
     * instead of two that can disagree about pin 32. The banks are only a
     * register layout; a pin number is a pin number. */
    uint64_t out;
    uint64_t enable;
    uint64_t in;

    /* The interrupt half: what has fired and is waiting to be read, how
     * each pin is configured to fire, and where this part keeps those
     * registers. `irq_level` is what was last put on the line, so the
     * device only ever tells the interrupt matrix about a change. */
    uint64_t status;
    uint32_t pin_cfg[ESP32_GPIO_PINS];
    hwaddr pin0_reg;
    hwaddr pcpu_int_reg;
    bool irq_level;

    /* Pin changes out, host-driven levels in. Its own chardev on purpose —
     * the UART belongs to the firmware, and interleaving the two would make
     * each unreadable to whoever wanted the other. */
    CharBackend pins;
    char host_line[ESP32_GPIO_HOST_LINE];
    unsigned host_at;

    /* The analog half, on the same pins and the same channel.
     *
     * `analog` is what the host says is on each pin, in the converter's own
     * counts. Counts and not volts, the same refusal rusty makes on its own
     * side: the emulator does not know anybody's divider or reference, and a
     * voltage it converted itself would be a confident number the firmware's
     * arithmetic disagreed with. */
    MemoryRegion adc_iomem;
    uint16_t analog[ESP32_GPIO_PINS];
    uint32_t adc_reg[ESP32_SARADC_WORDS];
    /* The last conversion each unit finished, and which pin it read — kept
     * apart from the shadow so a read of the data register cannot be
     * satisfied by whatever a driver happened to write there. */
    uint16_t adc_data[2];
    int adc_pin[2];
} Esp32GpioState;

typedef struct Esp32GpioClass {
    SysBusDeviceClass parent_class;
} Esp32GpioClass;
