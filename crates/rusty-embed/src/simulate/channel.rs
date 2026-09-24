//! The emulator's pin channel, from the host's side: what a run declares
//! before the firmware's first instruction, and the connection every later
//! press, slider and bus reply travels down.
//!
//! Only rusty's build of QEMU has the channel. Espressif's discards every
//! GPIO write, so there is nothing on the other end and a run falls back to
//! what the firmware prints about itself.
//!
//! One implementation for everything that runs a simulation — the window,
//! `rusty-cli sim` and the assistant's tool — because a board that behaved
//! differently depending on who was watching it would be two boards.

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::generator;
use crate::live::Live;
use crate::model::Sheet;
use crate::nets::{self, Behaviour, BusDevice, Row, WireDevice};
use crate::protocol;
use crate::sensor;
use crate::signal::Signal;
use crate::wave::{self, WaveTarget};

/// What a run says down the channel before anything moves.
///
/// The device clears its registers on reset like the silicon does, which
/// makes the host the one authority on what is on a pin or a bus — so the
/// host has to say, once, as soon as there is anything listening. Every
/// slider reads the same props, so the panel and the emulator cannot
/// disagree before anybody has touched anything.
#[derive(Debug, Clone, Default)]
pub struct Start {
    /// The GPIOs whose button pulls them *low* when pressed — a switch to
    /// ground, with the pull-up that `Pull::Up` + `is_low()` reads. A press
    /// on any other button drives high.
    pub low_when_pressed: HashSet<u32>,
    /// What each analog pin carries: an analog source's `start`, and a
    /// potentiometer whose track the sheet has committed to both rails.
    pub analog: Vec<(u32, u16)>,
    /// Everything on the I2C bus, sensors included.
    pub bus: Vec<BusDevice>,
    /// Everything on SPI2's chip selects.
    pub wire: Vec<WireDevice>,
    /// The devices on `bus` whose readings a slider moves.
    pub sensors: Vec<Sensor>,
    /// What plays against the firmware's own clock from the first
    /// instruction: a generator's pins, a sensor's moving readings.
    pub tables: Vec<wave::Table>,
    /// What the run should say about them, a line each: where each signal
    /// plays, and why one does not.
    pub said: Vec<String>,
    /// The sheet and its header, kept so a signal changed while the run
    /// goes on is rendered through the same circuit the first one was.
    pub sheet: Option<Sheet>,
    pub rows: Vec<Row>,
    /// Set by [`Start::without_signals`]: nothing plays on this run.
    pub silent: bool,
}

/// What a run on an emulator that cannot play a table says instead of what
/// its signals would have played.
pub const NO_WAVES: &str = "[rusty:signal] this emulator cannot play a signal against the \
                            firmware's own clock, so nothing plays — the Simulate panel's \
                            Upgrade installs the build that can";

impl Start {
    /// Everything a signal would play, taken off: for an emulator that
    /// cannot play one, which would be handed lines for a model it does
    /// not have. [`NO_WAVES`] is said instead when anything was going to
    /// play, and a signal changed while the run goes on is refused with it.
    pub fn without_signals(&mut self) {
        let had =
            !self.tables.is_empty() || self.sensors.iter().any(|sensor| !sensor.signals.is_empty());
        self.tables.clear();
        for sensor in &mut self.sensors {
            sensor.signals.clear();
        }
        self.said.clear();
        if had {
            self.said.push(NO_WAVES.to_string());
        }
        self.silent = true;
    }
}

/// A sensor on the bus that rusty answers for, register by register.
#[derive(Debug, Clone)]
pub struct Sensor {
    /// The part on the sheet, which is how a slider names it.
    pub part: String,
    pub address: u8,
    pub device: sensor::Device,
    /// The readings a signal moves, each one loop of samples at `rate` a
    /// second. None, and the device holds still where its sliders put it.
    pub signals: Vec<(String, Vec<f64>)>,
    pub rate: u32,
}

impl Sensor {
    /// Its register block as its signals move it, at the encoding the
    /// firmware has chosen — `None` when nothing moves it.
    pub fn table(&self) -> Option<wave::Table> {
        if self.signals.is_empty() {
            return None;
        }
        let block = self.device.block(&self.signals)?;
        Some(wave::Table::Block {
            address: self.address,
            rate: self.rate,
            block,
        })
    }
}

