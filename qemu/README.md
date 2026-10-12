# The GPIO the stock emulator does not have

Espressif's QEMU boots our firmware faithfully — the CPU, the UART, the
timers are all real — but its GPIO peripheral is a stub. In
`hw/gpio/esp32_gpio.c` the write handler is *empty*, and reads answer only
the boot-strapping register:

```c
static void esp32_gpio_write(void *opaque, hwaddr addr,
                       uint64_t value, unsigned int size)
{
}
```

So nothing is stored, and that is why probing the real register addresses
over QMP reads zero on both esp32 and esp32c3: there is no state to read.

Two consequences, and they are the ceiling on everything the board view can
honestly claim:

- **The firmware has to narrate its own pins.** A LED lights because the
  firmware printed `[rusty:gpio] 0=1`, not because a pin went high. Code
  that does not print tells the simulator nothing.
- **A pin cannot be read at all.** `Input::new(…).is_high()` is always
  false in the simulator, which is why a button press has to be injected as
  `B14=1` over the UART instead of through the GPIO the firmware actually
  reads.
- **And it can never interrupt.** Firmware that asks to be woken by an edge
  — how nearly every real button is read — waits for ever, which looks like
  a hang in the user's own code rather than a hole in the emulator.
- **And there is no analog side, and no buses.** Nothing answers at the SAR
  ADC's registers, the I2C master's, or `SPI2`'s, so `adc.read_oneshot()`
  polls a done bit nothing can set and a driver's first transaction waits for
  a peripheral that is not there. Same shape of failure three times over, in
  the user's own `read` call.

`esp32_gpio.c` here fills in the model: the output and enable registers,
their set/clear aliases, the input register, and the interrupt half — the
status register, its set/clear aliases, the CPU's pending view and each
pin's trigger configuration, with the edge types latched and the level
types following the level, as the silicon does. All forty pins, since the
original ESP32 keeps GPIO32..39 in a second bank and that is where its
input-only pins live. Pin changes leave on their own chardev — not the
UART, which belongs to the firmware — and input can be driven back in, so
unmodified firmware reads a real pin.

Replacement files rather than a patch: what is being replaced is a stub, so
almost every line changes and a unified diff's line numbers would be the
fragile part of an otherwise total substitution. `upstream.sha256` does the
job a patch's context would — if Espressif edits either file, the build stops
and says so rather than silently discarding their change.

## Two holes between the pin and the handler

A model that raises an interrupt is not yet an interrupt the firmware takes,
and both of the things in the way fail *silently* — the firmware simply
never runs its handler, which reads as a hang in the user's own code.
`patches.py` fills them, as anchored insertions after text it insists on
seeing exactly once; the files are pinned in `upstream.sha256` like the GPIO
ones.

- **Neither machine connects the device's interrupt line.** `sysbus_init_irq`
  gives the model a line, and nothing was ever on the other end of it,
  because the stub never raised one. `qemu_set_irq` on an unconnected line
  returns without doing anything, so the peripheral reports into nothing.
  Two lines, one per machine (`hw/riscv/esp32c3.c`, `hw/xtensa/esp32.c`).
- **The C3's interrupt matrix does not say which source is asserting.**
  `INTERRUPT_CORE0_INTR_STATUS_0/1` are the two registers a dispatcher reads
  to find out what to call, and `hw/riscv/esp32c3_intmatrix.c` answers zero
  for both. ESP-IDF gives each source its own CPU line and dispatches on the
  line number, so it never reads them and upstream never needed them;
  esp-hal shares one CPU line between sources and reads them on every
  interrupt. The state is already there in `irq_levels` — only the two
  registers reading it were missing. Until they answered, the CPU took the
  interrupt, esp-hal found nothing pending, and returned: from the
  firmware's side, identical to a line that was never raised. That is what
  gate 6 caught, with the model's own witness insisting it had raised it.

**The ESP32 had both holes and three more, and all five are filled.** Its
status words are DPORT's — three per core at 0xec, which upstream reads as
zero — and `hw/xtensa/esp32_intc.c` kept no level state to answer them
from. It keeps every source's level now (`rusty_levels`), and a second
region of the matrix, mapped over DPORT's window at 0xec, answers the words
for both cores and for every source. The first answer to this was narrower
— a region of *this* device answering for GPIO's source alone — and it made
an edge reach its handler while a timer on the same part never did.

