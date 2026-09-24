//! Parts that answer on the I2C bus the way the part does — declared, not
//! compiled in.
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
//! A [`Spec`] is that description, and it is **data**: which addresses the
//! part answers on, what it reads and in what units, where each reading
//! sits, how many counts a unit is worth, which register changes that, and
//! which bits the part clears once it has acted on them. `data/parts/*.toml`
//! holds the ones rusty ships; `<project>/.rusty/parts/` holds anybody
//! else's, and they are read the same way — the built-ins are not a
//! privileged path, which is the only way to be sure the declared one works.
//!
//! **What a declaration cannot express is named rather than approximated.**
//! Bosch's compensation is a polynomial over a calibration blob in the
//! part's own memory, and no `raw = (value - offset) * lsb` is going to be
//! it; a [`Quirk`] names that arithmetic, `sensor::bosch` is it, and a part
//! declared outside this crate may not name one, because a quirk is code.
//! Refuse rather than guess, applied to a file format: a linear stand-in for
//! a BME280 would read plausibly and be wrong by degrees.
//!
//! What no part here models is left out rather than approximated: no FIFO,
//! no DMP, no interrupts, no self-test and no timing. A measurement is
//! always ready, which is the one thing every driver waits for.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One quantity a part reports, in the unit people read it in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// The prop the sheet stores it under, and the slider's name: `ax`,
    /// `temp`, `pressure`.
    pub key: String,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    /// Where a part placed on a sheet starts: level, still, a room at 24 °C
    /// at sea level.
    pub rest: f64,
}

/// Which end of a multi-byte reading the part puts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Order {
    Big,
    Little,
}

/// Where a reading sits in the register file and what a count is worth:
/// `raw = round((value - offset) * lsb)`, held to what the width can hold,
/// because a part's converter clips rather than wrapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub key: String,
    pub at: u8,
    /// One to four bytes.
    pub width: u8,
    pub order: Order,
    pub signed: bool,
    /// Counts per unit — an MPU-6050 at ±2 g reads 16384 counts per g.
    pub lsb: f64,
    /// What a count of zero means, in the channel's unit. The MPU-6050's
    /// temperature is `raw / 340 + 36.53`, so its offset is 36.53.
    pub offset: f64,
}

/// A configuration register that changes what a count is worth.
///
/// The firmware writes a full-scale selection and every reading it covers
/// is encoded again — which is the whole reason a register file that only
/// stores is not enough: a driver that chose ±8 g divides by 4096, and
/// bytes encoded at ±2 g read as a quarter of the tilt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranged {
    pub at: u8,
    /// `index = (byte >> shift) & mask`.
    pub shift: u8,
    pub mask: u8,
    /// The channels this selection covers.
    pub keys: Vec<String>,
    /// One counts-per-unit per value of the field.
    pub lsbs: Vec<f64>,
}

/// A bit the part clears once it has acted on it.
///
/// `CTR.TRANS_START` on a bus, `MEASURING` on a converter, forced mode on a
/// Bosch part: the guest sets it, the hardware acts, and the driver reads it
/// back to find out that it has. A register file that only stores leaves a
/// driver polling a bit that can never fall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfClearing {
    pub at: u8,
    pub mask: u8,
    /// Only when the masked field holds one of these — empty means always.
    /// A BME280's forced mode is `01` or `10` and clears itself; `11` is
    /// normal mode and stays, and a mask alone could not tell them apart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<u8>,
}

/// A write that puts the part back the way it powered on.
///
/// Every range goes back to its first, because that is what a power-on
/// default is, and the registers named here go back to the bytes named
/// here — including the trigger's own, which is how a driver polling for
/// the reset to finish finds out that it has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reset {
    pub at: u8,
    /// Fires when any of these bits is written…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<u8>,
    /// …or when the byte is exactly this. Bosch's soft reset is the value
    /// `0xb6` and nothing else, where an MPU-6050's is one bit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<u8>,
    pub restores: Vec<Run>,
}

/// Bytes at a register: what a part answers with from `at` onwards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub at: u8,
    pub bytes: Vec<u8>,
}

/// Arithmetic no declaration can express, named so that a part needing it
/// is refused rather than approximated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quirk {
    /// Bosch's compensation polynomials over the calibration in the part's
    /// own memory, and their 20-bit packing — [`bosch`].
    Bmp280,
    /// The same with a humidity channel beside it.
    Bme280,
}