/// What `sheet` declares, read the way the run will say it. `specs` is the
/// part library the sheet's `model` props are resolved against.
pub fn start_of(sheet: &Sheet, rows: &[Row], specs: &[sensor::Spec]) -> Start {
    let behaviour = |reference: &str| sheet.symbol_of(reference).map(nets::behaviour_of);

    let low_when_pressed = sheet
        .parts
        .iter()
        .filter(|part| behaviour(&part.reference) == Some(Behaviour::Switch))
        .filter_map(|part| nets::button_drives(sheet, rows, &part.reference))
        .filter(|(_, high)| !high)
        .map(|(gpio, _)| u32::from(gpio))
        .collect();

    let sources = sheet
        .parts
        .iter()
        .filter(|part| behaviour(&part.reference) == Some(Behaviour::Analog))
        .filter_map(|part| {
            let gpio = nets::gpio_of(sheet, rows, &part.reference, "OUT")?;
            Some((u32::from(gpio), nets::analog_start(part)))
        });
    // A pot the sheet has not committed to sends nothing here and stays
    // what it always was — a `P<pin>=` line for firmware that reads rusty's
    // own text protocol.
    let pots = sheet.parts.iter().filter_map(|part| {
        let span = nets::pot_span(sheet, rows, &part.reference)?;
        Some((
            u32::from(span.gpio),
            span.counts(nets::pot_start(part), nets::adc_max(part)),
        ))
    });
    let analog = sources.chain(pots).collect();

    let bus = nets::bus_devices(sheet, rows, specs).0;
    let mut sensors: Vec<Sensor> = bus
        .iter()
        .filter_map(|device| {
            let part = sheet.parts.iter().find(|p| p.reference == device.part)?;
            let spec = nets::sensor_model(specs, part).ok().flatten()?;
            Some(Sensor {
                part: device.part.clone(),
                address: device.address,
                device: sensor::Device::new(spec.clone(), &part.props),
                signals: Vec::new(),
                rate: 0,
            })
        })
        .collect();

    // What plays against the firmware's own clock: the generators, through
    // the circuit to every converter they reach, and every sensor a signal
    // moves, in its own register block.
    let played = generator::generators(sheet, rows);
    let mut said = played.said;
    let mut tables = played.tables;
    for sensor in &mut sensors {
        let Some(part) = sheet.parts.iter().find(|p| p.reference == sensor.part) else {
            continue;
        };
        let moved = generator::sensor_signals(part, &sensor.device);
        said.extend(moved.said);
        sensor.signals = moved.signals;
        sensor.rate = moved.rate;
        match sensor.table() {
            Some(table) => tables.push(table),
            None if !sensor.signals.is_empty() => said.push(format!(
                "[rusty:signal] {}: its readings span more registers than the emulator plays \
                 as one sample ({} bytes), so no signal moves them",
                sensor.part,
                sensor::MAX_BLOCK
            )),
            None => {}
        }
    }
    // A reading a signal would move on a part rusty does not answer for is
    // a signal nobody would hear, and said as one.
    for part in &sheet.parts {
        let moves = part
            .props
            .keys()
            .any(|key| key.starts_with(generator::SIGNAL_OF) && key != generator::SIGNAL_RATE);
        if moves && !sensors.iter().any(|sensor| sensor.part == part.reference) {
            said.push(format!(
                "[rusty:signal] {}: a signal moves its readings, and rusty answers for it on \
                 the bus only with a `model` and wires to the bus, so nothing plays",
                part.reference
            ));
        }
    }

    Start {
        low_when_pressed,
        analog,
        bus,
        wire: nets::wire_devices(sheet, rows).0,
        sensors,
        tables,
        said,
        sheet: Some(sheet.clone()),
        rows: rows.to_vec(),
        silent: false,
    }
}

