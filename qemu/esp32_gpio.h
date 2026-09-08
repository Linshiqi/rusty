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

/* Longest host line worth accepting. A pin or an analog value needs a
 * handful of characters; an I2C device declaring a run of registers needs
 * two characters a byte, so the buffer is sized for a page of them. Anything
 * longer is dropped rather than truncated into a different command — a
 * truncated hex string is a valid, wrong one. */
#define ESP32_GPIO_HOST_LINE 512

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

REG32(RUSTY_SARADC_ONETIME, 0x0020)
REG32(RUSTY_SARADC_1_DATA, 0x002c)
REG32(RUSTY_SARADC_2_DATA, 0x0030)
REG32(RUSTY_SARADC_INT_ENA, 0x0040)
REG32(RUSTY_SARADC_INT_RAW, 0x0044)
REG32(RUSTY_SARADC_INT_ST, 0x0048)
REG32(RUSTY_SARADC_INT_CLR, 0x004c)

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

/*
 * The I2C master, which this device also answers for.
 *
 * The third peripheral in this file, for the reason the second is here:
 * everything in it exists to carry the host's view of the board, and they
 * share one socket. Four peripherals would otherwise mean four chardevs,
 * four protocols and four readers on rusty's side —
 * and the one rule that keeps the simulator honest is that the board's
 * traffic is parsed in exactly one place.
 *
 * A master and no bus. The devices on it are a register file each, declared
 * by the host over the same channel: a sensor is a set of registers a
 * driver reads, a display is a stream of bytes somebody wants to see, and
 * both are served by 256 bytes and a pointer. An address the host never
 * declared is *not* answered — the transaction NACKs, which is what a bus
 * scan needs and what tells a missing part from a silent one.
 *
 * Offsets from esp-idf's soc/esp32c3/i2c_reg.h.
 */
#define ESP32_I2C_REGION 0x1000
#define ESP32_I2C_WORDS (ESP32_I2C_REGION / 4)

REG32(RUSTY_I2C_CTR, 0x0004)
REG32(RUSTY_I2C_SR, 0x0008)
REG32(RUSTY_I2C_FIFO_CONF, 0x0018)
REG32(RUSTY_I2C_DATA, 0x001c)
REG32(RUSTY_I2C_INT_RAW, 0x0020)
REG32(RUSTY_I2C_INT_CLR, 0x0024)
REG32(RUSTY_I2C_INT_ENA, 0x0028)
REG32(RUSTY_I2C_INT_STATUS, 0x002c)
REG32(RUSTY_I2C_COMD0, 0x0058)

/* Eight command slots, each holding one step of a transaction. */
#define ESP32_I2C_COMMANDS 8

/* `CTR`: `TRANS_START` is write-triggered and runs the command list. */
#define ESP32_I2C_TRANS_START (1u << 5)

/* `FIFO_CONF`: the two resets, both of which the driver sets and clears
 * again — so the model acts on the bit going up. */
#define ESP32_I2C_RX_FIFO_RST (1u << 12)
#define ESP32_I2C_TX_FIFO_RST (1u << 13)

/* `SR`: what the driver reads to find out how the transaction went.
 * `RESP_REC` *set* is an ACK — a transaction that completed with it clear
 * is how esp-hal reports a device that did not answer its data. */
#define ESP32_I2C_SR_RESP_REC (1u << 0)
#define ESP32_I2C_SR_BUS_BUSY (1u << 4)
#define ESP32_I2C_SR_RXFIFO_CNT_SHIFT 8
#define ESP32_I2C_SR_TXFIFO_CNT_SHIFT 18

/* The interrupt bits, shared by `INT_RAW`, `INT_CLR`, `INT_ENA` and
 * `INT_STATUS`. */
#define ESP32_I2C_INT_END_DETECT (1u << 3)
#define ESP32_I2C_INT_TRANS_COMPLETE (1u << 7)
#define ESP32_I2C_INT_NACK (1u << 10)

/* A command word: an op code, how many bytes it moves, and a done bit the
 * driver waits on. */
#define ESP32_I2C_CMD_DONE (1u << 31)
#define ESP32_I2C_CMD_OP_SHIFT 11
#define ESP32_I2C_CMD_OP_MASK 0x7
#define ESP32_I2C_CMD_BYTES_MASK 0xff

#define ESP32_I2C_OP_RSTART 0
#define ESP32_I2C_OP_WRITE 1
#define ESP32_I2C_OP_READ 2
#define ESP32_I2C_OP_STOP 3
#define ESP32_I2C_OP_END 4

/* The hardware FIFOs are 32 bytes each on this part. Modelling the depth
 * rather than an unbounded queue is deliberate: a driver that queued more
 * than the silicon holds would work here and fail on the desk. */
#define ESP32_I2C_FIFO 32

/* How many devices the host may put on the bus. Sixteen is more than any
 * board here has and keeps the whole thing a few kilobytes. */
#define ESP32_I2C_DEVICES 16

/* Longest report a transaction produces: an address, a verb and two
 * characters for each of the FIFO's bytes. */
#define ESP32_I2C_REPORT (2 * ESP32_I2C_FIFO + 16)

