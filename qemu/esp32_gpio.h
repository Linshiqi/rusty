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
 * Offsets are from esp-idf's soc/gpio_reg.h for the ESP32-C3; GPIO_STRAP at
 * 0x38 agreeing with the definition that was already here is the check that
 * this is the right map. */
REG32(GPIO_OUT, 0x0004)
REG32(GPIO_OUT_W1TS, 0x0008)
REG32(GPIO_OUT_W1TC, 0x000c)
REG32(GPIO_ENABLE, 0x0020)
REG32(GPIO_ENABLE_W1TS, 0x0024)
REG32(GPIO_ENABLE_W1TC, 0x0028)
REG32(GPIO_IN, 0x003c)

#define ESP32_STRAP_MODE_FLASH_BOOT 0x12
#define ESP32_STRAP_MODE_UART_BOOT  0x0f

/* Longest host line worth accepting: "31=1\n" and slack. Anything longer is
 * garbage and gets dropped rather than truncated into a different command. */
#define ESP32_GPIO_HOST_LINE 32

typedef struct Esp32GpioState {
    SysBusDevice parent_obj;

    MemoryRegion iomem;
    qemu_irq irq;
    uint32_t strap_mode;

    /* What the guest has driven, and what is being driven at it. `enable`
     * decides which a reader should believe for a given pin: an output pin
     * reports what the guest set, an input pin what the host drove. */
    uint32_t out;
    uint32_t enable;
    uint32_t in;

    /* Pin changes out, host-driven levels in. Its own chardev on purpose —
     * the UART belongs to the firmware, and interleaving the two would make
     * each unreadable to whoever wanted the other. */
    CharBackend pins;
    char host_line[ESP32_GPIO_HOST_LINE];
    unsigned host_at;
} Esp32GpioState;

typedef struct Esp32GpioClass {
    SysBusDeviceClass parent_class;
} Esp32GpioClass;