/// The level a button state means: pressed is high, unless the button
/// pulls low — the button to ground with a pull-up on the pin, which is the
/// commonest wiring. Before this, every press drove high, and firmware
/// written for a pull-up button saw the emulator's button *release* when the
/// user pressed it.
pub fn pin_level(pressed: u8, active_low: bool) -> u8 {
    if active_low {
        u8::from(pressed == 0)
    } else {
        pressed
    }
}

/// The emulator's pin channel: its own account of every pin, and the way to
/// drive one back.
#[derive(Clone)]
pub struct PinChannel {
    out: Arc<Mutex<Option<TcpStream>>>,
    low_when_pressed: Arc<HashSet<u32>>,
    sensors: Arc<Mutex<Vec<Sensor>>>,
    generators: Arc<Mutex<Generators>>,
    /// Set when the run that opened the channel is over, which is the only
    /// thing that stops the connection being retried.
    closed: Arc<AtomicBool>,
}

/// What a run's generators are rendered from, kept so a signal changed
/// while it runs goes through the same circuit, and the pins their tables
/// are playing on — a pin a changed signal no longer reaches is stopped.
#[derive(Debug, Default)]
struct Generators {
    sheet: Option<Sheet>,
    rows: Vec<Row>,
    playing: Vec<u8>,
    /// An emulator that plays no tables: every change is refused, saying so.
    silent: bool,
}

/// The GPIO a table plays on, when it plays on one.
fn pin_of(table: &wave::Table) -> Option<u8> {
    match table.target() {
        WaveTarget::Pin(gpio) => Some(gpio),
        WaveTarget::Device(_) => None,
    }
}

impl PinChannel {
    /// The level a press or release puts on this button's pin.
    pub fn level_for(&self, pin: u32, pressed: u8) -> u8 {
        pin_level(pressed, self.low_when_pressed.contains(&pin))
    }

    /// Drive a pin from the host — a button press, reaching the firmware
    /// through `GPIO_IN` rather than through a message it had to be written
    /// to expect.
    pub fn drive(&self, pin: u32, level: u8) {
        self.say(&protocol::pin_line(pin, level));
    }

    /// Put an analog value on a pin, in the converter's own counts, so
    /// `adc.read_oneshot()` returns it. Counts and not volts, because rusty
    /// does not know the divider.
    pub fn analog(&self, pin: u32, count: u16) {
        self.say(&protocol::analog_pin_line(pin, count));
    }

    /// Put a device on the emulator's I2C bus: the address first so it
    /// acknowledges even with nothing behind it, then each run of registers.
    pub fn bus_device(&self, device: &BusDevice) {
        for line in protocol::bus_lines(device.address, &device.regs) {
            self.say(&line);
        }
    }

    /// Play a table against the firmware's own clock. One already playing
    /// on the same pin or device is replaced in step: the emulator keeps its
    /// phase, so a signal given a new amplitude does not start over.
    pub fn play(&self, table: &wave::Table) {
        for line in table.lines() {
            self.say(&line);
        }
    }

    /// Stop what plays on a pin or a device. A pin reads its `A<pin>=`
    /// value again; a device's registers hold what they were last given.
    pub fn stop(&self, target: wave::WaveTarget) {
        self.say(&wave::stop_line(target));
    }

    /// What a chip select answers with. Nothing declared is a device that is
    /// written to and says nothing back, which is what a display is.
    pub fn wire_device(&self, device: &WireDevice) {
        self.say(&protocol::wire_line(device.select, &device.miso));
    }

    /// Join or part two pads, which is what a key in a matrix does. See
    /// [`protocol::switch_line`] for why this is not a level.
    pub fn tie(&self, a: u8, b: u8, closed: bool) {
        self.say(&protocol::switch_line(a, b, closed));
    }

    /// A line the simulation is sent on its console. The two console
    /// messages that are also about a pin reach the pin as well: `B14=1`
    /// for firmware reading rusty's text protocol and `14=1` for firmware
    /// reading `Input::is_high()`, and the same for `A3=2048` and the
    /// converter. Sending only the second would break every example;
    /// sending only the first is the limitation this channel exists to
    /// remove.
    pub fn follow(&self, text: &str) {
        if let Some((pin, pressed)) = protocol::button_press(text) {
            self.drive(pin, self.level_for(pin, pressed));
        } else if let Some((pin, count)) = protocol::analog_set(text) {
            self.analog(pin, count);
        }
    }

