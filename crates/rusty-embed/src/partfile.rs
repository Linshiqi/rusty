//! Reading a part's declaration: `data/parts/*.toml` and anybody else's.
//!
//! The file format is a public contract with whoever writes a board's
//! parts; [`sensor::Spec`] is an internal one with the frontend. They are
//! different types on purpose (rule 2), so a field renamed for a slider
//! cannot drop a key out of everybody's file.
//!
//! Three layers, later ones winning by id, exactly as the symbol libraries
//! layer: the built-ins compiled in here, the data directory's `parts/`,
//! and the project's `.rusty/parts/`. A file that does not parse is named
//! in the warnings and skipped — a half-read part would answer on the bus
//! with some of its registers, which is worse than not answering at all.
//!
//! **The built-ins go through the same reader as everything else.** A
//! privileged path for the parts rusty ships is how the declared path rots
//! without anybody noticing: the three files below are the only parts this
//! crate has, so a reader that cannot read them cannot read anything.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::sensor::{Channel, Order, Quirk, Ranged, Reading, Reset, Run, SelfClearing, Spec};

/// The parts rusty ships, by the stem their id comes from.
const BUILTIN: &[(&str, &str)] = &[
    ("mpu6050", include_str!("../data/parts/mpu6050.toml")),
    ("bmp280", include_str!("../data/parts/bmp280.toml")),
    ("bme280", include_str!("../data/parts/bme280.toml")),
];

/// Every part a sheet may put on the bus, with what could not be read.
#[derive(Debug, Clone, Default)]
pub struct Parts {
    pub specs: Vec<Spec>,
    /// One line per file that could not be read, naming the file and why.
    pub warnings: Vec<String>,
}

/// The built-ins, then the data directory's `parts/`, then the project's
/// `.rusty/parts/` — a later layer with the same id replaces an earlier one,
/// so a project can correct a part rusty got wrong without waiting for a
/// release.
pub fn load(root: Option<&Path>) -> Parts {
    let mut parts = Parts::default();
    for (stem, text) in BUILTIN {
        match read(stem, text, true) {
            Ok(spec) => parts.push(spec),
            // A built-in that will not read is this crate's own mistake, and
            // saying so beats a part silently missing from the library.
            Err(why) => parts
                .warnings
                .push(format!("data/parts/{stem}.toml: {why}")),
        }
    }
    if let Some(dir) = crate::config::data_dir().map(|dir| dir.join("parts")) {
        read_dir(&dir, &mut parts);
    }
    if let Some(root) = root {
        read_dir(&root.join(".rusty").join("parts"), &mut parts);
    }
    parts
}

impl Parts {
    fn push(&mut self, spec: Spec) {
        match self.specs.iter_mut().find(|held| held.id == spec.id) {
            Some(held) => *held = spec,
            None => self.specs.push(spec),
        }
    }
}

fn read_dir(dir: &Path, parts: &mut Parts) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    // Read in a fixed order, so two files declaring one id resolve the same
    // way on every machine rather than however the filesystem listed them.
    files.sort();
    for path in files {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let shown = path.display().to_string();
        match std::fs::read_to_string(&path) {
            Err(why) => parts.warnings.push(format!("{shown}: {why}")),
            Ok(text) => match read(&stem, &text, false) {
                Ok(spec) => parts.push(spec),
                Err(why) => parts.warnings.push(format!("{shown}: {why}")),
            },
        }
    }
}

