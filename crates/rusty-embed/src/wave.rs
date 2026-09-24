//! Tables: a rendered signal, played by the emulator against the firmware's
//! own clock (`docs/signals.md`, "The emulator").
//!
//! The lines that put one on a pin or on a sensor's register block, and the
//! emulator's account of playing it. Compiled unconditionally, like the
//! rest of the protocol: the frontend reads `[rusty:wave@…]` as it passes,
//! to line the signal it drew up with the conversions the emulator reports.

use crate::protocol::hex_string;

/// Samples per `W<pin>@…` line. Four hex digits each, and the device takes
/// a line of at most 512 bytes; a longer one is dropped whole.
const PIN_CHUNK: usize = 96;
/// Bytes per `i2c <addr>~@…` line, at two hex digits each.
const BLOCK_CHUNK: usize = 224;

/// The converter's full scale, which a table's counts are clamped to — the
/// device clamps them too, and a table that disagreed with it would be a
/// signal the firmware never saw.
const FULL_SCALE: u16 = 0xfff;

/// Everything that puts a pin's table on the emulator and plays it: begin,
/// fill, on. `counts` is one period of what the pin reads, `rate` samples a
/// second; the device loops it. A pin already playing keeps its phase.
pub fn pin_table_lines(pin: u32, rate: u32, counts: &[u16]) -> Vec<String> {
    let mut lines = vec![format!("W{pin}={rate},{}\n", counts.len())];
    for (chunk, samples) in counts.chunks(PIN_CHUNK).enumerate() {
        let hex: String = samples
            .iter()
            .map(|counts| format!("{:04x}", (*counts).min(FULL_SCALE)))
            .collect();
        lines.push(format!("W{pin}@{}={hex}\n", chunk * PIN_CHUNK));
    }
    lines.push(format!("W{pin}=on\n"));
    lines
}

/// Stop a pin's table: it reads its `A<pin>=` value again.
pub fn pin_stop_line(pin: u32) -> String {
    format!("W{pin}=off\n")
}

/// Everything that puts a device's register-block table on the emulator
/// and plays it. `bytes` is the samples one after another, `width` bytes
/// each from register `reg` — what a burst read of the block returns at
/// that sample.
pub fn block_table_lines(
    address: u8,
    reg: u8,
    width: usize,
    rate: u32,
    bytes: &[u8],
) -> Vec<String> {
    let samples = bytes.len() / width.max(1);
    let mut lines = vec![format!(
        "i2c {address:02x}~{reg:02x}:{width}={rate},{samples}\n"
    )];
    for (chunk, run) in bytes.chunks(BLOCK_CHUNK).enumerate() {
        lines.push(format!(
            "i2c {address:02x}~@{}={}\n",
            chunk * BLOCK_CHUNK,
            hex_string(run)
        ));
    }
    lines.push(format!("i2c {address:02x}~=on\n"));
    lines
}

/// Stop a device's table: its registers hold what they were last given.
pub fn block_stop_line(address: u8) -> String {
    format!("i2c {address:02x}~=off\n")
}

/// A rendered table and what it plays on.
#[derive(Debug, Clone, PartialEq)]
pub enum Table {
    /// A pin's counts: one loop of what a conversion reads, `rate` a second.
    Pin {
        gpio: u8,
        rate: u32,
        counts: Vec<u16>,
    },
    /// A device's register block, a sample at a time.
    Block {
        address: u8,
        rate: u32,
        block: crate::sensor::Block,
    },
}

impl Table {
    /// Everything that puts it on the emulator and plays it.
    pub fn lines(&self) -> Vec<String> {
        match self {
            Table::Pin { gpio, rate, counts } => pin_table_lines(u32::from(*gpio), *rate, counts),
            Table::Block {
                address,
                rate,
                block,
            } => block_table_lines(*address, block.reg, block.width, *rate, &block.bytes),
        }
    }

    pub fn target(&self) -> WaveTarget {
        match self {
            Table::Pin { gpio, .. } => WaveTarget::Pin(*gpio),
            Table::Block { address, .. } => WaveTarget::Device(*address),
        }
    }
}

/// Stop whatever plays on `target`.
pub fn stop_line(target: WaveTarget) -> String {
    match target {
        WaveTarget::Pin(gpio) => pin_stop_line(u32::from(gpio)),
        WaveTarget::Device(address) => block_stop_line(address),
    }
}

/// What a table is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WaveTarget {
    Pin(u8),
    Device(u8),
}

/// What happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaveEvent {
    /// Playing, with sample 0 at `start_us` on the emulator's virtual clock.
    On {
        start_us: u64,
    },
    Off,
    /// Not taken — a table past the device's bounds, an `on` with nothing
    /// filled, a device not on the bus — in the device's own words.
    Refused(String),
}

/// One `[rusty:wave@…]` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveReport {
    pub at_us: u64,
    pub target: WaveTarget,
    pub event: WaveEvent,
}

/// Parse `[rusty:wave@1484] 3 on 1484`, `[rusty:wave@2357] i2c 68 on 2356`,
/// `… 3 off`, and a refusal (`… 3 refused 0,0`, `… i2c 68 ?absent`).
pub fn parse_wave_report(line: &str) -> Option<WaveReport> {
    let rest = line.trim().strip_prefix("[rusty:wave@")?;
    let (at, rest) = rest.split_once(']')?;
    let at_us = at.trim().parse().ok()?;
    let mut words = rest.split_whitespace();
    let target = match words.next()? {
        "i2c" => WaveTarget::Device(u8::from_str_radix(words.next()?, 16).ok()?),
        pin => WaveTarget::Pin(pin.parse().ok()?),
    };
    let verb = words.next()?;
    let event = match verb {
        "on" => WaveEvent::On {
            start_us: words.next()?.parse().ok()?,
        },
        "off" => WaveEvent::Off,
        _ => {
            let said: Vec<&str> = std::iter::once(verb).chain(words).collect();
            WaveEvent::Refused(said.join(" "))
        }
    };
    Some(WaveReport {
        at_us,
        target,
        event,
    })
}