    /// Move one reading of a sensor on the sheet and write what changed.
    /// False when the sheet has no such sensor or the sensor no such
    /// channel — a slider that silently did nothing would read as firmware
    /// ignoring it.
    pub fn set_sensor(&self, part: &str, key: &str, value: f64) -> bool {
        let lines = {
            let Ok(mut sensors) = self.sensors.lock() else {
                return false;
            };
            let Some(sensor) = sensors.iter_mut().find(|s| s.part == part) else {
                return false;
            };
            if sensor.device.value(key).is_none() {
                return false;
            }
            if !sensor.device.set(key, value) {
                return true;
            }
            // With a table playing, the block is latched from it at every
            // read, so a register written directly would be gone by the next
            // one: the still reading moves by the table being rendered again.
            match sensor.table() {
                Some(table) => table.lines(),
                None => protocol::bus_register_lines(sensor.address, &sensor.device.data()),
            }
        };
        for line in lines {
            self.say(&line);
        }
        true
    }

    /// Change what a signal plays while the run goes on: `signal` on a
    /// generator, `signal.<reading>` on a sensor rusty answers for, `None`
    /// to take it off. Rendered exactly as a run's start renders it and
    /// played in step with what it replaces — the emulator keeps the phase,
    /// so a tone given a new amplitude does not start over. What the run
    /// should say about it comes back, or why nothing changed.
    pub fn set_signal(
        &self,
        part: &str,
        key: &str,
        text: Option<&str>,
    ) -> Result<Vec<String>, String> {
        if let Some(text) = text {
            Signal::parse(text).map_err(|why| why.to_string())?;
        }
        let edit = |props: &mut BTreeMap<String, String>| match text {
            Some(text) => {
                props.insert(key.to_string(), text.to_string());
            }
            None => {
                props.remove(key);
            }
        };
        let gone = || "the run is over".to_string();
        let mut board = self.generators.lock().map_err(|_| gone())?;
        let Generators {
            sheet,
            rows,
            playing,
            silent,
        } = &mut *board;
        if *silent {
            return Err(NO_WAVES.trim_start_matches("[rusty:signal] ").to_string());
        }
        let Some(sheet) = sheet.as_mut() else {
            return Err(format!("this run has no sheet, so {part} is not on it"));
        };

        let (lines, said) = if key == generator::SIGNAL {
            let is_generator = sheet
                .symbol_of(part)
                .is_some_and(|symbol| nets::behaviour_of(symbol) == Behaviour::Generator);
            let Some(instance) = sheet.parts.iter_mut().find(|p| p.reference == part) else {
                return Err(format!("{part} is not on this run's sheet"));
            };
            if !is_generator {
                return Err(format!("{part} is not a signal generator"));
            }
            edit(&mut instance.props);
            let played = generator::generators(sheet, rows);
            let now: Vec<u8> = played.tables.iter().filter_map(pin_of).collect();
            let mut lines: Vec<String> = playing
                .iter()
                .filter(|pin| !now.contains(pin))
                .map(|pin| wave::stop_line(WaveTarget::Pin(*pin)))
                .collect();
            lines.extend(played.tables.iter().flat_map(wave::Table::lines));
            *playing = now;
            (lines, played.said)
        } else if key.starts_with(generator::SIGNAL_OF) && key != generator::SIGNAL_RATE {
            let Some(instance) = sheet.parts.iter_mut().find(|p| p.reference == part) else {
                return Err(format!("{part} is not on this run's sheet"));
            };
            let mut sensors = self.sensors.lock().map_err(|_| gone())?;
            let Some(sensor) = sensors.iter_mut().find(|s| s.part == part) else {
                return Err(format!("rusty does not answer for {part} on the bus"));
            };
            edit(&mut instance.props);
            let moved = generator::sensor_signals(instance, &sensor.device);
            sensor.signals = moved.signals;
            sensor.rate = moved.rate;
            // Nothing left moving it: the table stops and the registers hold
            // what its sliders say, as they did before anything played.
            let lines = match sensor.table() {
                Some(table) => table.lines(),
                None => {
                    let mut lines = vec![wave::stop_line(WaveTarget::Device(sensor.address))];
                    lines.extend(protocol::bus_register_lines(
                        sensor.address,
                        &sensor.device.data(),
                    ));
                    lines
                }
            };
            (lines, moved.said)
        } else {
            return Err(format!(
                "{key} is not where a signal is kept: a generator's is `signal`, a sensor \
                 reading's `signal.<reading>`"
            ));
        };
        drop(board);
        for line in lines {
            self.say(&line);
        }
        Ok(said)
    }