The matrix also drove each CPU line from whichever source had changed last.
Right under ESP-IDF, which gives every source a line of its own; wrong under
esp-hal, which maps sources onto a line by priority, so a TIMG alarm and
`FROM_CPU0` share one. A timer handler that wakes a task by raising the
software interrupt and *then* clears its own source lowered the line under
the switch it had just raised; the switch was never taken, nothing set the
next alarm, and an Embassy application's clock stopped at its first tick.
A line is the OR of every source mapped to it now, recomputed on every
change of a level and every write to the map.

And the ESP32's timer group enables a timer's level interrupt through that
timer's own `LEVEL_INT_EN` — `INT_ENA` does nothing for it on this part,
and esp-hal never writes it there — while upstream's model gated the line on
`INT_ENA`: `INT_RAW` set, the line down, the alarm silent. The model raises
the line on `LEVEL_INT_EN` and reports the raw bit in `INT_ST` the same way
(`hw/timer/esp32_timg.c`).

The last hole was not an interrupt model at all. With the status words
answered, a GPIO edge on an ESP32 reached esp-hal's dispatcher and its
handler still never ran — and the handler's own context save turned out to
be writing the CPU's registers through the GPIO window. Read back from the
guest, the chain said why: esp-hal's interrupt entry saves the
floating-point registers (`float-save-restore`, a default feature), the
FPU was switched off, and the save faulted inside itself. Upstream's system
emulation leaves `CPENABLE` at zero; the silicon has the FPU on from reset —
nothing in the ROM, the bootloader or an esp-hal application writes it,
disassembled, and esp-hal's interrupts work on real boards. `patches.py`
brings the CPU out of reset the way the board does
(`target/xtensa/cpu.c`). Gates 15 and 16 hold both.

## And the analog half

A pin is not only high or low. A knob, a divider, a light sensor and a
battery are all read through the SAR ADC, and **nothing is mapped at its
registers on the C3** — so `adc.read_oneshot()` does not return a wrong
number, it waits: the driver polls a done bit that nothing can set, and the
firmware hangs inside the user's own `read` call with nothing anywhere to
say why. Measured, not feared: the analog probe on the stock build prints
`the conversion never finished` and stops, which is the only reason it is
visible at all.

`esp32_gpio.c` answers for it as a second MMIO region, and `patches.py` maps
that region. Deliberately the same device: the analog value on a pin and its
digital level are two readings of one wire, they arrive on the same channel
from the same host, and a separate device would need a link back to this one
for every conversion. A second *file* would also mean a new entry in
upstream's build system, and every one of those is a way for a build to fail
that has nothing to do with what is being modelled.

What it models is one-shot conversion and nothing else: the channel and start
bits, the two data registers, the done bit and its clear. Everything else in
the window is shadowed, so a driver's read-modify-write of a register this
has no opinion about keeps what it put there. A conversion is instant —
the silicon takes microseconds and the driver waits for the bit either way,
so a timer here would only add a way to lose one.

**The original ESP32 has a different converter, not a different layout of
this one.** There is no `APB_SARADC` on it; its SAR is driven from `SENS`,
where one register per unit — `SAR_MEAS_STARTn` — carries the pad enable as
a bitmap, the start bit, the done bit and the counts. The same region
answers that way on that part, mapped at `SENS` and cut down to the 0x400
window it has there, and the channels are that part's own scattered table:
ADC1 on GPIO36..39 and 32..35, ADC2 on ten ordinary pads.

**And every other peripheral here is on both machines too**, each with the
layout of its own part chosen in `realize` exactly as the pin
configuration's offset always was: the I2C master's sixteen command slots
and its 0..4 op codes, SPI2's registers (every one of which moved, down to
the start bit), LEDC's two halves — the high-speed one with no latch at all
— and RMT's eight channels, their control bits in a second register and
their RAM twice as far in. Upstream maps its own models at I2C, SPI2 and
LEDC on the ESP32; none of them can reach this channel, so these go over
them at a higher priority rather than by deleting somebody else's device.

**Counts, not volts**, on the host's side and the model's. rusty does not
know anybody's divider or reference, and a voltage the emulator converted
itself would be a confident number the firmware's arithmetic disagreed with.
`A<pin>=<counts>` puts a value on a pin, `[rusty:adc@<us>] <pin>=<counts>`
reports what was taken off it — per *change*, because a driver polling in a
loop converts thousands of times a second and a line each would drown the
channel the console and the board share.

## And the wire

