# Examples

Each is a small, complete project that proves one thing, and each opens in
rusty as it is: Build, Run in the simulator, Flash. The firmware ones are for
the ESP32-C3 and run in rusty's emulator, most with the board their
`.rusty/sim.toml` draws; `draw-vectors` runs on this machine.

| Example | What it shows |
|---|---|
| [blink-rust](blink-rust) | The smallest whole loop: one LED, and the serial protocol the board view reads. |
| [motor-drive](motor-drive) | A toy car's drive and a fan — PWM duties on the board view rather than on/off. |
| [sense-board](sense-board) | A board that reads: a knob on the ADC and a sensor on I2C, through the ordinary drivers. |
| [pid-tune](pid-tune) | A control loop tuned while it runs: telemetry in the Plot panel, tunables with the range the firmware gives. |
| [rate-loop](rate-loop) | A quadcopter rate loop flown at a desk: a declared gyro fed by a plant, four motor duties out. |
| [filter-lab](filter-lab) | A filter judged against a signal the sheet plays into the ADC — the worked end of the Signals tab. |
| [rust-calls-c](rust-calls-c) | Rust calling a C function compiled into the crate with `cc`. |
| [c-calls-rust](c-calls-rust) | A C driver calling into Rust for each frame's value — the migration direction. |
| [draw-vectors](draw-vectors) | Two vectors and their cross product, drawn from code with `rusty-draw` into the Draw tab. |

The headless checks in the repository's `CLAUDE.md` run several of these
without the window — `sim_probe`, `loop_probe`, `flight_probe` and
`filter_probe` — and `rusty-cli sim <example>` runs any of the firmware ones
from a terminal.

For a part other than the ESP32-C3, File ▸ New project writes a project known
to build for it, and File ▸ Playground opens the ones that are simulated.
