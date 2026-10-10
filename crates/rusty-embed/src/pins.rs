//! Which pins the part has, and which ones the source already spoke for.
//!
//! Answering "where did I put the LED" by reading code is the job this
//! removes, and it has exactly two sources, kept apart because their
//! trustworthiness differs:
//!
//! - **What the source claims** — a text scan for the pin names a HAL's
//!   peripherals struct uses: `.GPIO<n>` (esp-hal), `.PC13` (embassy-stm32,
//!   ch32-hal), `.P0_13` (embassy-nrf), `.PIN_25` (embassy-rp). Always
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
//!
//! **A part that is not Espressif's has no esp-hal description**, and asking
//! for one told an STM32 user that rusty "could not find esp-hal's
//! description of stm32f411ce". What the part has is then the catalogue's
//! `gpio` list — which pins the die has, transcribed per package, and
//! nothing about what each can do — and the note says that much and no
//! more.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::{Chip, PinClaim, PinInfo, PinReport, Ports};

/// The part's pins and the project's claims on them.
///
/// **Two roots, because they answer different questions.** `root` is the
/// directory the user opened, and every claim's path is reported relative to
/// it — a claim is a place in the editor, and the editor belongs to the whole
/// repository. `firmware` is where the chip is, which is where esp-hal put its
/// device description; on the standard embedded workspace those are not the
/// same directory, and reporting `src/main.rs` relative to the firmware crate
/// gave the editor a path that does not exist from the root.
///
/// Identical for every ordinary project, where the root has its own chip.
pub fn report(root: &Path, firmware: &Path, chip: &str) -> PinReport {
    let part = crate::chip::by_id(chip);
    let claims = claims(root, firmware, part.as_ref().and_then(|p| p.ports));
    if let Some(part) = part.as_ref().filter(|p| p.vendor != "espressif") {
        return from_catalogue(part, claims);
    }
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

    let candidates = device_files(firmware, chip);
    if candidates.is_empty() {
        return blind(format!(
            "rusty could not find esp-hal's description of {chip} — esp-metadata's \
             `devices/{chip}.toml` or esp-metadata-generated's `_generated_{chip}.rs`, \
             at the versions Cargo.lock names — so it can only show what the source \
             names: not which pins exist, which are input-only, or which the module \
             has already spent on flash and USB. Building the project once fetches it.",
        ));
    }

    // Two spellings of the same vendor table: esp-metadata's TOML up to
    // esp-hal 0.23, and the generated Rust of esp-metadata-generated from
    // esp-hal 1.0 on, which every current project locks. A lock can name
    // several versions of the generated crate — a transitive dependency
    // pinned to an old one beside esp-hal's own — and their shapes differ,
    // so each is tried and the first that reads is the table.
    let mut parsed = None;
    for (path, text) in &candidates {
        let entries = if path.extension().is_some_and(|e| e == "toml") {
            toml::from_str::<DeviceFile>(text)
                .ok()
                .map(|device| device.device.gpio.pins)
        } else {
            generated_pins(text)
        };
        if let Some(entries) = entries {
            parsed = Some((path.clone(), entries));
            break;
        }
    }
    let Some((path, entries)) = parsed else {
        return blind(format!(
            "{} is not in the shape rusty knows, so the pin capabilities were not read. \
             The claims below still come from your own source.",
            candidates[0].0.display(),
        ));
    };

    let mut pins: Vec<PinInfo> = entries
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

/// What a part that is not Espressif's has, from rusty's catalogue: the
/// pins of its package, and the claims sorted onto them. Nothing about what
/// each pin can do — the catalogue does not say, and the note says that it
/// does not.
fn from_catalogue(part: &Chip, claims: Vec<PinClaim>) -> PinReport {
    if part.gpio.is_empty() {
        return PinReport {
            chip: part.id.clone(),
            pins: Vec::new(),
            source: None,
            note: Some(format!(
                "rusty's catalogue lists no pins for {}, so only what the source names is \
                 shown.",
                part.name
            )),
            unknown: claims,
        };
    }
    let mut pins: Vec<PinInfo> = part
        .gpio
        .iter()
        .map(|&gpio| PinInfo {
            gpio,
            input_only: false,
            analog: Vec::new(),
            reserved: None,
            claims: Vec::new(),
        })
        .collect();
    pins.sort_by_key(|pin| pin.gpio);
    let mut unknown = Vec::new();
    for claim in claims {
        match pins.iter_mut().find(|pin| pin.gpio == claim.gpio) {
            Some(pin) => pin.claims.push(claim),
            None => unknown.push(claim),
        }
    }
    PinReport {
        chip: part.id.clone(),
        pins,
        source: Some(format!("rusty's catalogue ({})", part.name)),
        note: Some(format!(
            "These are the pins {} has. Which of them are input-only, analog, or \
             already wired to something on your board is not in rusty's catalogue — \
             the board's schematic says.",
            part.name
        )),
        unknown,
    }
}

/// The pin a peripherals-struct field names, or `None`: `GPIO5` (esp-hal),
/// `PIN_25` (embassy-rp), and on a part whose pins are named by port,
/// `PC13` or `P0_13` (embassy-stm32, ch32-hal, embassy-nrf).
pub(crate) fn pin_of(name: &str, ports: Option<Ports>) -> Option<u32> {
    let numbered = |digits: &str| {
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
    };
    if let Some(digits) = name.strip_prefix("GPIO") {
        return numbered(digits);
    }
    if let Some(digits) = name.strip_prefix("PIN_") {
        return numbered(digits);
    }
    ports.and_then(|ports| ports.number(name)).map(u32::from)
}

/// Every pin the project's own sources name, as a field of the HAL's
/// peripherals struct (`pin_of`).
///
/// Anchored on the dot so a comment or a string mentioning "GPIO26" is not a
/// claim, and so `GPIO26` inside a longer identifier is not either.
///
/// **Scanned from `firmware`, reported relative to `root`.** Pins are named in
/// the firmware crate, which on the standard embedded workspace is a directory
/// the root excludes — so scanning the root finds nothing. But a claim is a
/// place the editor opens, and the editor is rooted at the opened directory,
/// so `firmware/src/main.rs` is the path that resolves. Getting either half
/// wrong is silent: the wrong scan root reports no claims at all, and the
/// wrong relative root reports paths that fail to open.
pub fn claims(root: &Path, firmware: &Path, ports: Option<Ports>) -> Vec<PinClaim> {
    fn walk(dir: &Path, root: &Path, ports: Option<Ports>, found: &mut Vec<PinClaim>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, ports, found);
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
                    for (at, _) in line.match_indices('.') {
                        let name: String = line[at + 1..]
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        let Some(gpio) = pin_of(&name, ports) else {
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
    walk(&firmware.join("src"), root, ports, &mut found);
    found
}

/// Every device description this project's lock names, most likely first:
/// the `esp-metadata-generated` version esp-hal itself depends on, then any
/// other version of it the lock holds (newest first), then `esp-metadata`'s
/// TOML for projects on esp-hal 0.x.
///
/// Resolved through `Cargo.lock` rather than by taking the newest on the
/// machine: two projects can pin different esp-hal versions, and a pin table
/// from the wrong one is worse than none. A lock with two versions of the
/// generated crate is ordinary — one project here had 0.1.0 pulled in by a
/// transitive dependency beside esp-hal's 0.4.0 — and the first one in the
/// file was the wrong one.
fn device_files(root: &Path, chip: &str) -> Vec<(PathBuf, String)> {
    let Ok(lock) = std::fs::read_to_string(root.join("Cargo.lock")) else {
        return Vec::new();
    };
    let mut generated: Vec<String> = Vec::new();
    if let Some(own) = dependency_version(&lock, "esp-hal", "esp-metadata-generated") {
        generated.push(own);
    }
    for version in locked_versions(&lock, "esp-metadata-generated")
        .into_iter()
        .rev()
    {
        if !generated.contains(&version) {
            generated.push(version);
        }
    }
    let mut relative: Vec<PathBuf> = generated
        .iter()
        .map(|version| {
            PathBuf::from(format!("esp-metadata-generated-{version}"))
                .join("src")
                .join(format!("_generated_{chip}.rs"))
        })
        .collect();
    for version in locked_versions(&lock, "esp-metadata").into_iter().rev() {
        relative.push(
            PathBuf::from(format!("esp-metadata-{version}"))
                .join("devices")
                .join(format!("{chip}.toml")),
        );
    }
    if relative.is_empty() {
        return Vec::new();
    }

    let Some(home) = crate::tools::cargo_home() else {
        return Vec::new();
    };
    // The registry directory carries a hash of the index URL, so it is found
    // rather than constructed.
    let Ok(registries) = std::fs::read_dir(home.join("registry/src")) else {
        return Vec::new();
    };
    let registries: Vec<PathBuf> = registries.flatten().map(|e| e.path()).collect();
    let mut found = Vec::new();
    for relative in relative {
        for registry in &registries {
            let candidate = registry.join(&relative);
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                found.push((candidate, text));
                break;
            }
        }
    }
    found
}

/// The pin table as `esp-metadata-generated` spells it: a `for_each_gpio!`
/// macro whose body lists one call per pin,
/// `_for_each_inner_gpio!((2, GPIO2(_2 => FSPIQ) (_2 => FSPIQ) ([Input] [Output])))`
/// — number, then the input and output functions by mux level, then the
/// capabilities, `([Input] [])` for a pin with no output driver — followed
/// by one `all(…)` call repeating every entry, which is skipped. Analog
/// functions come from `for_each_analog_function!`, one `(ADC1_CH2, GPIO2)`
/// per call. Anything not in that shape yields `None`, and the caller says
/// the table was not read rather than showing half of one.
fn generated_pins(text: &str) -> Option<Vec<PinEntry>> {
    let mut pins: BTreeMap<u32, PinEntry> = BTreeMap::new();
    let gpio = flattened_macro(text, "for_each_gpio")?;
    for piece in gpio.split("_for_each_inner_gpio!((").skip(1) {
        if piece.starts_with("all(") {
            continue;
        }
        let (number, rest) = piece.split_once(',')?;
        let pin: u32 = number.trim().parse().ok()?;
        let rest = rest.trim_start();
        let (inputs, rest) = paren_group(&rest[rest.find('(')?..])?;
        let (outputs, rest) = paren_group(rest.trim_start())?;
        let (capabilities, _) = paren_group(rest.trim_start())?;
        let mut functions = BTreeMap::new();
        for group in [inputs, outputs] {
            for (level, name) in mux_pairs(group) {
                functions.entry(level).or_insert(name);
            }
        }
        pins.insert(
            pin,
            PinEntry {
                pin,
                input_only: !capabilities.contains("[Output]"),
                functions,
                analog: BTreeMap::new(),
            },
        );
    }
    if pins.is_empty() {
        return None;
    }
    if let Some(analog) = flattened_macro(text, "for_each_analog_function") {
        for piece in analog.split("_for_each_inner_analog_function!((").skip(1) {
            let Some((inner, _)) = piece.split_once(')') else {
                continue;
            };
            let Some((name, gpio)) = inner.split_once(',') else {
                continue;
            };
            let Ok(gpio) = gpio.trim().trim_start_matches("GPIO").parse::<u32>() else {
                continue;
            };
            if let Some(pin) = pins.get_mut(&gpio) {
                let level = pin.analog.len().to_string();
                pin.analog.insert(level, name.trim().to_string());
            }
        }
    }
    Some(pins.into_values().collect())
}

/// One `macro_rules!` body with its whitespace collapsed, so entries rustfmt
/// wrapped across lines read as one token stream.
fn flattened_macro(text: &str, name: &str) -> Option<String> {
    let start = text.find(&format!("macro_rules! {name} "))?;
    let body = &text[start..];
    // The next top-level macro ends this one. Each body declares an indented
    // inner `macro_rules! _for_each_inner_…`, so only one at the start of a
    // line counts.
    let end = body[1..]
        .find(
            "
macro_rules!",
        )
        .map_or(body.len(), |at| at + 1);
    let flat = body[..end].split_whitespace().collect::<Vec<_>>().join(" ");
    // esp-metadata-generated 0.1.0 called every inner macro `_for_each_inner`;
    // later versions name it after the table. One spelling for the callers.
    let inner = format!(
        "_for_each_inner_{}!((",
        name.trim_start_matches("for_each_")
    );
    Some(flat.replace("_for_each_inner!((", &inner))
}

/// The text inside the parenthesised group `s` starts with, and what follows
/// the closing parenthesis.
fn paren_group(s: &str) -> Option<(&str, &str)> {
    if !s.starts_with('(') {
        return None;
    }
    let mut depth = 0usize;
    for (at, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[1..at], &s[at + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// `_0 => MTMS _2 => FSPIHD` → `[("0", "MTMS"), ("2", "FSPIHD")]`.
fn mux_pairs(group: &str) -> Vec<(String, String)> {
    let tokens: Vec<&str> = group.split_whitespace().collect();
    tokens
        .windows(3)
        .filter(|w| w[1] == "=>" && w[0].starts_with('_'))
        .map(|w| (w[0][1..].to_string(), w[2].to_string()))
        .collect()
}

/// Every version of `package` the lock holds, in the file's order — sorted by
/// name then version, as cargo writes it.
fn locked_versions(lock: &str, package: &str) -> Vec<String> {
    let mut versions = Vec::new();
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == format!("name = \"{package}\"")
            && let Some(version) = lines.next()
            && let Some(version) = version
                .trim()
                .strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))
        {
            versions.push(version.to_string());
        }
    }
    versions
}

/// The version of `dep` that `of` depends on, when the lock spells it out —
/// cargo writes `"dep 1.2.3"` in a dependency list only when the graph holds
/// more than one version of `dep`. `None` when it is unique (and
/// [`locked_versions`] has the one) or when `of` is not in the lock.
fn dependency_version(lock: &str, of: &str, dep: &str) -> Option<String> {
    let mut lines = lock.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() != format!("name = \"{of}\"") {
            continue;
        }
        // Inside this package's block, up to the next one.
        for line in lines.by_ref() {
            let line = line.trim();
            if line == "[[package]]" {
                return None;
            }
            if let Some(rest) = line.strip_prefix(&format!("\"{dep} "))
                && let Some(version) = rest.strip_suffix("\",").or_else(|| rest.strip_suffix('"'))
            {
                return Some(version.to_string());
            }
        }
        return None;
    }
    None
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

    /// esp-hal 1.x ships the same table as generated Rust. The entries are
    /// in the shape rustfmt leaves them — wrapped mid-entry — and the `all(`
    /// repetition at the end is not a twenty-second pin.
    #[test]
    fn the_generated_description_reads_like_the_toml_one() {
        let text = "\
macro_rules! for_each_gpio {
    ($($pattern:tt => $code:tt;)*) => {
        macro_rules! _for_each_inner_gpio { $(($pattern) => $code;)* ($other : tt) => {}
        } _for_each_inner_gpio!((0, GPIO0() () ([Input] [Output])));
        _for_each_inner_gpio!((12, GPIO12(_0 => SPIHD) (_0 => SPIHD) ([Input]
        [Output]))); _for_each_inner_gpio!((7, GPIO7(_2 => FSPID) (_0 => MTDO _2 =>
        FSPID) ([Input] [Output])));
        _for_each_inner_gpio!((34, GPIO34() () ([Input] [])));
        _for_each_inner_gpio!((all(0, GPIO0() () ([Input] [Output])), (12, GPIO12(_0 =>
        SPIHD) (_0 => SPIHD) ([Input] [Output]))));
    };
}
/// analog
macro_rules! for_each_analog_function {
    ($($pattern:tt => $code:tt;)*) => {
        macro_rules! _for_each_inner_analog_function { $(($pattern) => $code;)* ($other :
        tt) => {} } _for_each_inner_analog_function!((ADC1_CH0, GPIO0));
        _for_each_inner_analog_function!((TOUCH1, GPIO0));
        _for_each_inner_analog_function!((USB_DM, GPIO34));
    };
}
";
        let pins = generated_pins(text).expect("the generated shape is read");
        assert_eq!(
            pins.iter().map(|p| p.pin).collect::<Vec<_>>(),
            vec![0, 7, 12, 34],
            "every listed pin once, the all(…) repetition ignored",
        );
        let by_pin = |n: u32| pins.iter().find(|p| p.pin == n).unwrap();
        assert!(!by_pin(0).input_only);
        assert!(
            by_pin(34).input_only,
            "`([Input] [])` is a pin with no driver"
        );
        assert_eq!(
            by_pin(12).functions.get("0").map(String::as_str),
            Some("SPIHD")
        );
        assert!(by_pin(12).reserved().unwrap().contains("SPI flash"));
        // The output group's level 0 counts when the input group has none.
        assert_eq!(
            by_pin(7).functions.get("0").map(String::as_str),
            Some("MTDO")
        );
        assert_eq!(
            by_pin(7).functions.get("2").map(String::as_str),
            Some("FSPID")
        );
        assert_eq!(
            by_pin(0).analog.values().cloned().collect::<Vec<_>>(),
            vec!["ADC1_CH0", "TOUCH1"]
        );
        assert!(by_pin(34).reserved().unwrap().contains("native USB"));
        assert!(generated_pins("nothing here").is_none());
        // 0.1.0's spelling of the inner macro reads the same.
        let old = "macro_rules! for_each_gpio {
    ($($pattern:tt => $code:tt;)*) => {
        macro_rules! _for_each_inner { $(($pattern) => $code;)* ($other : tt) => {} }
        _for_each_inner!((0, GPIO0() () ([Input] [Output]))); _for_each_inner!((12,
        GPIO12(_0 => SPIHD) (_0 => SPIHD) ([Input] [Output])));
    };
}
";
        let pins = generated_pins(old).expect("the old spelling is read");
        assert_eq!(pins.iter().map(|p| p.pin).collect::<Vec<_>>(), vec![0, 12]);
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
        let reserved: Vec<Option<String>> = file
            .device
            .gpio
            .pins
            .iter()
            .map(PinEntry::reserved)
            .collect();

        assert!(
            reserved[0]
                .as_deref()
                .is_some_and(|r| r.contains("SPI flash"))
        );
        assert!(
            reserved[1]
                .as_deref()
                .is_some_and(|r| r.contains("SPI flash"))
        );
        assert!(
            reserved[2]
                .as_deref()
                .is_some_and(|r| r.contains("console"))
        );
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

        let found = claims(dir.path(), dir.path(), None);
        let pins: Vec<u32> = found.iter().map(|claim| claim.gpio).collect();
        assert_eq!(
            pins,
            vec![5, 21],
            "the comment and the string are not claims: {found:?}",
        );
        assert_eq!(
            found[0].line, 1,
            "zero-based, like every line that crosses the wire"
        );
        assert!(found[0].text.starts_with("let led ="));
    }

    /// The HALs name pins by port, and a pin map that read only esp-hal's
    /// `.GPIO<n>` had nothing to say about an STM32, an nRF or a Pico.
    #[test]
    fn each_hal_names_its_pins_its_own_way_and_each_is_read() {
        let st = Some(Ports {
            width: 16,
            numbered: false,
        });
        let nordic = Some(Ports {
            width: 32,
            numbered: true,
        });
        assert_eq!(pin_of("PC13", st), Some(45));
        assert_eq!(pin_of("PA0", st), Some(0));
        assert_eq!(pin_of("P0_13", nordic), Some(13));
        assert_eq!(pin_of("P1_05", nordic), Some(37));
        assert_eq!(pin_of("PIN_25", None), Some(25));
        assert_eq!(pin_of("GPIO5", None), Some(5));
        // Past a port's width is no pin, and a word that merely begins
        // with P is not one either.
        assert_eq!(pin_of("PA16", st), None);
        assert_eq!(pin_of("PERIPH", st), None);
        assert_eq!(pin_of("PC13", None), None);
    }

    /// An STM32 project's pin map: the catalogue's pins of its package, the
    /// claims sorted onto them, and a note that says what the catalogue does
    /// not know — never "could not find esp-hal's description".
    #[test]
    fn a_part_that_is_not_espressifs_is_mapped_from_the_catalogue() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "let led = Output::new(p.PC13, Level::High, Speed::Low);
             let button = Input::new(p.PA0, Pull::Up);