`SPI1` is the flash controller the machine boots through and upstream models
it. `SPI2` — the one a project puts a display or a sensor on — is not mapped
at all, so a driver's first transfer sets the start bit and polls it for
ever. The fourth region on the same device answers for it.

Simpler than the bus, because SPI is: bytes out and bytes in at once, and no
addressing. **What comes back is a buffer the host declares per chip select**
(`spi 0=1a68`), read from its start on every transfer. No register convention
is assumed, because SPI has none — any other rule would be one this model
invented. Past what the host declared is zero, which is what an undriven MISO
line reads as and what every transfer to a display gets.

`[rusty:spi@<us>] 0 w aea501` is what went out and `... 0 r 1a68` what came
back, with the same repeat rule as the bus — *per verb*, because a
full-duplex transfer alternates a write and a read and one shared slot would
suppress neither.

## And the bus

Almost every real board has something on I2C: a display, an IMU, a
temperature sensor. Nothing was mapped at the C3's I2C master either, so the
first transaction a driver ran waited on an interrupt nothing could raise —
the same shape of failure as the converter, inside the user's own `read`.

The master is a third region on the same device, and its bus is **register
files the host declares**. A sensor is a set of registers a driver
reads and a display is a stream of bytes somebody wants to see; 256 bytes and
a pointer serve both. `i2c 68:75=68` puts a byte behind an address,
`i2c 3c=+` declares one with nothing to read, `i2c 3c=-` takes it off.

**An address nobody declared is not answered.** The transaction NACKs, which
is what a bus scan needs and what tells "the part is not on this board" from
"the part is there and quiet". Answering zeros instead would make every scan
find every address — worse than finding none, because firmware that probes
for an optional device would find it every time.

`[rusty:i2c@<us>] 3c w 00ae` reports a write, `… 68 r 68` a read and
`… 68 nak` an address that answered nothing. The same transaction twice in a
row is said once: a driver reading an accelerometer at a kilohertz is the
ordinary case, and a line each would put twenty kilobytes a second down the
channel the console and the board share.

## And how hard

LEDC is the timer behind every servo, dimmed lamp and motor on a hobby
board, and nothing is mapped at it either: the driver writes a duty into a
hole and the pin never moves. A servo stands still, a lamp stays dark, and
there is no error anywhere to say why.

The fifth region answers for it, and reports **what the pin is driven at**
rather than what a channel was set to: `[rusty:pwm@<us>] 5=0.2500@24002.4`
is a quarter drive at 24 kHz on GPIO5. Both halves matter and each is a
different mistake to make.

- **The pin is the matrix's**, not the channel's number. A model reporting
  channels would put a servo's duty on a lamp's pin and look plausible doing
  it, so `FUNC_OUT_SEL_CFG` is read and a channel the matrix routes nowhere
  is not reported at all.
- **The frequency is the timer's**, and a channel points at one of four. Two
  channels on two timers must come back at two frequencies; a model that
  kept one would report a servo at 24 kHz, which is a duty nobody can read
  as an angle. The divider is Q10.8 over the clock the `CONF` register
  selects — 80 MHz APB, 40 MHz crystal or the 17.5 MHz RC oscillator.

The fade hardware is stored and not animated: `DUTY_R` answers with the
target and the fade-end interrupt is raised at once. A driver that waits for
a fade therefore continues, which is the thing that matters; what it does
not get is the middle of the ramp, and the report says where the duty
arrived rather than pretending to a sweep.

## And the strip

RMT clocks out pulse codes, and one-wire LEDs are what a hobby board uses it
for. With nothing mapped, the codes go into a hole, the transmission never
ends and `wait()` never returns — a strip that stays dark and a firmware
apparently stuck in the user's own `write`.

The sixth region is the channels' registers *and* their RAM. What is
modelled is the transmission: the read pointer, the threshold that asks for
a refill, and the end marker that finishes it.

**The bit is read from the shape of the code, not from a clock.** Every
one-wire LED protocol — WS2812, SK6812, WS2811 — sends a one as a long high
then a short low and a zero the other way round, so a code whose high half
is longer than its low half is a one. That is what makes this a model of
RMT rather than of one LED: a driver sending something else is reported by
the same rule and the host can say it does not recognise the bytes.
`[rusty:rmt@<us>] 8 100000002000000030` is three pixels on GPIO8.