/// A part rusty answers for, register by register.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    /// How a sheet names it: the `model` prop, and the file's own stem.
    pub id: String,
    /// How its datasheet names it.
    pub name: String,
    /// The first is the one it answers on with its address pin low, which
    /// is how a breakout ships.
    pub addresses: Vec<u8>,
    pub channels: Vec<Channel>,
    /// Bytes fixed for the run: identity, and anything a driver checks
    /// before it will talk to the part at all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixed: Vec<Run>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readings: Vec<Reading>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<Ranged>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clears: Vec<SelfClearing>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resets: Vec<Reset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quirk: Option<Quirk>,
}

impl Spec {
    pub fn channel(&self, key: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.key == key)
    }

    /// The spec a sheet's `model` prop names. Case, hyphens, underscores
    /// and spaces do not count — `MPU-6050` and `mpu6050` are one part —
    /// and nothing else is guessed at: a name that matches no id and no
    /// datasheet name is not a part.
    pub fn find<'a>(specs: &'a [Spec], text: &str) -> Option<&'a Spec> {
        let wanted = fold(text);
        specs
            .iter()
            .find(|spec| fold(&spec.id) == wanted || fold(&spec.name) == wanted)
    }
}

/// A name with its punctuation and case taken out, which is how two
/// spellings of one part are recognised as one part.
fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

impl Reset {
    /// Whether this write is the one that resets the part.
    fn fires(&self, value: u8) -> bool {
        self.mask.is_some_and(|mask| value & mask != 0)
            || self.equals.is_some_and(|want| value == want)
    }
}

/// The widest register block the emulator plays as one table
/// (`ESP32_WAVE_MAX_WIDTH`): past it, a sample would not travel whole.
pub const MAX_BLOCK: usize = 64;

/// A part's data registers as one block, a sample after another: what the
/// emulator plays so that a burst read at any instant is one sample whole
/// (`docs/signals.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The first register of the block.
    pub reg: u8,
    /// Bytes per sample.
    pub width: usize,
    /// The samples, one after another.
    pub bytes: Vec<u8>,
}

/// One part on the bus: what it is reading, and what the firmware has
/// configured that changes how the reading is encoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    spec: Spec,
    /// One per channel, in the spec's channel order.
    values: Vec<f64>,
    /// One per range register, as the firmware last wrote it. The same tilt
    /// is a different number at ±2 g than at ±8 g, and a driver that set
    /// ±8 g divides by 4096.
    ranges: Vec<u8>,
}

impl Device {
    /// A part reading what the sheet's props say, and resting where a prop
    /// is absent or is not a number. Values are held to the channel's range.
    pub fn new(spec: Spec, props: &BTreeMap<String, String>) -> Self {
        let values = spec
            .channels
            .iter()
            .map(|channel| {
                props
                    .get(&channel.key)
                    .and_then(|text| text.trim().parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .map_or(channel.rest, |value| value.clamp(channel.min, channel.max))
            })
            .collect();
        let ranges = vec![0u8; spec.ranges.len()];
        Device {
            spec,
            values,
            ranges,
        }
    }

    pub fn value(&self, key: &str) -> Option<f64> {
        let index = self.index_of(key)?;
        self.values.get(index).copied()
    }

    /// Move one reading. False when the key is not one of this part's, the
    /// value is not a number, or nothing changed, so a caller writes
    /// registers only when there is something new in them.
    pub fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(index) = self.index_of(key) else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        let channel = &self.spec.channels[index];
        let value = value.clamp(channel.min, channel.max);
        if self.values[index] == value {
            return false;
        }
        self.values[index] = value;
        true
    }

    /// The readings this part has, by the keys a sheet and a slider name
    /// them with, in its declaration's order.
    pub fn keys(&self) -> Vec<&str> {
        self.spec.channels.iter().map(|c| c.key.as_str()).collect()
    }

    fn index_of(&self, key: &str) -> Option<usize> {
        self.spec.channels.iter().position(|c| c.key == key)
    }

    fn get(&self, key: &str) -> f64 {
        self.value(key).unwrap_or_default()
    }

