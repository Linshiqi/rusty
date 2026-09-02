//! CMSIS-SVD, read for what a register view needs.
//!
//! Not a general SVD implementation: peripherals, their registers, and the
//! fields inside them. Clusters, dim arrays and `derivedFrom` inheritance
//! are what a code generator needs; a panel showing "what does GPIO_OUT say
//! right now" needs a name, an address and a bit range.
//!
//! Deliberately tolerant. Vendor SVDs are inconsistent — Espressif's carry
//! registers with no size and fields with no description — and a register
//! view that refuses the file it was given is worth less than one that
//! shows the ninety per cent it understood. What it cannot place, it drops;
//! what it drops, it counts, so the panel can say so. That rule has no
//! exceptions: an address that will not parse is not zero, it is a
//! peripheral the map does not have, and a file that ends early is a map
//! that is short and says where.

use quick_xml::events::Event;

use crate::{
    error::{Error, Result},
    model::{Peripheral, Register, RegisterField, RegisterMap},
};

/// Parse an SVD document.
///
/// Returns what was understood plus a count of what was not, rather than an
/// error: half a register map is useful, and "this file is broken" is not
/// an answer anybody can act on.
pub fn parse(xml: &str) -> RegisterMap {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut map = RegisterMap::default();
    // Where the walk is, as a stack of element names — SVD nests the same
    // tag names at different depths (`name` belongs to whatever contains
    // it), so position is the only way to know what a value describes.
    let mut path: Vec<String> = Vec::new();
    // The element being built at each level, and whether it is still worth
    // keeping. A number that would not parse poisons its element rather than
    // becoming a default: a peripheral at base 0 or a one-bit field that was
    // meant to be thirty-two is a panel that lies about the chip, and the
    // element is counted as dropped when it closes.
    let mut peripheral: Option<Peripheral> = None;
    let mut peripheral_ok = true;
    let mut register: Option<Register> = None;
    let mut register_ok = true;
    let mut field: Option<RegisterField> = None;
    let mut field_ok = true;
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let name = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                match name.as_str() {
                    "peripheral" => {
                        peripheral = Some(Peripheral::default());
                        peripheral_ok = true;
                    }
                    "register" => {
                        register = Some(Register::default());
                        register_ok = true;
                    }
                    "field" => {
                        field = Some(RegisterField::default());
                        field_ok = true;
                    }
                    _ => {}
                }
                path.push(name);
                text.clear();
            }
            Ok(Event::Text(bytes)) => {
                text = bytes
                    .decode()
                    .map(|t| t.trim().to_string())
                    .unwrap_or_default();
            }
            Ok(Event::End(element)) => {
                let name = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                let parent = path
                    .get(path.len().saturating_sub(2))
                    .map(String::as_str)
                    .unwrap_or_default();

                match (parent, name.as_str()) {
                    ("peripheral", "name") => {
                        if let Some(p) = &mut peripheral {
                            p.name = text.clone();
                        }
                    }
                    ("peripheral", "description") => {
                        if let Some(p) = &mut peripheral {
                            p.description = one_line(&text);
                        }
                    }
                    ("peripheral", "baseAddress") => {
                        if let Some(p) = &mut peripheral {
                            match number(&text) {
                                Some(base) => p.base = base,
                                None => peripheral_ok = false,
                            }
                        }
                    }
                    ("register", "name") => {
                        if let Some(r) = &mut register {
                            r.name = text.clone();
                        }
                    }
                    ("register", "description") => {
                        if let Some(r) = &mut register {
                            r.description = one_line(&text);
                        }
                    }
                    ("register", "addressOffset") => {
                        if let Some(r) = &mut register {
                            match number(&text) {
                                Some(offset) => r.offset = offset as u32,
                                None => register_ok = false,
                            }
                        }
                    }
                    ("register", "size") => {
                        if let Some(r) = &mut register {
                            match number(&text) {
                                Some(bits) => r.bits = bits as u32,
                                None => register_ok = false,
                            }
                        }
                    }
                    ("register", "access") => {
                        if let Some(r) = &mut register {
                            // Reading a write-only register can wedge a
                            // peripheral; the panel needs to know not to.
                            r.readable = !text.starts_with("write");
                        }
                    }
                    ("field", "name") => {
                        if let Some(f) = &mut field {
                            f.name = text.clone();
                        }
                    }
                    ("field", "description") => {
                        if let Some(f) = &mut field {
                            f.description = one_line(&text);
                        }
                    }
                    ("field", "bitOffset") | ("field", "lsb") => {
                        if let Some(f) = &mut field {
                            match number(&text) {
                                Some(offset) => f.offset = offset as u32,
                                None => field_ok = false,
                            }
                        }
                    }
                    ("field", "bitWidth") => {
                        if let Some(f) = &mut field {
                            match number(&text) {
                                Some(width) => f.width = width as u32,
                                None => field_ok = false,
                            }
                        }
                    }
                    ("field", "msb") => {
                        // The other spelling: `lsb`/`msb` instead of
                        // offset and width. Espressif's files use both.
                        if let Some(f) = &mut field {
                            match number(&text) {
                                Some(msb) => f.width = (msb as u32).saturating_sub(f.offset) + 1,
                                None => field_ok = false,
                            }
                        }
                    }
                    ("fields", "field") => {
                        if let (Some(r), Some(f)) = (&mut register, field.take()) {
                            if field_ok && !f.name.is_empty() {
                                r.fields.push(f);
                            } else {
                                map.dropped += 1;
                            }
                        }
                    }
                    ("registers", "register") => match (&mut peripheral, register.take()) {
                        (Some(p), Some(r)) if register_ok && !r.name.is_empty() => {
                            p.registers.push(r);
                        }
                        (_, Some(_)) => map.dropped += 1,
                        _ => {}
                    },
                    ("peripherals", "peripheral") => match peripheral.take() {
                        // A peripheral with no registers is `derivedFrom`
                        // another — inheritance this deliberately does not
                        // model. Counted, not guessed at.
                        Some(p)
                            if peripheral_ok && !p.name.is_empty() && !p.registers.is_empty() =>
                        {
                            map.peripherals.push(p);
                        }
                        Some(_) => map.dropped += 1,
                        None => {}
                    },
                    _ => {}
                }
                path.pop();
                text.clear();
            }
            Ok(Event::Eof) => {
                // A document that ends inside a peripheral was cut off: the
                // element in progress is the remainder, and it is counted and
                // named rather than quietly not there.
                if peripheral.take().is_some() {
                    map.dropped += 1;
                    map.note = Some(format!(
                        "the file ends inside a peripheral at byte {}; whatever followed is \
                         missing from this map",
                        reader.buffer_position(),
                    ));
                }
                break;
            }
            Err(error) => {
                if peripheral.take().is_some() {
                    map.dropped += 1;
                }
                map.note = Some(format!(
                    "the file could not be read past byte {}: {error}. Whatever followed is \
                     missing from this map.",
                    reader.buffer_position(),
                ));
                break;
            }
            _ => {}
        }
    }

    map.peripherals.sort_by(|a, b| a.name.cmp(&b.name));
    map
}