/// One file. `trusted` is whether a [`Quirk`] may be named — the built-ins
/// may, because a quirk is arithmetic in this crate and naming one from a
/// project would be asking for code that is not there.
pub fn read(stem: &str, text: &str, trusted: bool) -> Result<Spec, String> {
    let file: PartFile = toml::from_str(text).map_err(|why| why.to_string())?;
    file.into_spec(stem, trusted)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartFile {
    name: String,
    addresses: Vec<u8>,
    #[serde(default)]
    quirk: Option<String>,
    #[serde(default, rename = "fixed")]
    fixed: Vec<RunFile>,
    #[serde(default, rename = "channel")]
    channels: Vec<ChannelFile>,
    #[serde(default, rename = "range")]
    ranges: Vec<RangeFile>,
    #[serde(default, rename = "clear")]
    clears: Vec<ClearFile>,
    #[serde(default, rename = "reset")]
    resets: Vec<ResetFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunFile {
    at: u8,
    bytes: Vec<u8>,
}

/// One quantity: what a person reads, and where the part keeps it. The
/// second half is absent exactly when a quirk supplies it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelFile {
    key: String,
    unit: String,
    min: f64,
    max: f64,
    rest: f64,
    at: Option<u8>,
    width: Option<u8>,
    order: Option<String>,
    #[serde(default)]
    signed: bool,
    lsb: Option<f64>,
    #[serde(default)]
    offset: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeFile {
    at: u8,
    #[serde(default)]
    shift: u8,
    mask: u8,
    keys: Vec<String>,
    lsbs: Vec<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearFile {
    at: u8,
    mask: u8,
    #[serde(default)]
    only: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetFile {
    at: u8,
    mask: Option<u8>,
    equals: Option<u8>,
    restores: Vec<RunFile>,
}

impl PartFile {
    fn into_spec(self, stem: &str, trusted: bool) -> Result<Spec, String> {
        let id = stem.trim().to_ascii_lowercase();
        if id.is_empty() {
            return Err("the file name is the part's id, and this one has none".into());
        }
        let quirk = match self.quirk.as_deref() {
            None => None,
            Some(named) if !trusted => {
                return Err(format!(
                    "`quirk = \"{named}\"` names arithmetic that lives in rusty's own code, so a \
                     part declared here cannot ask for it. Say where each reading sits and what a \
                     count is worth instead, or open an issue naming the part."
                ));
            }
            Some("bmp280") => Some(Quirk::Bmp280),
            Some("bme280") => Some(Quirk::Bme280),
            Some(named) => return Err(format!("`quirk = \"{named}\"` is no arithmetic rusty has")),
        };
        if self.addresses.is_empty() {
            return Err("a part with no address cannot be found on the bus".into());
        }
        if self.channels.is_empty() {
            return Err("a part with no channel has nothing to report".into());
        }

        let mut channels = Vec::new();
        let mut readings = Vec::new();
        for channel in self.channels {
            if channel.min >= channel.max {
                return Err(format!(
                    "`{}` has min {} and max {}, which is no range to move it over",
                    channel.key, channel.min, channel.max
                ));
            }
            match (channel.at, quirk.is_some()) {
                // A quirk says where its readings sit, so a channel that
                // also says would be two answers to one question.
                (Some(_), true) => {
                    return Err(format!(
                        "`{}` gives an `at` beside `quirk`, and the quirk already says where its \
                         readings sit",
                        channel.key
                    ));
                }
                (None, false) => {
                    return Err(format!(
                        "`{}` says nothing about where it sits. Give it `at`, `width`, `order` and \
                         `lsb` — a channel with no encoding could only be guessed at.",
                        channel.key
                    ));
                }
                (None, true) => {}
                (Some(at), false) => {
                    let width = channel.width.unwrap_or(0);
                    if !(1..=4).contains(&width) {
                        return Err(format!(
                            "`{}` has width {width}; a reading is one to four bytes",
                            channel.key
                        ));
                    }
                    let order = match channel.order.as_deref() {
                        Some("big") => Order::Big,
                        Some("little") => Order::Little,
                        Some(other) => {
                            return Err(format!(
                                "`{}` has order \"{other}\"; it is \"big\" or \"little\"",
                                channel.key
                            ));
                        }
                        None if width == 1 => Order::Big,
                        None => {
                            return Err(format!(
                                "`{}` is {width} bytes and does not say which end comes first; \
                                 give it `order = \"big\"` or `order = \"little\"`",
                                channel.key
                            ));
                        }
                    };
                    let lsb = match channel.lsb {
                        Some(lsb) if lsb != 0.0 && lsb.is_finite() => lsb,
                        _ => {
                            return Err(format!(
                                "`{}` needs an `lsb`: how many counts one {} is worth",
                                channel.key, channel.unit
                            ));
                        }
                    };
                    readings.push(Reading {
                        key: channel.key.clone(),
                        at,
                        width,
                        order,
                        signed: channel.signed,
                        lsb,
                        offset: channel.offset,
                    });
                }
            }
            channels.push(Channel {
                key: channel.key,
                unit: channel.unit,
                min: channel.min,
                max: channel.max,
                rest: channel.rest,
            });
        }

        let known = |key: &str| channels.iter().any(|c: &Channel| c.key == key);
        let mut ranges = Vec::new();
        for range in self.ranges {
            if range.mask == 0 {
                return Err(format!("the range at {:#04x} masks nothing", range.at));
            }
            let wanted = usize::from(range.mask) + 1;
            if range.lsbs.len() != wanted {
                return Err(format!(
                    "the range at {:#04x} masks {wanted} values and gives {} counts-per-unit; a \
                     selection with no scale beside it would read as the wrong tilt rather than \
                     as a mistake",
                    range.at,
                    range.lsbs.len()
                ));
            }
            if let Some(key) = range.keys.iter().find(|key| !known(key)) {
                return Err(format!(
                    "the range at {:#04x} covers `{key}`, which is no channel of this part",
                    range.at
                ));
            }
            ranges.push(Ranged {
                at: range.at,
                shift: range.shift,
                mask: range.mask,
                keys: range.keys,
                lsbs: range.lsbs,
            });
        }

        let mut clears = Vec::new();
        for clear in self.clears {
            if clear.mask == 0 {
                return Err(format!("the clear at {:#04x} masks nothing", clear.at));
            }
            clears.push(SelfClearing {
                at: clear.at,
                mask: clear.mask,
                only: clear.only,
            });
        }

        let mut resets = Vec::new();
        for reset in self.resets {
            match (reset.mask, reset.equals) {
                (None, None) => {
                    return Err(format!(
                        "the reset at {:#04x} says neither `mask` nor `equals`, so nothing would \
                         ever fire it",
                        reset.at
                    ));
                }
                (Some(_), Some(_)) => {
                    return Err(format!(
                        "the reset at {:#04x} says both `mask` and `equals`; a part's reset is \
                         one or the other",
                        reset.at
                    ));
                }
                _ => {}
            }
            resets.push(Reset {
                at: reset.at,
                mask: reset.mask,
                equals: reset.equals,
                restores: reset.restores.into_iter().map(RunFile::into_run).collect(),
            });
        }

        Ok(Spec {
            id,
            name: self.name,
            addresses: self.addresses,
            channels,
            fixed: self.fixed.into_iter().map(RunFile::into_run).collect(),
            readings,
            ranges,
            clears,
            resets,
            quirk,
        })
    }
}

impl RunFile {
    fn into_run(self) -> Run {
        Run {
            at: self.at,
            bytes: self.bytes,
        }
    }
}

/// The props a part's channels rest at, for a sheet that has just placed
/// one: what the sliders start on, written into the file so the board says
/// what it is showing.
pub fn resting(spec: &Spec) -> BTreeMap<String, String> {
    spec.channels
        .iter()
        .map(|channel| (channel.key.clone(), channel.rest.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every part rusty ships is read by the same reader a project's file
    /// goes through, so a reader that broke would take all three with it.
    #[test]
    fn the_built_in_parts_read() {
        let parts = load(None);
        assert!(parts.warnings.is_empty(), "{:?}", parts.warnings);
        // `contains`, not equality: the data directory's own `parts/` is a
        // layer of this list, and a test that demanded exactly three would
        // fail on the machine of anybody who had declared one.
        let ids: Vec<&str> = parts.specs.iter().map(|s| s.id.as_str()).collect();
        for shipped in ["mpu6050", "bmp280", "bme280"] {
            assert!(ids.contains(&shipped), "{ids:?}");
        }

        let imu = Spec::find(&parts.specs, "MPU-6050").expect("found by its datasheet name");
        assert_eq!(imu.addresses, [0x68, 0x69]);
        assert_eq!(imu.channels.len(), 7);
        assert_eq!(imu.readings.len(), 7, "every channel says where it sits");
        assert_eq!(imu.ranges.len(), 2);
        assert!(imu.quirk.is_none(), "nothing an MPU-6050 does needs code");

        let climate = Spec::find(&parts.specs, "bme280").expect("found by its id");
        assert_eq!(climate.quirk, Some(Quirk::Bme280));
        assert!(
            climate.readings.is_empty(),
            "a quirk says where its readings sit"
        );
    }

    /// A project may declare a part; it may not ask for arithmetic that
    /// only exists in this crate.
    #[test]
    fn a_project_cannot_name_a_quirk() {
        let text = "name = \"X\"\naddresses = [0x40]\nquirk = \"bme280\"\n\
                    [[channel]]\nkey=\"t\"\nunit=\"C\"\nmin=0.0\nmax=1.0\nrest=0.0\n";
        let why = read("x", text, false).expect_err("refused");
        assert!(why.contains("quirk"), "{why}");
        assert!(read("x", text, true).is_ok(), "the built-ins may");
    }

    /// Every refusal names the field that would answer, because a part
    /// somebody is writing is a part they can fix.
    #[test]
    fn a_declaration_that_cannot_answer_says_which_field_would() {
        let base = "name = \"X\"\naddresses = [0x40]\n";
        let channel = |extra: &str| {
            format!(
                "{base}[[channel]]\nkey=\"t\"\nunit=\"C\"\nmin=-1.0\nmax=1.0\nrest=0.0\n{extra}"
            )
        };
        let why = |text: String| read("x", &text, false).expect_err("refused");

        assert!(why(channel("")).contains("`at`"), "no encoding at all");
        assert!(why(channel("at=0x10\nwidth=2\nlsb=1.0\n")).contains("order"));
        assert!(why(channel("at=0x10\nwidth=2\norder=\"big\"\n")).contains("lsb"));
        assert!(why(channel("at=0x10\nwidth=9\norder=\"big\"\nlsb=1.0\n")).contains("width"));
        assert!(
            why(format!("{base}[[channel]]\nkey=\"t\"\nunit=\"C\"\nmin=1.0\nmax=1.0\nrest=1.0\nat=0x10\nwidth=1\nlsb=1.0\n"))
                .contains("no range"),
            "a channel nobody can move"
        );
        assert!(why(base.to_string()).contains("no channel"));
        assert!(why("name=\"X\"\naddresses=[]\n".to_string()).contains("no address"));

        // A range with fewer scales than it selects would silently read as
        // the wrong tilt at the selections it does not cover.
        let short = format!(
            "{}[[range]]\nat=0x1b\nshift=3\nmask=0x03\nkeys=[\"t\"]\nlsbs=[1.0,2.0]\n",
            channel("at=0x10\nwidth=1\nlsb=1.0\n")
        );
        assert!(why(short).contains("counts-per-unit"));

        // And one covering a channel the part does not have.
        let stray = format!(
            "{}[[range]]\nat=0x1b\nshift=3\nmask=0x03\nkeys=[\"nope\"]\nlsbs=[1.0,2.0,3.0,4.0]\n",
            channel("at=0x10\nwidth=1\nlsb=1.0\n")
        );
        assert!(why(stray).contains("no channel of this part"));

        // A reset nothing fires, and one with two triggers.
        let dead = format!(
            "{}[[reset]]\nat=0x6b\nrestores=[]\n",
            channel("at=0x10\nwidth=1\nlsb=1.0\n")
        );
        assert!(why(dead).contains("ever fire it"));
        let both = format!(
            "{}[[reset]]\nat=0x6b\nmask=0x80\nequals=0xb6\nrestores=[]\n",
            channel("at=0x10\nwidth=1\nlsb=1.0\n")
        );
        assert!(why(both).contains("one or the other"));
    }

    /// A part a project declares reaches the bus, and its readings are
    /// encoded the way its file says — the whole point of the format, with
    /// no code in this crate knowing the part exists.
    ///
    /// Three axes, sixteen little-endian bits each, and a range register
    /// that halves the step three times over. The numbers are the test's,
    /// picked so the arithmetic can be checked by eye rather than
    /// transcribed from a datasheet — what is being held here is the
    /// format, not anybody's accelerometer.
    #[test]
    fn a_part_a_project_declares_answers_on_the_bus() {
        let project = tempfile::tempdir().unwrap();
        let dir = project.path().join(".rusty").join("parts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("adxl345.toml"),
            r#"
name = "ADXL345"
addresses = [0x53, 0x1d]

[[fixed]]
at = 0x00
bytes = [0xe5]

[[channel]]
key = "x"
unit = "g"
min = -16.0
max = 16.0
rest = 0.0
at = 0x32
width = 2
order = "little"
signed = true
lsb = 256.0

[[channel]]
key = "y"
unit = "g"
min = -16.0
max = 16.0
rest = 0.0
at = 0x34
width = 2
order = "little"
signed = true
lsb = 256.0

[[channel]]
key = "z"
unit = "g"
min = -16.0
max = 16.0
rest = 1.0
at = 0x36
width = 2
order = "little"
signed = true
lsb = 256.0

[[range]]
at = 0x31
shift = 0
mask = 0x03
keys = ["x", "y", "z"]
lsbs = [256.0, 128.0, 64.0, 32.0]
"#,
        )
        .unwrap();

        let parts = load(Some(project.path()));
        assert!(parts.warnings.is_empty(), "{:?}", parts.warnings);
        let spec = Spec::find(&parts.specs, "ADXL345").expect("declared by the project");
        assert_eq!(spec.addresses, [0x53, 0x1d]);

        let mut device = crate::sensor::Device::new(spec.clone(), &BTreeMap::new());
        let mut file = [0u8; 256];
        let lay = |runs: Vec<(u8, Vec<u8>)>, file: &mut [u8; 256]| {
            for (at, bytes) in runs {
                for (offset, byte) in bytes.into_iter().enumerate() {
                    file[usize::from(at.wrapping_add(offset as u8))] = byte;
                }
            }
        };
        lay(device.registers(), &mut file);
        let axis = |file: &[u8; 256], at: usize| i16::from_le_bytes([file[at], file[at + 1]]);
        assert_eq!(file[0x00], 0xe5, "the identity a driver refuses on");
        assert_eq!(axis(&file, 0x32), 0);
        assert_eq!(axis(&file, 0x36), 256, "one g down at ±2 g");

        // Tilt it, and the reading follows at the declared step.
        assert!(device.set("x", -0.5));
        lay(device.data(), &mut file);
        assert_eq!(axis(&file, 0x32), -128);

        // The firmware chooses ±16 g, and every axis is encoded again —
        // without which a driver dividing by 32 reads an eighth of the tilt.
        lay(device.wrote(&[0x31, 0x03]), &mut file);
        assert_eq!(axis(&file, 0x32), -16);
        assert_eq!(axis(&file, 0x36), 32);

        // And past the range it saturates rather than wrapping round.
        assert!(device.set("z", 16.0));
        lay(device.data(), &mut file);
        assert_eq!(axis(&file, 0x36), 512);
        assert!(device.set("z", -16.0));
        lay(device.data(), &mut file);
        assert_eq!(axis(&file, 0x36), -512);
    }

    /// An unknown key is a typo, and a typo that parses is a part quietly
    /// missing whatever the writer meant.
    #[test]
    fn an_unknown_key_is_refused() {
        let text = "name=\"X\"\naddresses=[0x40]\nwibble=1\n";
        assert!(read("x", text, true).is_err());
    }
}
