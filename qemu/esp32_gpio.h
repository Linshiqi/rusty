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

/*
 * `CTR`'s three write-triggered bits. Every one of them is `WT` in the
 * register map: the guest sets it, the hardware acts and clears it, and a
 * read never shows it set. A model that stored them instead would leave a
 * driver polling a bit that can never fall — which is not hypothetical, it
 * is how `Spi`'s `update()` hangs, and `ClearBusFuture` waits on the same
 * shape in `SCL_SP_CONF` below.
 */
#define ESP32_I2C_TRANS_START (1u << 5)
#define ESP32_I2C_FSM_RST (1u << 10)
#define ESP32_I2C_CONF_UPGATE (1u << 11)
#define ESP32_I2C_CTR_SELF_CLEARING \
    (ESP32_I2C_TRANS_START | ESP32_I2C_FSM_RST | ESP32_I2C_CONF_UPGATE)

/* `SCL_SP_CONF`, and the bit a driver clearing a stuck bus waits on. The
 * hardware pulses SCL nine times and clears it; here there is no bus to
 * unstick, so it clears at once. Left set, it is fifty milliseconds of
 * timeout on every recovery — and esp-hal recovers after every NACK, which
 * is once per address of a bus scan. */
REG32(RUSTY_I2C_SCL_SP_CONF, 0x0080)
#define ESP32_I2C_SCL_RST_SLV_EN (1u << 0)

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

/*
 * The op codes, from esp-idf's hal/esp32c3/i2c_ll.h. They are *not*
 * consecutive and they are not in the order a driver's `Command` enum lists
 * them — which is exactly the mistake that was here: 0, 1, 2, 3, 4, read off
 * esp-hal's Rust enum instead of the hardware.
 *
 * What that produced is worth remembering, because it looked like nothing at
 * all. A `Start` (6) fell through to the default case and set no address; the
 * `Write` (1) after it then had no device to talk to and returned before it
 * could even report a NACK. So every transaction completed having done
 * nothing, no byte was ever reported on the channel, and the bus read as
 * empty — indistinguishable, from outside, from a model that had never been
 * asked. It took a register dump from the firmware to see the four command
 * words and decode them.
 */
#define ESP32_I2C_OP_RSTART 6
#define ESP32_I2C_OP_WRITE 1
#define ESP32_I2C_OP_READ 3
#define ESP32_I2C_OP_STOP 2
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

/*
 * LEDC, which this device also answers for.
 *
 * The fifth peripheral in this file, and the one every servo, every motor
 * and every dimmed lamp on an ESP32 is driven through. Nothing is mapped at
 * it upstream, so a firmware that configures a timer and a channel writes
 * into a hole: the pin it chose stays wherever GPIO left it, the board
 * shows a servo asleep while the firmware sweeps it, and a `[rusty:pwm]`
 * line exists only when the firmware narrates its own duty.
 *
 * What is modelled is what a duty *is*: each timer's resolution and
 * divider, each channel's duty, which timer it follows, whether its output
 * is enabled, and the two `para_up` latches that make any of it take
 * effect. What is deliberately not modelled is the counter. A host can act
 * on a duty and a frequency — an angle, a speed, a brightness — and cannot
 * act on twenty thousand edges a second, which is the measurement the
 * `[rusty:pwm]` line exists to avoid putting on a channel the console
 * shares.
 *
 * Offsets from esp-idf's soc/esp32c3/ledc_reg.h, and the sequence a driver
 * writes them in from esp-hal's `ledc::low_level`.
 */
#define ESP32_LEDC_REGION 0x1000
#define ESP32_LEDC_WORDS (ESP32_LEDC_REGION / 4)

/* Six channels and four timers on the C3, each a small cluster. */
#define ESP32_LEDC_CHANNELS 6
#define ESP32_LEDC_TIMERS 4
#define ESP32_LEDC_CH0 0x0000
#define ESP32_LEDC_CH_STRIDE 0x14
#define ESP32_LEDC_TIMER0 0x00a0
#define ESP32_LEDC_TIMER_STRIDE 0x8

