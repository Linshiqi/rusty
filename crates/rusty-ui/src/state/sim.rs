//! The simulator's state: the plan, the board, what the firmware reported,
//! and the traces the Plot and Waves panels draw.

use super::*;

/// Whose clock the trace timestamps are on.
///
/// Mixing the two silently is how a waveform lies, so the panel shows which
/// one it got. Firmware means `[rusty:gpio@µs]` stamps from the systimer;
/// Host means the firmware sent none and arrival time stood in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceClock {
    Firmware,
    Host,
}

/// Named numeric channels over time — the analog half of the trace.
///
/// Kept per channel rather than as rows of samples because that is how it is
/// drawn and how it arrives: firmware prints whichever channels it has this
/// loop, and a channel that starts appearing halfway through a flight is
/// normal, not a schema change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plot {
    /// Channel name → its samples, `(µs, value)`, oldest first.
    pub channels: Vec<(String, Vec<(u64, f32)>)>,
    pub clock: Option<TraceClock>,
    /// True once the cap started dropping the oldest samples.
    pub truncated: bool,
}

impl Plot {
    /// The samples for a channel, creating it on first sight.
    pub fn channel(&mut self, name: &str) -> &mut Vec<(u64, f32)> {
        if let Some(index) = self.channels.iter().position(|(known, _)| known == name) {
            return &mut self.channels[index].1;
        }
        self.channels.push((name.to_string(), Vec::new()));
        &mut self.channels.last_mut().expect("just pushed").1
    }
}

/// A captured pin trace: time-ordered `(µs, pin, level)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimTrace {
    pub events: Vec<(u64, u8, bool)>,
    pub clock: Option<TraceClock>,
    /// True when the cap was hit and the oldest events were dropped.
    pub truncated: bool,
}

