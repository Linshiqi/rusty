# Signals: a generator for the simulator, and the tools to judge a filter with it

A filter is designed against signals and proven against them. On a desk that
means a function generator on the input and a scope on the output; in rusty it
means a signal played into the firmware's own ADC or sensor, the firmware's
filtered result read back off its telemetry, and the two compared in time, in
frequency and as a measured response. This document is the design; the code
follows it.

## What is claimed, and what is not

- **The signal reaches the firmware in the firmware's own time.** A table of
  samples is played by the emulator against its virtual clock — the clock the
  firmware's timers and systimer run on — so a 50 Hz tone sampled at 1 kHz is
  a 50 Hz tone to the firmware, whatever the host was doing. This is the whole
  reason the emulator changes: the host pushes a value about once a
  millisecond with jitter, which is a phase error of a third of a radian at
  50 Hz — enough to make a notch filter look broken and a working one look
  like noise.
- **What the host renders is exact; what it cannot know it says.** The table
  is the signal after the sheet's circuit (an RC on the pin shapes it as the
  solver says). A digital pin the firmware moves later, which changes that
  circuit, changes what the table should be; the table is re-rendered from
  that moment and the seam is said, not hidden.
- **A sensor on the bus is sample-exact too; one on the console is not.** A
  part with a `model` (MPU-6050, BMP280, BME280) is played the same way, its
  whole register block per sample, latched when a read transaction addresses
  it — so a burst read is one sample, never half of two (the rule "a sample
  travels whole or not at all" from the Flight panel, applied to the bus). A
  channel declared with `[rusty:sensor]` is fed over the console at the host's
  pace, and the panel says so.
- **An older emulator is not asked to pretend.** Playing tables is a model
  only rusty's newest build carries (`[rusty:wave@` is its marker). With an
  older copy a signal cannot be played in the firmware's time, the plan says
  so beside the Upgrade button, and nothing claims otherwise.

## The pieces

### `rusty_embed::signal` — what a generator produces

A `Signal` is a sum of components in the units of whatever it feeds: volts at
a pin, rad/s on a gyro axis, °C on a thermometer. Components: `dc`, `sine`,
`square`, `triangle`, `sawtooth`, `chirp` (a sweep, repeating), `white` and
`pink` noise, `spikes` (impulsive outliers, what a median filter is for) and
`step`. Rendering is deterministic — the same signal, rate, length and seed
give the same samples on every machine — so a run, a preview and a test all
see one signal.

The stored form is compact and hand-editable, because it lives in a part's
properties in `.rusty/sim.toml`:

```text
dc 1.2; sine f=50 a=0.1; white rms=0.005
```

Parsing is strict: an unknown kind or key, or a number that does not parse,
is an error naming it. Presets cover the real-world cases filters are built
for (a slow sensor under mains hum, a vibrating accelerometer axis, a drifting
noisy thermistor, a signal with spikes, a chirp, a step, two tones); the
interface names them by id.

A table loops. A signal whose periodic components are commensurate loops
without a seam over a whole number of their periods; one with noise or an
incommensurate tone loops after its table's length, which the interface shows.

### `rusty_embed::dsp` — what a filter does, and what a spectrum says

Pure and compiled to wasm, so the frontend's filter lab and the backend's
measurements are one implementation:

- a spectrum (radix-2 FFT, Hann/Blackman/rectangular windows, amplitude
  corrected so a sine of amplitude A reads A);
- `tone`: one frequency measured exactly — amplitude and phase — which is what
  a frequency-response sweep reads at each step, on the input and the output;
- filter designs: moving average, exponential, RBJ biquads (low, high, band,
  notch), Butterworth low/high of any order (cascaded sections, bilinear with
  prewarping), windowed-sinc FIR, and median; each realised at a sample rate,
  applied to samples, evaluated for its frequency response (none for the
  median), and exported as `no_std` Rust (`f32`, `step(&mut self, x)`) whose
  output is proven equal to the library's.

### The emulator: tables played against the virtual clock

Two additions to rusty's device (`qemu/esp32_gpio.c`), on the pin channel like
everything else the host says, and double-buffered so a table being replaced
never plays half-written:

```text
W<pin>=<rate>,<len>          begin a table for an ADC pin: len samples at rate Hz
W<pin>@<offset>=<hex>        fill it: four hex digits per sample (counts)
W<pin>=on                    play it; a pin already playing keeps its phase
W<pin>=off                   stop; the pin reads its A<pin>= value again

i2c <addr>~<reg>:<width>=<rate>,<len>   begin a register-block table
i2c <addr>~@<offset>=<hex>              fill it: bytes, two hex digits each
i2c <addr>~=on | i2c <addr>~=off        play or stop
```

A conversion of a playing pin reads the table at the virtual clock, linearly
interpolated between samples; a read transaction addressing a playing device
latches the block's sample first. Each `on` and `off` is announced —
`[rusty:wave@<us>] <pin> on <start>` — so the host knows exactly when sample 0
played and can line the ideal signal up with the conversions the emulator
reports (`[rusty:adc@<us>]`, per change, which with a signal playing is every
conversion).

Proven by a gate (`qemu/wave-probe`): every conversion the emulator reports is
the table's value at that instant, the firmware reads the same values, a tone
the firmware times with its own systimer has the table's frequency, and a
sensor's burst read is always one sample whole.

### The host: a generator on the sheet, and signals on sensors

- **`rusty:SignalGen`** is a part: `OUT` and `GND`, like the BNC of a bench
  generator, with its `signal` in its properties. The circuit treats it as a
  voltage source that changes with time; the table for each ADC pin it reaches
  is the pin's voltage over time, through whatever the sheet puts between
  them, in the converter's counts at the pin's `fullscale`.
- **A sensor reading can be a signal**: `signal.<reading>` on a part with a
  `model`, played as that part's register block. A range the firmware chooses
  re-encodes the table, as it re-encodes the static readings today.
- **A console sensor channel** (`[rusty:sensor]`) can be driven by a signal at
  the host's pace.

### The analysis: time, frequency, response, design

In one dock tab:

- **Time**: the ideal signal, what the converter handed over, and the
  firmware's filtered telemetry (`[rusty:tel]`), on one clock.
- **Spectrum**: any of those, windowed, in dB, with the generator's
  frequencies marked.
- **Response**: a sweep — a sine stepped across a range, each step measured by
  `tone` on the input and on a telemetry channel the firmware prints — drawn as
  gain and phase against frequency, beside the design's own curve when there is
  one. What it needs from the firmware (a filtered value printed at its sample
  rate) is said where the button is.
- **Design**: a filter chosen and tuned against the same signal, without
  running anything; its response; its `no_std` code to copy.
