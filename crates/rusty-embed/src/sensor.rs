//! Sensors that answer on the I2C bus the way the part does.
//!
//! A sheet's I2C part carries `regs`: bytes it answers with, fixed for the
//! run. That is enough for a bus scan and a `WHO_AM_I`, and no use for the
//! thing a sensor is for, a reading that moves. Firmware using an ordinary
//! driver crate (`mpu6050`, `bme280`) reads registers and converts them with
//! the part's own arithmetic, so the only way to show it a tilted board or a
//! warmer room is to put the right bytes in those registers: scaled for the
//! range the firmware chose, and calibrated the way the part's own memory
//! says it is.
//!
//! This module is that arithmetic run backwards. A [`Model`] names the part,
//! its [`Channel`]s are the quantities in the units a person thinks in (g,
//! °/s, °C, hPa, %), and a [`Device`] turns values into register runs. Pure,
//! so the backend encodes a run's first readings and every later slider move
//! with the same code the tests hold to the datasheets' own formulas going
//! forwards.
//!
//! What it does not model is left out rather than approximated: no FIFO, no
//! DMP, no interrupts, no self-test and no timing. A measurement is always
//! ready, which is the one thing every driver waits for.

use std::collections::BTreeMap;

/// A part rusty can answer for, register by register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Model {
    /// InvenSense's six-axis IMU: accelerometer, gyroscope, temperature.
    Mpu6050,
    /// Bosch's barometer: temperature and pressure.
    Bmp280,
    /// The BMP280 with a humidity sensor beside it.
    Bme280,
}

/// One quantity a model reports, in the unit people read it in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Channel {
    /// The prop the sheet stores it under, and the slider's name: `ax`,
    /// `temp`, `pressure`.
    pub key: &'static str,
    pub unit: &'static str,
    pub min: f64,
    pub max: f64,
    /// Where a part placed on a sheet starts: level, still, a room at 24 °C
    /// at sea level.
    pub rest: f64,
}

const fn channel(key: &'static str, unit: &'static str, min: f64, max: f64, rest: f64) -> Channel {
    Channel {
        key,
        unit,
        min,
        max,
        rest,
    }
}

/// The slider ranges are wider than the part's power-on full scale on
/// purpose: a reading past the range the firmware configured saturates, as
/// the part does, and that is worth being able to show.
const MPU6050_CHANNELS: &[Channel] = &[
    channel("ax", "g", -4.0, 4.0, 0.0),
    channel("ay", "g", -4.0, 4.0, 0.0),
    channel("az", "g", -4.0, 4.0, 1.0),
    channel("gx", "°/s", -500.0, 500.0, 0.0),
    channel("gy", "°/s", -500.0, 500.0, 0.0),
    channel("gz", "°/s", -500.0, 500.0, 0.0),
    channel("temp", "°C", -40.0, 85.0, 24.0),
];

const BMP280_CHANNELS: &[Channel] = &[
    channel("temp", "°C", -40.0, 85.0, 24.0),
    channel("pressure", "hPa", 300.0, 1100.0, 1013.25),
];

const BME280_CHANNELS: &[Channel] = &[
    channel("temp", "°C", -40.0, 85.0, 24.0),
    channel("pressure", "hPa", 300.0, 1100.0, 1013.25),
    channel("humidity", "%", 0.0, 100.0, 50.0),
];

impl Model {
    pub const ALL: [Model; 3] = [Model::Mpu6050, Model::Bmp280, Model::Bme280];

    /// How a sheet names it: the `model` prop.
    pub fn id(self) -> &'static str {
        match self {
            Model::Mpu6050 => "mpu6050",
            Model::Bmp280 => "bmp280",
            Model::Bme280 => "bme280",
        }
    }

    /// How its datasheet names it.
    pub fn name(self) -> &'static str {
        match self {
            Model::Mpu6050 => "MPU-6050",
            Model::Bmp280 => "BMP280",
            Model::Bme280 => "BME280",
        }
    }

    /// The model a sheet names: `mpu6050`, `MPU-6050`, `bme_280`. Case,
    /// hyphens and underscores do not count, and nothing else is guessed at:
    /// a name that is not one of these is not a model.
    pub fn from_id(text: &str) -> Option<Model> {
        let folded: String = text
            .chars()
            .filter(|c| !matches!(c, '-' | '_' | ' '))
            .map(|c| c.to_ascii_lowercase())
            .collect();
        Model::ALL.into_iter().find(|model| model.id() == folded)
    }

    /// The addresses the part can answer on. The first is the one it answers
    /// on with its address pin low, which is how a breakout ships.
    pub fn addresses(self) -> &'static [u8] {
        match self {
            Model::Mpu6050 => &[0x68, 0x69],
            Model::Bmp280 | Model::Bme280 => &[0x76, 0x77],
        }
    }

    pub fn channels(self) -> &'static [Channel] {
        match self {
            Model::Mpu6050 => MPU6050_CHANNELS,
            Model::Bmp280 => BMP280_CHANNELS,
            Model::Bme280 => BME280_CHANNELS,
        }
    }
}