/// A running simulation: the board, the plot, the trace, the tunables.
#[derive(Clone, Copy)]
pub struct Sim {
    /// What the firmware last printed to the `[rusty:disp]` channel.
    pub display: RwSignal<String>,
    /// The waveform capture for the current simulation run.
    pub trace: RwSignal<SimTrace>,
    /// Named numeric channels the firmware is printing, for the Plot panel.
    pub plot: RwSignal<Plot>,
    /// Which channels are drawn. Empty means all of them — a firmware with
    /// forty channels needs a filter, one with three does not.
    pub plot_shown: RwSignal<Vec<String>>,
    /// Tunables the firmware announced, newest value per name.
    pub params: RwSignal<Vec<rusty_embed::protocol::Param>>,
    /// The port rusty is holding open in both directions, if any. Distinct
    /// from a session merely running: a spawned `espflash monitor` is a
    /// session and cannot be written to, and a tunable that silently went
    /// nowhere would read as firmware ignoring it.
    pub link_port: RwSignal<Option<String>>,
    /// Whether the running simulation's clock is stopped. Optimistic — the
    /// button sets it and the command may still refuse — because the
    /// alternative is a button that does nothing for a round trip.
    pub paused: RwSignal<bool>,
    /// Pin levels for the board view, from whichever source [`sim_pin_source`]
    /// names.
    pub gpio: RwSignal<HashMap<u8, bool>>,
    /// Duty cycles for the board view, from `[rusty:pwm]` — the analogue
    /// half of [`Self::gpio`], and what a motor turns on and a lamp dims by.
    ///
    /// **Absent is not zero.** A pin with no entry has never been reported,
    /// and a motor on it says so rather than showing a commanded stop; a pin
    /// mapped to `0.0` was told to stop. The two look the same on a dial and
    /// mean opposite things when a motor will not start.
    ///
    /// **A pin is in this map or in `gpio`, never both**, and the newer
    /// report decides which. A pad the LED controller drives has no level —
    /// a level left from before it took the pin would light a lamp at full
    /// that the firmware is dimming — and one given back to GPIO has no duty,
    /// or a lamp would stay at the last duty whatever the pin did next.
    pub pwm: RwSignal<HashMap<u8, rusty_embed::Duty>>,
    /// Sensors the firmware has declared it wants fed, newest wins by name.
    ///
    /// Declared rather than guessed, for the reason the tunables are: a panel
    /// that invented `gyro` and a range for it would one day inject 2000°/s
    /// into a loop written for 250. Empty means the firmware has asked for
    /// nothing, and the panel offers nothing.
    pub sensors: RwSignal<Vec<rusty_embed::SensorDef>>,
    /// The last sample the panel *sent* for each sensor — what its sliders
    /// sit at. Not what the firmware did with it, which only the firmware can
    /// say and only by printing something.
    pub sensor_values: RwSignal<HashMap<String, Vec<f32>>>,
    /// Raw ADC counts the panel is holding on each pin.
    pub analog: RwSignal<HashMap<u8, u16>>,
    /// The readings of the sheet's register sensors the panel moved during
    /// this run, by part and channel (`("U2", "ax")`). Empty when a run
    /// starts, so the sliders stand where the sheet's props say the run
    /// began.
    pub readings: RwSignal<HashMap<(String, String), f64>>,
    /// The last transactions on the emulator's I2C bus, oldest first and
    /// capped, from `[rusty:i2c]`.
    ///
    /// A list and not a map: what a bus is worth showing is its *traffic* —
    /// a display's stream of bytes, a driver's probe that got no answer —
    /// and the last state per address would lose exactly that. Only rusty's
    /// build of QEMU fills it, so empty means either an idle bus or an
    /// emulator that has none.
    pub i2c: RwSignal<Vec<rusty_embed::I2cReport>>,
    /// The screens on that bus, by address: what the firmware's own display
    /// driver has drawn on each.
    ///
    /// **An entry is a declaration, not a discovery.** The sheet puts one
    /// here for every display part that names its controller, and traffic to
    /// an address with no entry is left as traffic. Which controller it is
    /// decides how the bytes are read — the SH1106's window sits two columns
    /// into its RAM — and a decoder that picked one for an address it had
    /// merely seen bytes on would draw a picture nobody could check. So the
    /// part says, and until it does the screen shows what the firmware
    /// prints to `[rusty:disp]`, as it always has.
    ///
    /// Rebuilt by the panel when the sheet's displays change and fed by
    /// `absorb`, which is the one place the protocol is read.
    pub screens: RwSignal<HashMap<u8, rusty_embed::screen::Screen>>,
    /// The last transmission RMT clocked out on each pin, from
    /// `[rusty:rmt]` — the bytes, not the colours, because what they mean
    /// is the part's business and a WS2812's order is not a WS2811's.
    ///
    /// The whole transmission and not the last pixel: one write sets every
    /// LED in a chain, so anything less would show a strip lighting one
    /// pixel at a time.
    pub rmt: RwSignal<HashMap<u8, Vec<u8>>>,
    /// The same for SPI2, from `[rusty:spi]`: what went out on each chip
    /// select and what came back.
    pub spi: RwSignal<Vec<rusty_embed::SpiReport>>,
    /// Raw ADC counts the firmware's own converter has *read* off each pin,
    /// from `[rusty:adc]` — the return half of [`Self::analog`].
    ///
    /// Two maps and not one, deliberately. One is what the host is driving
    /// and the other what the guest took; when they agree the loop is closed,
    /// and when they do not the panel can say which end is not moving.
    /// Merged, a slider that moved firmware which never read it would look
    /// exactly like one that worked. Only rusty's build of QEMU fills this,
    /// so empty is the ordinary state of a run on Espressif's.
    pub adc: RwSignal<HashMap<u8, u16>>,
    /// The simulated aircraft, when the physical loop is closed.
    ///
    /// Injecting a rate proves the controller *responds*; it cannot show
    /// whether the loop settles, because the rate never changed in answer to
    /// the motors. This is the integrator that closes it: motor duties in,
    /// body rates out, fed back as the sample the firmware reads.
    pub plant: RwSignal<rusty_embed::Plant>,
    /// Whether that feedback is running. Off by default — a panel that
    /// started injecting on its own would make a firmware that reads its own
    /// IMU see two sources disagreeing.
    pub plant_closed: RwSignal<bool>,
    /// Guards the plant's timer against a stale one from a previous run, the
    /// way the editor's pulse does.
    pub plant_gen: RwSignal<u64>,
    /// Where those levels came from, as announced by the run that started.
    ///
    /// `Firmware` until a run says otherwise, because that is what every
    /// emulator did until rusty shipped one that keeps pin state — and a
    /// caption claiming register-level truth over a stock QEMU would send a
    /// user with a dark LED to check their wiring instead of their `println!`.
    pub pin_source: RwSignal<rusty_embed::PinSource>,
    /// The simulation plan for the open project, when the panel asked.
    pub plan: RwSignal<Option<rusty_embed::SimPlan>>,
    /// Tools whose one-click install failed — those cards reveal the manual
    /// instructions, which stay hidden while the button still deserves trust.
    pub install_failed: RwSignal<Vec<String>>,
    /// The board editor's unsaved sheet, which Run writes before it starts
    /// so the emulator is wired as the screen shows — the board is what
    /// runs as much as the code is. The editor on screen registers it under
    /// a number of its own, so the one it replaces cannot take the new
    /// registration with it when it goes (`controller::save_sheet_then`).
    pub unsaved_sheet: StoredValue<Option<(u64, UnsavedSheet)>>,
}

/// Asks the board editor on screen for its sheet, when it has changes the
/// file does not.
pub type UnsavedSheet = Callback<(), Option<rusty_embed::Sheet>>;
