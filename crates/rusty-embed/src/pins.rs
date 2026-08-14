//! Which pins the part has, and which ones the source already spoke for.
//!
//! Answering "where did I put the LED" by reading code is the job this
//! removes, and it has exactly two sources, kept apart because their
//! trustworthiness differs:
//!
//! - **What the source claims** — a text scan for `.GPIO<n>`. Always
//!   available, needs no build, and sees only what is written literally. A
//!   pin reached through a binding (`let p = peripherals.GPIO5;`) is invisible
//!   to it, so what this reports is *pins the source names*, never *pins the
//!   firmware uses*. The panel says so in those words.
//! - **What the part has** — esp-hal's own device description, the same TOML
//!   the HAL is generated from, at the version this project's `Cargo.lock`
//!   pins. Available only once the project has been fetched, and absent is
//!   reported rather than filled in: guessing which pins a part has is how
//!   somebody ends up driving the SPI flash.
//!
//! The second source is worth the trouble because it carries what no amount
//! of reading the code can tell you — `input_only`, the ADC channels, and
//! which pins the module has already spent on flash, USB and the console.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::{PinClaim, PinInfo, PinReport};

/// The part's pins and the project's claims on them.
pub fn report(root: &Path, chip: &str) -> PinReport {
    let claims = claims(root);
    // With no capabilities, every claim is `unknown` — not because the pin
    // does not exist, but because nothing here can say that it does. The
    // note carries the difference.
    let blind = |note: String| PinReport {
        chip: chip.to_string(),
        pins: Vec::new(),
        source: None,
        note: Some(note),
        unknown: claims.clone(),
    };

    let Some((path, text)) = device_file(root, chip) else {
        return blind(format!(
            "rusty could not find esp-hal's description of {chip}, so it can only show \
             what the source names — not which pins exist, which are input-only, or \
             which the module has already spent on flash and USB. Building the project \
             once fetches it.",
        ));
    };

    let Ok(device) = toml::from_str::<DeviceFile>(&text) else {
        return blind(format!(
            "{} is not in the shape rusty knows, so the pin capabilities were not read. \
             The claims below still come from your own source.",
            path.display(),
        ));
    };

    let mut pins: Vec<PinInfo> = device
        .device
        .gpio
        .pins
        .into_iter()
        .map(|entry| PinInfo {
            reserved: entry.reserved(),
            gpio: entry.pin,
            input_only: entry.input_only,
            // `analog` answers "can this pin do an analog job" — ADC, DAC,
            // touch. The USB pair sits in the same table because it is also
            // not a digital function, but it is not something to reach for,
            // and `reserved` has already said so.
            analog: entry
                .analog
                .into_values()
                .filter(|name| !name.starts_with("USB_"))
                .collect(),
            claims: Vec::new(),
        })
        .collect();
    pins.sort_by_key(|pin| pin.gpio);

    let mut unknown = Vec::new();
    for claim in claims {
        match pins.iter_mut().find(|pin| pin.gpio == claim.gpio) {
            Some(pin) => pin.claims.push(claim),
            // Not a gap in the scan: after a chip switch these are precisely
            // the sites that have to be decided, and burying them among the
            // pins that do exist would hide the only work there is.
            None => unknown.push(claim),
        }
    }

    PinReport {
        chip: chip.to_string(),
        pins,
        source: Some(path.display().to_string()),
        note: None,
        unknown,
    }
}

/// Every `.GPIO<n>` in the project's own sources.
///
/// Anchored on the dot so a comment or a string mentioning "GPIO26" is not a
/// claim, and so `GPIO26` inside a longer identifier is not either.
pub fn claims(root: &Path) -> Vec<PinClaim> {
    fn walk(dir: &Path, root: &Path, found: &mut Vec<PinClaim>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let file = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                for (number, line) in text.lines().enumerate() {
                    for (at, _) in line.match_indices(".GPIO") {
                        let digits: String = line[at + 5..]
                            .chars()
                            .take_while(char::is_ascii_digit)
                            .collect();
                        let Ok(gpio) = digits.parse::<u32>() else {
                            continue;
                        };
                        found.push(PinClaim {
                            gpio,
                            file: file.clone(),
                            line: number as u32,
                            text: line.trim().to_string(),
                        });
                    }
                }
            }
        }
    }

    let mut found = Vec::new();
    walk(&root.join("src"), root, &mut found);
    found
}

/// The device description esp-hal was generated from, at the version this
/// project locks.
///
/// Resolved through `Cargo.lock` rather than by taking the newest on the
/// machine: two projects can pin different esp-hal versions, and a pin table
/// from the wrong one is worse than none.
fn device_file(root: &Path, chip: &str) -> Option<(PathBuf, String)> {
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).ok()?;
    let version = locked_version(&lock, "esp-metadata")?;

    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs_home().map(|home| home.join(".cargo")))?;
    // The registry directory carries a hash of the index URL, so it is found
    // rather than constructed.
    let registries = std::fs::read_dir(home.join("registry/src")).ok()?;
    for registry in registries.flatten() {
        let candidate = registry
            .path()
            .join(format!("esp-metadata-{version}"))
            .join("devices")
            .join(format!("{chip}.toml"));
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return Some((candidate, text));
        }
    }
    None
}