/// One sensor on the bus: what it is reading, and what the firmware has
/// configured that changes how the reading is encoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    model: Model,
    /// One per channel, in the model's channel order.
    values: Vec<f64>,
    /// The MPU-6050's full-scale selections as the firmware last wrote them:
    /// `AFS_SEL` and `FS_SEL`, 0 to 3. The same tilt is a different number
    /// at ±2 g than at ±8 g, and a driver that set ±8 g divides by 4096.
    accel_range: u8,
    gyro_range: u8,
}

impl Device {
    /// A sensor reading what the sheet's props say, and resting where a prop
    /// is absent or is not a number. Values are held to the channel's range.
    pub fn new(model: Model, props: &BTreeMap<String, String>) -> Self {
        let values = model
            .channels()
            .iter()
            .map(|channel| {
                props
                    .get(channel.key)
                    .and_then(|text| text.trim().parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .map_or(channel.rest, |value| value.clamp(channel.min, channel.max))
            })
            .collect();
        Device {
            model,
            values,
            accel_range: 0,
            gyro_range: 0,
        }
    }

    pub fn model(&self) -> Model {
        self.model
    }

    pub fn value(&self, key: &str) -> Option<f64> {
        let index = self.index_of(key)?;
        self.values.get(index).copied()
    }

    /// Move one reading. False when the key is not one of this model's, the
    /// value is not a number, or nothing changed, so a caller writes
    /// registers only when there is something new in them.
    pub fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(index) = self.index_of(key) else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        let channel = self.model.channels()[index];
        let value = value.clamp(channel.min, channel.max);
        if self.values[index] == value {
            return false;
        }
        self.values[index] = value;
        true
    }

    fn index_of(&self, key: &str) -> Option<usize> {
        self.model.channels().iter().position(|c| c.key == key)
    }

    fn get(&self, key: &str) -> f64 {
        self.value(key).unwrap_or_default()
    }

    /// Every register this device answers with, as runs from a start
    /// register: who it is, how it is calibrated, and what it reads. What a
    /// run declares before the firmware's first transaction.
    pub fn registers(&self) -> Vec<(u8, Vec<u8>)> {
        let mut runs = match self.model {
            // PWR_MGMT_1 powers on asleep, and WHO_AM_I is what every driver
            // checks first.
            Model::Mpu6050 => vec![(0x6b, vec![0x40]), (0x75, vec![0x68])],
            Model::Bmp280 | Model::Bme280 => {
                let bme = self.model == Model::Bme280;
                let mut runs = vec![
                    (0x88, bosch::calibration()),
                    (0xd0, vec![if bme { 0x60 } else { 0x58 }]),
                    // STATUS: not measuring, nothing being copied from NVM.
                    (0xf3, vec![0x00]),
                ];
                if bme {
                    runs.push((0xa1, vec![bosch::H1]));
                    runs.push((0xe1, bosch::humidity_calibration()));
                }
                runs
            }
        };
        runs.extend(self.data());
        runs
    }

    /// Only the registers that change when a reading does: what a slider
    /// moving in a running simulation writes.
    pub fn data(&self) -> Vec<(u8, Vec<u8>)> {
        match self.model {
            Model::Mpu6050 => vec![(0x3b, self.imu_data())],
            Model::Bmp280 | Model::Bme280 => vec![(0xf7, self.climate_data())],
        }
    }