/// The SVD for a chip, if this machine has one.
///
/// Three layers, like everything else the catalogue reads: the project's
/// own `.rusty/svd/` wins, then the data directory, then nothing. A vendor
/// file is a hundred thousand lines of XML nobody wants in a git repository
/// by accident, so it is fetched rather than bundled.
pub fn find(chip: &str, root: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    let file = format!("{chip}.svd");
    if let Some(root) = root {
        let local = root.join(".rusty").join("svd").join(&file);
        if local.is_file() {
            return Some(local);
        }
    }
    let shared = crate::config::data_dir()?.join("svd").join(&file);
    shared.is_file().then_some(shared)
}

/// Where a chip's SVD comes from, and where it goes.
///
/// Espressif publish theirs in one repository; anything else has to be
/// dropped into `.rusty/svd/` by hand, and saying so beats a download that
/// 404s into a confusing error.
pub fn source(chip: &str) -> Option<(String, std::path::PathBuf)> {
    let known = [
        "esp32", "esp32c2", "esp32c3", "esp32c6", "esp32h2", "esp32s2", "esp32s3",
    ];
    if !known.contains(&chip) {
        return None;
    }
    let url = format!("https://raw.githubusercontent.com/espressif/svd/main/svd/{chip}.svd",);
    let dest = crate::config::data_dir()?
        .join("svd")
        .join(format!("{chip}.svd"));
    Some((url, dest))
}

/// Fetch a chip's SVD into the data directory, over the same proxy ladder
/// every other download uses.
pub fn fetch(chip: &str, progress: impl FnMut(String)) -> Result<std::path::PathBuf> {
    let (url, dest) = source(chip).ok_or_else(|| {
        Error::refused(format!(
            "rusty has no SVD source for {chip}. Put one at \
             <project>/.rusty/svd/{chip}.svd and it will be used.",
        ))
    })?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    crate::install::download(&[url], &dest, progress)?;
    Ok(dest)
}

/// SVD numbers come as decimal, `0x…`, or `#0101` binary.
fn number(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok();
    }
    if let Some(binary) = text.strip_prefix('#') {
        return u64::from_str_radix(binary, 2).ok();
    }
    text.parse().ok()
}

/// SVD descriptions are wrapped prose with the source file's newlines in
/// them; a table cell wants one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real shape, trimmed: two peripherals, one of them derived (no
    /// registers of its own), a register with `lsb`/`msb` fields and one
    /// with `bitOffset`/`bitWidth`, and a write-only register.
    const SAMPLE: &str = r#"