/* The last report is remembered *per verb* — a write, a read, and
 * everything else. One slot for all of them looked like it would quieten a
 * polling driver and does not: `write_read` alternates a write and a read,
 * so each line differs from the one before it and nothing is ever
 * suppressed. Three slots, and a driver reading the same register in a loop
 * says so once. */
#define ESP32_BUS_VERBS 3

/*
 * The SPI master, the fourth and last of the peripherals here.
 *
 * Upstream models `SPI1` — the flash controller, which the machine needs to
 * boot — and nothing at `SPI2`, the one a project puts a display or a sensor
 * on. So a driver's first transfer sets `USR` and polls it for ever.
 *
 * Simpler than the bus above, because SPI is: bytes out and bytes in at the
 * same time, and no addressing at all. **What comes back is a buffer the
 * host declares per chip select**, read from its start on every transfer.
 * No register convention is assumed — SPI has none. A display, which is
 * written to and never read, needs nothing declared and its bytes are
 * reported; a sensor's driver sends a command byte and reads the answer out
 * of the same transfer, which is what a full-duplex buffer gives it.
 *
 * Offsets from esp-idf's soc/esp32c3/spi_reg.h.
 */
#define ESP32_SPI_REGION 0x1000
#define ESP32_SPI_WORDS (ESP32_SPI_REGION / 4)

REG32(RUSTY_SPI_CMD, 0x0000)
REG32(RUSTY_SPI_USER, 0x0010)
REG32(RUSTY_SPI_MS_DLEN, 0x001c)
REG32(RUSTY_SPI_MISC, 0x0020)
REG32(RUSTY_SPI_DMA_INT_CLR, 0x0038)
REG32(RUSTY_SPI_DMA_INT_RAW, 0x003c)
REG32(RUSTY_SPI_W0, 0x0098)

/* `CMD`: `USR` starts a transfer and the model clears it when the transfer
 * is over, which is exactly what the driver polls. `UPDATE` latches the
 * configuration and clears itself. */
#define ESP32_SPI_CMD_USR (1u << 24)
#define ESP32_SPI_CMD_UPDATE (1u << 23)

/* `USER`: which phases this transfer has. */
#define ESP32_SPI_USER_MISO (1u << 28)
#define ESP32_SPI_USER_MOSI (1u << 27)

/* `MS_DLEN` holds the length in bits, less one. */
#define ESP32_SPI_DLEN_MASK 0x3ffff

/* `DMA_INT_RAW`: the done flag an interrupt-driven driver waits on. */
#define ESP32_SPI_INT_TRANS_DONE (1u << 12)

/* `MISC` bits 0..5 *disable* each chip select, so the active one is the
 * lowest bit that is clear. */
#define ESP32_SPI_SELECTS 6

/* Sixteen 32-bit words, which is the whole of a CPU-driven transfer. */
#define ESP32_SPI_BUFFER 64

/* One device: an address and the 256 registers behind it.
 *
 * A pointer, because that is what an I2C sensor is: a write of one byte
 * moves it, and a read takes bytes from there onwards. A display ignores
 * the pointer and cares only that its writes were seen, which the same
 * model gives for free. */
typedef struct Esp32I2cDevice {
    bool present;
    uint8_t address;
    uint8_t pointer;
    uint8_t regs[256];
} Esp32I2cDevice;

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

    /* The I2C master, and the devices the host has put on its bus. */
    MemoryRegion i2c_iomem;
    uint32_t i2c_reg[ESP32_I2C_WORDS];
    Esp32I2cDevice i2c_devices[ESP32_I2C_DEVICES];
    uint8_t i2c_tx[ESP32_I2C_FIFO];
    uint8_t i2c_rx[ESP32_I2C_FIFO];
    /* Written by the guest and taken by the engine; produced by the engine
     * and read by the guest. Two indices each, because the count the status
     * register reports is what is *left*, not what was put in. */
    unsigned i2c_tx_len;
    unsigned i2c_tx_at;
    unsigned i2c_rx_len;
    unsigned i2c_rx_at;
    /* Held across an `END`, which pauses a transaction rather than ending
     * it: the bus is still held and the next start carries no address, so a
     * read longer than the FIFO continues to the same device. Losing this is
     * a long read that silently addresses nobody after its first thirty-two
     * bytes. The *direction* is not kept, because the command list says it —
     * a `READ` step reads and a `WRITE` step writes, and the address byte's
     * low bit is the driver telling the bus what its own next command
     * already says. */
    int i2c_address;
    bool i2c_expect_address;
    /* The last transaction reported, without its timestamp, so the same one
     * repeated is said once. */
    char i2c_last_report[ESP32_BUS_VERBS][ESP32_I2C_REPORT];

    /* The SPI master, and what each chip select answers with. */
    MemoryRegion spi_iomem;
    uint32_t spi_reg[ESP32_SPI_WORDS];
    uint8_t spi_miso[ESP32_SPI_SELECTS][ESP32_SPI_BUFFER];
    unsigned spi_miso_len[ESP32_SPI_SELECTS];
    char spi_last_report[ESP32_BUS_VERBS][ESP32_I2C_REPORT];
} Esp32GpioState;

typedef struct Esp32GpioClass {
    SysBusDeviceClass parent_class;
} Esp32GpioClass;