**It advances when the firmware refills, not on a clock.** A strip longer
than the 48 codes of a channel's RAM is sent in halves: the hardware raises
the threshold, the driver writes the next half over the half already sent
and clears it, and round again. This consumes a chunk, raises the threshold
and waits to be *asked* for the next — the driver's own poll of the
interrupt register is what asks — so it can never outrun the firmware, which
a timer-paced model could.

## And a screen

A display is the bus read the other way round: nothing is ever read from it,
so its writes are the whole of the picture. Two rules make that readable,
and each is invisible until it is wrong.

- **Every write to a device with no registers is reported.** The repeat
  suppression above is right for a driver polling a sensor and fatal for a
  framebuffer: clearing a screen is the same sixteen zero bytes sixty-four
  times over, each landing somewhere else in its memory. Suppressed,
  sixty-three of them vanish and the host draws a screen with one line on
  it. The rule is the declaration's — an address the host gave registers to
  is a sensor, one it declared bare is a display.
- **A continued transaction says so.** A write longer than the thirty-two
  byte FIFO crosses it in steps, and `w+` is "more of the message before
  this". The control byte that says what every byte after it means comes
  once per transaction; read as messages of their own, each step's first
  data byte is taken for one and the picture is nonsense.

## And the pads themselves

Two things a pad does that no register in the GPIO peripheral says, and
each one is the difference between a board that reads right and one that
reads plausibly wrong.

**A pull.** `Input::new(pin, Pull::Up)` with `is_low()` is how nearly every
button on every board is read, and the pull lives in IO_MUX, which upstream
does not map. With no pull modelled an input reads whatever it last read —
zero, from reset — so a button reads as *held down* from the moment the
firmware starts, until something drives the pad high. The seventh region
answers IO_MUX and two of its bits mean something: `FUN_WPU` and `FUN_WPD`.
Everything else in the register is stored and given back, so a driver's
read-modify-write keeps what it put there.

**Which register is which pad is the part's, not arithmetic.** The C3 puts
the pads after `IO_MUX_PIN_CTRL` in pin order; the ESP32's are a table in
pad-name order — `GPIO0` at 0x44, `GPIO2` at 0x40, `MTDI`, which is GPIO12,
at 0x34 — so the same arithmetic would put one pin's pull on another's
register. The device holds the map (`iomux_at`), filled per part in
`realize` from the field order of the ESP32's own SVD, and checked against
it by script when it was transcribed. And the ESP32's GPIO34..39 have no
pull circuitry at all: their two bits read back zero whatever is written,
so firmware asking for a pull there floats here as it does on the desk.

**A switch between two pads.** `sw 4-6=1` joins two of them and `sw 4-6=0`
parts them again. That is not a level: `4=0` says the *host* is driving pad
4, and a key says two pads are connected and lets whichever the firmware is
driving decide. A matrix keypad is sixteen of these, and the difference is
the whole of why one could not be simulated before — during a scan the row
is an output for a moment, so driving the column low instead would be
holding down every key in that column.

Both end in one place. `esp32_gpio_settle` works out what every pad is at —
a driver through a closed switch first, then a level the host stated, then
the pad's own pull, then what it was left at — and reports and interrupts on
the difference. `[rusty:sw@<us>] 4-6=1` is the model's own account of a
switch, and the marker a host recognises this generation by: the pulls and
the switches are in this one file and are built together, so a binary
carrying that string carries both.

## Building it

`.github/workflows/qemu.yml` clones `espressif/qemu` at the tag rusty pins
(read out of `QEMU_RELEASE`, wherever in `rusty-embed` it lives — searched
for rather than named, because naming it is how this workflow sat broken
through a refactor that moved the file), verifies the checksums, copies the
two model files in, runs `patches.py` over the three it patches, and
builds both `riscv32-softmmu` and `xtensa-softmmu` — the C3 and C6 on one,
the ESP32 and S3 on the other. Run it from the Actions tab; it packages each
platform as an artifact.

## What it is proven to do

Sixteen gates, each able to fail:

1. The upstream files still hash to what this was written against.
2. The built binary contains this model — `strings | grep '\[rusty:gpio@'`,
   by string because a sysbus device may not answer `-device help` at all.
3. Booting `examples/blink-rust` puts **real pin reports on the chardev**, so
   `-global driver=esp32.gpio,property=pins,value=pins` does reach a device
   the machine created, and the model does see the `W1TS`/`W1TC` writes
   esp-hal makes — it never touches `GPIO_OUT`, so a model handling only that
   register would have passed 1 and 2 and reported nothing.