REG32(RUSTY_LEDC_INT_RAW, 0x00c0)
REG32(RUSTY_LEDC_INT_ST, 0x00c4)
REG32(RUSTY_LEDC_INT_ENA, 0x00c8)
REG32(RUSTY_LEDC_INT_CLR, 0x00cc)
REG32(RUSTY_LEDC_CONF, 0x00d0)

/* A channel's cluster: `CONF0`, `HPOINT`, `DUTY`, `CONF1`, `DUTY_R`. */
#define ESP32_LEDC_CH_CONF0  0x0
#define ESP32_LEDC_CH_HPOINT 0x4
#define ESP32_LEDC_CH_DUTY   0x8
#define ESP32_LEDC_CH_CONF1  0xc
#define ESP32_LEDC_CH_DUTY_R 0x10

/* `CONF0`: which timer, whether the output reaches the matrix, the level a
 * stopped channel idles at, and the latch. `PARA_UP` is write-triggered —
 * the silicon acts on it and it reads back clear. */
#define ESP32_LEDC_CONF0_TIMER_MASK 0x3
#define ESP32_LEDC_CONF0_SIG_OUT_EN (1u << 2)
#define ESP32_LEDC_CONF0_IDLE_LV    (1u << 3)
#define ESP32_LEDC_CONF0_PARA_UP    (1u << 4)

/* `CONF1`: the fade, and `DUTY_START` which begins it. Also
 * write-triggered: a driver asking `is_duty_fade_running` reads this bit,
 * and a model that stored it would answer "still fading" for ever. */
#define ESP32_LEDC_CONF1_SCALE_MASK 0x3ff
#define ESP32_LEDC_CONF1_CYCLE_SHIFT 10
#define ESP32_LEDC_CONF1_NUM_SHIFT 20
#define ESP32_LEDC_CONF1_NUM_MASK 0x3ff
#define ESP32_LEDC_CONF1_INC (1u << 30)
#define ESP32_LEDC_CONF1_START (1u << 31)

/* `DUTY` carries four fractional bits: the duty a driver means is the
 * register shifted right by four. */
#define ESP32_LEDC_DUTY_FRACTION 4

/* A timer's `CONF`: the resolution in bits, the divider in Q10.8, the two
 * stop bits, and its own latch. */
#define ESP32_LEDC_TIMER_RES_MASK 0xf
#define ESP32_LEDC_TIMER_DIV_SHIFT 4
#define ESP32_LEDC_TIMER_DIV_MASK 0x3ffff
#define ESP32_LEDC_TIMER_PAUSE (1u << 22)
#define ESP32_LEDC_TIMER_RST (1u << 23)
#define ESP32_LEDC_TIMER_PARA_UP (1u << 25)

/* `INT_RAW` bit 4 + n: channel n's fade has finished. A driver waits on
 * this one, so a fade that is applied at once still has to raise it. */
#define ESP32_LEDC_INT_FADE_SHIFT 4

/* `CONF`'s clock select, and what each source runs at. The C3's LEDC is fed
 * by one of three; the divider and the resolution are counted against
 * whichever the firmware chose, so a frequency reported from the wrong one
 * would be wrong by a factor of two or five. */
#define ESP32_LEDC_CLK_SEL_MASK 0x3
#define ESP32_LEDC_CLK_APB 80000000.0
#define ESP32_LEDC_CLK_RC_FAST 17500000.0
#define ESP32_LEDC_CLK_XTAL 40000000.0

/* The GPIO matrix's output selection: `FUNC_OUT_SEL_CFG` per pin, and the
 * signal number that means "this pad is a plain GPIO output" rather than a
 * peripheral's. A pad pointed at a peripheral is not driven by `GPIO_OUT`,
 * and reporting it as though it were is how a lamp on a PWM pin reads as
 * dark while the firmware dims it.
 */