    /// A write the firmware made, as the bus reported it: the register
    /// pointer, then the bytes stored from there. Answers with what the part
    /// itself would change in reply, which is the registers to write back:
    /// a reset bit that clears itself, a forced measurement that returns the
    /// part to sleep, or every reading re-encoded for a range the firmware
    /// has just chosen.
    ///
    /// A one-byte write only moves the pointer, and changes nothing.
    pub fn wrote(&mut self, bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let Some((&start, stored)) = bytes.split_first() else {
            return Vec::new();
        };
        let mut back = Vec::new();
        let mut rescaled = false;
        for (offset, &value) in stored.iter().enumerate() {
            let register = start.wrapping_add(offset as u8);
            match (self.model, register) {
                (Model::Mpu6050, 0x1b) => {
                    let range = (value >> 3) & 0x03;
                    rescaled |= range != self.gyro_range;
                    self.gyro_range = range;
                }
                (Model::Mpu6050, 0x1c) => {
                    let range = (value >> 3) & 0x03;
                    rescaled |= range != self.accel_range;
                    self.accel_range = range;
                }
                // DEVICE_RESET: the configuration returns to its power-on
                // values, the part goes back to sleep and the bit clears
                // itself. A driver polling for that clear would otherwise
                // wait for ever on a register that only stores.
                (Model::Mpu6050, 0x6b) if value & 0x80 != 0 => {
                    self.accel_range = 0;
                    self.gyro_range = 0;
                    back.push((0x1b, vec![0x00]));
                    back.push((0x1c, vec![0x00]));
                    back.push((0x6b, vec![0x40]));
                    rescaled = true;
                }
                // Soft reset: the control registers return to zero and the
                // reset register reads as zero, as the part's do.
                (Model::Bmp280 | Model::Bme280, 0xe0) if value == 0xb6 => {
                    back.push((0xe0, vec![0x00]));
                    if self.model == Model::Bme280 {
                        back.push((0xf2, vec![0x00]));
                    }
                    back.push((0xf4, vec![0x00]));
                    back.push((0xf5, vec![0x00]));
                }
                // Forced mode: one measurement, then back to sleep. The
                // measurement is already there, so the part is asleep again
                // as soon as anybody looks.
                (Model::Bmp280 | Model::Bme280, 0xf4) if matches!(value & 0x03, 0x01 | 0x02) => {
                    back.push((0xf4, vec![value & !0x03]));
                }
                _ => {}
            }
        }
        if rescaled {
            back.extend(self.data());
        }
        back
    }

    /// `ACCEL_XOUT_H` through `GYRO_ZOUT_L`: fourteen bytes, big-endian,
    /// scaled for the ranges the firmware selected.
    fn imu_data(&self) -> Vec<u8> {
        const ACCEL_LSB_PER_G: [f64; 4] = [16384.0, 8192.0, 4096.0, 2048.0];
        const GYRO_LSB_PER_DPS: [f64; 4] = [131.0, 65.5, 32.8, 16.4];
        let accel = ACCEL_LSB_PER_G[usize::from(self.accel_range & 0x03)];
        let gyro = GYRO_LSB_PER_DPS[usize::from(self.gyro_range & 0x03)];
        let mut out = Vec::with_capacity(14);
        for key in ["ax", "ay", "az"] {
            out.extend(saturate(self.get(key) * accel).to_be_bytes());
        }
        // Temperature in °C is TEMP_OUT / 340 + 36.53, from the register map.
        out.extend(saturate((self.get("temp") - 36.53) * 340.0).to_be_bytes());
        for key in ["gx", "gy", "gz"] {
            out.extend(saturate(self.get(key) * gyro).to_be_bytes());
        }
        out
    }

    /// `press_msb` through `temp_xlsb`, and `hum_msb`/`hum_lsb` on a BME280:
    /// the raw converter outputs that the calibration in [`bosch`] turns back
    /// into the readings.
    fn climate_data(&self) -> Vec<u8> {
        let raw_t = bosch::raw_temperature(self.get("temp"));
        let fine = bosch::t_fine(raw_t);
        let raw_p = bosch::raw_pressure(self.get("pressure") * 100.0, fine);
        let mut out = Vec::with_capacity(8);
        out.extend(twenty_bits(raw_p));
        out.extend(twenty_bits(raw_t));
        if self.model == Model::Bme280 {
            let raw_h = bosch::raw_humidity(self.get("humidity"), fine);
            out.extend((raw_h as u16).to_be_bytes());
        }
        out
    }
}