    /// Every register this device answers with, as runs from a start
    /// register: who it is, how it is calibrated, and what it reads. What a
    /// run declares before the firmware's first transaction.
    pub fn registers(&self) -> Vec<(u8, Vec<u8>)> {
        let mut runs: Vec<(u8, Vec<u8>)> = self
            .spec
            .fixed
            .iter()
            .map(|run| (run.at, run.bytes.clone()))
            .collect();
        runs.extend(self.calibration());
        runs.extend(self.data());
        runs
    }

    /// The bytes a [`Quirk`] supplies that no declaration could. Bosch's
    /// compensation is meaningless without the calibration it reads, and a
    /// driver cannot tell that memory from any other chip's — so the two
    /// travel together rather than the numbers being copied into a file
    /// where nobody could check them against the arithmetic.
    fn calibration(&self) -> Vec<(u8, Vec<u8>)> {
        match self.spec.quirk {
            None => Vec::new(),
            Some(Quirk::Bmp280) => vec![(0x88, bosch::calibration())],
            Some(Quirk::Bme280) => vec![
                (0x88, bosch::calibration()),
                (0xa1, vec![bosch::H1]),
                (0xe1, bosch::humidity_calibration()),
            ],
        }
    }

    /// A table of the part's readings as `signals` move them: every data
    /// register as one block — from the first to the end of the last, a
    /// register between two readings holding what it always holds — one
    /// block per sample. `signals` gives each moving reading its samples,
    /// all the same length; a reading not named keeps its value, and each
    /// sample is encoded exactly as a slider's value is, at the range the
    /// firmware last chose, so a range change is a table rendered again.
    ///
    /// `None` when the part has no data registers, when they span more than
    /// the emulator plays as one block, or when there is nothing to play.
    pub fn block(&self, signals: &[(String, Vec<f64>)]) -> Option<Block> {
        let data = self.data();
        let reg = data.iter().map(|(at, _)| *at).min()?;
        let end = data
            .iter()
            .map(|(at, bytes)| usize::from(*at) + bytes.len())
            .max()?;
        let width = end - usize::from(reg);
        if width == 0 || width > MAX_BLOCK {
            return None;
        }
        let samples = signals.iter().map(|(_, samples)| samples.len()).min()?;

        // Into a block, whatever the runs cover of it.
        let lay = |block: &mut [u8], runs: &[(u8, Vec<u8>)]| {
            for (at, bytes) in runs {
                for (offset, byte) in bytes.iter().enumerate() {
                    let index = usize::from(*at) + offset;
                    if let Some(slot) = index.checked_sub(usize::from(reg))
                        && let Some(cell) = block.get_mut(slot)
                    {
                        *cell = *byte;
                    }
                }
            }
        };
        let mut rest = vec![0u8; width];
        lay(&mut rest, &self.registers());

        let mut moving = self.clone();
        let mut bytes = Vec::with_capacity(samples * width);
        for sample in 0..samples {
            for (key, values) in signals {
                moving.set(key, values[sample]);
            }
            let mut block = rest.clone();
            lay(&mut block, &moving.data());
            bytes.extend(block);
        }
        Some(Block { reg, width, bytes })
    }

    /// Only the registers that change when a reading does: what a slider
    /// moving in a running simulation writes.
    pub fn data(&self) -> Vec<(u8, Vec<u8>)> {
        match self.spec.quirk {
            Some(quirk) => self.climate_data(quirk),
            None => self.declared_data(),
        }
    }

    /// Every declared reading, encoded — with readings that sit next to
    /// each other merged into one run, because a part's data registers are
    /// read in one burst and so they are declared in one.
    fn declared_data(&self) -> Vec<(u8, Vec<u8>)> {
        let mut runs: Vec<(u8, Vec<u8>)> = Vec::new();
        for reading in &self.spec.readings {
            let bytes = self.encode(reading);
            match runs.last_mut() {
                Some((at, held)) if usize::from(*at) + held.len() == usize::from(reading.at) => {
                    held.extend(bytes);
                }
                _ => runs.push((reading.at, bytes)),
            }
        }
        runs
    }

    /// One reading as the part's own register holds it, clipped where the
    /// part's converter clips: a value past the configured range saturates
    /// rather than wrapping round to the opposite sign.
    fn encode(&self, reading: &Reading) -> Vec<u8> {
        let width = usize::from(reading.width).clamp(1, 4);
        let lsb = self.lsb_of(&reading.key, reading.lsb);
        let raw = ((self.get(&reading.key) - reading.offset) * lsb).round();
        let bits = (width * 8) as i32;
        let (low, high) = if reading.signed {
            (-(2f64.powi(bits - 1)), 2f64.powi(bits - 1) - 1.0)
        } else {
            (0.0, 2f64.powi(bits) - 1.0)
        };
        let raw = raw.clamp(low, high) as i64;
        let all = (raw as u64).to_be_bytes();
        let mut bytes = all[8 - width..].to_vec();
        if reading.order == Order::Little {
            bytes.reverse();
        }
        bytes
    }