#define ESP32_GPIO_FUNC_OUT_ESP32  0x0530
#define ESP32_GPIO_FUNC_OUT_MODERN 0x0554
#define ESP32_GPIO_OUT_SEL_MASK 0xff
#define ESP32_GPIO_OUT_SEL_GPIO 128

/* Which signal each LEDC channel puts on the matrix, on the C3. */
#define ESP32_LEDC_SIG0 45

/* One channel, as the model keeps it: the registers the guest wrote, and
 * the duty that is actually driving the pad — the two differ until a
 * `PARA_UP` or a `DUTY_START` latches the one into the other. */
typedef struct Esp32LedcChannel {
    uint32_t conf0;
    uint32_t hpoint;
    uint32_t duty;
    uint32_t conf1;
    uint32_t live;
} Esp32LedcChannel;

/* One timer: what the guest wrote, and the resolution and divider in force. */
typedef struct Esp32LedcTimer {
    uint32_t conf;
    unsigned res;
    unsigned div;
} Esp32LedcTimer;

/*
 * RMT, which this device also answers for.
 *
 * The sixth peripheral in this file, and the one an addressable LED strip is
 * driven by: `smart-leds` over `esp-hal-smartled` writes a pulse code per
 * bit into the channel's RAM and lets RMT clock them out. Nothing is mapped
 * at it upstream, so that write lands in a hole, the transmission never
 * ends, and a firmware waiting for it waits for ever — the strip dark, the
 * board silent, and the fault apparently in the user's own `write`.
 *
 * What is modelled is the transmission: the RAM, the read pointer, the
 * threshold that asks for a refill, and the end marker that finishes it. The
 * codes are turned into the bits they carry and reported as bytes.
 *
 * **The bit is read from the shape of the code, not from a clock.** Every
 * one-wire LED protocol — WS2812, SK6812, WS2811 — sends a one as a long
 * high followed by a short low and a zero the other way round, so a code
 * whose high half is longer than its low half is a one. That rule is what
 * makes this a *model of RMT* rather than a model of one LED: a driver
 * sending some other protocol is reported by the same rule and the host can
 * say it does not recognise the bytes. Timing is not modelled at all — the
 * divider and the clock source are stored and ignored, since nothing here
 * has to meet a deadline.
 *
 * **The transmission advances when the firmware refills, not on a clock.**
 * A strip longer than the channel's RAM is sent in halves: the hardware
 * raises the threshold interrupt, the driver writes the next half over the
 * half already sent and clears it, and round again. This model consumes a
 * chunk, raises the threshold, and waits to be asked for the next — the
 * driver's own poll of the interrupt register is what asks. So it can
 * never outrun the firmware, which a timer-paced model could.
 */
#define ESP32_RMT_REGION 0x1000
#define ESP32_RMT_WORDS (ESP32_RMT_REGION / 4)

/* Two transmitting channels on the C3, each with 48 codes of RAM at
 * `0x400`, and the two others (which receive) beside them. */
#define ESP32_RMT_TX_CHANNELS 2
#define ESP32_RMT_CHANNELS 4
#define ESP32_RMT_RAM 0x400
#define ESP32_RMT_CODES 48

REG32(RUSTY_RMT_INT_RAW, 0x0038)
REG32(RUSTY_RMT_INT_ST, 0x003c)
REG32(RUSTY_RMT_INT_ENA, 0x0040)
REG32(RUSTY_RMT_INT_CLR, 0x0044)
#define ESP32_RMT_CONF0 0x0010
#define ESP32_RMT_TX_LIM 0x0058

/* `CHnCONF0`: start, the two resets, wrap, and the stop. */
#define ESP32_RMT_TX_START (1u << 0)
#define ESP32_RMT_MEM_RD_RST (1u << 1)
#define ESP32_RMT_APB_MEM_RST (1u << 2)
#define ESP32_RMT_TX_CONTI (1u << 3)
#define ESP32_RMT_TX_WRAP (1u << 4)
#define ESP32_RMT_TX_STOP (1u << 7)