    /// What a sensor would change in answer to a write the firmware made:
    /// a reset bit clearing itself, a forced measurement going back to
    /// sleep, readings re-encoded for a range the firmware just chose.
    fn answer(&self, report: &protocol::I2cReport) {
        if report.verb != "w" {
            return;
        }
        let lines = {
            let Ok(mut sensors) = self.sensors.lock() else {
                return;
            };
            let Some(sensor) = sensors.iter_mut().find(|s| s.address == report.address) else {
                return;
            };
            let mut lines =
                protocol::bus_register_lines(sensor.address, &sensor.device.wrote(&report.bytes));
            // A range the firmware chose re-encodes every reading, and a
            // playing table carries the old encoding until it is rendered
            // again — its phase kept, so the signal does not start over.
            lines.extend(
                sensor
                    .table()
                    .map(|table| table.lines())
                    .unwrap_or_default(),
            );
            lines
        };
        for line in lines {
            self.say(&line);
        }
    }

    /// The run is over: stop trying to connect, and say nothing more. The
    /// emulator waits for this channel before it boots, so the connection
    /// is retried for as long as the run lasts rather than for a fixed time
    /// — a slow first start (a virus scanner reading a new executable) must
    /// not leave an emulator waiting for ever for a connection nobody is
    /// still trying to make.
    pub fn hang_up(&self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Ok(mut socket) = self.out.lock() {
            *socket = None;
        }
    }

    fn say(&self, line: &str) {
        if let Ok(mut socket) = self.out.lock()
            && let Some(stream) = socket.as_mut()
        {
            let _ = stream.write_all(line.as_bytes());
            let _ = stream.flush();
        }
    }
}