<device>
  <name>ESP32</name>
  <peripherals>
    <peripheral>
      <name>GPIO</name>
      <description>General Purpose
        Input/Output</description>
      <baseAddress>0x3FF44000</baseAddress>
      <registers>
        <register>
          <name>OUT</name>
          <description>Output value</description>
          <addressOffset>0x04</addressOffset>
          <size>32</size>
          <fields>
            <field>
              <name>DATA</name>
              <lsb>0</lsb>
              <msb>31</msb>
            </field>
          </fields>
        </register>
        <register>
          <name>OUT_W1TS</name>
          <addressOffset>0x08</addressOffset>
          <access>write-only</access>
          <fields>
            <field>
              <name>SET</name>
              <bitOffset>0</bitOffset>
              <bitWidth>32</bitWidth>
            </field>
          </fields>
        </register>
      </registers>
    </peripheral>
    <peripheral derivedFrom="GPIO">
      <name>GPIO2</name>
      <baseAddress>0x3FF45000</baseAddress>
    </peripheral>
  </peripherals>
</device>
"#;

    #[test]
    fn a_peripheral_carries_its_registers_and_their_fields() {
        let map = parse(SAMPLE);
        assert_eq!(map.peripherals.len(), 1, "the derived one is not invented");
        assert_eq!(
            map.dropped, 1,
            "and it is counted rather than silently gone"
        );
        assert!(map.note.is_none(), "a whole file has nothing to add");

        let gpio = &map.peripherals[0];
        assert_eq!(gpio.name, "GPIO");
        assert_eq!(gpio.base, 0x3FF4_4000);
        assert_eq!(
            gpio.description, "General Purpose Input/Output",
            "a wrapped description becomes one line",
        );

        let out = &gpio.registers[0];
        assert_eq!(out.offset, 4);
        assert_eq!(out.bits, 32);
        assert!(out.readable);
        assert_eq!(out.fields[0].offset, 0);
        assert_eq!(
            out.fields[0].width, 32,
            "lsb 0 / msb 31 is a 32-bit field, not a 31-bit one",
        );
    }

    #[test]
    fn a_write_only_register_says_so() {
        let map = parse(SAMPLE);
        let w1ts = &map.peripherals[0].registers[1];
        assert_eq!(w1ts.name, "OUT_W1TS");
        assert!(
            !w1ts.readable,
            "reading a write-only register can wedge the peripheral it belongs to",
        );
        assert_eq!(
            w1ts.fields[0].width, 32,
            "bitOffset/bitWidth is the other spelling"
        );
    }

    /// Half a file is half a map, not an error — and not a whole map either.
    /// A register view that showed the peripherals it reached with nothing
    /// saying more were expected would send somebody looking for a GPIO the
    /// parse never saw.
    #[test]
    fn a_truncated_file_yields_what_it_had_and_says_it_is_short() {
        let cut = &SAMPLE[..SAMPLE.len() / 2];
        let map = parse(cut);
        assert!(map.peripherals.is_empty() || map.peripherals[0].name == "GPIO");
        assert!(
            map.dropped >= 1,
            "the peripheral the cut fell inside is counted as dropped"
        );
        let note = map.note.expect("a short file says so");
        assert!(note.contains("byte"), "and says where: {note}");
    }

    /// The module's rule with no exceptions. A base address that does not
    /// parse used to become 0, an offset 0, a width 1 — each a confident
    /// wrong number in a panel that exists to show the right one.
    #[test]
    fn a_number_that_will_not_parse_drops_its_element_and_counts_it() {
        let broken = r#"
<device><peripherals>
  <peripheral>
    <name>BAD_BASE</name>
    <baseAddress>not-a-number</baseAddress>
    <registers><register><name>R</name><addressOffset>0</addressOffset></register></registers>
  </peripheral>
  <peripheral>
    <name>GOOD</name>
    <baseAddress>0x1000</baseAddress>
    <registers>
      <register><name>BAD_OFFSET</name><addressOffset>0xZZ</addressOffset></register>
      <register><name>BAD_SIZE</name><addressOffset>4</addressOffset><size>wide</size></register>
      <register>
        <name>OK</name>
        <addressOffset>8</addressOffset>
        <fields>
          <field><name>BAD_WIDTH</name><bitOffset>0</bitOffset><bitWidth>lots</bitWidth></field>
          <field><name>BAD_MSB</name><lsb>0</lsb><msb>?</msb></field>
          <field><name>FINE</name><bitOffset>4</bitOffset><bitWidth>2</bitWidth></field>
        </fields>
      </register>
    </registers>
  </peripheral>
</peripherals></device>
"#;
        let map = parse(broken);
        let names: Vec<&str> = map.peripherals.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["GOOD"],
            "a peripheral with no address is not at 0"
        );

        let good = &map.peripherals[0];
        let registers: Vec<&str> = good.registers.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            registers,
            vec!["OK"],
            "a register with no offset or no width is not at 0 or 32 bits"
        );
        let fields: Vec<&str> = good.registers[0]
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            fields,
            vec!["FINE"],
            "a field with no width is not one bit wide"
        );

        // One peripheral, two registers, two fields: five things the map does
        // not have, and it says five.
        assert_eq!(map.dropped, 5);
        assert!(map.note.is_none(), "nothing wrong with the file's shape");
    }

    #[test]
    fn numbers_come_in_three_spellings() {
        assert_eq!(number("0x3FF44000"), Some(0x3FF4_4000));
        assert_eq!(number("32"), Some(32));
        assert_eq!(number("#0101"), Some(5));
        assert_eq!(number("nonsense"), None);
    }
}