/* `INT_RAW`: end at bit 0 + channel, error at 4 + channel, the threshold
 * that asks for a refill at 8 + channel. */
#define ESP32_RMT_INT_END 0
#define ESP32_RMT_INT_THR 8

/* `CHn_TX_LIM`: how many codes go out before the threshold fires. */
#define ESP32_RMT_TX_LIM_MASK 0x1ff

/* A pulse code is two halves: fifteen bits of duration and a level each. */
#define ESP32_RMT_DURATION_MASK 0x7fff
#define ESP32_RMT_LEVEL0 (1u << 15)
#define ESP32_RMT_SECOND_SHIFT 16
#define ESP32_RMT_LEVEL1 (1u << 31)

/* Which signal each transmitting channel puts on the matrix, on the C3. */
#define ESP32_RMT_SIG0 51

/* The longest run of bytes one transmission reports. A strip of sixty is
 * 180 bytes, which is 360 characters of hex — beyond this the report says
 * how much it dropped rather than growing without limit. */
#define ESP32_RMT_BYTES 256

/* One transmitting channel, as the model keeps it. */
typedef struct Esp32RmtChannel {
    uint32_t conf0;
    uint32_t tx_lim;
    /* Where the next code comes from, and whether a transmission is going. */
    unsigned read_at;
    bool sending;
    /* Asked for the next chunk: set when the driver clears the threshold,
     * acted on when it next reads the interrupt register — which is after
     * it has refilled, because that is the order its loop writes them in. */
    bool hungry;
    /* The bits this transmission has carried, packed as they complete. */
    uint8_t bytes[ESP32_RMT_BYTES];
    unsigned byte_count;
    unsigned bit_count;
    unsigned dropped;
} Esp32RmtChannel;

/* One device: an address and the 256 registers behind it.
 *
 * A pointer, because that is what an I2C sensor is: a write of one byte
 * moves it, and a read takes bytes from there onwards. A display ignores
 * the pointer and cares only that its writes were seen, which the same
 * model gives for free. */
typedef struct Esp32I2cDevice {
    bool present;
    /* Whether the host has given it anything to be read from: a sensor has
     * registers and a display does not, which is what decides whether its
     * writes are worth repeating on the channel. */
    bool has_regs;
    uint8_t address;
    uint8_t pointer;
    uint8_t regs[256];
} Esp32I2cDevice;

/*
 * IO_MUX, which this device also answers for — the seventh peripheral, and
 * the smallest.
 *
 * A pad's pull-up and pull-down live here, not in the GPIO peripheral, and
 * without them an input nobody drives reads whatever it last read. That is
 * the quiet wrong answer behind every button: `Input::new(pin, Pull::Up)`
 * and `is_low()` is how nearly every button on every board is read, and a
 * model with no pull leaves the pin at zero — a button that reads as held
 * down from the moment the firmware starts, until the host happens to drive
 * it high. It is also the whole of what a matrix keypad rests on: the
 * columns float to their pull-ups and a pressed key drags one down to the
 * row driving it.
 *
 * Only the two bits are modelled. The drive strength, the function select
 * and the input enable are stored and answered so a driver's
 * read-modify-write keeps what it put there, and nothing here acts on them:
 * a pad's function is the matrix's business (`FUNC_OUT_SEL_CFG`), which
 * this device already reads.
 *
 * **Mapped on the C3 only.** The ESP32's IO_MUX registers are not in pin
 * order — they are a table indexed by pad name — so the same arithmetic
 * would put GPIO2's pull on another pin's register. With the region
 * unmapped there, `io_mux` stays zero, no pin has a pull, and that machine
 * behaves exactly as it did.
 */
#define ESP32_IOMUX_REGION 0x1000
/* `IO_MUX_PIN_CTRL` is at 0, and the pads follow it one word each. */
#define ESP32_IOMUX_PIN0 0x04
#define ESP32_IOMUX_WPD (1u << 7)
#define ESP32_IOMUX_WPU (1u << 8)