/// A reading as the part's signed 16-bit register holds it, clipped where
/// the part's converter clips.
fn saturate(value: f64) -> i16 {
    value
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

/// A 20-bit converter output as Bosch lays it out: `msb`, `lsb`, and the low
/// four bits in the top of `xlsb`.
fn twenty_bits(raw: i32) -> [u8; 3] {
    [
        ((raw >> 12) & 0xff) as u8,
        ((raw >> 4) & 0xff) as u8,
        ((raw & 0x0f) << 4) as u8,
    ]
}

/// The BMP280 and BME280's compensation, from Bosch's datasheets, and its
/// inverse.
///
/// The calibration is the BMP280 datasheet's own worked example, so the
/// forward formulas can be checked against the numbers printed beside it;
/// the humidity trimming is a real BME280's. A driver reads these from the
/// part and cannot tell them from any other chip's.
pub(crate) mod bosch {
    pub const T1: u16 = 27504;
    pub const T2: i16 = 26435;
    pub const T3: i16 = -1000;
    pub const P1: u16 = 36477;
    pub const P2: i16 = -10685;
    pub const P3: i16 = 3024;
    pub const P4: i16 = 2855;
    pub const P5: i16 = 140;
    pub const P6: i16 = -7;
    pub const P7: i16 = 15500;
    pub const P8: i16 = -14600;
    pub const P9: i16 = 6000;
    pub const H1: u8 = 75;
    pub const H2: i16 = 362;
    pub const H3: u8 = 0;
    pub const H4: i16 = 313;
    pub const H5: i16 = 50;
    pub const H6: i8 = 30;

    /// `dig_T1` through `dig_P9`, little-endian, from `0x88`.
    pub fn calibration() -> Vec<u8> {
        let mut out = Vec::with_capacity(24);
        out.extend(T1.to_le_bytes());
        out.extend(T2.to_le_bytes());
        out.extend(T3.to_le_bytes());
        out.extend(P1.to_le_bytes());
        for word in [P2, P3, P4, P5, P6, P7, P8, P9] {
            out.extend(word.to_le_bytes());
        }
        out
    }

    /// `dig_H2` through `dig_H6`, from `0xE1`, with `H4` and `H5` sharing a
    /// byte the way the part packs them.
    pub fn humidity_calibration() -> Vec<u8> {
        let [h2_low, h2_high] = H2.to_le_bytes();
        vec![
            h2_low,
            h2_high,
            H3,
            (H4 >> 4) as u8,
            ((H4 & 0x0f) | ((H5 & 0x0f) << 4)) as u8,
            (H5 >> 4) as u8,
            H6 as u8,
        ]
    }

    // The datasheets write these in 32-bit arithmetic that sits within a
    // few percent of overflowing at the ends of the converter's range; they
    // are computed in 64 bits here, which gives the same answers wherever
    // the 32-bit forms do not overflow, and cannot panic a debug build at
    // the ends of a search.

    /// The temperature compensation's shared intermediate, which pressure
    /// and humidity both take as an input.
    pub fn t_fine(raw: i32) -> i32 {
        let (raw, t1) = (i64::from(raw), i64::from(T1));
        let var1 = (((raw >> 3) - (t1 << 1)) * i64::from(T2)) >> 11;
        let var2 = (((((raw >> 4) - t1) * ((raw >> 4) - t1)) >> 12) * i64::from(T3)) >> 14;
        (var1 + var2) as i32
    }

    /// Hundredths of a degree.
    pub fn temperature(t_fine: i32) -> i32 {
        ((i64::from(t_fine) * 5 + 128) >> 8) as i32
    }

    /// Pascals in Q24.8: divide by 256.
    pub fn pressure(raw: i32, t_fine: i32) -> u32 {
        let mut var1 = i64::from(t_fine) - 128_000;
        let mut var2 = var1 * var1 * i64::from(P6);
        var2 += (var1 * i64::from(P5)) << 17;
        var2 += i64::from(P4) << 35;
        var1 = ((var1 * var1 * i64::from(P3)) >> 8) + ((var1 * i64::from(P2)) << 12);
        var1 = (((1_i64 << 47) + var1) * i64::from(P1)) >> 33;
        if var1 == 0 {
            return 0;
        }
        let mut p = 1_048_576 - i64::from(raw);
        p = (((p << 31) - var2) * 3125) / var1;
        let var1 = (i64::from(P9) * (p >> 13) * (p >> 13)) >> 25;
        let var2 = (i64::from(P8) * p) >> 19;
        (((p + var1 + var2) >> 8) + (i64::from(P7) << 4)) as u32
    }

    /// Percent relative humidity in Q22.10: divide by 1024.
    pub fn humidity(raw: i32, t_fine: i32) -> u32 {
        let x = i64::from(t_fine) - 76_800;
        let offset =
            ((i64::from(raw) << 14) - (i64::from(H4) << 20) - (i64::from(H5) * x) + 16_384) >> 15;
        let gain = ((((((x * i64::from(H6)) >> 10) * (((x * i64::from(H3)) >> 11) + 32_768))
            >> 10)
            + 2_097_152)
            * i64::from(H2)
            + 8_192)
            >> 14;
        let mut v = offset * gain;
        v -= ((((v >> 15) * (v >> 15)) >> 7) * i64::from(H1)) >> 4;
        (v.clamp(0, 419_430_400) >> 12) as u32
    }

    /// The smallest input the rising `forward` maps at or past `target`,
    /// or its neighbour below when that one lands nearer — a converter
    /// reading as close to the value as the part can report.
    fn nearest(low: i32, high: i32, target: i64, forward: impl Fn(i32) -> i64) -> i32 {
        let (mut lo, mut hi) = (low, high);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if forward(mid) < target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo > low && (target - forward(lo - 1)).abs() < (forward(lo) - target).abs() {
            lo - 1
        } else {
            lo
        }
    }

    /// The converter output that compensates to `celsius`.
    pub fn raw_temperature(celsius: f64) -> i32 {
        let target = (celsius * 100.0).round() as i64;
        nearest(0, 0xf_ffff, target, |raw| {
            i64::from(temperature(t_fine(raw)))
        })
    }

    /// The converter output that compensates to `pascals` at this
    /// temperature. Pressure falls as the raw value rises, so the search
    /// runs on its negation.
    pub fn raw_pressure(pascals: f64, t_fine: i32) -> i32 {
        let target = (pascals * 256.0).round() as i64;
        nearest(0, 0xf_ffff, -target, |raw| {
            -i64::from(pressure(raw, t_fine))
        })
    }

    /// The converter output that compensates to `percent` at this
    /// temperature.
    pub fn raw_humidity(percent: f64, t_fine: i32) -> i32 {
        let target = (percent * 1024.0).round() as i64;
        nearest(0, 0xffff, target, |raw| i64::from(humidity(raw, t_fine)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The register runs as the bus model would hold them after a run
    /// declared them: 256 bytes, zero where nothing was said.
    fn file(runs: &[(u8, Vec<u8>)]) -> [u8; 256] {
        let mut regs = [0u8; 256];
        for (start, bytes) in runs {
            for (offset, byte) in bytes.iter().enumerate() {
                regs[usize::from(start.wrapping_add(offset as u8))] = *byte;
            }
        }
        regs
    }

    fn be(regs: &[u8; 256], at: usize) -> i16 {
        i16::from_be_bytes([regs[at], regs[at + 1]])
    }

    #[test]
    fn a_model_is_read_by_its_name_and_nothing_else() {
        assert_eq!(Model::from_id("mpu6050"), Some(Model::Mpu6050));
        assert_eq!(Model::from_id("MPU-6050"), Some(Model::Mpu6050));
        assert_eq!(Model::from_id(" bme_280 "), Some(Model::Bme280));
        assert_eq!(Model::from_id("bmp280"), Some(Model::Bmp280));
        assert_eq!(Model::from_id("mpu6500"), None, "a cousin is not the part");
        assert_eq!(Model::from_id(""), None);
        for model in Model::ALL {
            assert_eq!(Model::from_id(model.id()), Some(model));
            assert_eq!(Model::from_id(model.name()), Some(model));
            assert!(!model.addresses().is_empty());
        }
    }

    /// What the `mpu6050` crate does with the registers, done here: a level
    /// board reads 1 g down and nothing else, at the power-on ranges.
    #[test]
    fn an_imu_at_rest_reads_one_g_down_and_the_room_temperature() {
        let device = Device::new(Model::Mpu6050, &BTreeMap::new());
        let regs = file(&device.registers());
        assert_eq!(regs[0x75], 0x68, "WHO_AM_I");
        assert_eq!(regs[0x6b], 0x40, "powers on asleep");
        assert_eq!(be(&regs, 0x3b), 0);
        assert_eq!(be(&regs, 0x3d), 0);
        assert_eq!(be(&regs, 0x3f), 16384, "1 g at ±2 g full scale");
        let celsius = f64::from(be(&regs, 0x41)) / 340.0 + 36.53;
        assert!((celsius - 24.0).abs() < 0.01, "{celsius}");
        assert_eq!(be(&regs, 0x43), 0);
    }

    /// The same tilt is a different number once the firmware chooses
    /// another range, and a driver that set ±8 g divides by 4096 — so the
    /// write that sets the range is answered with the readings re-encoded.
    #[test]
    fn a_range_the_firmware_chooses_rescales_what_it_reads() {
        let mut device = Device::new(Model::Mpu6050, &props(&[("ax", "0.5"), ("gz", "-100")]));
        let regs = file(&device.registers());
        assert_eq!(be(&regs, 0x3b), 8192, "0.5 g at ±2 g");
        assert_eq!(be(&regs, 0x47), -13100, "-100 °/s at ±250 °/s");

        // ACCEL_CONFIG = ±8 g, GYRO_CONFIG = ±1000 °/s, in one burst write
        // starting at GYRO_CONFIG.
        let back = device.wrote(&[0x1b, 0x10, 0x10]);
        let regs = file(&back);
        assert_eq!(be(&regs, 0x3b), 2048, "0.5 g at ±8 g");
        assert_eq!(be(&regs, 0x47), -3280, "-100 °/s at ±1000 °/s");

        // The same ranges again change nothing and write nothing.
        assert!(device.wrote(&[0x1b, 0x10, 0x10]).is_empty());
        // A pointer on its own is not a write.
        assert!(device.wrote(&[0x3b]).is_empty());
    }

    /// Past the configured range a reading clips, as the part's does.
    #[test]
    fn a_reading_past_the_range_saturates() {
        let device = Device::new(Model::Mpu6050, &props(&[("ax", "3"), ("gx", "-400")]));
        let regs = file(&device.registers());
        assert_eq!(be(&regs, 0x3b), i16::MAX, "3 g at ±2 g");
        assert_eq!(be(&regs, 0x43), i16::MIN, "-400 °/s at ±250 °/s");
    }

    /// A driver that resets the part polls DEVICE_RESET until it clears.
    /// The register file only stores, so the clear has to be written back.
    #[test]
    fn a_reset_clears_its_own_bit_and_the_ranges() {
        let mut device = Device::new(Model::Mpu6050, &props(&[("az", "1")]));
        device.wrote(&[0x1c, 0x18]);
        let back = file(&device.wrote(&[0x6b, 0x80]));
        assert_eq!(back[0x6b], 0x40, "reset done, asleep");
        assert_eq!(back[0x1c], 0x00);
        assert_eq!(be(&back, 0x3f), 16384, "back at ±2 g");
    }

    #[test]
    fn a_slider_moves_only_what_it_names() {
        let mut device = Device::new(Model::Mpu6050, &BTreeMap::new());
        assert!(device.set("ay", -0.25));
        assert!(!device.set("ay", -0.25), "unchanged is not a change");
        assert!(!device.set("pressure", 1000.0), "not this model's channel");
        assert!(!device.set("ay", f64::NAN));
        assert!(device.set("ay", 99.0), "clamped into range, still a move");
        assert_eq!(device.value("ay"), Some(4.0));
        let data = file(&device.data());
        assert_eq!(be(&data, 0x3d), i16::MAX);
    }

    /// The BMP280 datasheet's worked example: these raw values and this
    /// calibration give 25.08 °C and 100653 Pa. The forward formulas are the
    /// yardstick every encoding below is held to, so they are held to the
    /// datasheet first.
    #[test]
    fn the_compensation_agrees_with_the_datasheets_worked_example() {
        let fine = bosch::t_fine(519_888);
        assert_eq!(fine, 128_422);
        assert_eq!(bosch::temperature(fine), 2508);
        let pascals = f64::from(bosch::pressure(415_148, fine)) / 256.0;
        assert!((pascals - 100_653.27).abs() < 1.0, "{pascals}");
    }

    /// Decoded the way a driver decodes them, the registers carry what the
    /// sliders say — to the part's own resolution.
    #[test]
    fn a_climate_sensor_reads_back_what_it_was_set_to() {
        let device = Device::new(
            Model::Bme280,
            &props(&[
                ("temp", "31.5"),
                ("pressure", "987.6"),
                ("humidity", "63.2"),
            ]),
        );
        let regs = file(&device.registers());
        assert_eq!(regs[0xd0], 0x60, "a BME280's chip id");
        let raw20 = |at: usize| {
            (i32::from(regs[at]) << 12)
                | (i32::from(regs[at + 1]) << 4)
                | (i32::from(regs[at + 2]) >> 4)
        };
        let fine = bosch::t_fine(raw20(0xfa));
        let celsius = f64::from(bosch::temperature(fine)) / 100.0;
        assert!((celsius - 31.5).abs() <= 0.01, "{celsius}");
        let hpa = f64::from(bosch::pressure(raw20(0xf7), fine)) / 25_600.0;
        assert!((hpa - 987.6).abs() < 0.01, "{hpa}");
        let raw_h = i32::from(u16::from_be_bytes([regs[0xfd], regs[0xfe]]));
        let percent = f64::from(bosch::humidity(raw_h, fine)) / 1024.0;
        assert!((percent - 63.2).abs() < 0.1, "{percent}");

        // And the calibration a driver reads is the calibration used.
        assert_eq!(u16::from_le_bytes([regs[0x88], regs[0x89]]), bosch::T1);
        assert_eq!(u16::from_le_bytes([regs[0x8e], regs[0x8f]]), bosch::P1);
        assert_eq!(regs[0xa1], bosch::H1);
        let h4 = (i16::from(regs[0xe4] as i8) << 4) | i16::from(regs[0xe5] & 0x0f);
        let h5 = (i16::from(regs[0xe6] as i8) << 4) | i16::from(regs[0xe5] >> 4);
        assert_eq!((h4, h5), (bosch::H4, bosch::H5));
    }

    /// A driver computing in floating point, as several do, gets the same
    /// answer to within what the part itself resolves.
    #[test]
    fn a_floating_point_driver_reads_the_same_values() {
        let device = Device::new(
            Model::Bmp280,
            &props(&[("temp", "-12.3"), ("pressure", "850")]),
        );
        let regs = file(&device.registers());
        assert_eq!(regs[0xd0], 0x58, "a BMP280's chip id");
        let raw20 = |at: usize| {
            f64::from(
                (i32::from(regs[at]) << 12)
                    | (i32::from(regs[at + 1]) << 4)
                    | (i32::from(regs[at + 2]) >> 4),
            )
        };
        let (t1, t2, t3) = (
            f64::from(bosch::T1),
            f64::from(bosch::T2),
            f64::from(bosch::T3),
        );
        let adc_t = raw20(0xfa);
        let var1 = (adc_t / 16384.0 - t1 / 1024.0) * t2;
        let var2 = (adc_t / 131072.0 - t1 / 8192.0).powi(2) * t3;
        let fine = var1 + var2;
        let celsius = fine / 5120.0;
        assert!((celsius + 12.3).abs() < 0.02, "{celsius}");

        let p = |x: i16| f64::from(x);
        let mut var1 = fine / 2.0 - 64000.0;
        let mut var2 = var1 * var1 * p(bosch::P6) / 32768.0;
        var2 += var1 * p(bosch::P5) * 2.0;
        var2 = var2 / 4.0 + p(bosch::P4) * 65536.0;
        var1 = (p(bosch::P3) * var1 * var1 / 524288.0 + p(bosch::P2) * var1) / 524288.0;
        var1 = (1.0 + var1 / 32768.0) * f64::from(bosch::P1);
        let mut pascals = 1_048_576.0 - raw20(0xf7);
        pascals = (pascals - var2 / 4096.0) * 6250.0 / var1;
        let var1 = p(bosch::P9) * pascals * pascals / 2_147_483_648.0;
        let var2 = pascals * p(bosch::P8) / 32768.0;
        pascals += (var1 + var2 + p(bosch::P7)) / 16.0;
        assert!((pascals / 100.0 - 850.0).abs() < 0.05, "{pascals}");
    }

    /// Forced mode is one measurement and back to sleep; a soft reset reads
    /// back as zero. Both are what a register file that only stores would
    /// get wrong.
    #[test]
    fn a_forced_measurement_returns_the_part_to_sleep() {
        let mut device = Device::new(Model::Bme280, &BTreeMap::new());
        assert_eq!(device.wrote(&[0xf4, 0x25]), vec![(0xf4, vec![0x24])]);
        assert!(device.wrote(&[0xf4, 0x27]).is_empty(), "normal mode stays");
        let reset = file(&device.wrote(&[0xe0, 0xb6]));
        assert_eq!((reset[0xe0], reset[0xf2], reset[0xf4]), (0, 0, 0));
    }
}