",
        )
        .unwrap();
        let report = report(dir.path(), dir.path(), "stm32f411ce");
        assert!(!report.pins.is_empty(), "{report:?}");
        let claimed: Vec<u32> = report
            .pins
            .iter()
            .filter(|pin| !pin.claims.is_empty())
            .map(|pin| pin.gpio)
            .collect();
        assert_eq!(claimed, [0, 45]);
        assert!(report.unknown.is_empty(), "{:?}", report.unknown);
        let note = report.note.unwrap_or_default();
        assert!(!note.contains("esp-hal"), "{note}");
        assert!(report.source.unwrap_or_default().contains("catalogue"));
    }

    /// A workspace whose firmware is one directory down still reports paths
    /// the editor can open.
    ///
    /// Both halves have to be right and each fails silently on its own: scan
    /// the root and there are no claims to show, report relative to the
    /// firmware crate and every claim opens `src/main.rs` from a root that has
    /// no `src/`. That second one is what shipped — clicking a pin raised
    /// "could not read src/main.rs (os error 3)".
    #[test]
    fn an_excluded_firmware_crate_reports_paths_from_the_project_root() {
        let dir = tempfile::tempdir().unwrap();
        let firmware = dir.path().join("firmware");
        std::fs::create_dir_all(firmware.join("src")).unwrap();
        std::fs::write(
            firmware.join("src/main.rs"),
            "let led = Output::new(peripherals.GPIO26, Level::High);\n",
        )
        .unwrap();

        let found = claims(dir.path(), &firmware, None);
        assert_eq!(
            found.len(),
            1,
            "the firmware's claim was not found: {found:?}"
        );
        assert_eq!(
            found[0].file, "firmware/src/main.rs",
            "the path has to resolve from the opened directory, not from the chip's",
        );
        assert!(
            dir.path().join(&found[0].file).is_file(),
            "the reported path must exist relative to the root the editor uses",
        );
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

        let report = report(dir.path(), dir.path(), "esp32");
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
        assert_eq!(
            locked_versions(lock, "esp-metadata"),
            vec!["0.8.0".to_string()]
        );
        assert!(locked_versions(lock, "nothing-here").is_empty());
    }

    /// A lock with two versions of the generated crate names, in esp-hal's
    /// own dependency list, which one esp-hal was built from. The first
    /// `[[package]]` in the file is the older one and was the wrong table.
    #[test]
    fn esp_hals_own_generated_version_is_read_off_its_dependency_list() {
        let lock = "\
[[package]]
name = \"esp-hal\"
version = \"1.1.2\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
dependencies = [
 \"bitflags\",
 \"esp-metadata-generated 0.4.0\",
 \"esp-riscv-rt\",
]

[[package]]
name = \"esp-metadata-generated\"
version = \"0.1.0\"

[[package]]
name = \"esp-metadata-generated\"
version = \"0.4.0\"

[[package]]
name = \"esp-rom-sys\"
version = \"0.1.1\"
dependencies = [
 \"esp-metadata-generated 0.1.0\",
]
";
        assert_eq!(
            locked_versions(lock, "esp-metadata-generated"),
            vec!["0.1.0".to_string(), "0.4.0".to_string()]
        );
        assert_eq!(
            dependency_version(lock, "esp-hal", "esp-metadata-generated").as_deref(),
            Some("0.4.0")
        );
        assert_eq!(
            dependency_version(lock, "esp-rom-sys", "esp-metadata-generated").as_deref(),
            Some("0.1.0")
        );
        // A unique dependency is written without a version: nothing to read
        // here, and `locked_versions` has the one.
        assert_eq!(dependency_version(lock, "esp-hal", "bitflags"), None);
        assert_eq!(
            dependency_version(lock, "not-locked", "esp-metadata-generated"),
            None
        );
    }
}