/* How many pin-to-pin switches the host may declare.
 *
 * Sixteen is a 4x4 keypad, which is the case this exists for; a larger
 * matrix declares more switches than this and the ones past the end are
 * refused by name rather than silently dropped. */
#define ESP32_GPIO_SWITCHES 24

/* A switch between two pads, as the host declared it.
 *
 * Not a level and not a drive: a closed switch *joins* two pads, and which
 * way the level then flows is whichever of them is driving. That is the
 * difference between this and `<pin>=<level>`, and the reason a matrix
 * needs it — a key ties a row to a column, and the row is an output only
 * during the moment the firmware scans it. */
typedef struct Esp32GpioSwitch {
    bool present;
    bool closed;
    uint8_t a;
    uint8_t b;
} Esp32GpioSwitch;

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
    /* Which pins the host has *said* a level for. A pull only answers for a
     * pad nobody is driving, so "the host drove it low" and "the host has
     * said nothing about it" have to be different states — `in` alone
     * cannot tell them apart, and a pull-up would then never be believed. */
    uint64_t host_driven;
    /* What each pad is at once the switches and the pulls have been read:
     * what an input reads, and what the pin channel reports. Recomputed by
     * `esp32_gpio_settle`, which is the only writer. */
    uint64_t resolved_in;

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
    /* Whether anything has been reported since the transaction began, which
     * is what tells a continuation from a message of its own. */
    bool i2c_continues;
    /* The last transaction reported, without its timestamp, so the same one
     * repeated is said once. */
    char i2c_last_report[ESP32_BUS_VERBS][ESP32_I2C_REPORT];

    /* The SPI master, and what each chip select answers with. */
    MemoryRegion spi_iomem;
    uint32_t spi_reg[ESP32_SPI_WORDS];
    uint8_t spi_miso[ESP32_SPI_SELECTS][ESP32_SPI_BUFFER];
    unsigned spi_miso_len[ESP32_SPI_SELECTS];
    char spi_last_report[ESP32_BUS_VERBS][ESP32_I2C_REPORT];

    /* LEDC: the timers, the channels, and what was last said about each
     * channel's pin, so a duty that has not moved is not said twice. */
    MemoryRegion ledc_iomem;
    Esp32LedcChannel ledc_ch[ESP32_LEDC_CHANNELS];
    Esp32LedcTimer ledc_timer[ESP32_LEDC_TIMERS];
    uint32_t ledc_conf;
    uint32_t ledc_int_raw;
    uint32_t ledc_int_ena;
    int ledc_said_pin[ESP32_LEDC_CHANNELS];
    char ledc_said[ESP32_LEDC_CHANNELS][ESP32_I2C_REPORT];

    /* The GPIO matrix's output selection per pad, which is how a
     * peripheral's signal is followed to the pin it reaches — and how a pad
     * a peripheral drives is told from one `GPIO_OUT` drives. */
    uint32_t func_out[ESP32_GPIO_PINS];
    hwaddr func_out_reg;

    /* RMT: the transmitting channels, the RAM they read their codes from,
     * and the interrupts a driver's refill loop polls. */
    MemoryRegion rmt_iomem;
    Esp32RmtChannel rmt_ch[ESP32_RMT_TX_CHANNELS];
    uint32_t rmt_ram[ESP32_RMT_CHANNELS][ESP32_RMT_CODES];
    uint32_t rmt_int_raw;
    uint32_t rmt_int_ena;
    uint32_t rmt_sys_conf;

    /* IO_MUX: one register per pad, of which two bits are modelled. */
    MemoryRegion iomux_iomem;
    uint32_t io_mux[ESP32_GPIO_PINS];

    /* The switches the host has put between pads. */
    Esp32GpioSwitch switches[ESP32_GPIO_SWITCHES];
} Esp32GpioState;

typedef struct Esp32GpioClass {
    SysBusDeviceClass parent_class;
} Esp32GpioClass;