    /// What a count of this channel is worth, after whatever full scale the
    /// firmware selected.
    fn lsb_of(&self, key: &str, declared: f64) -> f64 {
        for (index, ranged) in self.spec.ranges.iter().enumerate() {
            if !ranged.keys.iter().any(|k| k == key) {
                continue;
            }
            let choice = usize::from(self.ranges.get(index).copied().unwrap_or(0));
            if let Some(lsb) = ranged.lsbs.get(choice) {
                return *lsb;
            }
        }
        declared
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
        let spec = self.spec.clone();
        let mut back = Vec::new();
        let mut rescaled = false;
        for (offset, &value) in stored.iter().enumerate() {
            let register = start.wrapping_add(offset as u8);
            // A reset first, so a range register written after it in the
            // same burst still takes: a driver that resets and configures
            // in one transaction means the configuration.
            for reset in spec.resets.iter().filter(|r| r.at == register) {
                if !reset.fires(value) {
                    continue;
                }
                self.ranges.fill(0);
                back.extend(reset.restores.iter().map(|r| (r.at, r.bytes.clone())));
                rescaled = true;
            }
            for (index, ranged) in spec.ranges.iter().enumerate() {
                if ranged.at != register {
                    continue;
                }
                let choice = (value >> ranged.shift) & ranged.mask;
                if self.ranges[index] != choice {
                    self.ranges[index] = choice;
                    rescaled = true;
                }
            }
            for clear in spec.clears.iter().filter(|c| c.at == register) {
                if value & clear.mask == 0 {
                    continue;
                }
                let field = (value & clear.mask) >> clear.mask.trailing_zeros();
                if !clear.only.is_empty() && !clear.only.contains(&field) {
                    continue;
                }
                back.push((register, vec![value & !clear.mask]));
            }
        }
        if rescaled {
            back.extend(self.data());
        }
        back
    }

    /// `press_msb` through `temp_xlsb`, and `hum_msb`/`hum_lsb` on a BME280:
    /// the raw converter outputs that the calibration in [`bosch`] turns back
    /// into the readings.
    fn climate_data(&self, quirk: Quirk) -> Vec<(u8, Vec<u8>)> {
        let raw_t = bosch::raw_temperature(self.get("temp"));
        let fine = bosch::t_fine(raw_t);
        let raw_p = bosch::raw_pressure(self.get("pressure") * 100.0, fine);
        let mut out = Vec::with_capacity(8);
        out.extend(twenty_bits(raw_p));
        out.extend(twenty_bits(raw_t));
        if quirk == Quirk::Bme280 {
            let raw_h = bosch::raw_humidity(self.get("humidity"), fine);
            out.extend((raw_h as u16).to_be_bytes());
        }
        vec![(0xf7, out)]
    }
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

// The declarations these hold to are the files rusty ships, read by the
// reader every other part goes through — so a fixture cannot drift from
// what a user's own part would get. That reader touches the disk, hence the
// feature gate; the arithmetic below is wasm-safe and has no other door.
#[cfg(all(test, feature = "backend"))]
mod tests {
    use super::*;

    /// One of the parts rusty ships.
    fn built_in(id: &str) -> Spec {
        crate::partfile::load(None)
            .specs
            .into_iter()
            .find(|spec| spec.id == id)
            .unwrap_or_else(|| panic!("data/parts/{id}.toml"))
    }

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
    fn a_part_is_found_by_its_name_and_nothing_else() {
        let specs = crate::partfile::load(None).specs;
        let id = |text: &str| Spec::find(&specs, text).map(|spec| spec.id.as_str());
        assert_eq!(id("mpu6050"), Some("mpu6050"));
        assert_eq!(id("MPU-6050"), Some("mpu6050"), "its datasheet name");
        assert_eq!(id("bme_280"), Some("bme280"));
        assert_eq!(id("bmp280"), Some("bmp280"));
        assert_eq!(id("mpu6500"), None, "a cousin is not the part");
        assert_eq!(id(""), None);
        for spec in &specs {
            assert_eq!(id(&spec.id), Some(spec.id.as_str()));
            assert_eq!(id(&spec.name), Some(spec.id.as_str()));
            assert!(!spec.addresses.is_empty());
        }
    }