4. The emulator's account of GPIO0 alternates **and contains the firmware's
   own** `[rusty:gpio]` narration of the same pin, in order.

5. A level driven **from the host** reaches the firmware's `is_high()`.

6. A pin edge **interrupts** the firmware, on the C3. `irq-probe/` asks to
   be woken by both edges of GPIO4 and never reads the pin outside its
   handler, so a printed count is an interrupt the peripheral raised and the
   CPU took — the one thing polling cannot fake. It must also be *quiet*
   until the pin moves, and fire once per edge: a model that raised the line
   on configuration, or raised it and left it raised, passes neither.

   Three independent accounts, because "nothing happened" is the least
   useful sentence a gate can end on and this one ended on it twice. The
   device announces every change of its interrupt line on the pin channel
   (`[rusty:irq@<us>] 4`, or `unconnected` when nothing is on the other end
   of the line). The probe prints what it programmed — the pin's
   configuration, the matrix's routing and enables, the machine CSRs — and
   the interrupt latch again whenever it moves. And it counts the handler's
   *entry* separately from its completion, so a handler that ran and died is
   not reported as one that never ran. Its panic handler talks, for the same
   reason. Each of those was added the round after a failure that could not
   say which half was broken; the second of them is what found the interrupt
   matrix's missing status registers, with the model insisting it had raised
   the line and the firmware insisting it had never been interrupted.

Gate 4 is what makes 3 mean something: a model reporting a stuck level, or
the wrong pin, passes everything above it. The two accounts are independent —
one is the register file, the other is a `println!` — so agreement is
evidence and disagreement names which is wrong.

Gate 5 is the other direction, and it needs its own firmware because blinky
never reads a pin. `gpio-probe/` configures GPIO4 as an ordinary input and
prints its level on change, knowing nothing about the simulator; the test
drives `4=1` then `4=0` down the chardev and requires the firmware to have
read 0, 1, 0. Both writes, because a model that ORed every write into the
input register would pass a test that only ever drove a pin high.

It first ran as:

```
emulator : [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]
firmware : [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0]
```

and the check rejected it for disagreeing on the first level. It was wrong to.
`Output::new(peripherals.GPIO0, Level::Low, …)` drives the pin *before* the
loop that prints, so the emulator legitimately holds one transition the
firmware never announced — the model was rejected for being more truthful
than the firmware, which is the entire reason it exists. The check now aligns
the two and reports the lead.

7. The **board** lights. `board_probe` boots `examples/blink-rust` with its
   own `.rusty/sim.toml`, replays the sheet's rules over the pins the
   emulator reports, and requires every lamp wired to a GPIO to have lit
   and gone out. That is the panel's whole claim — this LED is on that pin
   — checked by a machine instead of by somebody looking at it.