/// Connect to the emulator's pin channel on `port`, say what `start`
/// declares, and hand every line the emulator reports to `on_line` until it
/// closes the channel.
///
/// QEMU listens and rusty connects. It has to get as far as opening its
/// listening socket, which is after argument parsing and machine creation,
/// so the connection is retried rather than assumed — and given up quietly:
/// a missing pin channel is a board that falls back to the firmware's own
/// narration, not an error.
///
/// `live` is the sheet's own circuit, walked in step with the firmware:
/// what the converter should read comes back from it and is sent to the pin
/// the firmware samples.
pub fn connect(
    port: u16,
    start: Start,
    live: Option<Live>,
    mut on_line: impl FnMut(String) + Send + 'static,
) -> PinChannel {
    let channel = PinChannel {
        out: Arc::new(Mutex::new(None)),
        low_when_pressed: Arc::new(start.low_when_pressed.clone()),
        sensors: Arc::new(Mutex::new(start.sensors.clone())),
        generators: Arc::new(Mutex::new(Generators {
            sheet: start.sheet.clone(),
            rows: start.rows.clone(),
            playing: start.tables.iter().filter_map(pin_of).collect(),
            silent: start.silent,
        })),
        closed: Arc::new(AtomicBool::new(false)),
    };
    let handle = channel.clone();

    std::thread::spawn(move || {
        // Every ten milliseconds until it answers or the run is over. The
        // emulator waits for this connection before the guest's first
        // instruction ([`super::pins_args`]), so the sooner it is made the
        // sooner the board boots.
        let socket = loop {
            if handle.closed.load(Ordering::Relaxed) {
                return;
            }
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(socket) => break socket,
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        let Ok(reader) = socket.try_clone() else {
            return;
        };
        if let Ok(mut slot) = handle.out.lock() {
            *slot = Some(socket);
        }
        // The first thing said down the channel, before any line is read:
        // what the sheet says is on each analog pin and on each bus. The
        // emulator starts with nothing on them and no way to find out.
        for (pin, count) in &start.analog {
            handle.analog(*pin, *count);
        }
        for device in &start.bus {
            handle.bus_device(device);
        }
        for device in &start.wire {
            handle.wire_device(device);
        }
        // Before the guest's first instruction, like everything above: a
        // firmware that samples as it boots samples the signal, not the
        // silence before it arrived.
        for table in &start.tables {
            handle.play(table);
        }

        // The circuit runs on its own thread, and that is not tidiness — it
        // is the fix for a deadlock a real run found. The emulator reports a
        // conversion only when the value *changed*, and the value only
        // changes when the host sends one; so after a pin moves, nothing is
        // said, the reader blocks, the circuit stays frozen at the instant of
        // the edge, and the firmware's next reading jumps to wherever it had
        // got to by the following edge. A reading that steps instead of
        // climbing is a host echoing a pin level in a circuit's clothes. So
        // this side has a clock: a line advances the circuit to the instant
        // the guest names, and silence advances it by the slice.
        let mut watched: Option<std::sync::mpsc::Sender<String>> = None;
        if let Some(mut board) = live {
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            watched = Some(tx);
            let back = handle.clone();
            std::thread::spawn(move || {
                // Half a millisecond: shorter than any interval a firmware
                // polls a converter on, so the host is never what limits the
                // shape the firmware can see.
                const SLICE: Duration = Duration::from_micros(500);
                loop {
                    let moved = match rx.recv_timeout(SLICE) {
                        Ok(line) => board.absorb(&line),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) if board.settling() => {
                            board.advance_by(SLICE.as_secs_f64())
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(Vec::new()),
                        // The reader is gone, so the run is over.
                        Err(_) => return,
                    };
                    match moved {
                        Ok(counts) => {
                            for (pin, count) in counts {
                                back.analog(u32::from(pin), count);
                            }
                        }
                        // A circuit that stops having an answer stops
                        // answering, and says so once rather than every
                        // line: the run is still worth watching, and a
                        // failure repeated at the emulator's rate is a log
                        // nobody can read.
                        Err(trouble) => {
                            eprintln!("the sheet's circuit could not be solved: {trouble}");
                            return;
                        }
                    }
                }
            });
        }

        let lines = BufReader::new(reader)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.is_empty());
        for text in lines {
            if let Some(report) = protocol::parse_i2c_report(&text) {
                handle.answer(&report);
            }
            if let Some(tx) = watched.as_ref()
                && tx.send(text.clone()).is_err()
            {
                watched = None;
            }
            on_line(text);
        }
    });

    channel
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The board file's polarity is what turns "pressed" into a level. A
    /// pull-up button pressed is *low*; before this every press drove high,
    /// and firmware reading `is_low()` saw a release.
    #[test]
    fn a_pressed_button_drives_the_level_its_wiring_means() {
        assert_eq!(pin_level(1, false), 1, "to 3V3: pressed is high");
        assert_eq!(pin_level(0, false), 0);
        assert_eq!(
            pin_level(1, true),
            0,
            "to ground with a pull-up: pressed is low"
        );
        assert_eq!(pin_level(0, true), 1, "and released rests high");
    }

    /// A fake emulator on a real socket: whatever the channel writes is
    /// heard, and whatever the test hands it is said back. `None` ends it
    /// once the channel has gone quiet.
    type Emulator = (
        u16,
        std::thread::JoinHandle<Vec<String>>,
        std::sync::mpsc::Sender<Option<String>>,
    );

    fn emulator() -> Emulator {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (say, said) = std::sync::mpsc::channel::<Option<String>>();
        let heard = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let mut writer = socket.try_clone().unwrap();
            let mut reader = BufReader::new(socket);
            let (mut heard, mut done) = (Vec::new(), false);
            loop {
                while let Ok(next) = said.try_recv() {
                    match next {
                        Some(line) => writer.write_all(line.as_bytes()).unwrap(),
                        None => done = true,
                    }
                }
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => heard.push(line.trim_end().to_string()),
                    Err(_) if done => break,
                    Err(_) => {}
                }
            }
            heard
        });
        (port, heard, say)
    }

    /// A sensor on the sheet is declared with its registers, a slider move
    /// writes the new reading, and a range the firmware chose is answered
    /// with the readings re-encoded — all on one channel, in that order.
    #[test]
    fn a_sensor_is_declared_moved_and_rescaled_on_the_channel() {
        let mut props = std::collections::BTreeMap::new();
        props.insert("az".to_string(), "1".to_string());
        let imu = crate::partfile::load(None)
            .specs
            .into_iter()
            .find(|spec| spec.id == "mpu6050")
            .expect("the built-in library carries an MPU-6050");
        let device = sensor::Device::new(imu, &props);
        let start = Start {
            bus: vec![BusDevice {
                part: "U2".into(),
                address: 0x68,
                regs: device.registers(),
            }],
            sensors: vec![Sensor {
                part: "U2".into(),
                address: 0x68,
                device,
                signals: Vec::new(),
                rate: 0,
            }],
            ..Start::default()
        };

        let (port, heard, say) = emulator();
        let (seen_tx, seen) = std::sync::mpsc::channel();
        let channel = connect(port, start, None, move |line| {
            let _ = seen_tx.send(line);
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(channel.set_sensor("U2", "ax", 0.5));
        assert!(!channel.set_sensor("U2", "pressure", 1.0), "not an IMU's");
        assert!(!channel.set_sensor("U9", "ax", 0.5), "no such part");
        // The firmware selects ±8 g.
        say.send(Some("[rusty:i2c@10] 68 w 1c10\n".to_string()))
            .unwrap();
        let reported = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            reported, "[rusty:i2c@10] 68 w 1c10",
            "the report still reaches the reader"
        );
        say.send(None).unwrap();

        let heard = heard.join().unwrap();
        assert_eq!(
            heard.first().map(String::as_str),
            Some("i2c 68=+"),
            "{heard:?}"
        );
        assert!(heard.iter().any(|l| l == "i2c 68:75=68"), "{heard:?}");
        // 0.5 g at ±2 g, then at ±8 g.
        assert!(
            heard.iter().any(|l| l.starts_with("i2c 68:3b=2000")),
            "{heard:?}"
        );
        assert!(
            heard.iter().any(|l| l.starts_with("i2c 68:3b=0800")),
            "{heard:?}"
        );
    }

    /// A reading a signal moves plays as a table from the first
    /// instruction — the whole data block, a sample at a time — and the
    /// firmware choosing another range sends the table again, re-encoded:
    /// the block is latched from the table at every read, so a register
    /// rewritten on its own would be gone by the next one.
    /// A signal changed while the run goes on is rendered through the same
    /// circuit and played in step with what it replaces; one that does not
    /// read changes nothing and says why, and a part that is no generator
    /// is refused rather than quietly given a property.
    #[test]
    fn a_signal_changed_mid_run_plays_through_the_same_circuit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".rusty")).unwrap();
        std::fs::write(
            dir.path().join(".rusty/sim.toml"),
            "version = 2\n[board]\nchip = \"esp32c3\"\n\n\
             [[part]]\nref = \"V1\"\nsymbol = \"rusty:SignalGen\"\nx = 0.0\ny = 0.0\n\
             props = { signal = \"dc 1.65; sine f=50 a=1\", fullscale = \"3.3\" }\n\n\
             [[part]]\nref = \"R1\"\nsymbol = \"Device:R\"\nvalue = \"10k\"\nx = 0.0\ny = 0.0\n\n\
             [[wire]]\nfrom = \"V1.1\"\nto = \"U1.GPIO3\"\n\n\
             [[wire]]\nfrom = \"V1.2\"\nto = \"U1.GND\"\n\n\
             [[wire]]\nfrom = \"R1.1\"\nto = \"U1.GPIO4\"\n\n\
             [[wire]]\nfrom = \"R1.2\"\nto = \"U1.GND\"\n",
        )
        .unwrap();
        let sheet = crate::simulate::load_board_for_test(dir.path(), "esp32c3").unwrap();
        let rows = nets::kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5]);
        let start = start_of(&sheet, &rows, &[]);
        assert!(
            start
                .said
                .iter()
                .any(|line| line.starts_with("[rusty:signal] V1 play on GPIO3")),
            "{:?}",
            start.said
        );
        assert_eq!(start.tables.len(), 1);

        let (port, heard, say) = emulator();
        let channel = connect(port, start, None, |_| {});
        std::thread::sleep(Duration::from_millis(300));

        let said = channel
            .set_signal("V1", "signal", Some("dc 0.825"))
            .expect("a signal that reads");
        assert!(said.iter().any(|line| line.contains("GPIO3")), "{said:?}");
        let refused = channel
            .set_signal("V1", "signal", Some("sine fq=5 a=1"))
            .unwrap_err();
        assert!(refused.contains("fq"), "{refused}");
        let refused = channel
            .set_signal("R1", "signal", Some("dc 1"))
            .unwrap_err();
        assert!(refused.contains("not a signal generator"), "{refused}");
        std::thread::sleep(Duration::from_millis(200));
        say.send(None).unwrap();

        let heard = heard.join().unwrap();
        // Played at the start, a second of a 50 Hz tone at 20 kHz, and then
        // again as one sample at a quarter of the full scale — and nothing
        // else, since neither refusal said anything to the emulator.
        let begun: Vec<&String> = heard.iter().filter(|l| l.starts_with("W3=")).collect();
        assert_eq!(
            begun,
            ["W3=20000,20000", "W3=on", "W3=20000,1", "W3=on"],
            "{heard:?}"
        );
        assert!(heard.contains(&"W3@0=0400".to_string()), "{heard:?}");
    }

    #[test]
    fn a_sensor_moved_by_a_signal_plays_a_table_and_plays_it_again_rescaled() {
        let imu = crate::partfile::load(None)
            .specs
            .into_iter()
            .find(|spec| spec.id == "mpu6050")
            .expect("the built-in library carries an MPU-6050");
        let device = sensor::Device::new(imu, &std::collections::BTreeMap::new());
        let sensor = Sensor {
            part: "U2".into(),
            address: 0x68,
            device: device.clone(),
            signals: vec![("gz".to_string(), vec![0.0, 100.0])],
            rate: 1000,
        };
        let table = sensor.table().expect("a reading moves");
        let start = Start {
            bus: vec![BusDevice {
                part: "U2".into(),
                address: 0x68,
                regs: device.registers(),
            }],
            sensors: vec![sensor],
            tables: vec![table],
            ..Start::default()
        };

        let (port, heard, say) = emulator();
        let (seen_tx, seen) = std::sync::mpsc::channel();
        let _channel = connect(port, start, None, move |line| {
            let _ = seen_tx.send(line);
        });
        std::thread::sleep(Duration::from_millis(300));
        // The firmware selects ±2000 °/s.
        say.send(Some("[rusty:i2c@10] 68 w 1b18\n".to_string()))
            .unwrap();
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        say.send(None).unwrap();

        let heard = heard.join().unwrap();
        let begun: Vec<usize> = heard
            .iter()
            .enumerate()
            .filter(|(_, line)| line.as_str() == "i2c 68~3b:14=1000,2")
            .map(|(at, _)| at)
            .collect();
        assert_eq!(begun.len(), 2, "played at the start and again: {heard:?}");
        assert!(
            heard.iter().filter(|l| l.as_str() == "i2c 68~=on").count() == 2,
            "{heard:?}"
        );
        // Sample 1's gz, 100 °/s, at ±250 °/s and then at ±2000 °/s: the
        // block's last two bytes of the second sample, 14 bytes in.
        let gz_of = |from: usize| -> i16 {
            let hex = heard[from + 1].split_once('=').unwrap().1;
            let at = (14 + 12) * 2;
            i16::from_str_radix(&hex[at..at + 4], 16).unwrap()
        };
        assert_eq!(gz_of(begun[0]), 13100, "100 °/s at 131 counts a degree");
        assert_eq!(
            gz_of(begun[1]),
            1640,
            "and at 16.4, once the firmware chose"
        );
    }
}