    /// A table of the readings as signals move them is the part's data
    /// registers, one block a sample, each sample decoding to the values it
    /// was given — at the range the firmware chose — and the readings no
    /// signal moves holding still across every sample.
    #[test]
    fn a_block_is_every_data_register_a_sample_at_a_time() {
        let mut device = Device::new(built_in("mpu6050"), &props(&[("az", "1")]));
        let gz: Vec<f64> = vec![0.0, 10.0, -20.0, 250.0];
        let block = device
            .block(&[("gz".to_string(), gz.clone())])
            .expect("the MPU-6050 has data registers");
        assert_eq!(block.reg, 0x3b, "from ACCEL_XOUT_H");
        assert_eq!(block.width, 14, "to GYRO_ZOUT_L");
        assert_eq!(block.bytes.len(), 4 * 14);
        for (sample, want) in gz.iter().enumerate() {
            let at = |offset: usize| {
                let base = sample * 14 + offset;
                i16::from_be_bytes([block.bytes[base], block.bytes[base + 1]])
            };
            assert_eq!(
                f64::from(at(0x47 - 0x3b)),
                (want * 131.0).round(),
                "gz at ±250 °/s"
            );
            assert_eq!(at(0x3f - 0x3b), 16384, "az holds its 1 g throughout");
        }

        // The firmware chooses ±2000 °/s, and the same readings encode at
        // 16.4 counts a degree.
        device.wrote(&[0x1b, 0x18]);
        let block = device.block(&[("gz".to_string(), gz)]).unwrap();
        let gz_at = |sample: usize| {
            let base = sample * 14 + (0x47 - 0x3b);
            i16::from_be_bytes([block.bytes[base], block.bytes[base + 1]])
        };
        assert_eq!(f64::from(gz_at(3)), (250.0f64 * 16.4).round());

        assert_eq!(device.block(&[]), None, "nothing to play");
    }

    /// What the `mpu6050` crate does with the registers, done here: a level
    /// board reads 1 g down and nothing else, at the power-on ranges.
    #[test]
    fn an_imu_at_rest_reads_one_g_down_and_the_room_temperature() {
        let device = Device::new(built_in("mpu6050"), &BTreeMap::new());
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
        let mut device = Device::new(
            built_in("mpu6050"),
            &props(&[("ax", "0.5"), ("gz", "-100")]),
        );
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
        let device = Device::new(built_in("mpu6050"), &props(&[("ax", "3"), ("gx", "-400")]));
        let regs = file(&device.registers());
        assert_eq!(be(&regs, 0x3b), i16::MAX, "3 g at ±2 g");
        assert_eq!(be(&regs, 0x43), i16::MIN, "-400 °/s at ±250 °/s");
    }

    /// A driver that resets the part polls DEVICE_RESET until it clears.
    /// The register file only stores, so the clear has to be written back.
    #[test]
    fn a_reset_clears_its_own_bit_and_the_ranges() {
        let mut device = Device::new(built_in("mpu6050"), &props(&[("az", "1")]));
        device.wrote(&[0x1c, 0x18]);
        let back = file(&device.wrote(&[0x6b, 0x80]));
        assert_eq!(back[0x6b], 0x40, "reset done, asleep");
        assert_eq!(back[0x1c], 0x00);
        assert_eq!(be(&back, 0x3f), 16384, "back at ±2 g");
    }

    #[test]
    fn a_slider_moves_only_what_it_names() {
        let mut device = Device::new(built_in("mpu6050"), &BTreeMap::new());
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
            built_in("bme280"),
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
            built_in("bmp280"),
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
        let mut device = Device::new(built_in("bme280"), &BTreeMap::new());
        assert_eq!(device.wrote(&[0xf4, 0x25]), vec![(0xf4, vec![0x24])]);
        assert!(device.wrote(&[0xf4, 0x27]).is_empty(), "normal mode stays");
        let reset = file(&device.wrote(&[0xe0, 0xb6]));
        assert_eq!((reset[0xe0], reset[0xf2], reset[0xf4]), (0, 0, 0));
    }
}