8. An **analog value** on a pin reaches `adc.read_oneshot()`. `adc-probe/`
   configures ADC1 channel 3 and prints every reading that changes; the test
   drives `A3=1234`, then `3000`, then `99999`, and requires the firmware to
   have read 1234, 3000 and 4095 — two values because a converter answering
   with a constant passes any single one, and a third out of range because a
   slider dragged past full scale must read as full scale rather than wrap
   to nothing. The emulator's own `[rusty:adc]` account has to carry the same
   numbers.

   The failure it exists for is worse than a wrong reading. With nothing
   mapped at the converter's registers the driver polls a done bit nothing
   can set, and the firmware hangs inside the user's own `read` call. That
   is what the stock build does, measured before the model was written —
   which is why the probe's poll is bounded and prints `the conversion never
   finished` instead of spinning: a witness that reports a hang as silence
   is no witness.

9. A **device on the I2C bus** answers the firmware's driver. `i2c-probe/`
   watches three addresses, each carrying its own assertion: a sensor whose
   registers the host loaded, which must answer *those bytes* and answer six
   of them from one register onwards; a display, whose writes must reach the
   host; and an address nobody declared, which must **not** answer. The last
   is the one the others cannot make — a bus where every address
   acknowledges is a bus where a missing part looks exactly like a present
   one. Then the sensor is taken off the bus and must stop answering, which
   a scan that remembered rather than asked would fail.

   Its `SoftwareTimeout` is there for the same reason the analog probe's
   poll is bounded: without it the driver's first transaction hung, and the
   probe reported the hole as silence.

10. An **SPI transfer** reaches a device and brings back what the host said.
    `spi-probe/` writes three bytes with nothing declared — which is exactly
    a display, and the case where an undriven MISO must read as zeros rather
    than hang — and then reads the two bytes the host puts on chip select 0.
    The second of those is the one a driver reads: it sends a command byte
    and takes the answer out of the same transfer, which is what full duplex
    means and why the buffer is read from its start.

11. A **duty** reaches the pin at the frequency its timer sets.
    `ledc-probe/` drives GPIO5 at a quarter and then three quarters of 24
    kHz, and GPIO6 at eight percent of 50 Hz from a second timer. The second
    pin is the assertion the first cannot make twice over: a model keeping
    one frequency for every channel, or reporting channel numbers instead of
    the pins the matrix routes them to, passes everything about GPIO5 and
    fails here.

12. A **strip's codes** reach the pin as the bytes they carry.
    `rmt-probe/` sends three pixels, which is 72 codes through 48 of RAM —
    so the driver refills once and a model that sent its RAM and stopped
    fails. Both accounts are required: the firmware saying its `wait()`
    returned, which with nothing mapped it never does, and the model saying
    which bytes went out on which pin.

13. A **display's whole frame** reaches the host. `display-probe/` drives a
    panel with the `ssd1306` crate and `embedded-graphics` — somebody else's
    stream, not one written beside the model — and the gate requires the
    write that crosses the FIFO to come back as a `w+` continuation, and
    two frames to arrive as the hundred and twenty-eight transactions they
    are. Suppressed repeats would leave three.

14. A **matrix keypad** rests high and reads the key that is down.
    `keypad-probe/` scans four rows against four columns the way firmware
    does, and the gate asserts both halves: with nothing pressed it must
    report **no key**, which is only true if the columns rest at their
    pull-ups — without them every one of the sixteen reads as held down
    before the firmware has done anything — and with `sw 3-6=1` it must
    report the key where that row crosses that column, which a model
    driving the column instead would spread across the whole column.

15. An **ESP32 application survives its first float**, and the emulator says
    so when it does not. The one gate whose subject is the CPU, written
    against two claims rusty made and took back: that "the emulator stops
    at the first floating-point instruction" (it does not), and then that
    the *application* had to switch its FPU on (it does not either — on the
    silicon the FPU is on from reset; see *Two holes between the pin and the
    handler*). `float-probe/` counts in integers, multiplies two floats and
    counts again, doing **nothing** to `CPENABLE`, as an ordinary application
    does not; the gate requires the product **and** the last line. Built
    **twice**: once more with `FLOAT_PROBE_DISABLE=1`, which switches
    coprocessor 0 off itself before the float — what xtensa-lx-rt does
    inside every interrupt when `float-save-restore` is off — and then the
    run must go quiet after "about to multiply" *and* the emulator must print
    `[rusty:cpu] coprocessor 0 is disabled`, because the symptom on its own
    is silence.
16. An **ESP32 gets the board the C3 gets**. `esp32-probe/` walks every
    peripheral this device answers for, on the original ESP32, where each
    one failed in a way of its own before it was modelled: the pulls land on
    the right pads (IO_MUX's pad-name-ordered table, with pins chosen so the
    table read as arithmetic answers wrongly for all of them) and an
    input-only pad refuses its pull; the converter answers through `SENS`;
    the bus reads a declared register (its op codes are 0..4 here, not the
    C3's 6, 1, 3, 2, 4 — found as `?op0` on this probe's first run); the
    wire reads a declared buffer; both halves of LEDC report a duty and a
    carrier; the strip's bytes come out of RMT; and then the host turns two
    knobs, presses a button and drives an edge, and the gate requires the
    converter to follow, the pulled-up pad to fall, and **the firmware's own
    handler** to count the edge — the chain that needs the dispatcher's
    status words answered and the FPU on from reset. Before the host touches
    anything, **the clock has to run**: a one-shot TIMG alarm whose handler
    raises `FROM_CPU0` and then clears itself, and a switch handler that
    sets the next alarm — Embassy's chain on this part, in which every link
    waits on the one before. Run against a matrix that drives a line from
    the last source to change, it stops at `timer 1 switch 0`; and it is
    checked before the host's edge, because that edge shares the line and
    picks up a dropped switch, which turns a stopped chain into one that
    limps.

## What each desktop needed

Nothing, on Linux and macOS. Windows needed four things, and one platform
each needed a dependency nobody had named — none of it in the emulator, whose
own source compiled clean on the first attempt that reached it, everywhere:

- **A prefix with a drive letter.** QEMU's configure defaults the mingw prefix
  to `/qemu` and meson 1.5 will not call that absolute, so it stops at
  `../meson.build:1:0` before reading anything else. Nothing is installed from
  here, so this only has to satisfy the validation.
- **`--disable-guest-agent`.** qemu-ga's VSS provider needs Microsoft's Volume
  Shadow Copy SDK. It is a program that runs inside a guest OS and has nothing
  to do with the emulator.
- **`static: false` on the slirp dependency.** Espressif's fork writes
  `dependency('slirp', …, static: true)` where upstream 9.2 does not. On MSYS2
  that resolves libslirp's `Libs.private` statically and pulls in
  `libglib-2.0.a` while QEMU links glib as a DLL, so the final link dies in
  289 lines of `multiple definition of g_*`. Not something to silence with
  `--allow-multiple-definition`: two glibs in one binary is two allocators and
  two main contexts.
- **`--disable-debug-info`**, to keep a very large PE within what mingw's ld
  handles. Nothing shipped needs it.

And **libgcrypt**, which only appeared once the xtensa target was added:
`hw/misc/esp32_flash_enc.c` models that part's flash encryption and includes
`<gcrypt.h>`. Ubuntu's runner image happens to ship the headers and the other
two do not, so the Linux build passed while macOS and Windows failed on a
dependency nobody had chosen — this project's own recorded trap, and it fails
first on the platform you were not looking at. Named on all three now.

That one `static: true` produced three different failures before it was
found — undefined `__imp_slirp_*` at link time, then `--disable-slirp` having
no effect at all, then duplicate glib. The middle one is worth knowing about
on its own: the fork wraps the dependency in `declare_dependency`, which
always returns a *found* object, so with slirp disabled `net/slirp.c` is still
added to the build while its include path is gone. configure's own summary
says `slirp: disabled` and `slirp support: YES` on the same page.

The behavioural gates stay on Linux. What differs across these platforms is
glib, pixman and ninja, not `s->out |= word`, and booting firmware three times
would mostly test whether espflash and a TCP port behave the same everywhere.

## Licence

QEMU is GPL-2.0. These files are derivatives of it and carry the same terms,
not rusty's licence. They are kept here, outside the cargo workspace and
applied at build time, so that stays unambiguous.

## Shipped

`qemu-release.yml` runs on a `qemu-v*` tag: it calls the build workflow, and
only if every gate passes does it attach the packages to a Release.
`qemu_download` in `crates/rusty-embed/src/simulate.rs` asks for that Release
first and Espressif's second, so there is nothing for a user to install by
hand and a failure to reach ours degrades to the emulator rusty has always
used rather than to no emulator at all.

Each package carries **both** emulators — `qemu-system-riscv32` for the C3 and
C6, `qemu-system-xtensa` for the ESP32 and S3 — laid out as `qemu/bin` beside
`qemu/share/qemu`, which is Espressif's own layout and therefore a drop-in for
a downloader that already knew how to unpack theirs.

Windows carries its mingw DLLs and macOS is run through `dylibbundler`,
because a dynamically linked build otherwise only runs on the machine that
built it. Linux relies on the system's glib, pixman and slirp, exactly as
Espressif's does; it is built on Ubuntu 22.04, so that is the oldest glibc it
is known to run against.

ARM Linux and Intel macOS are not built here. Espressif publishes both,
`qemu_download` says so rather than 404ing, and those users get the stock
emulator with everything working as it always has, minus real pin state.

Intel macOS is absent for a dull reason worth recording: GitHub retired the
`macos-13` runner, so that job sat queued for 103 minutes while the other
three finished in four to fifteen, and would never have been picked up.
Adding it back needs a runner label somebody has watched work — guessing one
costs an hour of queue to disprove.

## Which one is running

Never assumed. `has_gpio_model` reads the binary for `[rusty:gpio@`, the
marker only this model emits — a version file beside the binary would answer
about the install rather than about the emulator, and a user who dropped
Espressif's build into the same directory would get the wrong answer.

The run then announces `[rusty:pins] emulator` and the board's caption follows
it. That matters more than it sounds: a caption promising register-level truth
over a stock build sends somebody with a dark LED to check their wiring when
the bug is a missing `println!`, and the reverse sends them to re-read
firmware that was right all along.