/// What a conversion at `at_us` reads from a pin's table started at
/// `start_us`: the device's own arithmetic — integer, interpolated between
/// the samples either side — so the host can say what the firmware saw at
/// an instant, and a test can hold the device to it.
pub fn counts_at(counts: &[u16], rate: u32, start_us: u64, at_us: u64) -> u16 {
    if counts.is_empty() {
        return 0;
    }
    let position = u128::from(at_us.saturating_sub(start_us)) * u128::from(rate);
    let len = counts.len() as u128;
    let index = ((position / 1_000_000) % len) as usize;
    let millionths = position % 1_000_000;
    let a = u128::from(counts[index].min(FULL_SCALE));
    let b = u128::from(counts[(index + 1) % counts.len()].min(FULL_SCALE));
    ((a * (1_000_000 - millionths) + b * millionths + 500_000) / 1_000_000) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table crosses as begin, chunks and on, every chunk short enough
    /// for the device's line and the samples in order: the device drops an
    /// over-long line whole, and a chunk lost that way would be a table
    /// with a silent run of zeros in it.
    #[test]
    fn a_pin_table_crosses_in_lines_the_device_takes() {
        let counts: Vec<u16> = (0..250).map(|n| n * 16).collect();
        let lines = pin_table_lines(3, 20_000, &counts);
        assert_eq!(lines.first().map(String::as_str), Some("W3=20000,250\n"));
        assert_eq!(lines.last().map(String::as_str), Some("W3=on\n"));
        assert!(lines.iter().all(|line| line.len() <= 512), "{lines:?}");
        let chunks: Vec<&String> = lines[1..lines.len() - 1].iter().collect();
        assert_eq!(chunks.len(), 3, "96, 96 and 58 samples");
        assert!(chunks[1].starts_with("W3@96="));
        // The samples come back out in order, clamped to the converter's
        // twelve bits.
        let back: Vec<u16> = chunks
            .iter()
            .flat_map(|line| {
                let hex = line.trim().split_once('=').unwrap().1.to_string();
                (0..hex.len() / 4)
                    .map(move |i| u16::from_str_radix(&hex[i * 4..i * 4 + 4], 16).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        let clamped: Vec<u16> = counts.iter().map(|c| (*c).min(0xfff)).collect();
        assert_eq!(back, clamped);
        assert_eq!(pin_stop_line(3), "W3=off\n");
    }

    #[test]
    fn a_block_table_names_its_register_width_and_samples() {
        let bytes: Vec<u8> = (0..14 * 40).map(|n| n as u8).collect();
        let lines = block_table_lines(0x68, 0x3b, 14, 1000, &bytes);
        assert_eq!(lines[0], "i2c 68~3b:14=1000,40\n");
        assert_eq!(lines.last().map(String::as_str), Some("i2c 68~=on\n"));
        assert!(lines[1].starts_with("i2c 68~@0="));
        assert!(lines[2].starts_with("i2c 68~@224="));
        assert!(lines.iter().all(|line| line.len() <= 512));
        assert_eq!(block_stop_line(0x68), "i2c 68~=off\n");
    }

    /// The emulator's account, every form it takes — and a refusal kept in
    /// its own words rather than dropped as unparseable.
    #[test]
    fn the_emulators_account_of_a_table_is_read() {
        assert_eq!(
            parse_wave_report("[rusty:wave@1484] 3 on 1484"),
            Some(WaveReport {
                at_us: 1484,
                target: WaveTarget::Pin(3),
                event: WaveEvent::On { start_us: 1484 },
            })
        );
        assert_eq!(
            parse_wave_report("[rusty:wave@2357] i2c 68 on 2356").map(|r| (r.target, r.event)),
            Some((WaveTarget::Device(0x68), WaveEvent::On { start_us: 2356 }))
        );
        assert_eq!(
            parse_wave_report("[rusty:wave@9] 34 off").map(|r| r.event),
            Some(WaveEvent::Off)
        );
        assert_eq!(
            parse_wave_report("[rusty:wave@9] 3 refused 0,0").map(|r| r.event),
            Some(WaveEvent::Refused("refused 0,0".to_string()))
        );
        assert_eq!(
            parse_wave_report("[rusty:wave@9] i2c 50 ?absent").map(|r| r.event),
            Some(WaveEvent::Refused("?absent".to_string()))
        );
        assert_eq!(parse_wave_report("[rusty:adc@9] 3=100"), None);
    }

    /// The device's arithmetic, checked where it can be checked by hand:
    /// on a sample, halfway between two, and after the table has looped.
    #[test]
    fn a_conversion_reads_the_table_where_the_clock_is() {
        let counts = [0u16, 1000, 2000, 3000];
        // 1000 samples a second: a sample every millisecond.
        assert_eq!(
            counts_at(&counts, 1000, 500, 500),
            0,
            "sample 0 at the start"
        );
        assert_eq!(counts_at(&counts, 1000, 500, 1500), 1000);
        assert_eq!(counts_at(&counts, 1000, 500, 2000), 1500, "halfway between");
        assert_eq!(
            counts_at(&counts, 1000, 500, 4000),
            1500,
            "between the last and the first: 3000 and 0"
        );
        assert_eq!(
            counts_at(&counts, 1000, 500, 5500),
            1000,
            "a second time round"
        );
        assert_eq!(
            counts_at(&counts, 1000, 500, 100),
            0,
            "before it started, sample 0"
        );
    }
}