fn locked_version(lock: &str, package: &str) -> Option<String> {
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == format!("name = \"{package}\"") {
            let version = lines.next()?.trim();
            return version
                .strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))
                .map(str::to_string);
        }
    }
    None
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[derive(Deserialize)]
struct DeviceFile {
    device: Device,
}

#[derive(Deserialize)]
struct Device {
    gpio: Gpio,
}

#[derive(Deserialize)]
struct Gpio {
    pins: Vec<PinEntry>,
}

/// One row of the vendor's pin table. Keys in the inline tables are mux
/// levels; level 0 is the pin's default wiring, which is what says whether
/// the module has already spent it.
#[derive(Deserialize)]
struct PinEntry {
    pin: u32,
    #[serde(default)]
    input_only: bool,
    #[serde(default)]
    functions: BTreeMap<String, String>,
    #[serde(default)]
    analog: BTreeMap<String, String>,
}

impl PinEntry {
    /// What this pin is already doing on a module, or `None` when it is free
    /// for the firmware.
    ///
    /// Read off the level-0 name rather than a list of pin numbers per chip,
    /// which is the same fact stated once by the vendor instead of retyped
    /// per part: ESP32 spends 6..11 on the flash and calls them `SD_*`, the
    /// C3 spends 12..17 and calls them `SPI*`, and both are the same rule.
    fn reserved(&self) -> Option<String> {
        if let Some(name) = self.functions.get("0") {
            if name.starts_with("SD_") || (name.starts_with("SPI") && name != "SPI") {
                return Some(format!("SPI flash ({name})"));
            }
            if name.starts_with("U0") {
                return Some(format!("UART0 console ({name})"));
            }
        }
        if let Some(name) = self.analog.get("0")
            && name.starts_with("USB_")
        {
            return Some(format!("native USB ({name})"));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(pins: &str) -> DeviceFile {
        toml::from_str(&format!("[device.gpio]\npins = [\n{pins}\n]\n")).expect("parsed")
    }

    /// The rule that matters most: a pin the module has already spent is not
    /// a pin the firmware may take, and nothing in the code says so.
    #[test]
    fn the_pins_a_module_has_already_spent_are_named_from_their_default_function() {
        let file = device(
            "{ pin = 6, functions = { 0 = \"SD_CLK\", 1 = \"SPICLK\" } },\n\
             { pin = 12, functions = { 0 = \"SPIHD\" } },\n\
             { pin = 20, functions = { 0 = \"U0RXD\" } },\n\
             { pin = 18, analog = { 0 = \"USB_DM\" } },\n\
             { pin = 5, functions = { 2 = \"FSPIWP\" } },\n",
        );
        let reserved: Vec<Option<String>> =
            file.device.gpio.pins.iter().map(PinEntry::reserved).collect();

        assert!(reserved[0].as_deref().is_some_and(|r| r.contains("SPI flash")));
        assert!(reserved[1].as_deref().is_some_and(|r| r.contains("SPI flash")));
        assert!(reserved[2].as_deref().is_some_and(|r| r.contains("console")));
        assert!(reserved[3].as_deref().is_some_and(|r| r.contains("USB")));
        assert_eq!(
            reserved[4], None,
            "an alternate function at another mux level is available, not spent",
        );
    }

    /// A comment is not a claim, and neither is a longer name.
    #[test]
    fn only_a_field_access_counts_as_naming_a_pin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "// wire the LED to GPIO26 one day\n\
             let led = Output::new(peripherals.GPIO5, Level::High);\n\
             let name = \"GPIO7\";\n\
             let two = io.GPIO21;\n",
        )
        .unwrap();

        let found = claims(dir.path());
        let pins: Vec<u32> = found.iter().map(|claim| claim.gpio).collect();
        assert_eq!(
            pins,
            vec![5, 21],
            "the comment and the string are not claims: {found:?}",
        );
        assert_eq!(found[0].line, 1, "zero-based, like every line that crosses the wire");
        assert!(found[0].text.starts_with("let led ="));
    }

    /// Without the device description the report is still useful, and says
    /// exactly what it cannot do rather than showing an empty chip.
    #[test]
    fn a_project_with_no_device_description_reports_claims_and_why() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "Output::new(peripherals.GPIO26, Level::High);\n",
        )
        .unwrap();

        let report = report(dir.path(), "esp32");
        assert!(report.pins.is_empty());
        assert!(
            report.note.is_some_and(|n| n.contains("could not find")),
            "the absence is explained",
        );
        assert_eq!(report.unknown.len(), 1, "and the claim survives it");
    }

    #[test]
    fn the_locked_version_is_read_rather_than_the_newest_on_the_machine() {
        let lock = "[[package]]\nname = \"esp-hal\"\nversion = \"1.1.2\"\n\n\
                    [[package]]\nname = \"esp-metadata\"\nversion = \"0.8.0\"\n";
        assert_eq!(locked_version(lock, "esp-metadata").as_deref(), Some("0.8.0"));
        assert_eq!(locked_version(lock, "nothing-here"), None);
    }
}
